# YamiChain Milestone 12: segmented archive storage

Base: `b25d124`, permanent branch `development/yamichain`. This is an explicit
local storage upgrade. Consensus encodings, genesis IDs, certificates, block IDs,
state roots and application-state limits are unchanged. YPP, financial actions,
staking and production elections remain disabled. Archive remains the storage
mode that retains all finalized history.

## Operator workflow and compatibility

Stop the node, preserve its complete store, and use the same pinned roster or
committee plan as the node:

```sh
yami-validator migrate-archive --dir STORE --index N --isolated-network CONFIG
yami-validator verify-archive --dir STORE --index N --isolated-network CONFIG \
  --archive-anchor HEIGHT:ROOT
```

`migrate-archive` also initializes a fresh empty archive. Run it before starting a
new archive node to avoid creating a legacy prefix. Migration is opt-in: stores
that have not been upgraded continue using the existing format. Pruned stores
continue using their existing compact base and bounded legacy tail; attempting to
migrate a pruned store refuses without changing its base or history.

The command prints the verified current archive height/root. Retain that minimum
**outside** the storage being backed up or restored, and supply
`run --archive-anchor HEIGHT:ROOT` on restart. The check precedes WAL recovery and
consensus work. Keep the existing independent signing anchor as well; an archive
anchor does not replace signing reservations or their anchor. Refresh anchors as
history advances; a stale anchor protects only its prefix.

An upgraded archive can serve authenticated historical synchronization and export
both historical and compact checkpoints through the existing verification paths.
It refuses in-place pruning before publishing any compact base. To obtain a
pruned node, export a compact checkpoint and import it into a fresh store using
an independently obtained checkpoint anchor and a fresh validator identity. This
is not a way to restore a used identity without its WAL, reservations or evidence.

## Representation and verification

A frame contains the unchanged canonical finalized record (block and certificate),
its height and bounded byte length, and the existing durable-record receipt hash.
Frames are appended to canonically numbered segments of at most 4 MiB. The tip
commits to height, segment/end position, and a domain-separated hash chain over
every complete frame. Metadata has domain-separated checksums. No checksum is
substituted for certificate authentication or deterministic execution.

Opening an active archive checks the retained legacy inventory, migration witness,
version fence, all segment names and boundaries, every frame/receipt and the
complete chained root against the tip. Missing, additional, truncated, reordered,
noncanonical or corrupt records refuse. It rebuilds a height index containing
verified positions, frame hashes and prefix roots; there is no serialized index
cache to trust. Indexed reads seek directly to the frame and check its segment
length and independently loaded frame hash. Live appends and reads recheck the
tip, activation witness, manifest and version fence. History and certificate
verification still run through certified replay before a node can start work.

The existing one-million-record replay refusal bound is also the archive index
work bound. This is a refusal limit, not a retention policy or permission to omit
history. No application-state or signing limit was removed. At the index bound,
positions and hashes require roughly 88 MB before vector-capacity overhead;
application state and signing indexes consume additional RAM.

## Append and crash recovery

Writers share the existing exclusive process lock and journal serialization lock.
A new append preflights capacity, durably publishes an intent containing the old
tip, exact complete frame and exact next tip, appends/fsyncs the segment, atomically
publishes/fsyncs the new tip, then unlinks/directory-syncs the intent. Only then is
the in-memory index advanced and success returned. Identical retries compare all
original bytes; conflicts refuse.

Recovery accepts only the exact partial prefix described by that durable intent.
It completes the missing suffix and tip, verifies the entire resulting archive,
and only then clears the intent. Unknown tails and conflicting bytes are never
truncated or overwritten to make recovery pass. A full filesystem can refuse
recovery while retaining the intent; restoring headroom permits exact resumption.
An uncertain acknowledged/result boundary does not authorize deleting a record.

## Migration, completeness and downgrade protection

Migration first authenticates **all** certificates and executes the complete
legacy chain, including existing receipt/gap checks. A streaming inventory then
commits to every canonical record and to each receipt's bytes and presence or
absence. All legacy finalized-record and receipt files remain byte-for-byte present after activation.
This deliberately duplicates the migrated prefix: it reduces future growth,
not the already allocated legacy prefix or its existing inode count.

An immutable manifest binds the original genesis, full inventory digest and
expected packed prefix. Staging constructs deterministic complete segments and
fsyncs each once. Interrupted staging accepts only exact existing prefixes; it
never drops an unknown tail. The source inventory is checked again, the initial
tip and activation witness are published, and all staged frames, positions and
roots are independently reread before activation. Finally, an atomic, synced
storage-version prefix is installed in the local `genesis.bin` file. The canonical
genesis inside it, its ID and every consensus encoding remain unchanged.

Official M11 software compares that file to the canonical genesis during replay
and refuses the fence before starting consensus. The new software unwraps the
fence only for its storage comparison. A pending migration cannot accept new
finalized appends or start a node; rerun the authenticated migration command.
Missing or changed retained source files refuse resumption. Restoring an old
unversioned fence over post-migration records also refuses. There is no downgrade
or conversion-back command.

