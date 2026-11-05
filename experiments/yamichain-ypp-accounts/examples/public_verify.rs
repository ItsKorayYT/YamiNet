//! Separate-process replay using public artifacts only. Does not authenticate finality.
use ark_bls12_381::Bls12_381;
use ark_groth16::{Proof, VerifyingKey};
use ark_serialize::CanonicalDeserialize;
use yamichain_ypp_accounts::{bytes, verify, Ledger, Statement};
fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("artifact directory"));
    let read = |name: &str| std::fs::read(dir.join(name)).unwrap();
    let vk =
        VerifyingKey::<Bls12_381>::deserialize_compressed(read("verifying-key.bin").as_slice())
            .unwrap();
    let proof = Proof::<Bls12_381>::deserialize_compressed(read("proof.bin").as_slice()).unwrap();
    let statement = Statement::deserialize_compressed(read("statement.bin").as_slice()).unwrap();
    let mut ledger = Ledger::deserialize_compressed(read("before.bin").as_slice()).unwrap();
    assert!(verify(&vk, &statement, &proof));
    ledger.apply(&vk, &statement, &proof).unwrap();
    assert_eq!(bytes(&ledger), read("after.bin"));
    println!("public proof and encrypted-state replay verified");
}
