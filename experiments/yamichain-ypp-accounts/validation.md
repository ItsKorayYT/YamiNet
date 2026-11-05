# Validation: 2026-11-05

Isolated release build: Rust 1.90.0, Linux x86_64, Intel Xeon Platinum 8573C,
3 visible vCPUs. Single fresh process/sample per size; timings are observations,
not statistical guarantees. Runtime varies with cloud scheduling. Python 3 with
cryptography 50.0.0 for Paillier arithmetic. Raw results are checked in beside this file.

| Accounts | Constraints | Transaction bytes | Proof bytes | Setup s | Prove s | Verify ms | Recovery ms | Peak RSS MiB |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 16 | 64987 | 2336 | 192 | 3.39 | 4.02 | 32.31 | 609.60 | 203.6 |
| 64 | 227371 | 8480 | 192 | 11.00 | 10.78 | 93.35 | 654.88 | 720.1 |
| 128 | 443883 | 16672 | 192 | 20.84 | 18.57 | 175.59 | 462.83 | 1393.9 |

Recovery reads and validates a saved cohort snapshot, verifies the externally
provided checkpoint root, and runs BSGS with the complete 32-bit bound. No history
scan or recipient interaction is performed during receiving. Setup randomness and
wallet secrets are not written to public artifacts. Memory is VmHWM for the whole
process, including setup, not a prover-only allocation estimate.

Validation performed:

- `cargo test --release --locked`: four cryptographic/adversarial unit tests and
  one production-boundary integration test. Real Groth16 proofs are generated
  and verified, including a new debit by the offline recipient after rollover.
- Unauthorized key, overspend/false remainder, nonconservation, overlapping
  selectors, zero amount, bad epoch/tag and incorrect updates fail constraints.
  Tampered public statements fail actual proof verification and ledger application
  without mutating state. Forged, duplicate, identity and low-order registrations
  are rejected. Wrong recovery key and changed checkpoint state are rejected.
- Concurrent credits in both orders produce identical encrypted pending balances;
  replay and distinct conflicting debits from the same epoch/key are rejected.
- Actual observer-key decryption eliminates controlled decoys at all three sizes.
- Every one of the 65,536 production proof versions remains rejected for both
  confidential payments and protocol actions, in a non-cfg(test) production build.
- Separate `public_verify` processes loaded only saved public verifying keys,
  statements, proofs and before/after snapshots for 16,64,128; each verified the
  proof and reproduced the exact after-state. This is public replay, not finality.
- `cargo clippy --release --all-targets --locked -- -D warnings` and formatting
  checks passed. Python scripts compiled; leakage and Paillier assertions passed.

Paillier: see `paillier.json`. Actual u64::MAX recovery and homomorphic credit
succeed, with 512-byte ciphertexts. There is no Paillier transfer proof or measured
proof cost. `null` values are intentional blockers, not zero-cost placeholders.

The executable uses Groth16 with local trusted setup, not Firn's specialized
proof. Account values and recovery are 32-bit. Cohorts are fixed. Genesis is
trusted. Checkpoint finality/freshness, Sybil resistance, network-metadata privacy
and arbitrary-malicious-validator privacy remain unimplemented or unestablished.
Read README.md for exact assumptions, equations, source revisions and blockers.

Only this experiment directory changed. Main, production consensus, the production
workspace and every earlier research experiment remain untouched.
