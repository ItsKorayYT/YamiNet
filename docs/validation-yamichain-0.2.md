# YamiChain 0.2 validation — 2026-11-02

Linux x86_64, Debian 13, Rust/Cargo 1.90.0. This is implementation evidence,
not an audit, Windows/ARM64 attestation or production-release approval.

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --locked` | PASS: 474 passed, 0 failed, 9 ignored |
| `cargo build --workspace --locked` | PASS |
| `cargo test -p yami-chain-privacy --release --locked` | PASS; all 65,536 versions rejected for both payments and protocol value actions |
| `python tools/check-chain-independence.py` | PASS |
| `python tools/yamichain-v02-vectors.py` | PASS; independently reproduces genesis and state vectors |
| Python driver syntax checks | PASS |

Build commands used `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`,
`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`. Optimizations and debug
assertions retain their normal dev/test settings. The environment lacked ALSA
headers/link metadata: official Debian `libasound2-dev` was extracted to a local
prefix, using the existing system runtime and `PKG_CONFIG_PATH`. No workspace
source or lint was weakened for this setup. An earlier symbol-heavy build exhausted
the environment's approximately 8-GB filesystem; cleaning generated artifacts and
disabling symbols/incremental compilation resolved it.

Adversarial cases include 256 signed same-round conflicts plus future-round input,
forged signatures, one retained genuine equivocation pair, progress/recovery in the
next round, and retention across 80 distinct proposal rounds. Genuine Malachite
core schedules exercise reordering, one offline/equivocating validator, failure to
finalize without quorum, and 4/7/3-member height-specific sets. Certificate tests
cover 1/2/4/7/25/100-member sets, weighted supermajority, duplicates, malformed
signatures, altered genesis/schedules, wrong transport identities and historical
verification. The 25/100-member tests are certificate tests, not 25/100-node network
or performance claims.

Storage tests cover gaps, corruption, regular-file/name/symlink rejection, signer
conflicts after reopen, exact checkpoint commitments, replay-budget refusal,
history retention and persistence of a full 64-epoch/100-member genesis. The
staking simulation checks registration possession/role separation, nonce replay,
duplicate backing/positions, insufficient balances, commissions/reward reserve,
unbonding/withdrawal timing, historical signed double-vote evidence, duplicate and
forged evidence, and deterministic randomized failures with invariant/atomicity
checks. It is compiled only as tests; its parameters and rounding are not approved
live PoS policy.

## Process and resource evidence

The process driver checks exact finalized roots, authenticated independent replay,
exit codes, disabled payments and genuine certified catch-up. It also adds a
seven-node committee, 4 → 7 → 5 rotation, a durable epoch-boundary crash/restart and
a late observer fetching history across both membership changes.

Early runs are retained locally under `work/`: one catch-up run reached height
38/40 before a 90-second deadline while compilation competed for CPU. An initial
rotation run shut down history providers before a lagging node fetched the last
certificate. A second exposed the global timeout including terminal serving time
even after durable target finality. The runner now has a separately bounded history
linger; consensus deadlines still fail if target finality was not achieved. No
quorum, signature, payment or storage check was relaxed.

[All 11 final process scenarios passed](evidence/yamichain-0.2-processes.json).
[Check provenance](evidence/yamichain-0.2-checks.json) records the implementation tree,
binary/driver/log digests, environment flags and counts.

[Measured resource run](evidence/yamichain-0.2-resources.json): four processes, 12
blocks, 26.322 seconds including eight seconds of terminal history serving;
34.43 total validator CPU-seconds. Kernel high-water RSS sampled every 50 ms was
30.81–31.09 MiB per validator; sampled combined peak was 123.98 MiB. Final allocated
store size was 188–212 KiB per validator. The container's CPU quota was
`200000 100000` (two CPU equivalents); memory limit was 8 GiB. This was an
unoptimized dev build with debug symbols/incremental artifacts disabled. Resource measurements use only one initial test
naming action followed by empty blocks and do not estimate proof performance,
large-state sync, adversarial soak or production/pruned-state size. Native ARM64,
2-GB pressure, 20-GB production storage and approximately 5-GB pruned state remain
unverified. Full-history/snapshot pruning remains disabled.

## Dependency audit remains blocking

`cargo-audit` 0.22.2, official release binary, scanned `Cargo.lock` with RustSec DB
`263a4d951a0867d4bddc29dddd6d59427c78f9a7` (`--no-fetch --json`).
[Raw result](evidence/yamichain-0.2-audit.json): **3 vulnerabilities, not a clean
audit**. No advisories were suppressed.

| Package | Findings | Safe-upgrade result |
| --- | --- | --- |
| `hickory-proto` 0.25.2 | [RUSTSEC-2026-0118](https://rustsec.org/advisories/RUSTSEC-2026-0118.html), [0119](https://rustsec.org/advisories/RUSTSEC-2026-0119.html) | Dry-run upgrade to fixed 0.26.1 rejected by the pinned libp2p graph's `^0.25.2` requirement. |
| `crypto-bigint` 0.5.5 | [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html) | Fixed range starts at 0.7.6; it conflicts with `elliptic-curve`/P-256's `^0.5` requirement (0.7.6 itself is also yanked). Requires coordinated upstream remediation. |

Unmaintained warnings: `paste` 1.0.15 in the added networking graph; existing
browser dependencies `rustybuzz` 0.14.1 and `ttf-parser` 0.20.0/0.21.1. No unrelated
browser dependency upgrade or unreviewed consensus/crypto fork was introduced.
Public networking and real-key/value activation remain blocked pending dependency
remediation, independent consensus/cryptography review and hostile-network testing.
