# Milestone 6 — non-financial infrastructure hardening

Baseline: `ba3d3b3708db42e86d2e6370ef5cb76c0aa5ac2e`, on the existing
`development/yamichain` branch. No financial features, production proof versions,
YPP experiments, block formats, signing domains, quorum thresholds or committee
selection rules change. No public deployment. The node remains a loopback-only
development-key Malachite runner, not a production permissionless cryptocurrency.

## Existing infrastructure audited

The existing integration already uses Malachite 0.8.2 with independently verified
commit certificates, height-specific pinned committees, bounded transport/sync,
durable signing preimages, immutable finalized records, exclusive store locks,
candidate retention and process crash/partition/catch-up harnesses. These are
preserved. Genuine permissionless eligibility and committee elections remain
disabled: unauthenticated weights or a peer-discovered identity must not grant
voting authority. Scheduled validators may join, synchronize, activate and retire
according to the pinned genesis; discovery is not an admission/election rule.

## Changes

### Detect lost or replaced acknowledged history

Previously, deleting the last finalized record left a shorter contiguous history;
replay could report that prefix as the tip. `Journal::append` now syncs the
immutable record and then an immutable `receipt-HEIGHT.bin`, containing a
domain-separated hash of its complete canonical bytes. Success requires both
writes. Replay compares receipts with independently verified records and rejects
receipts above the recovered height, malformed receipt filenames and corrupt
receipt bytes. A missing tail cannot silently become a successful shorter replay
while its receipt survives.

An older store without receipts remains compatible. Only after all certificates,
transitions and directory validation succeed does replay seal its recovered tip.
An interruption between record and receipt leaves a complete record that replay
must authenticate normally before sealing. Existing idempotent writes re-sync
uncertain renames; neither receipts nor finalization bypass certificate checks.
Unix store initialization also syncs ancestor directories so a newly created
store's directory entries are durably published, not just its own contents.

This is local corruption/truncation detection, not protection from an adversary
who rolls back/deletes the complete store and all receipts. Receipts are not
consensus certificates or external anti-rollback hardware. Keep full backups,
WAL, signing reservations and evidence. Filesystem power-loss behavior and
non-Unix directory durability still require platform qualification. Receipts add
one 32-byte logical file per finalized block, with filesystem allocation/inode and
sync overhead; the existing directory budget includes those files. Replay byte
budgets continue to count canonical history payloads, with separate bounded
directory-entry and fixed-size receipt checks.

### Revalidate cached proposals and transport metadata

A cache hit previously accepted an identical retransmission without rerunning
the authorization/execution checks used on first admission. Disk cache metadata
is not an authentication capability. Identical retransmissions and evidence
sources now require valid original signatures, scheduled proposer, height/round
metadata and state execution. Corruption fails closed rather than becoming a
`Valid` proposal or poisoned equivocation evidence.

The node now enforces its existing publication convention: a single sequence-zero
data part has a stream ID exactly equal to its block ID. Empty, wrong, truncated
or oversized IDs, resequenced payloads and terminal parts do not reach candidate
admission. This tightens the application transport boundary; all existing honest
publish/restream paths already use this convention. Block validity, certificate
verification and wire encodings remain unchanged.

### Bounded, verified history snapshot export and resumable restore

New library APIs `export_history_snapshot`, `history_snapshot_digest` and
`restore_history_snapshot` supplement the existing inventory with actual
transportable history. Limits: 1–1024 records, at most 64 MiB encoded, existing
per-record caps. The snapshot binds genesis, height, tip, state root and next
committee, followed by the complete canonical certified history.

The importer checks an independently pinned transport digest, every certificate
under the height's committee, every state transition, terminal commitments and
exact framing before installing any finalized record. It rejects a conflicting
local prefix and rollback to a shorter history. Borrowed archive slices avoid
another full-history payload copy. Installation uses the normal durable append
path and can resume idempotently from a certified prefix.
Different valid quorum certificates for the same prefix block are compatible;
the importer retains the destination's original certificate bytes and receipts.
Restore requires an exclusive mutable journal borrow, preventing concurrent node
appends while validating and installing the archive.

Atomicity is **per certified record**, not all-or-nothing replacement of a whole
database. An interrupted installation can leave a valid prefix and never returns
success for a partial import. This is intentionally full-history restoration,
not a compact state bootstrap or pruning authorization. There is no new public
RPC or CLI import surface. The caller must bound downloads before allocation;
hashing a download and accepting a hash from that same untrusted response does
not authenticate the latest head. All certificate/execution checks still run.
Snapshots exclude signing keys, identity reservations and WAL; they must not be
used to clone/recover an existing validator identity without its entire safety
store.

### Enforced benchmark resource limit

`tools/yamichain-stress.py --address-space-mib 2048` launches and restarts each
validator through Linux `prlimit`, setting both soft and hard `RLIMIT_AS` before
exec. The report hashes the actual validator binary and records the limit.
Invalid limits or a missing limiter fail before a campaign. A child-process test
checks the actual inherited kernel limit. Virtual address space is not RSS or a
per-validator cgroup limit; the report distinguishes these explicitly.

## Adversarial coverage

New Rust tests cover acknowledged tail deletion, replacement of otherwise
decodable/executable records, corrupt/oversized/noncanonical receipts, deferred
legacy migration, receipt publication crashes at pre-file-sync/post-rename/
pre-directory-sync boundaries, restart and idempotent retry; cached signature,
proposer and execution corruption; misleading proposal stream metadata; and
snapshot wrong-genesis/digest, truncation, trailing bytes, certificate corruption,
forged terminal state, excessive record counts, resume across committee changes,
idempotency, rollback and conflicting certified local history. Child crash tests
exit without destructors at actual file-write hooks. They simulate process death,
not loss of a physical disk's volatile write cache.

