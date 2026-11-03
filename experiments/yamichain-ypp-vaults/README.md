# YPP Milestone 1: persistent vault / inbox experiment

**Synthetic funds only. Experimental, unaudited, and publicly linkable.** This is
an independent Rust workspace. Nothing depends on it from production, it registers
no chain verifier, and it implements no live issuance, ordinary-payment fallback,
staking, delegation, treasury, RPC, networking, or wallet activation. The existing
`experiments/yamichain-privacy-lab` RingCT reference remains unchanged.

This milestone implements the strongest bounded prototype here: two persistent,
in-place confidential vault commitments and a temporary encrypted credit inbox.
Real Groth16 proofs authorize debit and absorb operations. This is deliberately a
**hybrid**, not a claim to have invented a secure private-account protocol. Inbox
credits are note-like claims, and vault slots reveal the transaction graph. The
original objective of unlinkable persistent financial state remains unresolved.

## Reproduce

Rust 1.90; Linux x86_64 was measured. Commands run from the repository root:

```sh
manifest=experiments/yamichain-ypp-vaults/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo clippy --manifest-path "$manifest" --all-targets --locked -- -D warnings
RAYON_NUM_THREADS=2 cargo test --manifest-path "$manifest" --release --locked -- --test-threads=1
cargo build --manifest-path "$manifest" --release --locked
RAYON_NUM_THREADS=2 cargo run --manifest-path "$manifest" --release --locked -- demo 3 /tmp/ypp-public-fixture
RAYON_NUM_THREADS=2 experiments/yamichain-ypp-vaults/target/release/yamichain-ypp-vaults verify /tmp/ypp-public-fixture
```

The demo generates fresh independent Alice/Bob spending and viewing authorities
with OS randomness, constructs a synthetic genesis, sends Alice's encrypted inbox
credits with Bob offline, proves/validates each debit, scans the authenticated
record, and proves Bob's absorption into his existing vault. It rejects replay,
restores Bob's age-encrypted backup, and replays history to the trusted final head.
The optional fixture directory contains **only a verifying key and public packet**
for measurement in a fresh verifier process. No secret/proving key is written.
Each run creates a new local trusted setup: proofs from different runs are not
interchangeable. See [validation.md](validation.md) for actual measurements and
[spec.md](spec.md) for the relation and exact blockers.

## What is working

- Established SHA-256 commitments, arkworks Groth16 over BLS12-381 with the actual
  SHA-256 R1CS gadget, 64-bit range constraints, ownership knowledge, monotonic
  versions, strictly positive credits and exact monetary conservation per step.
- Verification takes only public inputs. The ledger authenticates the original
  commitment in its current two-leaf root and atomically replaces it only after
  successful proof verification. Same-root conflicts, replay, and reanchored proofs
  fail. No ring signatures, decoys, transparent balances, or public transfer values.
- Independent age X25519 viewing authority and a view-only scanner. A spending key
  is required to authorize an absorption; knowing the incoming credit alone is
  insufficient. Full encrypted backup restore and historical rescan work.
- Bounded canonical transaction decode; checked arithmetic; group/subgroup checked
  proof decode; public ciphertext, root, slot, and mode mutation invalidate proofs.
- Adversarial tests include an accepted, real proof with deliberately invalid
  ciphertext to demonstrate the unsolved delivery/data-availability condition.

## Privacy and deployment limits

Stable slots and exposed old/new commitments deterministically link every update
of each vault. The same public credit commitment links a debit to its absorption.
Owner hashes and addresses are not in plaintext in the packet, but prior identity
correlation identifies all activity at a slot. **No sender or recipient anonymity
is claimed.** Independent random blinding hides amounts in commitments under the
hash assumptions; age protects payload confidentiality under its own assumptions.
Traffic, timing, slot, ciphertext sizes, record count, credits and update graph
are visible. There are only two vaults and no meaningful anonymity set.

The proof binds the supplied ciphertext bytes but does **not** prove encryption
correctness or recoverability. A malicious authorized payer can create an
undecryptable pending credit; a malicious vault owner can withhold its next state
opening. Historical hashes cannot make missing data available. These are explicit
blockers, tested and documented, not mocked away. Groth16 setup, the new relation,
wallet/history adapter, dependency review, reorgs, durable storage, scalable state,
pruning, network anonymity, native ARM64 and full validator operation need further
work and independent review before any production proposal.
