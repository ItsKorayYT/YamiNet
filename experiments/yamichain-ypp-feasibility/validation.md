# Milestone 3 validation — 2026-11-03

Baseline `7c807445b22e7ae3c02de7175e097520fd1da8aa`, existing
`development/yamichain`. Only this independent experiment is added. RingCT,
Milestones 1/2, production crates/apps, the root manifest/lockfile and main remain
unchanged. No verifier registration, payment protocol, live YAMI operation,
additional monetary module or deployment was introduced.

## Checked behavior

- Formatting and Clippy `--all-targets --locked -- -D warnings` passed.
- Locked release build passed. Full release suite: **6 passed**, zero failures or
  ignored tests (five unit tests plus the production-boundary integration test).
  Unit suite took 69.95 seconds. An added independent full-rebuild check for the
  archive was also run against awkward tree sizes and every resulting leaf path.
- The real-proof counterexample test generates and independently verifies five
  Groth16 proofs: two unrelated writes from one root, a same-vault conflict, a
  subsequent Alice version and a Bob proof refreshed to Alice's root.
- Rebound inflation uses a correctly recomputed new root, so balance equality
  must reject it. Rebound u64 version wrap recomputes roots/marker. Forged owners,
  wrong private path/index, malformed packets, altered roots/marker/proof and
  excessive verifying-key vector lengths are rejected.
- Tests cover stale overwrites, global conflict/replay rejection, deterministic
  replay of an unsafe history to the same wrong root, incorrect additive Merkle
  root composition, public-leaf enumeration, failed offline recipient access,
  interrupted/corrupt replay, stale/updated encrypted backups, wrong backup keys,
  and corruption. Full-width u64 balances are exercised.
- Actual KZG commit/open/check verifies linear update composition and refreshed
  witnesses. Public zero-evaluation checks identify the changed position, including
  repeated writes. Modified evaluation values and wrong witness locations fail.
- A cached SHA-256 public anchor archive is tested through **100,000 entries**.
  Old/current proofs, corrupted hashes/counts and out-of-range indices are checked.
  A separate naive rebuild verifies cached roots at non-power-of-two sizes.
- Existing production chain/privacy/storage/consensus/naming regressions:
  **34 passed, one existing ignored scale test**. The non-test privacy library
  still rejects all **65,536 proof versions** for payments and protocol actions.
- Root formatting, existing chain-independence script, production dependency
  isolation and baseline-file comparisons passed.

These are genuine proofs and commitment checks. The deliberately unsafe node
policies and algebraic KZG tests are failure harnesses, not financial validation.
The candidate SNARK authorizes value-preserving rotations only, not transfers or
verifiable delivery. No mock proof is used as evidence of cryptographic validity.

## Measured native performance

Raw samples, concrete roots, all five public proof packets and simulation grids:
[measurements-x86_64.json](measurements-x86_64.json). Linux x86_64, AMD EPYC 9V74
VM, Rust 1.90.0, two CPU-equivalents cgroup quota, `RAYON_NUM_THREADS=2`, 8 GiB
runtime memory ceiling. No unrelated compilation/tests ran during measurement.
Each run uses fresh local setup; root/proof bytes differ between runs. The
reproducible properties, not fixed random roots, are the regression assertions.

| Measurement | Actual result |
| --- | ---: |
| Hidden-write relation constraints | 371,864 |
| Local Groth16/Pedersen setup | 20.181 s |
| Sequential proof generation, median of three | 7.639 s |
| Sequential proof range | 7.618–8.300 s |
| Two actual concurrent proof threads, wall time | 14.257 s |
| Concurrent per-proof latencies | 13.474, 14.238 s |
| Fresh verifier median of 20 | 2.592 ms |
| Verifier key preparation + packet decode | 7.696 ms |
| Groth16 proof / complete root-only packet | 192 / 288 bytes |
| Whole setup/provers/experiments/recovery peak RSS | 2,892,152 KiB (2.758 GiB) |
| Separate verifier-only peak RSS | 3,108 KiB |
| Two-record authenticated replay | 4.881 ms |
| age backup restoration | 0.137 ms |

The fresh verifier process had a **2 GiB virtual address-space limit**, did not
load the proving key and ran no setup. RSS is Linux VmHWM. Concurrent generation
uses real OS threads and the shared proving key; the event model is separate.
These proofs perform a weaker job than Milestone 2: they have no transfer outputs
or encrypted delivery. Their shorter runtime is **not** an improvement to a
complete private payment protocol or a final-parameter claim. No native ARM64,
large-state SNARK or complete low-end validator performance is established.

| KZG vector entries | Public refresh bytes | Hint generation ms | Observer identifies slot ms |
| --- | ---: | ---: | ---: |
| 4 | 196 | 1.703 | 13.676 |
| 16 | 784 | 16.246 | 57.443 |
| 64 | 3,136 | 153.681 | 223.925 |
| 256 | 12,544 | 1382.793 | 937.668 |

At 256 entries, the delta commitment is 48 bytes and the honest refreshed
witness/check takes 3.590 ms. All four vector sizes correctly merge independent
updates and reveal the targeted slot through the tested public hint channel. No
KZG opening-privacy claim is made before or after the attack. Scalars are algebraic
test values; this component is not a monetary range/authorization circuit.

For 100,000 public anchors, total cached append time was
199.665 ms, membership refresh 0.005318 ms, and verification
0.002593 ms. The oldest anchor requires 17 sibling hashes / 544 bytes,
plus index/count/anchor metadata. Cached archive hash payloads occupy
6,400,192 bytes, excluding vector/allocator overhead. A validator's authoritative
checkpoint is 32 bytes; authoritative/finalized checkpoint distribution and
permanent conflict history are separate requirements, not supplied by membership.
This public archive does **not** update private vault-state witnesses.

The unsafe two-write trace stores two 288-byte packets, 64 marker bytes and 96
historical-root bytes, excluding its root/head fields and container/key overhead.
Its compact state is still wrong. Stored roots, proof history and conflict markers
grow. No safe pruning policy or full-node storage limit is established.

## Concurrent-admission simulation

The report includes 36 scenarios using archived **actual Milestone 2** latency
samples, and 36 using this experiment's measured sequential rotation latency.
Remote provers are modelled independently; verification is serialized. Client
counts are 2/8/64, background root rates 0/1/4 per second, and root policies are
4/64/4096/unlimited. At two clients and one background root per second, the
Milestone 2 samples produce **0 accepted / 2 expired** under four roots, and
**2 accepted / 0 expired** under 64 or archived/unlimited anchors.

This is event simulation based on measured costs, not native network execution,
shared-host proof throughput or a consensus liveness proof. It assumes unique,
conflict-free inputs and available state witnesses. A larger anchor window cannot
make a stale private in-place overwrite correct; the separate real-proof trace
shows exactly that failure.

## Decision and limits

The run rejects both tested compositions. The correct merged root differs from
both the stale-replacement root and the additive-field root; actual bytes are in
the report. Publishing new leaf commitments identifies slots **0, 1, 0** across
Alice, Bob and Alice's subsequent write. KZG public refresh similarly reidentifies
repeated writes. Old-backup history replay authenticates a head but does not
recover the omitted fresh opening. The owner-only candidate cannot credit an
offline recipient and is rejected rather than completed with note semantics.

[DECISION.md](DECISION.md) explains why the general feasibility question remains
open, identifies relevant positive prior art and keeps production on hold. No
universal impossibility theorem, original secure vault protocol, reviewed setup,
dependency audit, side-channel hardening or production readiness is claimed.
