# Canonical vectors — format 1

SHA-256 preimage for each domain is
`u64_be(domain_len) || domain || u64_be(payload_len) || payload`.
These values were independently encoded with Python `struct`, `hashlib` and
`cryptography` Ed25519, and frozen in `crates/yami-chain/tests/invariants.rs`.
The library derives the same Ed25519 public keys with ed25519-dalek 3.0.0.

| Fixture | SHA-256 digest |
| --- | --- |
| Local genesis | `d8b06ded9d9a1159dd872bf164abdba3e9f260b13369414fc1df7d4b8f19c806` |
| Initial state, height zero | `ed53a6b3ac52317b8492c2e54b5c4f9af3a6938c35e73f7f7e94335b6b73e381` |
| State at height 20, development workload | `cb30e18ba81cad8f669389e40f08daa3e19be24ed257b50c557867b90198df1c` |

Key seed: hash domain `yamichain-public-development-key-v1`, payload UTF-8 role
(`consensus`, `transport`, `registry`) followed by one index byte. Every seed is
public and is for isolated development only. Consensus/transport indices are 0..3;
the sole automatic test registration uses registry index 0.

Genesis payload is the length-prefixed ASCII chain ID `yamichain-local-0.1`,
u16 protocol 1, u64 feature mask 0, then four consensus public keys in index order
and four transport public keys in index order. Domain: `yamichain-genesis-v1`.
Committee hash sorts the four consensus keys and hashes their concatenation
under `yamichain-fixed-committee-v1`; each member's power is implicitly one.

State payload is genesis digest, u64 height, u32 nonce-map count, sorted
`(32-byte owner key, u64 next nonce)` pairs, u32 naming count, then sorted
`(u32 label length, ASCII label, 32-byte owner)` entries. Domain:
`yamichain-state-v1`. Height 20 has one nonce entry of value 1 and the single
`test-local` registration; no monetary supply, pools, notes or spent-value state.
Block/transaction fields and further domains are specified in YC-002/003 and the
codec source. No field from this prototype specifies a future YPP proof format.
