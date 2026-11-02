# YamiChain validator scale and inactive PoS interfaces

A real 100-process network now passes partition/crash recovery and committee
rotation with an explicit local timing profile under two shared CPU equivalents
and 2 GiB RAM. The original timing control still exhausts that memory limit.

Continues draft #16 on a separate branch. No merge, public deployment, real stake,
issuance, election, payment acceptance or cryptographic backend change.

## Implemented

* Optional `--sparse-peers true`: a symmetric ring/chord graph with at most four
  authenticated scheduled neighbors. The default full mesh remains available.
  Tests check connectivity after each possible single-node crash at 4/25/50/100
  members. This rehearsal graph is not a reviewed permissionless discovery or
  eclipse-resistance design. Discovery stays disabled and genesis stays pinned.
* The process lab supports 25/50/100 members, delay/partition/crash recovery and
  four-epoch rotation. A distinct-source-IP mode gives each simulated host a stable
  address in 127.1.0.0/24. Encrypted transport and PeerId/key checks are unchanged.
  Malachite's five-inbound-connections-per-IP guard is never raised or disabled.
* Optional bounded local startup coordination and configurable Malachite timeouts
  retain the original 800-ms default and enforce 800..60000 ms. The initial prevote
  and precommit windows remain half the proposal window; rebroadcast is twice it.
  A 45-second lab barrier plus a five-second proposal window is tested separately
  from the original profile. Wall-clock startup coordination is never a consensus
  timestamp, election input or public deployment mechanism. Labs serialize via a
  host file lock to avoid accidentally running competing measured clusters.
* `YAMI_VALIDATOR_LOG` enables optional Malachite diagnostic logs on stderr. Normal
  stdout remains machine-readable. The diagnostic case traces one node only.
* `Journal::storage_footprint` samples signing, evidence, WAL and candidate bytes
  with an entry budget, without loading payloads or deleting anything. It refuses
  unexpected entry types and refuses excess work rather than reporting partial
  inspection as complete.
* A history snapshot inventory hashes exact certified records and binds them to a
  checkpoint authenticated by sequential execution. Construction is limited to
  1,024 records and a caller byte budget. Tests reject wrong genesis and exhausted
  budgets. This is transport inventory infrastructure, not compact state import,
  an independently trusted snapshot, or permission to prune.
* Inactive eligibility/provider/selector/epoch-proposal interfaces bind accounting
  and eligibility roots, population size/weight, requested committee size, parent
  checkpoint, epoch and activation height. Candidate paging has an abstract witness
  type; no proof format or accepting verifier is supplied. The eligible population
  is not limited to the committee's 100-member cap. `ElectionsDisabled` always
  rejects. Registration possession, unique shielded backing, delegation accounting,
  slashing and unbonding must be authenticated by a future reviewed provider.

## Diagnosis and limitations

The previous all-to-all fixture asks every node to persistently dial every other
node while all clients share one loopback IP. Malachite limits inbound connections
per IP to five, and persistent peers bypass reconnect throttling, not that cap.
Its default connection-per-peer allowance also permits duplicate connections.
Tokio already explicitly uses two workers per validator; host CPU detection did
not create an unbounded worker count.

The sparse graph improves the 25/50-process cases. The 100-process shared-IP case
hit its 2-GiB memory limit: Docker recorded an OOM event. Distinct IPs alone did not
make the 100-process rotation run finalize before its deadline. That rotation run
also overlapped compilation, so its timing cannot establish isolated hardware
performance. These results do not justify blaming only the IP guard, weakening it,
changing quorum rules, or claiming the 100-node problem solved.

Every container uses two CPU equivalents **for the entire cluster**, not per node,
and 2 GiB without swap (the diagnostic case uses 6 GiB). The parent workspace also
has a two-CPU quota. Local loopback, four naming actions per block, 16 blocks and
public development keys are short non-financial rehearsals, not WAN or shielded
payment benchmarks. Initial 25/50 cases overlapped unit checks; inspect provenance
and treat those times as observed under contention. Per-process RSS includes shared
mappings; cgroup memory is a different measurement. Network totals include handshake,
gossip and proxy traffic, not just transaction bytes.

The opt-in trace confirms launch skew: node 0 starts its 800-ms proposal timer
before later processes listen, and its last scheduled neighbor connects about 14
seconds later. The trace shows repeated step timeouts/rebroadcasts with authenticated
peers. A larger CPU budget per validator or coordination/appropriate operational
timeouts must be distinguished from changing BFT. No threshold, signature, payload
validation, anti-equivocation or recovery check is bypassed.

## Gates that remain closed

