# YamiChain Milestone 16: inactive authenticated V2 reader

Base `f3fb721`, shared branch `development/yamichain`. This implements the
**root-authenticated completion** subset of the activation proposal. It does not
approve or activate production V2 recovery, writing, pruning or higher limits.
No consensus signature, block, transaction, financial-proof policy, flat root,
production checkpoint byte or production limit changes.

## Scope and trust

`yami-chain-consensus/inactive-checkpoint-v2` is off by default. The consensus
module is public operator tooling, with no calls from startup, networking,
finalization, recovery or pruning. Its `VerifiedContext` cannot install a compact
base, seed consensus, update a rollback minimum, prune history or recover an
identity. Completion returns an opaque `InactiveState`, without `Deref`, `State`
extraction or a production import capability. V1 serialization still enforces
all V1 limits.

The operator supplies the exact genesis/schedule and an independently retained
`StateAnchor` minimum. A height-zero minimum must equal the actual genesis state.
The envelope carries a distinct version domain, genesis ID, next-committee hash,
exact existing finalized record/certificate, and bounded manifest with counts,
bytes and transport commitments. `verify_context` verifies the terminal BFT
certificate, rollback floor, next committee and **every consecutive certified
ancestry record** between the independent minimum and target. Height, parent and
terminal tip/root must match exactly. If target equals the minimum, both tip and
root must match. Different valid quorum subsets for an identical block are fine.
No record vector is allocated by the verifier.

Only after this succeeds can callers open state chunks. The peer's manifest and
chunk digests are untrusted framing, **never independent finality authority**.
Changing a framing digest need not fail certificate verification; matching data
must still pass chunk integrity, record ordering, key/role/nonce/label/ownership
and registration-count invariants, complete counts/bytes, and the unchanged flat
root authenticated by the certified header. A peer that rewrites a valid label
and recomputes its chunk hash still fails the final root check. No application
state or success is returned early.

The new codec/store is explicitly forked from M14's tested lab codec to
freeze its bytes and keep both protocols separate. Domains are
`YC-INACTIVE-CHECKPOINT-ENVELOPE-2` and `YC-INACTIVE-CHECKPOINT2-*`, with a dedicated
`inactive-checkpoint-v2.bin` directory guard. The store retains `lab.lock` as
an old-binary refusal fence; current journals also reject the new guard before
creating a lock or touching safety/pending files. Neither codec reinterprets
`YC-LAB-STATE2-*` artifacts. This is an inactive protocol candidate, not a final
approved production envelope specification. Duplication is deliberate; fixes to
shared algorithms must be assessed and tested for each version.

## Bounded import and recovery

Inactive-only caps: 500,000 accounts/names each, 80 MiB payload, 64 KiB manifest,
512 chunks, 256 KiB/4,096 records per chunk. Certified ancestry is at most 8,192
records and 128 MiB; record decoding retains existing record limits. These are
reader experiment budgets, not production capacity approvals. Unsupported
versions and oversized counts fail before application indexes are allocated.
The consensus reader admits one verification/completion/recovery/transfer at a
time per process and releases admission on errors or unwinding.

Transport currently means a finite synchronous reader or dedicated local
filesystem generation, not a new peer-network protocol. The `Read` caller must
supply finite EOF and enforce deadlines. The implemented integration does not
connect an arbitrary blocking reader to a production peer. Service-wide admission
across processes, network deadlines, rate limits, peer penalties and capability
negotiation remain unimplemented gates.

Immutable directory publication pins exact state and transport framing, checks
bounded file namespaces, refuses links/conflicting prefixes, syncs pending files,
renames and syncs directories. Completion is published only after destination
readback rebuilds the state and verifies its root. A crash resumes only identical
prefixes. Recovery rechecks every record/root even with a completion witness.
Conflicting bytes are never overwritten, and failed generations are not silently
deleted. A different manifest requires a fresh operator-selected generation.
Directories must be privately owned: hostile concurrent local filesystem mutation
is outside this finite-file protocol's threat model.

A generation or its local anchor/marker is **not** independent rollback storage.
Callers must reverify the context against independently retained minima on every
restart. This milestone does not implement an atomic installation/independent
minimum-update protocol: it installs no production base. Original signing state,
WAL, archives, irrevocable reservations and identity fences stay in their original
stores. Application checkpoint completion cannot substitute for identity backup.

