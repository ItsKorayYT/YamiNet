# YamiChain stress and hardware validation — 2026-11-02

Draft #16 is stacked on #15 and #14; neither was merged. Payments, real staking,
issuance and randomness remain disabled. This is a local development prototype
using public keys. These results do not authorize public deployment.

Final production Rust checks: formatting, Clippy (`-D warnings`), workspace build
and **479 passed / 0 failed / 10 ignored**; exhaustive release payment rejection
and YamiNet independence checks also passed. [Check digests](evidence/yamichain-stress/checks.json).

## Changes and regression coverage

* A retry after uncertain rename/directory-sync failure now re-syncs the existing
  identical file and directory before acknowledging persistence. Fault injection
  covers file sync, rename, directory sync, retry, conflict and reopen.
* History replay has explicit record/byte budgets and rejects over-budget recovery.
  Candidate metadata stores the authenticated proposal once, without duplicating
  full payloads; PR #15 records remain readable. This is compaction, not pruning.
* The delayed 25-process campaign exposed a genuine adapter liveness bug: exact
  current-round admission dropped honest parts arriving before `StartedRound`.
  Admission now follows Malachite's existing ten-round lookahead. Signatures,
  proposer selection, one-payload-per-round limits, evidence retention, certificate
  rules and acknowledged-payload retention remain enforced. Boundary and adversarial
  tests cover early arrival, forged/conflicting proposals and restart.
* Reproducible process labs add signed non-financial naming workloads, encrypted
  TCP proxies, delays, partitions, SIGKILL, malformed inputs, resource sampling and
  independent authenticated replay. Case directories cannot overwrite old stores.
  The disk-full case fills only an explicitly bounded temporary filesystem.

## Reproduce locally

Use fresh output directories and the local Docker daemon. The wrapper pins its
multiarch image, disables external networking/swap, enforces CPU/memory limits and
records failures as failures. The 100-process cases request 6 GiB for the entire
lab; they are not a successful 2-GiB measurement.

```sh
cargo build --release -p yami-validator --locked
python3 tools/yamichain-container-lab.py --case sustain4 --output work/new-lab
# Other cases: fault4, stress25, stress100, stress100-fault, transitions, disk-full
cargo test --release -p yami-chain-consensus scale_stress_authenticated_core --locked -- --ignored --nocapture
```

See [fuzz instructions](../fuzz/README.md). Keep failed stores for investigation;
never delete safety records to restart a validator or make a benchmark pass.

## Measured workloads

[Machine-readable measurements](evidence/yamichain-stress/measurements.json)
include binary/driver digests, kernel limits, per-node storage/RSS, CPU, network
bytes and time series. Failed campaigns are retained. CPU seconds sum validator
processes; wall time below ends when every node reaches the target. Proxy/monitor
CPU is reported separately. The four-node sustained test executes 256 blocks and
4,096 signed test naming registrations, starting with empty state.

| Run | Result | Time to target | Validator CPU seconds | Sampled RAM | Final store/node |
| --- | --- | ---: | ---: | --- | --- |
| Native ARM64, 4 validators | PASS | 104.20 s | 17.84 | 17.3–17.5 MiB/node | about 3.3 MiB allocated |
| Native x86_64, 4 validators | PASS | 104.23 s | 21.68 | 19.4–20.2 MiB/node | about 3.3 MiB allocated |
| Local x86_64, 4 validators | PASS | 104.58 s | 25.32 | 19.2–19.7 MiB/node | about 3.3 MiB allocated |
| 25 validators, delayed partition/crash after fix | PASS, 24 blocks / 96 actions | 41.37 s | 55.42 | 562.5 MiB combined RSS | see JSON |
| 25 processes, 16 epochs alternating 25/19 members | PASS, 64 blocks / 256 actions | 54.74 s | 104.58 | 587.3 MiB combined RSS | see JSON |
| 4 validators, real ENOSPC/restart | PASS, 48 blocks / 384 actions | 20.88 s | 3.86 | 74.8 MiB combined RSS | see JSON |

The successful rows used **two CPU equivalents total, 2 GiB RAM, no swap**,
enforced and recorded through cgroup v2. ARM64 ran natively on GitHub's ARM runner.
The table uses native artifacts from the lookahead-fix commit `f226924`;
initial native measurements from `71384fc` are also retained. All four final CI jobs (including Linux and Windows) passed in <https://github.com/ItsKorayYT/YamiNet/actions/runs/41476153728>.
The initial CI run passed Linux, Windows, native ARM64 and native x86_64 jobs:
<https://github.com/ItsKorayYT/YamiNet/actions/runs/41469047815>.

Sustained four-node transport forwarded about 15.4 MiB aggregate on ARM64;
loopback RX/TX counters additionally include proxy hops and TCP overhead. This is
not per-node WAN bandwidth. `/proc/io` counters and allocated/logical store bytes
are reported; transient unavailable samples may undercount I/O. Older driver
reports mark availability when any counters were observed; the final driver also
records process counts and marks complete availability only when every launched
process was observed. Cached reads, shared mappings and filesystem allocation mean RSS,
cgroup memory and physical disk I/O measure different things. They are not added.

