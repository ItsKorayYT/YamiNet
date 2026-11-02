# Milestone A validation — 2026-11-02

Baseline: `b39be13` on the existing `development/yamichain` branch, fetched before
work. Rust 1.90.0, Linux x86_64, AMD EPYC 9V74 VM; two CPU-equivalents of cgroup
quota, 8 GiB environment memory limit. No native ARM64 hardware was available.

Passed locally:

- Root and lab formatting checks.
- Root workspace and lab `clippy --all-targets --locked -- -D warnings`.
- `cargo test --workspace --locked`: 485 passed, 10 existing ignored tests.
  Includes naming, consensus, signing, storage/recovery and ordinary YamiNet tests.
- `cargo build --workspace --locked`.
- `cargo build -p yami-validator --release --locked`.
- Release tests for `yami-chain`, `yami-chain-naming`, `yami-chain-privacy`,
  `yami-chain-storage`, `yami-chain-consensus`: 34 passed, one existing ignored
  manual scale test. Includes exhaustive production proof-version rejection.
- Lab `cargo test --locked` and `cargo test --release --locked`: each 10 passed
  (one canonical-binding unit vector, eight offline integration/adversarial tests,
  one real-packet chain-isolation test).
- Release lab example build; no node or network is needed for the example.
- `cargo check --manifest-path experiments/yamichain-privacy-lab/Cargo.toml
  --target aarch64-unknown-linux-gnu --locked`: passed. **Compile check only**;
  no ARM64 link/runtime, side-channel, CPU or RAM claim.
- `python tools/check-chain-independence.py`; unchanged root `Cargo.lock` and
  production verification code; no Zcash/Orchard/Sapling/Halo 2 lab dependencies.

The VM lacked ALSA development metadata. Validation used a Debian development
package extracted under scratch, linked to the existing runtime library; no
repository workaround or system installation was introduced. No physical audio
device was available. Developer/test debug symbols and Cargo incremental output
were disabled locally to conserve disk; debug assertions remained enabled.

## Measured offline performance

Five fresh-process release samples per case, two outputs, rings of 16. Medians:

| Inputs | Packet bytes | Prunable proof bytes | Create/prove/sign ms | Verify ms | Scan ms |
| --- | --- | --- | --- | --- | --- |
| 1 | 1660 | 1251 | 51.618 | 7.445 | 0.260 |
| 2 | 2319 | 1859 | 51.943 | 9.660 | 0.265 |

Whole-process maximum RSS was **6932 KiB**. Each child ran with an enforced
2 GiB virtual-address-space limit, inside the VM's two-CPU quota. RSS/CPU cover
the complete example, including backup recovery and an onward spend; these are
not separate verifier-memory measurements or a validator benchmark. Creation
includes upstream transaction construction, range proving and authorization.
Prunable proof bytes include range proof, CLSAGs, pseudo-outputs and framing.
These small fresh-process samples do not establish sustained resource limits,
large-state scaling, constant-time behavior or the full 2-core/2-GB validator
target. No native ARM64 measurements were made.

Raw samples and hardware/quota metadata: [measurements-x86_64.json](measurements-x86_64.json).
Reproduce with `measure.py` after building the release example.

The source inspection, negative tests and older upstream audit do not establish
production security or anonymity. See README for unreviewed integration/dependency
risks and the exact Milestone B blockers. No financial feature was activated.
