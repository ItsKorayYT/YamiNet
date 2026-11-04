# YamiChain Milestone 11: safe signing-log storage

Base: `61b66fb`, permanent branch `development/yamichain`. This changes local
validator storage, not consensus messages, YPP, money, staking or production PoS.
Archive remains the default. Production proof versions remain rejected.

## Representation and invariants

A reservation binds the exact `(height u64, round u32, step u8)` to **every byte**
of its original signing preimage. Releasing a signature is permitted only after
`reserve_signature` succeeds. An error must never be interpreted as permission to
use another preimage: publication of a durable pending intent irrevocably reserves
its value even if the caller never received success.

Canonical reservation frames have a 21-byte header followed by the bounded
preimage. Frames are appended to numbered segments of at most 1 MiB. A separate
append-only commit stream contains one 88-byte entry per authorization: sequential
count, segment/end position, chained SHA-256 root and a domain-separated checksum.
The root commits to the previous root and the complete canonical frame, including
its tuple and length. A separately atomically published `signing-tip.bin` holds
the latest committed count/root/position. Counts, positions, contiguous segment
numbers, frame boundaries, duplicate tuples and the complete chain are verified
on open. Missing, extra, truncated or corrupt segments/commit records/tip fail
closed; the implementation never truncates unknown tails to make recovery pass.

The in-memory ordered index maps tuples to verified positions, sequence numbers
and expected per-authorization roots. Repeating a tuple compares the exact
preimage and rechecks its on-disk commit binding against that independently loaded
root. A conflicting preimage always fails. Even a rewritten old frame plus a
recomputed local checksum cannot override a loaded index binding. The current
commit head, tip and identity/migration controls are checked before each signing
request. Startup rebuilds the index by streaming verification, never trusting a
serialized lookup cache. Historical sector corruption is detected during startup
or the relevant indexed read; this is not a background scrub of every byte before
every append.

There is a refusal limit of 1,000,000 reservations, including migrated entries.
It is a work/memory bound, not a retention policy. The commit stream is therefore
bounded to approximately 88 MB plus its header; it is append-only, not itself
segmented. Preimages remain bounded by the existing record cap. No reservation,
legacy signing file, equivocation evidence, WAL, finalized record or compact
checkpoint is evicted to satisfy this limit or a quota.

## Durable append protocol

All writers share the journal's exclusive OS lock and serialization mutex.

1. Verify the current committed head and look up the tuple. Identical retries
   resync existing data and return; conflicts refuse. Preflight space/inodes for
   the transaction before publishing new work.
2. Atomically publish and directory-sync `signing-intent.bin`. Its checksum,
   predecessor count/root, complete frame and exact next commit entry bind the
   entire operation. This intent is now an authorization that cannot be forgotten.
3. Append the frame to its segment; fsync the file and directory. Append and
   fsync its commit entry. Neither stream replaces previously committed bytes.
4. Publish the new tip using a synced temporary file, atomic rename and directory
   sync. Only after that succeeds unlink and directory-sync the intent.
5. Update the verified in-memory index and return success to the signer.

After an interruption, recovery accepts only the exact prefixes described by the
validated durable intent. It finishes missing suffix bytes, completes the tip and
clears the intent only after the complete authorization is durably represented.
An unexpected prefix, unknown tail, wrong predecessor or conflicting witness
refuses instead of discarding data. Recovery re-verifies the entire resulting
stream before the journal can sign. A full/quota-limited store can refuse pending
recovery while retaining the intent; restoring headroom allows exact resumption.
No key can be reused for another preimage while that operation is unresolved.

## Migration, completeness and downgrade fence

Legacy reservations remain present **byte-for-byte after migration**. Migration
reduces future growth, not the disk allocation of the retained legacy prefix.

The migrator enumerates canonical legacy tuple filenames under the store lock,
checks regular-file types and bounds, sorts the tuples, and computes a chained
inventory digest over every complete canonical tuple/preimage frame. An immutable
migration manifest records the count, digest and validator-index binding. It then
versions `validator-index.bin` into a pending-format fence before staging packed
data. Official M10 validators expect the old one-byte identity record and refuse
this fence, including when an explicit index is supplied. Retaining old signing
files is **not** permission to downgrade after new packed reservations exist.
There is no conversion-back command.