The fresh RustSec audit still reports Hickory 0.25.2 (RUSTSEC-2026-0118/0119) and
crypto-bigint 0.5.5 (RUSTSEC-2026-0387). Registry checks still find no compatible
patched release in the pinned Malachite 0.8 series. Solver dry runs reject fixed
Hickory 0.26.3 and bigint 0.7.10 because their upstream requirements remain
`^0.25.2` and `^0.5`. No warning is suppressed or incompatible crypto forced.
Diagnostic logging adds tracing-subscriber; the final lockfile is audited again.

Storage work bounds inspection and inventory construction, **not total disk growth
at an indefinitely stalled height**. Acknowledged candidates, signer reservations,
WAL and required evidence remain retained. A hard quota that discards these records
or refuses every honest reproposal would violate the safety/progress requirement.
No historical pruning is enabled. A reviewed retention/evidence window, crash-safe
snapshot publication/restore and exact future spent-tag retention are still needed.

Random selection remains disabled pending approval of sampling/weight rules,
unbiased randomness, grinding/adaptive-corruption defenses and authenticated epoch
activation. Ordinary YAMI payments remain shielded-only and fail closed while no
approved proof backend exists. YamiNet/browser/YNS remain independent.

## Reproduce

```sh
cargo build --release -p yami-validator --locked
python3 tools/yamichain-container-lab.py --case distinct100 --output work/new-scale
# Also: sparse25/50/100, distinct25/50, rotate25/50/100, barrier100, tuned-rotate100, diagnose100
cargo test --release -p yami-chain-consensus --locked scale_stress_authenticated_core -- --ignored --nocapture
```

## Measured results

[Full measurements](evidence/yamichain-scale-pos/measurements.json),
[diagnostic/OOM evidence](evidence/yamichain-scale-pos/diagnosis.json),
[12 authenticated core schedules](evidence/yamichain-scale-pos/core.json), and
[final dependency audit](evidence/yamichain-scale-pos/audit.json).

Wall time ends when every process reports durable target finality; it includes
startup coordination. CPU sums validators, including crash/recovery and shutdown
replay; proxy/monitor CPU and subsequent inspection are recorded separately.
RAM below is cgroup peak for the whole cluster. Forwarded bytes include encrypted
protocol traffic; they are not per-node WAN bandwidth.

| Case | Result | Height | All-target seconds | Validator CPU seconds | Cgroup peak MiB | Forwarded MiB |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| sparse25 | PASS | 16 | 31.211 | 27.013 | 195.4 | 33.9 |
| sparse50 | PASS | 16 | 85.17 | 124.947 | 432.4 | 348.9 |
| sparse100 | FAIL | — | — | — | — | — |
| distinct25 | PASS | 16 | 23.491 | 25.318 | 194.6 | 27.3 |
| distinct50 | PASS | 16 | 55.605 | 94.398 | 414.5 | 129.4 |
| distinct100 | FAIL | — | — | — | — | — |
| rotate25 | PASS | 16 | 11.731 | 21.431 | 197.1 | 26.9 |
| rotate50 | PASS | 16 | 42.297 | 83.379 | 401.9 | 128.0 |
| rotate100 | FAIL | — | — | — | — | — |
| barrier100 | PASS | 8 | 163.67 | 210.072 | 838.2 | 430.0 |
| tuned-rotate100 | PASS | 12 | 156.488 | 235.485 | 833.3 | 294.4 |

`barrier100` passed delay, partition/healing and SIGKILL/restart through height 8
(32 naming actions). `tuned-rotate100` passed 100 → 97 → 100 → 99 membership and
crash/recovery through height 12 (48 actions). Both use the 45-second startup
barrier and 5,000-ms proposal profile. The matched original-timing distinct-IP
100-process control exhausted its memory limit without finality. This establishes
a working bounded local profile, not a proof of one isolated root cause or a
reviewed permissionless topology. Timings and topology remain explicit operator
choices; defaults and all authentication/quorum checks are preserved.

Final checks: **482 passed, 0 failed, 10 ignored**; formatting, Clippy (`-D warnings`),
workspace build, exhaustive release payment rejection, independence, all 12 core
schedules and all 11 original process scenarios passed. A Python regression also
checks that a container exiting zero without measurements cannot report success.
[Check provenance](evidence/yamichain-scale-pos/checks.json) and
[original scenarios](evidence/yamichain-scale-pos/original-processes.json).

Next: repeat the chosen profiles across independent hosts with per-validator quotas
and WAN faults, then review discovery/eclipse resistance and operational defaults.
Approve retention and randomness/selection security models before enabling pruning
or elections. Payments and public deployment remain disabled.
