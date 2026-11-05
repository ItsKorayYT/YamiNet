use ark_relations::r1cs::{ConstraintSynthesizer, ConstraintSystem};
use ark_serialize::CanonicalDeserialize;
use std::time::Instant;
use yamichain_ypp_accounts::*;
fn main() {
    let n = std::env::args()
        .nth(1)
        .unwrap_or("16".into())
        .parse::<usize>()
        .unwrap();
    assert!([16, 64, 128].contains(&n));
    let (mut ledger, keys) = fixture(n);
    let output = std::env::temp_dir().join(format!("ypp-accounts-{n}"));
    std::fs::create_dir_all(&output).unwrap();
    std::fs::write(output.join("before.bin"), bytes(&ledger)).unwrap();
    let relation = ledger.relation(0, 1, keys[0], 1000, 7);
    let cs = ConstraintSystem::new_ref();
    relation.clone().generate_constraints(cs.clone()).unwrap();
    assert!(cs.is_satisfied().unwrap());
    let constraints = cs.num_constraints();
    drop(cs);
    let start = Instant::now();
    let pk = setup(relation.clone());
    let setup_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let p = prove(&pk, relation.clone());
    let prove_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    assert!(verify(&pk.vk, &relation.statement, &p));
    let verify_ms = start.elapsed().as_secs_f64() * 1000.;
    ledger.apply(&pk.vk, &relation.statement, &p).unwrap();
    let state = bytes(&ledger);
    std::fs::write(output.join("after.bin"), &state).unwrap();
    std::fs::write(output.join("statement.bin"), bytes(&relation.statement)).unwrap();
    std::fs::write(output.join("proof.bin"), bytes(&p)).unwrap();
    std::fs::write(output.join("verifying-key.bin"), bytes(&pk.vk)).unwrap();
    let root = ledger.root();
    let start = Instant::now();
    let restored_bytes = std::fs::read(output.join("after.bin")).unwrap();
    let restored = Ledger::deserialize_compressed(restored_bytes.as_slice()).unwrap();
    assert_eq!(restored.recover(root, 1, keys[1], u32::MAX).unwrap(), 1007);
    let recovery_ms = start.elapsed().as_secs_f64() * 1000.;
    let rss = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .unwrap()
        .to_owned();
    println!(
        "{}",
        serde_json::json!({"accounts":n,"constraints":constraints,"setup_ms":setup_ms,"prove_ms":prove_ms,"verify_ms":verify_ms,"proof_bytes":bytes(&p).len(),"transaction_bytes":bytes(&relation.statement).len()+bytes(&p).len(),"state_bytes":state.len(),"recovery_bound":u32::MAX,"recovery_ms":recovery_ms,"memory_high_water":rss,"samples":1})
    );
}