Segments and the initial commit stream are built deterministically. Existing
staged artifacts must match exactly; conflicting ones are never overwritten.
Before activation, the implementation re-reads the legacy inventory and verifies
every staged frame, position, count and chained prefix root against it. It publishes
the initial tip, an immutable activation witness, then the active-format identity
fence, with file and directory fsyncs in that order. Only then can new packed
reservations be accepted. The fence retains the original validator index.

Interrupted staging can resume from the complete retained legacy source, with the
same completeness checks. Missing/changed source entries refuse. A missing
activation witness behind an active fence refuses; restoring a pending fence over
a log with post-migration authorizations refuses. No partial migration becomes an
empty or partially populated signing store. If migration cannot finish (including
insufficient headroom or the work limit), original reservations remain retained;
restore capacity and retry with the new software rather than deleting witnesses
or starting the key in a fresh directory.

Bare storage-only fixtures without an assigned validator index receive an
unassigned version fence. They cannot later be silently bound to a live validator
identity. Real validator startup binds its index before initializing/migrating
signing storage. Fresh compact imports still refuse validator identity/safety
stores, including the new signing namespace.

## Rollback trust boundary

The tip witness detects selective segment/commit rollback, and full verification
detects stale tips against newer streams. Hash chains do **not** provide a
monotonic clock or protection against complete local rollback. Rewinding the
segments, commit stream and tip together to an internally consistent older
snapshot can evade local detection, even if other validator files remain newer.
Deleting/replacing all trusted format controls is also outside local integrity
protection. An attacker controlling the key or signing program can bypass software
storage rules entirely.

An independently protected `SigningAnchor { reservations, root }` rejects rollback
below its count and mismatched history at that count. `run --signing-anchor
COUNT:ROOT` checks this minimum before WAL recovery or new consensus work;
`verify-signing` provides the offline check. Keep anchors outside the storage being
restored/rolled back, or use a genuinely monotonic external service/hardware root.
A stale external anchor protects only its committed prefix, not later reservations.
This milestone does not provision an external per-signature anchoring service and
does not claim to detect every complete local rollback by default.

## Storage accounting

Durable commit counts, positions and frame roots provide verifiable signing
accounting; `signing-info` reports the count, total preimage bytes, segments and
retained legacy count, rebuilt from verified files on every open.

Whole-store physical/logical accounting uses a conservative in-memory credit
cache, not a persisted counter that can silently roll back. Before I/O, writers
precharge bytes, rounded allocation units and entries for their complete writes.
Failed writes, temporary files, replacements and deletions can overcount until
reconciliation; they never create a negative credit or subtract unverified usage.
The cache is discarded on restart. Full filesystem audits run after 256 write credits, at explicit checks and on
the existing two-second node pressure tick. The node also explicitly checks after
finalization, so archive-mode audits still scan a growing history each height;
this change does not eliminate all quadratic cumulative archive scan work. A failed explicit audit invalidates/replaces the cache as appropriate.
The independent WAL writer is sampled live on every credit; new WAL directories
are conservatively counted too. Discovery cache replacements now use the same
serialized accounting writer.

Available blocks and the 128-inode reserve are checked on every capacity operation.
These remain refusal preflights, not an atomic hard quota against external writers.
An operator bypassing the exclusive journal writer can invalidate a between-audit
sample; a filesystem quota/isolated volume is still required for an absolute 20-GB
allocation. Inode/directory/journal metadata is not included in file-block totals.
Very large within-quota WAL recovery and active application-state growth remain
separate limitations. The accounting cache never authorizes signatures or replaces
signing-log verification.

