//! Wallet-local stdin/stdout adapter. Passwords and keys are never command-line arguments or RPC payloads.
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::io::Read;
use yamichain_tee_m20::{vault::Vault, *};
fn main() -> Result<()> {
    let mut raw = String::new();
    std::io::stdin().take(16385).read_to_string(&mut raw)?;
    if raw.len() > 16384 {
        return Err("wallet frame limit".into());
    }
    let v: serde_json::Value = serde_json::from_str(&raw)?;
    let password = v["password"]
        .as_str()
        .ok_or("local wallet password required")?;
    let out = match v["op"].as_str().ok_or("operation")? {
        "create" => serde_json::to_value(Vault::seal(
            SigningKey::generate(&mut OsRng).to_bytes(),
            password,
        )?)?,
        "unlock" => {
            let sealed: Vault = serde_json::from_value(v["vault"].clone())?;
            serde_json::json!({"secret":sealed.unlock(password)?,"address":sealed.address})
        }
        "import" => serde_json::to_value(Vault::seal(
            serde_json::from_value(v["secret"].clone())?,
            password,
        )?)?,
        _ => return Err("unsupported wallet operation".into()),
    };
    println!("{out}");
    Ok(())
}
