//! Trusted custodian-side CLI. Verifies fresh enrollment with native Intel QVL before provisioning.
use ed25519_dalek::SigningKey;
use rand::{rngs::OsRng, RngCore};
use std::{io::Read, path::PathBuf};
use x25519_dalek::{PublicKey, StaticSecret};
use yamichain_tee_m20::{attestation::*, tdx_guest::*, *};
fn main() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin()
        .take(32 * 1024 * 1024)
        .read_to_string(&mut raw)?;
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let out = match v["op"].as_str().ok_or("op")? {
        "key" => {
            let signing = SigningKey::generate(&mut OsRng);
            let recovery = StaticSecret::random_from_rng(OsRng);
            serde_json::json!({"signing":signing.to_bytes(),"operator":signing.verifying_key().to_bytes(),"recovery_secret":recovery.to_bytes(),"recovery":PublicKey::from(&recovery).to_bytes()})
        }
        "challenge" => {
            let mut c = [0; 32];
            OsRng.fill_bytes(&mut c);
            serde_json::to_value(c)?
        }
        "activate" => {
            let id: Identity = serde_json::from_value(v["identity"].clone())?;
            let key =
                SigningKey::from_bytes(&serde_json::from_value(v["operator_secret"].clone())?);
            if key.verifying_key().to_bytes() != id.operator {
                return Err("wrong operator".into());
            }
            serde_json::to_value(backup_ack(
                &id,
                &serde_json::from_value(v["backup"].clone())?,
                &key,
            ))?
        }
        "collateral-chain" => {
            let qvl = IntelQvl::load(
                &PathBuf::from(v["qvl_library"].as_str().ok_or("QVL path")?),
                serde_json::from_value(v["qvl_sha256"].clone())?,
            )?;
            serde_json::json!({"pck_chain":qvl.unverified_pck_chain(&serde_json::from_value::<Vec<u8>>(v["quote"].clone())?)?})
        }
        "configure" => {
            let qvl = IntelQvl::load(
                &PathBuf::from(v["qvl_library"].as_str().ok_or("QVL path")?),
                serde_json::from_value(v["qvl_sha256"].clone())?,
            )?;
            let policy: SoftwarePolicy = serde_json::from_value(v["software_policy"].clone())?;
            let identity: Identity = serde_json::from_value(v["identity"].clone())?;
            verify_enrollment(
                &qvl,
                &policy,
                &identity,
                serde_json::from_value(v["challenge"].clone())?,
                &serde_json::from_value(v["evidence"].clone())?,
            )?;
            let owner =
                SigningKey::from_bytes(&serde_json::from_value(v["operator_secret"].clone())?);
            let mut body: Provision = serde_json::from_value(v["provision"].clone())?;
            if !v["backup"].is_null() {
                let seeds = decrypt_seeds(
                    serde_json::from_value(v["recovery_secret"].clone())?,
                    &serde_json::from_value(v["backup"].clone())?,
                )?;
                body.root = seeds.root;
                body.restored = Some(seeds);
            }
            serde_json::to_value(provision(
                &identity,
                serde_json::from_value(v["policy"].clone())?,
                v["authority"].as_str().ok_or("authority")?.parse()?,
                &body,
                &owner,
            )?)?
        }
        _ => return Err("unknown operation".into()),
    };
    println!("{out}");
    Ok(())
}