## Reproduction and measured scope

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=1
cargo build --workspace --release
cargo build --release -p yami-chain-consensus --example storage-bench
cargo build --release -p yami-chain-storage --example signing-bench
# Stop the validator; preserve its complete original store and identity.
yami-validator migrate-signing --dir STORE
yami-validator signing-info --dir STORE
yami-validator verify-signing --dir STORE --signing-anchor COUNT:ROOT
```

The serial unit-test runner avoids transient inherited flock descriptors while
parallel tests fork child crash processes; a dedicated 16-thread conflict test
still exercises simultaneous signing requests. Process tests exit at migration,
intent, data, commit, tip and cleanup boundaries, including atomic-file sync/rename
failures. They also test exact partial tails, missing intents, altered/missing
legacy inputs, corrupt/missing segments, forged historical frames, rolled-back
witnesses, complete rollback with an external anchor, quota refusal and resumption.

## Upgrade and restart campaign

The [private process campaign](evidence/yamichain-m11-process.json) passed in
393.65 seconds. Three validators used the exact M10 executable through height
512; a fourth started from an independently anchored compact import using M11.
After stopping, all stores passed verified migration, retained their historical
files, and refused the exact M10 executable against cloned upgraded stores.
All four then converged at 576, 640 and 704 through three restarts with independently
observed signing anchors. Interrupted compact-base publication, pruning unlink,
checkpoint export and fresh import also recovered with verification intact.

| Node at height 704 | Allocated file bytes | Entries | Reservations | Retained legacy files |
| --- | ---: | ---: | ---: | ---: |
| Upgraded archive | 11,108,352 | 2,610 | 1,628 | 1,190 |
| Upgraded pruned 1 | 5,750,784 | 1,269 | 1,628 | 1,190 |
| Upgraded pruned 2 | 5,746,688 | 1,268 | 1,627 | 1,189 |
| Fresh packed, pruned | 843,776 | 79 | 1,450 | 0 |

Between heights 512 and 704 the upgraded archive added 8,789.3 allocated bytes
and two directory entries per height. The upgraded pruned stores added 789.3
bytes/height with no net entry growth; the fresh packed pruned store added 810.7
bytes/height with no net entry growth. These are observed slopes over 192 heights,
including changing compact application snapshots, not lifetime upper bounds.
All legacy allocations remain. A fresh compact import has fewer signing records
because it never possessed the importing key's pre-import authorizations; used
validator identities must retain their own complete original safety store.

## Capacity conclusion

For a 158-byte preimage the packed representation adds 21 bytes of frame and 88
bytes of commit, or 267 serialized bytes per authorization, plus fixed controls,
filesystem rounding and one segment inode per roughly 5,858 authorizations.
At an illustrative 2.25 authorizations/height this is about 601 bytes/height of
new signing data versus 9,216 allocated bytes/height in the old 4-KiB-per-file
layout. Actual rounds and preimage lengths vary; this is not a consensus bound.

The one-million-reservation cap deliberately refuses further signing while
retaining everything. It does not permit indefinite operation: extra rounds can
consume that limit sooner than height alone suggests. The old prefix is duplicated
and retained after migration. Archive bodies and receipts still use two files per
height; evidence has no deletion policy, stalled-height WAL can grow, and active
registry/nonces and compact snapshots still grow. The existing application-state
caps also remain (the eight-new-names workload reaches 100,000 names at height
12,500). Signing-log migration does not solve these independent limits.

Two CPUs and 2 GiB are tested container limits, not dedicated SSD performance.
Allocated totals sum file blocks and exclude directory/inode/journal overhead.
A hard 20-GB SSD volume was not filled; real block and inode exhaustion tests use
smaller isolated tmpfs volumes. Keep headroom for migration, atomic snapshots,
OS/filesystem metadata and recovery, with an OS quota or separate volume. The
18-GB application refusal budget is not a guarantee of fitting a complete host
into 20 GB. This materially reduces future signing allocation and inode growth,
but indefinite 20-GB operation remains unsupported by the evidence.

## Unaligned imported-checkpoint regression

A fresh follow-up campaign imported height 65. At finalized height 96, the old
32-height scheduler requested floor 64, below that base. The existing rollback
check refused and the node exited safely. M11 now skips periodic pruning until
its proposed floor reaches the verified local base; the explicit offline API
still refuses every backward request. A certified-history regression checks
that the base is unchanged while waiting and later pruning preserves the exact
finalized state. The original failure is retained in
[evidence](evidence/yamichain-m11-unaligned-before.json); it is not counted as a
passing process campaign. The corrected executable then passed a fresh
[132.37-second process campaign](evidence/yamichain-m11-process-final.json):
all four nodes converged at height 128 and after restart at 160, including the
node imported at height 65. Base/unlink/export/import interruptions also passed.

## Measurement reproduction

Freeze the M10 release validator before building M11 (the campaign records its
SHA-256), then use fresh output directories:

```sh
python3 tools/yamichain-storage-lab.py --output work/m11/process \
  --legacy-binary work/m11/m10-validator --height 512 --restart-count 3 --restart-step 64
python3 tools/yamichain-signing-lab.py --output work/m11/signing \
  --baseline-validator work/m11/m10-validator
python3 tools/yamichain-storage-bench.py --output work/m11/chainbench \
  --heights 1024 4096 8192