## Compatibility and validation

Tests cover certificate/ancestry verification across 4→7→4 committee schedules,
wrong genesis, gaps, excess records, lower/conflicting minima, altered certified
records, unknown domains, framing changes and self-consistent hostile rehashes.
V1 and inactive V2 readers reject each other's envelopes; V1 pruned recovery
continues unchanged after V2 inspection. A verified V1 compact floor plus only
its retained certified suffix also authenticates V2; a missing first suffix record
is rejected. The existing archive/pruned peer
regression remains mandatory. This is reader-level compatibility, not negotiated
mixed-version V2 peer networking or permission for old pruned readers to exceed
100k entries.

An actual previous validator built from `f3fb721` and a current validator also
exercise observer archive inspection, byte-identical V1 exports and cross-version
V1 pruned imports/inspection in both directions. Both refuse new inactive
envelopes and generation directories without changing source bytes. The retained
`lab.lock` fence makes the previous binary refuse before journal writes, despite
not knowing the new guard. The old writer still refuses a 250k V1 export and
preserves its complete archive. The standalone inactive reader also rejects a
real lab store paired with an otherwise valid new envelope. These eight process
cases pass; [the compatibility evidence](evidence/yamichain-m16-compat.json)
records the exact previous/current/reader hashes. These are offline binary/reader
checks, not a negotiated V2 wire protocol.

The inactive codec separately runs canonical roundtrips, malformed records,
missing/reordered/duplicate chunks, links, conflicting retries, simultaneous store
readers and abrupt child-process exits at file sync/rename/directory-sync
boundaries. ENOSPC tests use actual bounded tmpfs block and inode exhaustion.
These simulate process crashes/filesystem pressure, not physical power loss.

`large-state-bench`, when both lab and inactive-reader features are selected,
adds a separate inactive profile to the existing complete signed-history fixture
campaign. It exports a new distinct format, verifies every certified ancestry
record from genesis, then measures full recovery and transfer/readback. It never
converts experimental lab files into the inactive format. The source is an
executed complete history, not an invented state map. `tools/yamichain-v2-integration.py`
freezes the binary and runs sequential network-isolated containers with two CPUs,
2 GiB RAM and no extra swap. Fixture throughput is not live-engine finality.

Formatting, strict Clippy with default and combined lab/inactive features,
workspace release build and regressions pass: **556 default tests, 10 existing
ignores; 81 combined-feature tests, one existing ignore; six Python tooling
tests**. The default privacy boundary exhaustively rejects all 65,536 proof
versions. Twelve new codec child-process crash cases (six boundaries each for
export and transfer), the existing lab crash cases and actual tmpfs block/inode
exhaustion pass. Final checks and source/binary/log hashes are retained in
[the validation evidence](evidence/yamichain-m16-checks.json); actual pressure
results are in [the fault evidence](evidence/yamichain-m16-faults.json).

Validation results and measurements are recorded in the accompanying evidence.
Page cache is not dropped: recovery results are fresh-index measurements, not
cold-device SLAs. Whole-process CPU/RSS and cgroup memory include generation,
retained branches, lab codec work and archive replay as well as the new reader;
phase elapsed times are reported separately.

## Signed-history measurements

The complete distinct-account fixtures pass at exactly 250,000 and 500,000 signed
registrations, including partial terminal blocks. Both preserve all source
archives and fixture signing reservations. V1 returns its existing `Error::Limit`
at both sizes; no larger V1 checkpoint is installed. The cases have distinct
pinned genesis schedules and are controlled quorum-certified fixtures, not a live
Malachite throughput claim.

| Entries/accounts each | Certified ancestry s | Inactive recovery s | Transfer/readback s | Whole-campaign CPU s | Peak RSS MiB | Cgroup peak MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 250,000 | 1.23 | 1.78 | 1.90 | 215.48 | 139.5 | 290.4 |
| 500,000 | 2.52 | 3.75 | 4.18 | 795.60 | 274.9 | 569.4 |

These phase times use the same process with freshly rebuilt indexes and warm OS
page cache. Verification of certified ancestry is separate from recovery; it must
be included when the receiver's independently retained minimum precedes the
checkpoint. Whole-campaign CPU and memory include fixture construction, signing,
retained branches, both codecs and full archive replay. At 500k that replay alone
takes 227.75 seconds, and construction takes 602.36 seconds. New-height hashing
still costs about 53.7 ms; this milestone does not solve the flat-root replay cost.
Raw measurements and the frozen binary hash are in
[the signed-history evidence](evidence/yamichain-m16-history.json).