These witnesses detect selective rollback, not a monotonic clock. Rewinding all
archive data and metadata together to an internally consistent older snapshot can
pass local checks; the independent archive anchor rejects a snapshot below its
height or conflicting with its root. Deleting every trusted control, using an old
binary without its independently retained controls, or controlling the signing
program/key is outside local storage integrity protection. Existing independently
anchored compact-checkpoint and signing rollback protections remain intact.

## Maintenance costs and remaining limits

Candidate/proposal names are indexed once on open and tracked by the serialized
writer. Finalized cleanup visits only indexed candidates at or below the floor;
it no longer enumerates the archive at every height. Finalization uses the
existing conservative allocation-credit preflight instead of a fresh full-store
scan. Live WAL sampling, available-block checks, inode reserves, periodic full
reconciliation and the existing two-second pressure audit remain enabled.

Migration inventory construction streams by height with constant inventory memory.
Segment staging writes each deterministic segment once. Startup necessarily reads
and verifies retained source and packed history; certificate authentication and
state execution remain mandatory. Periodic quota audits still enumerate actual
entries, including a retained legacy prefix. These are not an atomic filesystem
quota against external writers. Use an isolated volume/filesystem quota for an
absolute capacity bound. File-block accounting excludes directory/inode/journal
metadata, as in M11.

Application-state growth and recomputing its authenticated roots, full historical
certificate verification, signing reservations, evidence, WAL, the retained legacy
prefix and refusal/index limits remain separate bottlenecks. Segmentation does
not promise indefinite operation within 20 GB.

## Validation and reproduction

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked -- --test-threads=1
cargo build --workspace --locked --release
cargo build --release -p yami-chain-consensus --example storage-bench
python3 -m unittest discover -s tools -p 'test_yamichain*.py'
python3 tools/yamichain-storage-lab.py --segmented-archive \
  --legacy-binary M11_VALIDATOR --binary M12_VALIDATOR --output NEW_PROCESS_DIR
python3 tools/yamichain-archive-faults.py \
  --test-binary STORAGE_UNIT_TEST_EXECUTABLE --output NEW_REPORT.json
python3 tools/yamichain-storage-bench.py --binary BENCHMARK --format fresh \
  --heights 1024 4096 --output NEW_BENCH_DIR
