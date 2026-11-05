use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::{collections::BTreeMap, time::Instant};
use x25519_dalek::StaticSecret;
use yamichain_tee_m20::*;
fn main() -> Result<()> {
    let dir = std::env::args()
        .nth(1)
        .ok_or("usage: measure EMPTY_DIRECTORY")?;
    std::fs::create_dir_all(&dir)?;
    let path = std::path::Path::new(&dir).join("bench.db");
    let a = SigningKey::generate(&mut OsRng);
    let b = SigningKey::generate(&mut OsRng);
    let signer = SigningKey::generate(&mut OsRng);
    let transport = StaticSecret::random_from_rng(OsRng);
    let storage = [7; 32];
    let count: usize = std::env::args().nth(2).unwrap_or("2".into()).parse()?;
    let mut accounts = BTreeMap::from([(
        a.verifying_key().to_bytes(),
        Account {
            balance: 10000,
            nonce: 0,
        },
    )]);
    for i in 2..count {
        accounts.insert(hash(&i.to_le_bytes()), Account::default());
    }
    let mut e = Engine::create(
        &path,
        State {
            chain: 20,
            epoch: 0,
            accounts,
        },
        storage,
        signer.clone(),
        transport.clone(),
    )?;
    let start = Instant::now();
    for nonce in 0..1000 {
        let t = Signed::sign(
            Transfer {
                sender: a.verifying_key().to_bytes(),
                recipient: b.verifying_key().to_bytes(),
                amount: 1,
                nonce,
                chain: 20,
            },
            &a,
        );
        let p = encrypt(e.public(), &bytes(&t), b"YAMI-M20-payment-v1")?;
        let old = e.head();
        let r = e.execute(&p)?;
        r.verify(e.receipt_key(), &old)?;
    }
    let elapsed = start.elapsed();
    let head = e.head();
    drop(e);
    let start = Instant::now();
    let _e = Engine::load(&path, storage, signer, transport, &head)?;
    let recovery = start.elapsed();
    println!(
        "{}",
        serde_json::json!({"mode":"SOFTWARE ONLY","accounts":count,"transfers":1000,"wallet_encrypt_execute_receipt_ms":elapsed.as_secs_f64()*1000.0,"tps":1000.0/elapsed.as_secs_f64(),"snapshot_bytes":std::fs::metadata(path)?.len(),"recovery_ms":recovery.as_secs_f64()*1000.0})
    );
    Ok(())
}