Existing fork/conflicting-certificate, quorum loss, authenticated message
reordering, validator rotation/failure, signing safety, proof rejection and
network partition/catch-up/restart regressions remain required.

## Validation and measurements

Final validation used Rust 1.90.0 with locked dependencies:

* `cargo fmt --all -- --check`: pass.
* `cargo clippy --workspace --all-targets --locked -- -D warnings`: pass.
* `cargo test --workspace --locked`: **492 passed, 10 existing ignored**.
* Release tests for chain, privacy, naming, storage, consensus and validator:
  **42 passed, one existing manual scale test ignored** in the normal suite.
* That manual scale test was then run explicitly in release: all **12 schedules**
  pass (4/25/50/100 validators, reordered messages, malicious participant and
  partition recovery). [Raw core results](measurements/milestone6-core-schedules.txt).
  These schedules model timers/I/O; they are not full-node resource measurements.
* `cargo build --workspace --release --locked`: pass.
* Python harness tests: **3 passed**, including actual child `RLIMIT_AS` checking.
* Existing real-process local suite: all **11 scenarios pass**, including normal,
  one offline validator, quorum halt, late catch-up, crashes after proposal/vote,
  pre-commit crash, seven validators, rotation, epoch-boundary crash and rotating
  catch-up. [Raw process results](measurements/milestone6-recovery.json).
* Chain-independence check and unchanged production privacy/experiment/manifest/
  lockfile comparisons: pass. The non-test privacy library rejects every one of
  65,536 proof versions for both payments and value-bearing protocol actions.

The wider GUI workspace needed ALSA development metadata absent from this VM.
Validation used the official Debian package extracted under scratch, linked to
the existing runtime library; no repository build workaround or system package
installation was added. No physical audio hardware was available. Dev/test debug
symbols and incremental compilation were disabled locally to control disk usage;
debug assertions remained enabled.

Complete-node runs used release binaries on Linux x86_64, AMD EPYC 7763, shared
`cpu.max=200000 100000` (two CPU equivalents), host `memory.max=8589934592`, and
an enforced per-validator 2-GiB virtual-address-space limit. No unrelated builds
or test suites ran during the three resource campaigns. Four real Malachite
processes executed and independently replayed the non-financial naming workload.
All three campaigns passed with identical finalized roots across their nodes.

| Campaign | Blocks / transactions | Seconds to all nodes at target | Total validator CPU seconds | Largest sampled node peak RSS | Four-node sequential authenticated replay |
|---|---:|---:|---:|---:|---:|
| Direct loopback | 64 / 1,024 | 25.984 | 4.876 | 17.25 MiB | 0.548 s |
| Partition + SIGKILL/restart + malformed TCP | 64 / 1,024 | 38.097 | 5.009 | 17.98 MiB | 0.541 s |
| Maximum 64-transaction blocks | 128 / 8,192 | 52.031 | 20.744 | 20.44 MiB | 4.096 s |

The maximum-block run used 3,293,184–3,305,472 allocated store bytes per node
(1,579,151–1,579,662 logical bytes). The fault campaign split the network at
2.559 s, observed no-quorum height 5, healed it at 9.631 s, killed one process,
and restarted its complete store at 11.654 s. It completed 64 malformed TCP
probes (1,060,992 bytes drained to sockets) and then finalized all 64 blocks.
Transport rejection is not evidence that these payloads reached a decoder.

These are short loopback naming benchmarks, not payment/proof workloads,
saturation throughput, a WAN deployment, large-state growth or pruning tests.
The approximately 400-ms configured finalization delay constrains observed block
rates. The available filesystem is an approximately 7.9-GB overlay, not a
qualified 20-GB SSD. Physical SSD latency, long-term capacity/inode exhaustion,
power-loss durability and native ARM64 remain unverified. No claim that the full
production hardware target is met follows from these measurements.

Raw reports include binary/driver SHA-256, host limits, sampled memory, I/O
availability, storage allocation, transport counters, progress and finalized
roots: [direct](measurements/milestone6-performance.json),
[adversarial](measurements/milestone6-adversarial.json), and
[maximum blocks](measurements/milestone6-max-blocks.json).

Reproduce from the repository root after a locked release validator build:

```sh
python3 tools/yamichain-stress.py --directory work/m6-performance \
  --report work/m6-performance.json --height 64 --transactions 16 \
  --validators 4 --timeout 120 --direct --address-space-mib 2048
python3 tools/yamichain-stress.py --directory work/m6-adversarial \
  --report work/m6-adversarial.json --height 64 --transactions 16 \
  --validators 4 --timeout 180 --faults partition,crash,socket-fuzz \
  --address-space-mib 2048
python3 tools/yamichain-stress.py --directory work/m6-max-blocks \
  --report work/m6-max-blocks.json --height 128 --transactions 64 \
  --validators 4 --timeout 180 --direct --address-space-mib 2048
python3 tools/yamichain-local.py --binary target/release/yami-validator \
  --report work/m6-recovery.json
```

Use fresh campaign directories. The driver refuses reuse so old records cannot
masquerade as new progress. Every successful campaign independently authenticates
each final store; successful process exit alone is insufficient.