## Fresh-process recovery

A separate inactive-only fixture reader uses an **exact independently retained
certified target** from the preceding completed local campaign, not a peer's
manifest. It verifies the existing certificate against that target before reading
chunks. This measures restart from that trusted checkpoint minimum, not genesis
bootstrap without ancestry. The API still requires the full certified suffix
when the independent minimum precedes the target.

All cases run under one aggregate two-CPU/2-GiB limit, no network and no additional
swap, with read-only source mounts. Every process rebuilds its own indexes and
root. OS page cache remains warm. Individual peak RSS is distinguished from
sampled aggregate RSS and the cgroup peak.

| Entries each | Fresh processes | Elapsed s | Combined CPU s | Sampled combined RSS MiB | Cgroup peak MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| 250,000 | 1 | 1.81 | 1.80 | 80.3 | 88.4 |
| 250,000 | 4 | 3.85 | 7.59 | 321.0 | 322.0 |
| 500,000 | 1 | 3.79 | 3.78 | 157.4 | 165.8 |
| 500,000 | 4 | 7.99 | 15.80 | 586.3 | 588.8 |

The quiet final run completes four 500k recoveries in 7.99 seconds with a
588.8-MiB cgroup peak. Another guarded run during background validation took
12.03 seconds; both are retained in [the reader evidence](evidence/yamichain-m16-readers.json).
Elapsed time and overlapping memory residency depend on scheduling. Neither
result approves unlimited concurrent imports or a production service budget.

Signed-history generation/phase timings use the earlier frozen binary recorded
in the history evidence. A final hardening change makes the direct transport
helper reject oversized slices before hashing; its consensus envelope callers
already bounded those slices. Final fresh readers, pressure/compatibility tests
and the real live-history audit use the guarded reader. No codec bytes, roots,
signatures or workload algorithm changed. The final live-audit binary also stops
the lab-recovery stopwatch before the separate inactive audit. Source, binary and
log hashes distinguish these artifacts explicitly.

## Real live-engine history and final reader audit

Four separate Malachite validator processes finalize **3,907 blocks and 250,048
signed registrations with distinct accounts** under a private pinned roster.
Each has a two-CPU quota and 2-GiB memory limit, no extra swap; all share CPUs 0–1,
with builds/compatibility checks on CPU 2. Only each validator's own private
identity and public roster/bootstrap files are mounted. No public service is
exposed. The terminal height, tip and root agree after authenticated full replay
of all four original stores.

At heights 1,564–1,565, disconnecting two validators removes quorum for a
12-second observation window; one in-flight block completes. Healing restores
progress. Node 0 is actually SIGKILLed at height 1,568. Restart retains its original
identity, WAL, signing logs and independently observed signing minimum. It
catches up and continues with the other validators; the independently captured
signing prefix remains intact.

First-to-last finalized-event observations on an uninterrupted validator span
1,601.40 seconds, approximately **156.10 registrations/s**, including the fault
window. Median inter-finalization interval is about 403 ms, p95 about 407 ms.
These are private finalized-event observations, not transaction-submission or WAN
latency guarantees. The three uninterrupted processes peak at roughly 75.6–76.6
MiB RSS and consume 162.36–194.06 CPU seconds each. The killed attempt has no final
`getrusage`; node 0's retained resource summary covers only its restarted attempt.
Final host inspections take about 61–62 seconds per archive and are not claimed
as cgroup-constrained measurements. The complete campaign, including linger and
four inspections, takes 1,881.41 seconds. Raw limits, observations, faults and
verified states are retained in [the live evidence](evidence/yamichain-m16-live.json).

A final network-isolated audit of that genuine engine history enforces one
aggregate two-CPU/2-GiB budget. It first replays against the independently retained
live target, then exports a new inactive generation and verifies the entire
certified ancestry, recovery and transfer/readback. It does not translate lab
files. Inactive ancestry verification takes **1.03 s**, recovery **1.78 s**, and
transfer/readback **1.79 s**. The complete replay plus both codec audits takes
72.07 seconds, 72.00 CPU seconds, about 135.3 MiB peak RSS and 213.2 MiB cgroup
peak. Every byte of all 28 original node-0 archive, WAL, signing and metadata
files is unchanged. Application outputs are separate tmpfs siblings, never
installed as a production base. [The final audit evidence](evidence/yamichain-m16-live-audit.json)
records its exact independently retained target, source digest and final binary.