The initial 25-node partition run stalled at height 4; the same campaign passed
after the lookahead fix. The 100-process full-mesh proxy attempts failed their
deadlines before any finality under **two shared CPU equivalents and 6 GiB**.
An external diagnostic snapshot recorded about 4.14 GiB cgroup peak and heavy
loopback traffic without OOM. The upstream per-IP inbound limit is five (persistent IPs bypass reconnect
throttling, not that cap), while all lab peers share loopback. Each process also
dials all scheduled peers. The proxy, connection churn and shared-CPU topology need investigation;
this does not establish a per-validator hardware requirement. The direct-connect
100-process baseline also failed: all nodes timed out after 300 seconds without a
finalized block. The 100-process crash campaign likewise failed before its
trigger height, so it supplies no crash coverage. Resource counters/time series
are retained separately. No quorum or timeout
failure is counted as a successful benchmark.

Native ARM compatibility and this small-state workload are measured. Raspberry Pi
performance, physical SSD latency/endurance, 20-GB disk capacity, months of hostile
traffic, large-state recovery, 5-GB pruned state and shielded-proof workloads are
**unverified**. The workspace filesystem itself is smaller than 20 GB.

## Parsing, failures and ignored tests

The release core stress harness passed nine genuine signed Malachite schedules:
4/25/100 members × reordered delivery, equivocation, partition/healing. These use
modeled delivery/timers and are distinct from real process/network measurements.
The four-process fault run passed a 25-ms delay per proxy read, no-quorum partition,
healing, SIGKILL/restart and 64 malformed TCP probes. The real ENOSPC run retained
all 13 pre-fault signing reservations unchanged and independently verified recovery. A stronger repeat also required the full-disk
process to exit unsuccessfully before restarting and preserved all 14 existing
reservations; it passed. All [11 original process scenarios](evidence/yamichain-stress/original-processes.json)
also passed again, including epoch-boundary and proposal/vote/commit crashes.

[Fuzz evidence](evidence/yamichain-stress/fuzz.json): 924,772 libFuzzer/ASan runs in
61 seconds, no crash found. Fifteen valid seeds cover eleven wire/WAL codecs plus
blocks, transactions and records. The locked node graph also passes seed
truncations, 20,000 deterministic mutations and maximum-length boundaries. The
nightly fuzz workspace has a separate lockfile; this short campaign is not an audit
or exhaustive coverage. It never bypasses signatures or enables payments.

All nine originally ignored tests were reviewed:

| Tests | Outcome / why still manual |
| --- | --- |
| Browser `capture_discovery`, `capture_verified`, `capture_screenshots`, `capture_sites` | All four passed; write screenshots, some bind fixed local ports. |
| Browser `measure_native_game_rendering` | Passed; manual rendering benchmark, unrelated to chain throughput. |
| Browser `discovered_node_opens_via_verified_tls_and_preserves_navigation`; node `two_discoverable_nodes_an_undiscoverable_node_and_authenticated_fetch` | Both passed on an owned Docker internal LAN with actual multicast/TLS. [Evidence](evidence/yamichain-stress/ignored-lan.json). |
| Browser `browser_opens_owned_remote_site_over_pinned_tls`; node `owned_remote_node_authenticates_and_serves_a_document` | Blocked: no explicitly owned public `YAMINET_TEST_REMOTE_URL`. Public-endpoint checks were preserved. |

The new ignored core scale harness was explicitly run in release mode; its ignore
reason documents the manual cost. Default test runs therefore report ten ignored.

## Dependency blockers

[Fresh cargo-audit result](evidence/yamichain-stress/audit.json): **three unresolved
vulnerabilities**, no suppressions. Official cargo-audit 0.22.2 and RustSec commit
`263a4d951a0867d4bddc29dddd6d59427c78f9a7` scanned the unchanged node lockfile.

| Package | Advisories | Resolution blocker |
| --- | --- | --- |
| Hickory proto 0.25.2 | RUSTSEC-2026-0118, RUSTSEC-2026-0119 | Latest compatible 0.25 release remains 0.25.2; libp2p's DNS/mDNS graph requires `^0.25.2`. Fixed 0.26.3 is incompatible. |
| crypto-bigint 0.5.5 | RUSTSEC-2026-0387 | Latest compatible 0.5 release remains 0.5.5; elliptic-curve/P-256 requires `^0.5`. Fixed, non-yanked 0.7.10 is incompatible. |

[Compatible-series registry evidence](evidence/yamichain-stress/registry-check.json)
and [renewed solver dry runs](evidence/yamichain-stress/dependency-dry-runs.json) reject
both upgrades. Malachite 0.8.2 enables the affected networking features upstream;
disabling application default features cannot remove those unconditional edges.
Remediation needs coordinated upstream upgrades/backports or reviewed feature
changes, not an incompatible crypto override. Existing unmaintained dependency
warnings also remain. Public-network activation remains blocked.

## Safe pruning gate

Finalized-history/snapshot pruning remains **disabled**. Replay budgets refuse
oversized work; they do not skip certificate verification or make a node recover
from an unauthenticated snapshot. Long stalled heights can still grow WAL, signer
reservations, candidates and evidence. Operators must preserve safety files and
provide space; disk pressure fails closed rather than rolling back signing history.

A future pruner requires a reviewed checkpoint/snapshot format binding state,
genesis, height, current/next committees and the authenticated historical transition
chain. It must verify certificates and snapshot contents before atomic activation,
survive crashes at every publication/deletion boundary, and retain a recoverable
previous checkpoint. Historical voting-power verification and the required
evidence/unbonding window must remain available; retention policy needs approval.
Future shielded spent tags require exact permanent uniqueness, including through
snapshots and restores. A probabilistic filter or discarded old tags is insufficient.
No spent-tag implementation or live payment state is introduced here.

The next gates are dependency remediation, 100-process/WAN soak investigation,
reviewed disk-growth limits and pruning/evidence policy, and independent consensus
and cryptographic review. Payment activation needs separate explicit authorization.
