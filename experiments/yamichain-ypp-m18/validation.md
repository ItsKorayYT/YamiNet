# Milestone 18 validation — candidate rejected

Date: 2026-11-05. Base: `5b9f43b5a44a4779b07162c4ec8700eec4336cfd`.
Rust 1.90.0, Linux x86_64, Intel Xeon Platinum 8573C, 3 visible vCPUs.
Only `experiments/yamichain-ypp-m18/` is added. No earlier experiment or
production file changes. The real proof backend is a test-only dependency on
the unchanged previous experiment. No production dependency or verifier is added.

## Feasibility verdict

**REJECTED.** A directly indexed sparse update exposes mandatory endpoint
membership. The fixed-width/cohort candidate fails global participant
indistinguishability even with honest uniform coins, ideal transport, no corrupt
owners and honest finality. This is a scoped negative result, not a proof that
every encrypted account representation is impossible. A private sparse state
access/update operation with authenticated offline recovery remains unresolved.

## Real cryptographic accounting replay

64 real registered curve accounts, eight cohorts of width 8, four cohorts/32
accounts selected per transfer. Four actual Groth16 proofs are generated with
the unchanged prior circuit and local trusted setup. No setup-free proof or
500,000-account cryptographic registry is claimed.

| Measurement | Observed |
|---|---:|
| Local setup | 8.825 s |
| First proof generation | 5.452 s |
| Verification, including public-key preparation | 47.389 ms |
| Authenticated record recovery, bound 128,000 | 2.508 ms |
| Proof bytes | 192 |
| Serialized statement + proof + u64 indices | 4640 bytes |
| Record / Merkle siblings at 64 accounts | 168 / 192 bytes |
| Crypto integration-test process peak RSS | 388.8 MiB |

Peak RSS includes the complete integration-test process, setup and repeated
proofs, not a per-proof isolated allocation. One recorded run, no timing
distribution or Groth16 optimization. Recovery is bounded here; the 32-bit
full-range recovery measurement remains in the previous unchanged experiment.

## Registry-scale authentication and metadata, not payments

| Measurement | Observed |
|---|---:|
| Synthetic public record slots | 500,000 |
| Actual SHA-256 values/tree initialization | 186.158 ms |
| Tree node allocation | 33,554,432 bytes |
| Selection mean, 1000 samples | 1.138 us |
| Incremental tree updates for 256 slots | 0.631 ms |
| One path build and verification | 3.078 us |
| Sibling path payload | 608 bytes |
| Metadata benchmark peak RSS | 50.5 MiB |

These timings exclude curve key generation, registration proofs, transfer
proving, consensus, tag-set maintenance, database durability and network I/O.
The raw JSON marks synthetic values and missing full-registry proof explicitly.
Sparse account authentication is affordable here; privacy fails independently.

## Adversarial results

- Chosen-pair game: 19,996/20,000 correct (99.98%). The analytical success probability under uniform cover is 0.9998719754.
- Eight linked pair transfers: cohort intersection 4 -> 2 after two observations.
- Same sender/changing recipient, and converse: 4 -> 1 cohort after two.
- Ideal unpredictable public repartitioning: account intersection 256 -> 2 after
  two linked observations. These profiles explicitly assume/give linkage.
- Reusing cover coins keeps four public cohorts, but does not restore global
  anonymity. Sender-controlled entropy can grind a specified cover cohort;
  the recorded metadata test took 11,702 attempts.
- Selective censorship based on support has different inclusion probabilities
  in the two challenge worlds. This is an analytical consequence of the measured
  classifier, not a consensus liveness benchmark.
- Real colluding-key decryptions identify every zero-update owned decoy. Valid
  proof-of-possession registration accepts 128 keys controlled by one party.
- Real related-key rogue registration, absent possession checking, exposes a
  target update of 19 units. A forged possession proof is rejected.
- Real limb encryption recovers standalone u64::MAX. A valid credit from total
  65,535 to 65,536 exceeds the low-limb 16-bit decoding bound, exposing the
  unimplemented offline encrypted carry operation. No 64-bit transfer proof.
- Exact small-universe support-family checks reject identical sparse supports
  across every possible endpoint pair. An exact uniform 4-of-8 challenge has
  success 5/6. The general support bound is proved in README.md.

## Checks passed

- `cargo test --release --locked`: 2 primitive unit tests, 3 cryptographic
  integration tests and 1 production-boundary test (6 total).
- Cross-cohort real proofs: atomic failure leaves the head unchanged; relabeled
  indices fail; different-sender credits commute; a separately verified valid
  conflicting-debit proof is rejected across different supports; replay and
  expired epochs fail. Untouched accounts retain exact record hashes.
- Epoch advancement changes no account records. The offline recipient recovers
  1016, then spends 1001 next epoch and recovers 15. Tampered records, wrong
  account paths and substituted epoch fail against the expected checkpoint.
- Every u16 proof version (65,536 total) remains rejected for both financial
  payments and confidential protocol actions, in a non-cfg(test) production
  library. Neither main nor production consensus changed.
- `cargo clippy --release --all-targets --locked -- -D warnings`.
- `cargo fmt --check`, Python compilation and all attack assertions.
- No changes outside the new isolated experiment relative to the base.

## Source evidence

- `firnprotocol/contracts` revision
  `fa6dd7dde6e782008a6048e3ef725f175706731e`: `src/Firn.sol`,
  `src/TransferVerifier.sol`, `src/Utils.sol`. Public account lists, shared
  randomness, epoch tag, pending rollover, explicit verifier checks.
- `Consensys-inc-archive/anonymous-zether` revision
  `be12ba6336607b26f235730a0c792a00d79625b5`: `docs/AnonZether.pdf`.
  PDF SHA-256:
  `053ee0199badf68f0d3173beeb64e035a1ae4e61527e54fd80360f3dc458b97b`.
  The document was read locally with pdftotext; it was not represented as an
  audit. Section 6.2 gives the related-key attack and possession remedy; 6.7
  gives the fixed-set challenge; 6.8 distinguishes ledger privacy from generic
  zero knowledge. Sections 4/5/6.3 explain many-out-of-many commitments, ring
  authorization techniques and the parity constraint.
- https://eprint.iacr.org/2020/293 landing page corroborates the paper identity;
  its PDF fetch returned 403, so byte identity to the ePrint PDF is not claimed.
- https://docs.firn.io/overview/cryptographic-explainer was read for the public
  account-state and offline retrieval model; marketing/client claims are not
  adopted as security proofs or performance results.

No specialized proof port, transparent payment proof, private-write primitive,
secret-bearing worker, ring signature, custody service or financial activation
is introduced. The report and tests document a rejected construction.
