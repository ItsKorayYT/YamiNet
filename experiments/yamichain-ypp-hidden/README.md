# YPP Milestone 2 — hidden-record experiment

**Synthetic, isolated, unaudited. No production verifier or financial activation.**
Milestone 1 and the RingCT reference are unchanged. This separate Rust workspace
implements a working private-record branch of the investigation. It removes the
Milestone 1 slot and commitment-equality graph, but **does not establish a novel,
unlinkable in-place persistent-vault protocol**. Its main balance is carried by a
sequence of privately consumed vault-version records; its inbox is explicitly
note-like. Those similarities to conventional shielded records are fundamental.

## Working implementation

- Real arkworks Groth16 proofs over BLS12-381, SHA-256 blinded record commitments,
  established Pedersen compressed CRH for a depth-five authenticated accumulator,
  private membership paths/directions and HMAC-SHA256 spent markers.
- No public old commitment, input slot/index, input owner, input credit ID or
  live/dummy flag. Each packet always has two nullifiers and two encrypted outputs.
  One logical vault version advances; optionally one incoming credit is absorbed.
- Six independent, nonzero ElGamal encryptions per packet, using the upstream
  Jubjub implementation and R1CS gadget. The proof verifies encryption of the
  amount, blinding point and complete spending authority to the viewing key bound
  in each output commitment. No custom cipher, KDF or proof mathematics.
- Four retained roots: independent transactions built against an earlier accepted
  root apply without reproving. Permanent global nullifiers reject conflicts.
- View-only discovery, spent-record tracking and authenticated history replay;
  age-encrypted full wallet backup and restore. A pinned final head is required.

**Prototype bounds:** at most 32 accumulated leaves; u16 balances/amounts;
ElGamal amount decoding uses baby-step/giant-step over the complete u16 range.
These are explicit engineering bounds, not a privacy parameter, production
monetary policy or a practical 64-bit encrypted-amount format.

| Observable behavior | Milestone 1 | This experiment |
| --- | --- | --- |
| Vault access | Public fixed slot | Private Merkle path |
| State transition | Public old/new pair | Hidden input, public fresh output |
| Credit consumption | Repeats public credit ID | Secret-key HMAC marker |
| Unrelated root change | Invalidates pending proof | Four-root acceptance window |
| Payload validity | Ciphertext bytes bound only | Entire opening encryption proved |
| State architecture | Linkable in-place vault | Append-only record lineage + inbox claims |

## Reproduce

Rust 1.90, from repository root:

```sh
manifest=experiments/yamichain-ypp-hidden/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo clippy --manifest-path "$manifest" --all-targets --locked -- -D warnings
RAYON_NUM_THREADS=2 cargo test --manifest-path "$manifest" --release --locked -- --test-threads=1
cargo build --manifest-path "$manifest" --release --locked
RAYON_NUM_THREADS=2 cargo run --manifest-path "$manifest" --release --locked -- demo /tmp/ypp2-public
experiments/yamichain-ypp-hidden/target/release/yamichain-ypp-hidden verify /tmp/ypp2-public
python experiments/yamichain-ypp-hidden/measure.py > /tmp/ypp2-measurements.json
```

The demo uses four independently generated wallets, repeated payments, three
unchanged same-anchor proofs, offline discovery, credit absorption and full backup
recovery. It emits JSON metrics and the entire public packet transcript. Optional
fixtures contain only a verifying key and public packet, never wallet/proving keys.

See [spec.md](spec.md) for the relation, counterexample and every remaining public
identifier; [validation.md](validation.md) and the raw measurement report record
actual results. Local setup is trusted and disposable, not a reviewed ceremony.
No independent security review or production privacy claim follows from tests.
