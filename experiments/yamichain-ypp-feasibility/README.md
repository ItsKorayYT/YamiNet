# YPP Milestone 3 — feasibility counterexamples

**Decision: reject the tested constructions and keep production integration on
hold. Private persistent vaults have not been established. These results do not
prove that all possible private-account architectures are impossible.**

This independent Rust experiment adds no payment system, note system, production
verifier or financial module. Milestones 1/2 and RingCT remain unchanged. The
unsafe validator modes are explicitly named counterexample harnesses, never a
suggested consensus implementation.

| Tested mechanism | Reproducible result |
| --- | --- |
| Real ZK hidden in-place Merkle write, current root only | Rejects a valid unrelated concurrent write |
| Historical-root replacement plus conflict markers | Accepts both independent proofs but rolls back one vault |
| Additive differences of Merkle roots | Produces a root different from the correctly merged state |
| Publish the new leaf to refresh witnesses | Prior-state enumeration reveals the slot and repeated writes |
| Standard KZG vector deltas | Independent updates compose correctly |
| Public KZG opening-refresh hints | Pairing checks reveal the changed slot; omitting refresh leaves stale witnesses |
| Root-only history and an old encrypted backup | History verifies, but the new private opening is unrecoverable from those inputs |
| Public authenticated anchor archive | Historical-anchor proofs refresh through 100,000 entries; private vault witnesses remain unsolved |

## Reproduce

Rust 1.90; commands from repository root:

```sh
manifest=experiments/yamichain-ypp-feasibility/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo clippy --manifest-path "$manifest" --all-targets --locked -- -D warnings
RAYON_NUM_THREADS=2 cargo test --manifest-path "$manifest" --release --locked -- --test-threads=1
cargo build --manifest-path "$manifest" --release --locked
RAYON_NUM_THREADS=2 cargo run --manifest-path "$manifest" --release --locked -- demo /tmp/ypp3-public
experiments/yamichain-ypp-feasibility/target/release/yamichain-ypp-feasibility verify /tmp/ypp3-public
python experiments/yamichain-ypp-feasibility/measure.py > /tmp/ypp3-measurements.json
```

The demo generates real Groth16 proofs, including two generated concurrently,
reproduces all failed acceptance policies, executes upstream KZG commit/open/check,
and emits full public proof packets, concrete roots, attack results and timings.
Only public verifying-key/packet fixtures are saved. Local trusted Groth16/KZG/
Pedersen setup is experimental, not a reviewed ceremony or independent audit.

The hidden-write circuit preserves a **u64** balance while advancing its version.
It intentionally does not implement transfer or credit semantics: the concurrency
counterexample already occurs for value-preserving updates. Its four-leaf test
fixture is a counterexample size, not a proposal for final capacity. KZG vector
scalars are algebraic test data, not monetary values. Liveness simulation uses
measured latency samples and is labelled separately from native benchmarks.

[DECISION.md](DECISION.md) states the exact rejection boundaries, offline/recovery
failure and unresolved research question. [validation.md](validation.md) records
checks and measurements. Raw public transcripts and simulation grids are in
[measurements-x86_64.json](measurements-x86_64.json).
