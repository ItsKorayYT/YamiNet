# YamiChain Milestone 10: long-term storage reliability

This milestone retains archive mode as the default and all validator signing
reservations/equivocation evidence. It does not establish indefinite operation
on a 20-GB disk. No YPP, staking, monetary or production-election policy changes
are included. Independently pinned fresh-import anchors, certificate verification,
execution, durable receipts and minimum checkpoint rollback markers remain required.

## Changes and safety argument

The `prune_certified_before` path used by automatic pruning and the benchmark
now captures its requested floor during one complete authenticated replay, rather
than executing/verifying the prefix twice. The captured state is private
intermediate data: a failure anywhere in the suffix, receipt checks, directory gap
checks or replay budgets aborts before checkpoint publication or deletion. Export
still verifies the produced checkpoint against the captured anchor. Base fsync,
marker fsync and exact installed-base verification still precede every history
unlink. Replay serializes each record once for byte-budget and receipt checks.
No record verification is cached or omitted. The offline `prune` CLI still
performs a separate authenticated head lookup before invoking this path; its
extra replay remains a maintenance performance cost.

Pressure enforcement now checks both logical bytes and Unix allocated file blocks,
rounding the incoming write up to at least a 4-KiB allocation unit. It reserves two
directory entries before new writes and refuses at fewer than 128 available inodes
on filesystems that expose inode accounting. Discovery freshness checkpoint writes
now also pass the journal pressure check. `storage-info` reports allocated bytes.
A validator resuming work checks pressure before starting network actors or WAL
recovery; repeated pressured restarts preserve the original WAL/reservations and
return the quota error before trying to recover a malformed WAL.
Existing identical reservations still fsync successfully at pressure; conflicting
preimages still fail. Allocation, create, write and sync errors propagate without
releasing a new signature. These are preflight refusal bounds, not an atomic OS
quota; WAL and external writers can race checks. The startup refusal is not a WAL
compactor or a proven recovery-memory bound for every large within-quota WAL;
those adversarial WAL sizes are not covered by these measured fixtures. Provision
a filesystem quota or isolated volume to enforce a hard 20-GB maximum. File-block accounting excludes
directory, inode, filesystem journal and other metadata costs; sparse files are
also checked by logical length. Non-Unix platforms retain logical byte checks.

## Growth audit and remaining limits

| State | Policy / remaining growth |
| --- | --- |
| Signing reservations | One immutable preimage per signed height/round/step, including unsuccessful rounds. Never deleted. O(number of signing decisions), plus one inode/block per file. No time-based expiry is safe: a restarted old key must still refuse conflicting historical messages. |
| Equivocation evidence | Immutable authenticated conflicting-message evidence; retained without age-based deletion. Grows with observed conflicts. Finality alone does not authorize erasure. |
| WAL | Malachite 0.8.2 `wal/thread.rs` resets when starting a different height; `wal/log.rs::reset` truncates to its header and syncs. Ordinary growth is per current height, not total finalized history. Repeated activity/stalls at one height remain pressure-relevant. This milestone neither rewrites nor deletes WAL bytes. |
| Archive records and receipts | O(finalized heights + transaction payload bytes), with full verification on recovery. |
| Pruned records | N..2N-1 suffix records between automatic checkpoints, plus one compact base and marker. Safety files remain. |
| Candidates/proposals | Finalized payloads can be reproduced and are removed only at authenticated finalized heights. Unresolved data is retained subject to existing admission/refusal limits. |
| Application state | Registry and sender nonce state grows with workload. Compact base is capped at 17 MiB; state snapshot at 16 MiB / 100,000 entries per collection. Exceeding a bound refuses; it does not discard state. |
| External checkpoint exports / lab artifacts | Each export is size-bounded, but operator-created copies and external output directories have no global retention quota. They remain operator-managed and are not silently deleted to reclaim space. |
| Directories | Flat root still grows with reservations/evidence/archive. The 4,000,000-entry refusal/recovery bound is not a retention policy. Full pressure scans remain O(entries) per write; generation has quadratic cumulative scan work. |

No signing-log migration, segmentation, cached accounting or evidence compaction
is introduced. A rigorous design would need to preserve exact key/domain/height/
round/step-to-preimage bindings, crash-atomic migration, durable completeness and
rollback detection, legacy-store compatibility, and conflict refusal after every
interruption. A compact file without a durable completeness witness could silently
lose a reservation. Retaining the current files is safer than claiming that such a
migration has been proved here. The existing finite demonstration height/round
limits are additional refusal bounds, not long-term capacity evidence.

