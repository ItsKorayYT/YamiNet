//! Local synthetic demo and actual measurements. Never broadcasts anything.
use std::{path::Path, time::Instant};
use yamichain_ypp_vaults::*;
fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0)
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("verify") {
        let dir = Path::new(args.get(2).ok_or("verify requires fixture directory")?);
        let start = Instant::now();
        let v = Verifier::decode(&std::fs::read(dir.join("verifier.bin"))?)?;
        let tx = Transaction::decode(&std::fs::read(dir.join("transaction.bin"))?)?;
        let load_ms = ms(start);
        let mut times = Vec::new();
        for _ in 0..20 {
            let now = Instant::now();
            v.verify(&tx)?;
            times.push(ms(now));
        }
        println!("{{\"phase\":\"verifier_only\",\"load_ms\":{load_ms},\"verification_ms\":{times:?},\"peak_rss_kib\":{}}}",rss_kib());
        return Ok(());
    }
    let rounds: usize = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(1);
    if rounds == 0 || rounds > 16 {
        return Err("rounds must be 1..16".into());
    }
    let constraints = constraint_count()?;
    eprintln!("Isolated synthetic YPP demo; stable slots and credits are publicly linkable. {constraints} constraints.");
    let start = Instant::now();
    let (p, v) = setup()?;
    let setup_ms = ms(start);
    eprintln!("Ephemeral local Groth16 setup complete ({setup_ms:.0} ms). No reviewed ceremony.");
    let alice = Wallet::new();
    let bob = Wallet::new();
    let backup_identity = age::x25519::Identity::generate();
    let backup = bob.backup(&backup_identity.to_public())?;
    let (genesis, [mut a, mut b]) = synthetic_genesis(&alice, &bob, [1_000, 0])?;
    let mut ledger = Ledger::from_genesis(&genesis);
    let view = bob.viewing_authority();
    let mut scanner = Scanner::new(&view, &genesis, genesis.checkpoint())?;
    let mut proving = Vec::new();
    let mut verification = Vec::new();
    let mut scanning = Vec::new();
    let mut bytes = 0;
    let mut proof_sizes = Vec::new();
    let mut first = None;
    for _ in 0..rounds {
        let now = Instant::now();
        let (tx, new) = p.debit(&alice, &ledger, 0, &a, &bob.address(), 7)?;
        proving.push(ms(now));
        a = new;
        let now = Instant::now();
        v.verify(&tx)?;
        verification.push(ms(now));
        bytes += tx.encode().len();
        proof_sizes.push(tx.proof.len());
        if first.is_none() {
            first = Some(tx.clone());
        }
        ledger.apply(&v, tx.clone())?;
        assert_eq!(ledger.apply(&v, tx), Err(Error::StaleRoot));
        let now = Instant::now();
        scanner.advance(&view, &v, &ledger.records()[ledger.records().len() - 1])?;
        scanning.push(ms(now));
        let now = Instant::now();
        let (tx, new) = p.absorb(&bob, &ledger, 1, &b, &scanner.incoming()[0])?;
        proving.push(ms(now));
        b = new;
        let now = Instant::now();
        v.verify(&tx)?;
        verification.push(ms(now));
        bytes += tx.encode().len();
        proof_sizes.push(tx.proof.len());
        ledger.apply(&v, tx)?;
        let now = Instant::now();
        scanner.advance(&view, &v, &ledger.records()[ledger.records().len() - 1])?;
        scanning.push(ms(now));
    }
    scanner.require_head(ledger.head())?;
    let now = Instant::now();
    let restored = Wallet::restore(&backup_identity, &backup)?;
    let restore_ms = ms(now);
    let restored_view = restored.viewing_authority();
    let now = Instant::now();
    let mut recovery = Scanner::new(&restored_view, &genesis, genesis.checkpoint())?;
    for record in ledger.records() {
        recovery.advance(&restored_view, &v, record)?;
    }
    recovery.require_head(ledger.head())?;
    let recovery_ms = ms(now);
    assert_eq!(
        recovery.states[1]
            .as_ref()
            .ok_or("missing Bob state")?
            .balance(),
        7 * rounds as u64
    );
    assert_eq!(a.balance() + b.balance(), 1_000);
    if let Some(dir) = args.get(3) {
        std::fs::create_dir_all(dir)?;
        let dir = Path::new(dir);
        std::fs::write(dir.join("verifier.bin"), v.encode()?)?;
        std::fs::write(
            dir.join("transaction.bin"),
            first.ok_or("no transaction")?.encode(),
        )?;
    }
    eprintln!("Alice -> Bob: {rounds} confidential inbox payments absorbed; backup restored; authenticated history replayed; duplicate rejected. Synthetic total conserved.");
    println!("{{\"phase\":\"prover_and_recovery\",\"rounds\":{rounds},\"constraints\":{constraints},\"setup_ms\":{setup_ms},\"proving_ms\":{proving:?},\"verification_ms\":{verification:?},\"proof_bytes\":{proof_sizes:?},\"scan_record_ms\":{scanning:?},\"backup_restore_ms\":{restore_ms},\"recovery_ms\":{recovery_ms},\"history_transaction_bytes\":{bytes},\"record_overhead_bytes\":{},\"genesis_ciphertext_bytes\":{},\"live_root_bytes\":32,\"live_leaf_bytes\":64,\"retained_credit_marker_bytes\":{},\"peak_rss_kib\":{}}}",ledger.records().len()*64,genesis.ciphertexts.iter().map(Vec::len).sum::<usize>(),ledger.marker_count()*32,rss_kib());
    Ok(())
}