```

The serial unit runner prevents inherited flock descriptors from unrelated tests
interfering with child-process crash fixtures. The dedicated concurrent signing
test still exercises simultaneous conflicting reservations. Build scratch space
is separate from a validator's operating storage budget. This environment uses
Rust 1.90 and disables debug symbols/incremental compilation to fit its smaller
build workspace; test coverage is unchanged.

## Measured results

[Raw benchmarks](evidence/yamichain-m12-bench.json),
[private process campaign](evidence/yamichain-m12-process.json),
[actual ENOSPC tests](evidence/yamichain-m12-enospc.json), and
[validation metadata/binary and log hashes](evidence/yamichain-m12-checks.json).

All **546 Rust workspace tests passed**, with zero failures and 10 existing
ignores. Strict workspace/all-target Clippy, formatting, workspace release build,
release benchmark build, and all six Python tooling tests passed. The non-test
privacy boundary still exhaustively rejects all 65,536 production proof versions.
The measured validator is byte-identical to the final release validator. A
benchmark-only fixture correction copied the identity fence alongside the signing
stream into its separate pruned mirror; Clippy and the benchmark build were rerun
and all final benchmark cases passed. Production storage code was unchanged.

The private mixed-M11/M12 campaign passed in 182.20 seconds. The archive upgraded
at height 65. A fresh pruned identity imported an independently anchored height-65
checkpoint, waited until the segmented source reached 128, then fetched historical
records and converged. All four nodes also agreed after two restarts at heights
160 and 192. The exact M11 executable refused the archive version fence. All five
migration publication interruptions resumed with the same authenticated state;
compact-base, unlink, export and import interruption checks also passed.

Six separate real-filesystem cases reached actual block or inode ENOSPC on a
128-MiB tmpfs: append, interrupted migration and pending-intent recovery, each with
both exhaustion types. They retained original source/safety bytes, refused under
pressure and resumed after capacity was restored. Identical committed append
retries remained valid. Unit process exits additionally cover every append boundary,
exact partial tails, corrupt/missing/extra segments, changed/missing migration
inputs, selective rollback and complete consistent rollback with an external
anchor. Hostile historical snapshots with recomputed transport digests still
failed certificate validation before installing any record. These are process and
filesystem fault tests, not physical-device power-cut experiments.

The benchmark freezes each executable, uses sequential two-CPU/2-GiB containers,
eight naming transactions/block, real signatures, a 4→7→4 committee, one signing
reservation/height, and five warm reopen/replay cycles. M11 is the exact unchanged
`b25d124` executable. Fixtures contain no hardlinks; each reported entry consumes
an inode. Physical counts exclude directory/inode/journal metadata, as in M11.
Migration cases upgrade halfway through generation and retain the entire old prefix.

| Layout | Heights | Archive allocated bytes | Entries/inodes | Bytes/height |
| --- | ---: | ---: | ---: | ---: |
| M11 | 1,024 | 8,695,808 | 2,056 | 8492.0 |
| M11 | 4,096 | 34,680,832 | 8,200 | 8467.0 |
| M12 fresh | 1,024 | 2,600,960 | 12 | 2540.0 |
| M12 fresh | 4,096 | 10,285,056 | 14 | 2511.0 |
| M12 migrated halfway | 1,024 | 6,791,168 | 1,036 | 6632.0 |
| M12 migrated halfway | 4,096 | 27,058,176 | 4,110 | 6606.0 |

Fresh allocation fell about **70%** at both measured heights. At 4,096, entry count
fell from 8,200 to 14 (99.83% fewer). The measured allocation slope between the two
heights was about 2,501 bytes/height versus 8,459 in M11. These are workload-specific
file-block slopes, not lifetime capacity bounds. Logical bytes increase slightly
because frames add height/length fields; savings come from eliminating filesystem
allocation slack and per-height inodes, not dropping certificate or receipt bytes.

| Layout | Heights | Generation (s) | Mean archive open/replay (s) | Export (s) | CPU user+system (s) | Peak RSS (KiB) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| M11 | 1,024 | 7.75 | 1.823 | 1.76 | 19.01 | 16,800 |
| M11 | 4,096 | 55.81 | 17.390 | 17.21 | 181.04 | 34,200 |
| M12 fresh | 1,024 | 10.65 | 1.918 | 2.04 | 21.21 | 16,448 |
| M12 fresh | 4,096 | 54.94 | 17.667 | 16.57 | 180.06 | 34,536 |
| M12 migrated halfway | 1,024 | 8.66 | 1.844 | 1.78 | 20.08 | 16,784 |
| M12 migrated halfway | 4,096 | 65.77 | 17.127 | 17.14 | 189.33 | 34,472 |

This does **not** establish faster archive replay or lower CPU/RAM usage. Replay
and CPU costs are broadly similar at 4,096. Fresh generation at 1,024 was slower
than M11 (10.65 vs 7.75 seconds); the intent/tip protocol adds durability operations.
M12 CPU totals include its extra legacy-mirror construction for pruned diagnostics
(1.53–1.75 seconds at 1,024; 7.51–7.88 seconds at 4,096). Compare individual phase
times rather than claiming a speedup from whole-campaign totals. These are single
warm-cache campaigns, with filesystem and scheduling variance.

The current reader's legacy-layout control at 1,024 took 10.13 microseconds per
lookup. Fresh segmented lookups averaged 30.28 and 36.20 microseconds at 1,024 and
4,096; migrated cases averaged 25.70 and 26.51 microseconds. Each case samples
1,024 deterministic dispersed heights and checks the returned height. The index
avoids directory searches and scales independently of historical height, but
segment/frame integrity and metadata revalidation add latency. This is not a
claim that packed lookup is faster than a legacy filename read, nor a cold-device
or network latency measurement. The lookup control uses the M12 legacy reader;
the untouched M11 executable has no separate lookup timing field.

Migration at heights 512 and 2,048 took 0.837 and 5.492 seconds, respectively,
including full authentication/replay and staged verification. It added 1,150,976
and 4,575,232 allocated bytes before subsequent growth, without removing the old
prefix. At final height 4,096 the halfway-migrated store used 27,058,176 bytes:
about 22% below M11, substantially above a fresh segmented archive. There is no
immediate reclamation of legacy space or legacy inodes.

Existing pruned interoperability remains intact: at height 4,096 the legacy
pruned diagnostic mirror used 2,883,584 allocated bytes and 26 entries, with five
reopens averaging 91.58 ms (M11: 2,891,776 bytes, 26 entries, 75.40 ms). Fresh compact
imports used the same 1,697,458-byte checkpoint encoding and independently pinned
anchors. Signing preimages remained exactly 158 × height bytes and every
reservation remained retained. The separate live campaign validates actual
pruned-node catch-up and restarts rather than inferring interoperability from
allocation figures.

## Is long-term operation within 20 GB more realistic?

**Yes for archive storage density, especially for a fresh segmented archive; it
is not established for indefinite node operation.** This workload's fresh archive
allocation is about one-third of M11's, with almost all per-height inode growth
removed. Upgrades retain their legacy allocation and need temporary headroom to
build the packed copy. Periodic audits still scan that retained prefix.

Full historical verification/state execution remains expensive and can grow
superlinearly with active application state. State/snapshot limits, the existing
one-million signing-reservation limit, the archive replay/index limit, evidence,
WAL and normal multiple signing authorizations per live height remain constraints.
The benchmark's one authorization per height is not a lifetime validator workload.
No limit was silently removed and no safety data is evicted to fit the budget.
An isolated filesystem quota is still needed for a hard 20-GB bound. No public
deployment or change to `main` occurred.