There is **no 500k live-engine finality claim**, nor a large live committee
transition or simultaneous hostile-import campaign. The exact 500k result is a
complete genuinely signed/quorum-certified fixture and fresh reader campaign.

## Independent review and remaining activation gates

The inactive subset is suitable for independent code/security review after its
recorded checks pass. The complete production V2 integration is **not ready for
activation**. Remaining gates include:

- Final envelope/canonical vectors and fuzzing; externally reviewed trust API,
  admission and numerical capacity/work budgets, including maximum labels.
- Real large live-engine histories under simultaneous import/Byzantine traffic,
  committee transitions, partitions and larger quorum/fault cases. Offline
  quorum-certified fixtures do not discharge these requirements.
- Explicit network capability negotiation, deadlines, peer admission/penalties,
  aggregate multi-process/fork budgets and mixed-version bootstrap/downgrade
  campaigns. Old V1 pruned readers still cannot import larger states.
- Reviewed generation-installation and independently anchored monotonic rollback
  update protocol; physical power-loss, backup/restore and hardware/filesystem
  drills. No copied local anchor can discharge this gate.
- Checkpoint refusal/retention policy beyond V1 capacity, operator migration and
  quorum readiness. No silent state-growth admission or storage-mode change.
- Recovery/synchronization targets and an explicit decision on linear-per-height
  flat-root hashing and approximately quadratic growing-history replay.
- Separate explicit authorization for any production writer, recovery or pruning
  activation. Payments, staking, public deployment and all 65,536 financial proof
  versions remain disabled/rejected.

## Reproduction

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --locked \
  --features yami-chain-consensus/large-state-lab,yami-chain-consensus/inactive-checkpoint-v2 -- -D warnings
cargo build --workspace --release --locked
cargo test --workspace --release --locked -- --test-threads=1
cargo test -p yami-chain -p yami-chain-consensus --release --locked \
  --features yami-chain-consensus/large-state-lab,yami-chain-consensus/inactive-checkpoint-v2 -- --test-threads=1
cargo build -p yami-chain-consensus --example large-state-bench --release --locked \
  --features large-state-lab,inactive-checkpoint-v2
python3 tools/yamichain-v2-integration.py --binary target/release/examples/large-state-bench \
  --output NEW_DIRECTORY --sizes 1024 250000 500000 --cpuset 0,1
cargo build -p yami-chain-consensus --example checkpoint-v2-reader --release --locked \
  --features inactive-checkpoint-v2
python3 tools/yamichain-v2-readers.py --binary target/release/examples/checkpoint-v2-reader \
  --history COMPLETED_LOCAL_HISTORY_DIRECTORY --output NEW_READER_DIRECTORY --cpuset 0,1
python3 tools/yamichain-v2-faults.py --test-binary FEATURE_CHAIN_TEST_EXECUTABLE \
  --output NEW_FAULT_REPORT.json --cpuset 0,1
python3 tools/yamichain-v2-compat.py --previous PREVIOUS_F3FB721_VALIDATOR \
  --current target/release/yami-validator --reader target/release/examples/checkpoint-v2-reader \
  --history COMPLETED_LOCAL_HISTORY_DIRECTORY --output NEW_COMPAT_DIRECTORY
python3 tools/yamichain-large-live.py --binary target/release/yami-validator \
  --output NEW_LIVE_DIRECTORY --height 3907 --fault-height 1563 --timeout 3300 --cpuset 0,1
python3 tools/yamichain-v2-live-audit.py --binary target/release/examples/large-state-bench \
  --source COMPLETED_LOCAL_LIVE_DIRECTORY --output NEW_LIVE_AUDIT_DIRECTORY --cpuset 0,1
python3 -m unittest discover -s tools -p 'test_yamichain*.py'
```

Reserve a different CPU for builds while measuring on CPUs 0–1; do not overlap
measured campaigns. Resource caps are enforced by the existing managed Docker
runtime. Development dependencies/toolchain setup is separate from these timing
measurements.