python3 tools/yamichain-container-lab.py --case disk-full --output work/m11/stress
python3 -m unittest discover -s tools -p 'test_yamichain*.py'
```

The signing-only legacy workload uses the old per-file representation and explicit
full audits before each write; it is compiled with the new storage crate and is
not the exact M10 executable. Its legacy recovery verifies every known preimage,
stronger than a directory-only open. The process upgrade/downgrade tests do use
the exact retained M10 validator. The separate real-chain benchmark uses real
signatures, eight naming transactions/block, a 4→7→4 committee schedule, five
archive and five pruned reopens, and independently anchored compact import.
Its one authorization/height is a controlled comparison, not typical live rounds.

## Final code validation

Formatting, strict workspace/all-target Clippy (`-D warnings`), the workspace
release build and both release benchmark builds passed. All **531 workspace tests
passed**, with zero failures and 10 existing ignores; all six Python tooling tests
passed. The non-test privacy boundary integration test still exhaustively rejects
all 65,536 production proof versions and public financial actions. The final
signing-benchmark executable is byte-identical to the measured executable; the
later node-only scheduler fix does not change that storage workload.
[Validation metadata and log hashes](evidence/yamichain-m11-checks.json) identify
the exact binaries and completed runs. No public deployment occurred.

## Signing-only results

[Raw measurements and filesystem faults](evidence/yamichain-m11-signing.json).
All cases used two CPU and 2-GiB container limits and five verified reopen cycles.
The opaque preimage was 158 bytes; these are storage-only tests, not elections.
The allocated-byte slope excludes each case's existing empty/legacy store.

| Layout | Authorizations | Allocated bytes | Added bytes/auth | Added entries | Mean recovery (ms) | Generation (s) | CPU user+system (s) | Peak RSS (KiB) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| legacy | 1,024 | 4,202,496 | 4096.000 | 1,024 | 5.92 | 1.86 | 0.92 | 16,660 |
| fresh | 1,024 | 307,200 | 292.000 | 5 | 0.94 | 2.99 | 0.44 | 16,660 |
| legacy | 8,192 | 33,562,624 | 4096.000 | 8,192 | 53.77 | 76.86 | 68.30 | 16,668 |
| fresh | 8,192 | 2,228,224 | 271.000 | 6 | 4.99 | 25.29 | 3.86 | 16,716 |
| fresh | 65,536 | 17,580,032 | 268.125 | 16 | 43.75 | 164.63 | 27.28 | 16,508 |
| migrate | 65,536 | 51,122,176 | 267.714 | 10 | 90.30 | 160.67 | 37.59 | 16,532 |

Migration itself took 0.239 seconds and increased allocation
from 33,562,624 to 35,770,368 bytes before
new authorizations: the entire old prefix remains and is duplicated in the packed
stream. The added-entry column for migration covers only the subsequent 57,344
authorizations. Fresh 65,536 storage has 19 total entries, including 12 segments;
its incremental inode slope is 16/65,536 = 0.000244 entries/authorization.

At 8,192 reservations packed generation was 3.04× faster, with about 15.1× less
allocation, and recovery 10.8× faster in this run. At 1,024 it was slower (2.99 vs
1.86 seconds): extra intent/commit/tip fsyncs have a cost. Five open cycles use warm
filesystem caches; these are not cold-device power-loss latency measurements.
Full-directory audit counts fell from 8,193 to 97 at 8,192 reservations, and were
769 at 65,536. Audit counts reset on reopen; the chain benchmark's final journal
therefore reports a different scope.

All four actual filesystem-fault cases passed: append/block, append/inode,
migration/block and migration/inode. Migration remained inactive under pressure,
retained all original signing and nonsigning bytes, and resumed after headroom
was restored. Actual inode/block allocation reached ENOSPC, not just injected
exceptions. Existing authorization retries succeeded; new requests refused.
The exact M10 executable refused the upgraded identity fence without modifying
the cloned source. Process-exit fault injection separately covers every durable
publication stage, partial tails, corruption, missing segments, selective rollback
and conflicting attempts. Full local rollback remains the documented external
anchoring boundary.

## Comparable real-chain measurements

[Raw M11 measurements](evidence/yamichain-m11-bench.json) and
[M10 baseline](evidence/yamichain-m10-bench.json). Both use eight naming
transactions/block, one signing reservation/height, real signatures, a 4→7→4
committee, five reopens per mode, and an eight-record pruned tail. The signing
preimages are exactly 158 × height bytes in every M11 case and all remain retained.
These are single campaigns on the same managed environment with warm caches,
not repeated controlled SSD trials or a universal speedup guarantee.

| Version | Heights | Archive allocated bytes | Pruned allocated bytes | Archive bytes/height | Pruned bytes/height | Entries archive/pruned |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |

| M10 | 1,024 | 12,587,008 | 4,685,824 | 12292.0 | 4576.0 | 3,074/1,044 |
| M11 | 1,024 | 8,695,808 | 794,624 | 8492.0 | 776.0 | 2,056/26 |
| M10 | 4,096 | 50,335,744 | 18,546,688 | 12289.0 | 4528.0 | 12,290/4,116 |
| M11 | 4,096 | 34,680,832 | 2,891,776 | 8467.0 | 706.0 | 8,200/26 |
| M10 | 8,192 | 100,667,392 | 37,027,840 | 12288.5 | 4520.0 | 24,578/8,212 |
| M11 | 8,192 | 69,337,088 | 5,697,536 | 8464.0 | 695.5 | 16,393/27 |

Bytes/height here are total file allocation divided by height, including application
state and fixed files; they are not a marginal or lifetime bound. Fresh archive
entry growth remains approximately two files/height instead of three. Pruned entry
counts remain 26–27 in this controlled workload, with one new segment at 8,192.

| Version | Heights | Generation (s) | Mean archive reopen/replay (s) | Mean pruned reopen/replay (ms) | Export / prune (s) | CPU user+system (s) | Peak RSS (KiB) | Total wall (s) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| M10 | 1,024 | 15.92 | 1.884 | 24.80 | 1.83 / 1.88 | 28.19 | 16,604 | 31.02 |
| M11 | 1,024 | 8.96 | 1.723 | 23.19 | 1.71 / 1.77 | 18.98 | 16,432 | 23.16 |
| M10 | 4,096 | 221.29 | 15.918 | 81.16 | 16.90 / 18.28 | 339.66 | 33,640 | 352.22 |
| M11 | 4,096 | 54.70 | 15.696 | 79.66 | 17.70 / 18.18 | 167.66 | 34,280 | 184.68 |
| M10 | 8,192 | 913.90 | 91.527 | 226.92 | 86.71 / 97.99 | 1620.21 | 64,344 | 1651.97 |
| M11 | 8,192 | 171.29 | 81.451 | 198.14 | 84.97 / 84.59 | 791.01 | 65,612 | 827.68 |

At 8,192 heights M11 pruned allocation is 5.70 MB versus 37.03 MB (about 6.5×
less); archive allocation is 69.34 MB versus 100.67 MB. Generation is 5.34× faster
and total campaign wall time approximately halves. Full archive verification
remains expensive: the first replay took 78.20 seconds and five complete reopens
averaged 81.45 seconds. Pruned reopens averaged 198.14 ms, including signing-log
verification. At 1,024 the first archive replay was slightly slower (1.92 vs
1.79 seconds). Peak RSS at 8,192 was 65,612 KiB versus 64,344 KiB in M10; this
change primarily improves file allocation, inode use and scan work, not state RAM.

The real-chain benchmark's final `accounting_audits=1` describes its last reopened
journal during export/pruning; it is not the audit count over history generation.
The signing-only table records audit counts before closing the generation journal.
The measured histories fit comfortably inside the two-CPU/2-GiB limits, but these
results do not establish a lifetime RAM or recovery-time bound for arbitrary WAL,
evidence, state or authorization counts. Building the whole workspace also uses
separate scratch disk and is not part of the validator's 20-GB operating budget.

## Final filesystem-pressure campaign

The [live disk-full campaign](evidence/yamichain-m11-diskfull.json) passed. An actual
128-MiB isolated tmpfs reached ENOSPC; the node exited 1 at finalized height 4,
preserved observed immutable safety bytes and append-only prefixes, then restarted
with its entire original store and independently observed minimum signing anchor.
All four nodes converged to the identical certified height-48 state. No OOM event
occurred under the combined two-CPU/2-GiB container limit. Validators consumed
2.996 CPU seconds, sampled combined peak RSS was 81,661,952 bytes, and sequential
authenticated replay of all four stores took 0.219 seconds. These live-fault
figures are a small adversarial case, not the long-history benchmark above.

Separate offline prune tests reached actual block ENOSPC and inode ENOSPC,
refused both at the inode reserve (127 free) and at full exhaustion, and preserved
every committed file hash. The four signing-specific append/migration pressure
cases also passed, including successful resumption with all legacy bytes intact.
None of these tests deletes safety data to create headroom.