## Reproduction

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --release
cargo test --workspace
cargo build --release -p yami-chain-consensus --example storage-bench
python3 tools/yamichain-storage-lab.py --output work/m10/process \
  --height 512 --restart-count 3 --restart-step 64
python3 tools/yamichain-storage-bench.py --output work/m10/bench \
  --heights 1024 4096 8192 --process-lab-output work/m10/process
```

The benchmark uses real signatures, pinned 4→7→4 committees and eight naming
transactions per block. It generates an archive and then prunes once near the
tip; the separate 704-height live campaign exercises sustained automatic pruning.
It closes/reopens archive and pruned journals five times each, compares complete recovered states and checks historical conflict refusal.
It reports both logical and allocated bytes, entry counts, recovery times, CPU and
RSS under two-CPU/2-GiB network-disabled containers. The private process lab uses
four independent validators, archive and pruned stores, independently anchored
bootstrap, three restarts with advancement, and interrupted base/unlink/export/
import processes. Additional tmpfs tests exercise actual block exhaustion and
inode pressure with committed-file hashes checked before/after refusal. Raw
identity material and stores remain in ignored `work/`; public evidence is key-free.

## Validated process and regression results (2026-11-03)

All **516 workspace tests passed**, with zero failures and 10 existing
manual/environment ignores. Strict workspace/all-target Clippy and formatting
passed. The non-test privacy boundary integration test still rejects all 65,536
production proof versions and public financial actions. Three Python tooling
regressions also passed. The pinned toolchain is Rust 1.90.0. Missing ALSA/X11
build headers were supplied in a local sysroot; no repository dependencies or
system security settings were changed.

The private four-validator process campaign passed in **442.39 seconds**.
A never-used fourth identity imported an independently anchored **14,330-byte
checkpoint at height 64**, joined the certified suffix and converged with archive
and pruned validators at **512**. Three complete restart-and-advance cycles reached
**576, 640 and 704**, comparing all previously retained signing/evidence hashes
after each cycle. Offline exits at base publication, first unlink, uncommitted
export and import recovered with authenticated state and preserved safety data;
the offline WAL comparisons were byte-for-byte. Live WALs at 704 were 2,016 bytes.

| Validator at 704 | Logical bytes | Allocated file bytes | Entries | History floor |
| --- | ---: | ---: | ---: | ---: |
| Archive | 1,185,845 | 12,468,224 | 3,046 | 1 |
| Pruned 1 | 439,932 | 7,102,464 | 1,703 | 673 |
| Pruned 2 | 444,056 | 7,208,960 | 1,729 | 673 |
| Late-joining pruned | 410,424 | 6,295,552 | 1,506 | 673 |

Every pruned validator retained its original safety records, one compact base and
32 recent finalized records. The late-joining identity signed fewer historical
heights. No equivocation was generated by this normal convergence workload;
corrupt/equivocating-message and evidence-retention tests are separate regressions.
Peak sampled validator CPU was 22.38% of one core. The host itself had a two-core
quota shared with builds during this correctness campaign, and each validator had
a two-CPU/2-GiB container limit. This is not a throughput or dedicated SSD claim.
An initial short-deadline rehearsal was deliberately stopped before completion;
its raw stores remain private. Harness deadlines now scale with requested history.
Only rebuildable compiler incremental caches were removed to recover build space.

Public key-free process evidence: [restarts and interruptions](evidence/yamichain-m10-process.json).

## Long-history measurements and target assessment

All long-history cases passed, including five close/reopen cycles per mode,
complete state equality and historical conflicting-preimage refusal after restart.
Every container reported an actual `200000 100000` CPU quota and `2147483648`
memory limit. There were no background builds or live validators during this
benchmark. The filesystem fragment size was 4,096 bytes. These are shared managed
host/container filesystem measurements, not dedicated physical SSD latency tests.

| Heights | Archive logical / allocated MB | Pruned logical / allocated MB | Archive replay | Pruned journal reopen + replay mean (8-record tail) | Peak RSS MiB |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,024 | 2.427 / 12.587 | 0.596 / 4.686 | 1.791 s | 24.80 ms | 16.21 |
| 4,096 | 9.733 / 50.336 | 2.360 / 18.547 | 15.541 s | 81.16 ms | 32.85 |
| 8,192 | 19.473 / 100.667 | 4.711 / 37.028 | 94.085 s | 226.92 ms | 62.84 |

CPU totals cover generation, the initial recovery, five additional archive
reopens, export/import, pruning and five additional pruned reopens. They are not
per-block CPU costs. All times below are measured wall times except CPU columns.

| Heights | Generation | Checkpoint export | Prune | CPU user / system seconds | Entire campaign |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1,024 | 15.92 s | 1.83 s | 1.88 s | 20.89 / 7.30 | 31.02 s |
| 4,096 | 221.29 s | 16.90 s | 18.28 s | 198.42 / 141.24 | 352.22 s |
| 8,192 | 913.90 s | 86.71 s | 97.99 s | 1026.75 / 593.46 | 1651.97 s |

At 8,192 heights the compact snapshot was **3,401,394 bytes**.
Fresh import took **77.36 ms** and compact recovery
without a tail took **79.25 ms**. All
**1,294,336 logical signing bytes** remained after pruning.
Five archive journal reopens averaged **91.527 s**;
five pruned journal reopens averaged **226.92 ms**.
These journal/state measurements exclude OS process startup, networking and WAL
recovery, which the separate live process campaign exercises. Archive/pruned
directories retained **24,578 / 8,212 entries**. The fixture deliberately retains
only one precommit reservation per height; live validators retain both votes and proposals and can consume much more
space, as the 704-height process data demonstrates.

Filling an isolated **64-MiB tmpfs** to actual block ENOSPC made pruning refuse
without changing committed-file hashes. A separate **256-MiB tmpfs with 16,384
inodes** tested refusal at 127 free inodes, then filled every remaining inode to
actual ENOSPC and tested refusal again. Both refusals preserved all committed
files. The workspace release build copied into the benchmark performed these
filesystem-pressure checks. After the startup-only fix, the final workspace
release binary reopened all four 704-height stores with identical states and recovered/completed
an interrupted prune while preserving 1,631 safety/WAL
files. Three real-process restarts with a malformed WAL and an exhausted quota
then refused new work without altering the pressure fixture safety data.
Executable hashes are recorded per campaign/build: workspace feature unification
and the final startup-only refusal fix produce different binaries. The benchmark's storage/checkpoint/example
code stayed unchanged during the final startup-only refusal fix; its recorded
executable hash identifies the actual measured artifact.

**The 20-GB target is not established as sustainable long-term storage.** It easily
holds these finite fixtures, and these measurements fit the two-core/2-GiB limits,
but that is not indefinite-operation evidence. One tiny reservation per height
alone consumes about 4,096 allocated bytes/height on this filesystem. At a
hypothetical 2.25 reservations/height (two votes plus a proposal in one quarter of
heights), safety files alone need about **9.216 GB per million heights**, before
evidence, state, history, directory metadata and extra rounds. Archive history
adds at least two more file allocations per height in this fixture, about
**8.192 GB per million heights**. These are conditional allocation estimates,
not a tested million-height campaign or a block-time/lifetime prediction.
The four-million-entry limit, available inode count and existing finite lab
height/round limits may refuse earlier than the byte budget. For example, a
hypothetical 20-GB filesystem formatted at one inode per 16 KiB has only about
1.22 million inodes; verify actual inode provisioning rather than assuming byte
capacity implies enough files. This is an allocation estimate, not a formatted
20-GB-volume test.

The eight-new-names-per-block fixture has 65,536 names at 8,192 heights. Continuing
that workload would cross the snapshot's 100,000-name collection bound around
12,501 heights, regardless of spare disk; exporting/pruning would safely refuse.
The single registry sender in this fixture does not exercise nonce-account growth.
State growth and repeated full directory/metadata scans remain material CPU and
recovery costs. Proven safety-log packing/indexing, bounded active-state policy,
hard filesystem quotas and a substantially larger endurance campaign are still
needed before making a long-term hardware promise. No production feature or
public deployment is enabled.

Public key-free evidence: [checks and final release recovery](evidence/yamichain-m10-checks.json),
[long-history CPU/RSS/storage and exhaustion](evidence/yamichain-m10-bench.json),
and [live restarts and interruptions](evidence/yamichain-m10-process.json).
