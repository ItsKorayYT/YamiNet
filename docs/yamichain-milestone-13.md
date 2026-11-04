# YamiChain Milestone 13: scalable application state

Base: `d0d972f`, shared branch `development/yamichain`. This changes in-memory
representation and computation, not transaction semantics, protocol limits,
canonical state/snapshot bytes, genesis IDs, block IDs or state roots. Test naming
remains non-authoritative. All 65,536 financial proof versions remain rejected;
YPP, financial functionality, staking and production elections remain disabled.

## Profiling findings and implementation

M12 staged transactions twice during `execute`: once to construct the expected
block, then again to produce the result. Each stage deeply cloned the naming and
nonce BTreeMaps and the genesis schedule. Root computation collected all name
references, allocated a complete canonical state buffer, recomputed the genesis
ID, and hashed it on every call. Snapshot serialization also collected all name
references and used a growing buffer. Compact export built another full temporary
snapshot; restore materialized another snapshot to check canonical equality.

M13 uses `im::OrdMap` 15.1.0 for both maps. A clone shares immutable ordered tree
nodes; mutations copy only affected paths/chunks. Genesis and the exact set of
reserved consensus/transport keys across **all** epochs are shared immutable
indexes. Membership in that reserved-key index has the same meaning as the old
full schedule scan. Read-only `TestNames::get`, `len`, and `State::nonce` expose
indexed queries without introducing registry update/delete operations. Status reporting
uses the verified map length instead of enumerating the registry.

Execution validates and stages each transaction once, builds the same expected
block, compares every field, then returns that already validated staged state.
Proposal construction still validates independently. Failure discards the staged
branch; it cannot mutate its parent, other proposals or earlier checkpoints.
Nonce, expiry, signature, duplicate, reserved-key and module-error precedence is
checked against a separate M12-style BTreeMap execution oracle.

Each immutable state has a private `OnceLock<Digest>` for its computed root.
Height changes invalidate it even for empty blocks. A successful execution keeps
the root already computed while checking its expected header. State equality
compares actual consensus data and ignores cache initialization. Concurrent root
readers initialize the same digest; caches never authorize a transaction.

The root's exact old byte stream is hashed in order using a small stack buffer.
Validated registration maintains the exact encoded entry-byte count, eliminating
an extra length scan. The root still includes the genesis ID, height, counts,
sorted nonce entries and sorted name entries with their original framing. Snapshot
serialization uses that same application byte stream, preflights the unchanged
limits, reserves its exact output size, and can append directly to the compact
checkpoint envelope. Restore retains canonical byte equality verification using
a streaming comparison instead of allocating a second complete snapshot.

Recovery uses a fresh sorted-name builder: strict ordering prevents duplicate or
existing-key overwrites, so it can insert each validated entry without a redundant
membership search. Previous labels borrow the bounded input during parsing; the
finished registry owns its keys. Temporary nonce counts use a standard BTreeMap
because that validation table needs no persistent sharing. The same final nonce
equality, root and canonical-byte checks remain mandatory.

The new dependency is `im` (MPL-2.0+), with four newly locked packages in total:
`im`, `bitmaps`, `sized-chunks`, and `rand_xoshiro`; existing dependencies were not
upgraded. `OrdMap` canonical iteration is ordered by keys, independent of tree
shape or allocation. Hash-map iteration/randomness is not used for consensus.

## Authentication, crashes and compatibility

No tree index, byte counter, reserved-key index or root cache is serialized or
loaded from a sidecar. Recovery reconstructs them from validated entries. Compact
restore still checks the independently supplied genesis/height/tip/root, strict
key and label order, duplicate exclusion, valid non-weak non-reserved account
keys, positive nonces, name ownership and registration/nonce-count equality. It
then computes the complete root and checks every canonical snapshot byte before
returning a state. It never installs the peer's asserted root into the cache.

Archive replay still authenticates certificates and executes the full history.
Segmented archive framing, intents, roots, retained legacy files, signing
reservations, WAL, evidence and independent rollback anchors are unchanged.
Compact export still verifies the resulting checkpoint through the existing
certificate and snapshot verification path. Old and new validators can consume
the same blocks and compact checkpoint bytes.

The legacy-oracle test compares block encodings, roots, snapshots, execution errors,
failed-transaction atomicity and retained forks through committee transitions.
Existing fixed external vectors remain unchanged. Additional tests exercise empty
height cache invalidation, concurrent readers, corrupted/truncated snapshots,
reserved keys across future epochs, maximum-length labels at 100,000 entries,
and successful execution followed by snapshot refusal at 100,001 names.

## What the 100,000-entry limit actually means

`MAX_STATE_SNAPSHOT_ENTRIES` remains **100,000 independently for each map**.
`MAX_STATE_SNAPSHOT_BYTES` remains **16 MiB**, and compact envelopes remain capped
at **17 MiB**. M13 raises none of these constants and adds no transaction-level
state-count limit. Execution was already able to exceed the snapshot count limit;
a node then cannot export a v1 compact checkpoint, and periodic pruning can refuse.
M13 deliberately preserves this behavior instead of silently changing consensus
acceptance or claiming unlimited compact recovery.

For account count A, name count N and total label bytes L, the exact v1 state
snapshot length is:

```
90 + 40*A + 36*N + L
```

The current module only registers names. Every registration increments its owner's
nonce, so snapshot recovery requires each account's nonce to equal its number of
registered names; consequently A ≤ N in reachable state. Names cannot expire,
update or delete. Re-registering an existing label, including with another owner,
remains an error. The benchmark's shared-account case measures repeated **nonce
updates**, not unsupported changes to registered names. No new deletion operation
was invented to free capacity.

With 100,000 names and accounts and 63-byte labels, the encoding would be
13,900,090 bytes, below 16 MiB. Thus the entry cap currently binds before the byte
cap even at the maximum label length. This does not make increasing it safe:
250,000 such names/accounts would need 34,750,090 bytes, exceeding both existing
byte envelopes and every old reader's accepted entry count. Key validation,
ordered-index construction, nonce-count verification, full root hashing, envelope
copies, archive replay, peak fork memory and bounded recovery time also require
measurements at the proposed new capacity.

The flat root is an important remaining bound. Height appears near the beginning
of its SHA-256 input and changes every block. Sorted insertions and nonce changes
also alter that input. A cached subtree hash cannot replace these bytes while
preserving the same digest. M13 removes redundant roots and allocations, but a
new height still requires a full canonical hash. Long-history replay therefore
still has a state-size-dependent cost.

## Concrete capacity upgrade path — requires separate approval

1. Choose and approve an explicit entry count, maximum label/ownership workload,
   byte envelope, recovery-time budget and memory budget. Measure worst-case
   accounts as well as names, retained branches, concurrent recovery and filesystem
   pressure under the 2-core/2-GB target. Publish refusal behavior before rollout.
2. Specify a separately versioned snapshot codec/transport with explicit count,
   total-byte and chunk bounds before allocation. Keep v1 decoding and its current
   limits intact. Preserve sorted canonical records, ownership/nonce checks,
   independently anchored roots, certificates, rollback markers and complete
   histories. Chunk transport requires authenticated completeness and ordering;
   peer-supplied digests do not become checkpoint authority.
3. Upgrade readers, writers, storage envelope bounds and operator tooling together;
   explicitly opt into or negotiate the new export format only after compatible
   recovery is available. Old pruned readers reject counts above 100,000 even if
   blocks still use the same root semantics. Plan quorum availability, backups,
   archive fallback and rollback protection before crossing that boundary. Do not
   merely change a constant in one binary.
4. A larger snapshot codec can preserve the existing flat state root. Achieving
   logarithmic root updates instead requires a separately approved consensus
   state-root version: define canonical authenticated-tree keys/nodes/domains,
   deterministic ordering/shape rules, an activation policy, and a verified bridge
   from the complete old state to the new commitment. Preserve old verification
   and rejection of unknown formats; do not reinterpret old block headers.
5. Before activation, run cross-version roots/bytes vectors, old/new recovery and
   interoperability campaigns, maximum-size and over-limit workloads, malformed
   counts/chunks, duplicate ownership, crash/ENOSPC and independently anchored
   rollback tests. Updates or deletions would also require explicit transaction
   semantics and a replacement for v1's registration-count/nonce invariant.

No part of this future capacity/root-version plan is enabled by M13.

## Reproduction

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked -- --test-threads=1
cargo build --workspace --release --locked
cargo build --release --locked -p yami-chain-consensus --example application-bench
python3 -m unittest discover -s tools -p 'test_yamichain*.py'
python3 tools/yamichain-state-bench.py --binary APPLICATION_BENCH \
  --output NEW_DIRECTORY --sizes 1024 10000 50000 100000
python3 tools/yamichain-storage-lab.py --segmented-archive --transactions 64 \
  --binary M13_VALIDATOR --legacy-binary M12_VALIDATOR --output NEW_PROCESS_DIR \
  --height 128 --restart-count 2 --restart-step 32
python3 tools/yamichain-state-recovery.py --binary M13_VALIDATOR \
  --snapshot CHECKPOINT --anchor HEIGHT:BLOCK:STATE --output NEW_RECOVERY_DIR
```

To reproduce the M12 control, build the same harness against archived M12 sources
and substitute its existing public iterator for the new lookup API:

```sh
mkdir -p work/m13-m12-source
git archive d0d972f | tar -x -C work/m13-m12-source
python3 - <<'PYCODE'
from pathlib import Path
source = Path('crates/yami-chain-consensus/examples/application-bench.rs').read_text()
source = source.replace('state.names().get(&label)',
    'state.names().entries().find(|(key, _)| *key == label)')
Path('work/m13-m12-source/crates/yami-chain-consensus/examples/application-bench.rs').write_text(source)
PYCODE
CARGO_TARGET_DIR=work/m13-m12-target cargo build --release --locked \
  --manifest-path work/m13-m12-source/Cargo.toml \
  -p yami-chain-consensus --example application-bench
```

Application profiles use real signed registration traces, 64 transactions per
block, four real certificate signatures per finalized record, segmented archives,
and fresh compact imports. Shared and distinct account cases separate growth of
the name registry from growth of both maps. M12 uses unchanged `d0d972f` production
crates with the matching profiling example; its read-query control scans the only
public naming iterator available at that revision. That is an API comparison,
not a claim that BTreeMap itself lacks logarithmic lookup. M13 uses the new indexed
query. All other profile phases use matching calls and deterministic inputs.

The eight retained clone/fork states are intentional: they expose the difference
between deep copies and shared immutable branches in peak RSS. CPU totals include
trace generation, signing, persistence, all profile phases, certified export,
import and replay. There are no validator WAL/signing-reservation workloads in
this application microbenchmark; the separate live campaign covers those stores.
Every measurement container has a two-CPU/two-GiB limit. Cases run sequentially
without concurrent compilation or another campaign. Process-cold recovery still
uses warm filesystem caches; these are not physical power-cut/cold-SSD trials.

## Measured results

[Raw application profiles](evidence/yamichain-m13-bench.json),
[mixed-version live campaign](evidence/yamichain-m13-process.json),
[100,000-account concurrent recovery](evidence/yamichain-m13-recovery.json),
[actual ENOSPC cases](evidence/yamichain-m13-enospc.json), and
[validation logs/source/binary hashes](evidence/yamichain-m13-checks.json).

| Accounts | Names | Version | Generation (s) | CPU user+system (s) | Peak RSS (KiB) | Archive reopen/replay (s) |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| 1 (updated) | 1,024 | M12 | 0.287 | 0.543 | 16,760 | 0.140 |
| 1 (updated) | 1,024 | M13 | 0.305 | 0.439 | 16,900 | 0.079 |
| 1 (updated) | 10,000 | M12 | 3.043 | 5.298 | 16,712 | 1.255 |
| 1 (updated) | 10,000 | M13 | 2.952 | 4.218 | 16,908 | 0.875 |
| 1 (updated) | 50,000 | M12 | 23.117 | 44.112 | 68,848 | 11.528 |
| 1 (updated) | 50,000 | M13 | 16.340 | 23.886 | 37,496 | 5.079 |
| 1 (updated) | 100,000 | M12 | 61.096 | 148.016 | 134,656 | 46.784 |
| 1 (updated) | 100,000 | M13 | 39.110 | 59.098 | 74,696 | 13.926 |
| 1,024 | 1,024 | M12 | 0.305 | 0.575 | 16,868 | 0.114 |
| 1,024 | 1,024 | M13 | 0.240 | 0.392 | 16,872 | 0.065 |
| 10,000 | 10,000 | M12 | 3.285 | 6.361 | 22,724 | 1.481 |
| 10,000 | 10,000 | M13 | 2.505 | 3.936 | 16,860 | 0.695 |
| 50,000 | 50,000 | M12 | 25.423 | 59.669 | 100,684 | 18.228 |
| 50,000 | 50,000 | M13 | 15.950 | 27.572 | 61,096 | 5.997 |
| 100,000 | 100,000 | M12 | 82.393 | 227.431 | 198,512 | 80.563 |
| 100,000 | 100,000 | M13 | 43.512 | 80.300 | 122,136 | 18.868 |

At 100,000 names, shared-account replay is 3.36× faster and distinct-account
replay 4.27× faster. For the distinct-account trace, total CPU falls 64.7% and
peak RSS 38.5%. Both revisions fit the measured 2-GB limit; M13 leaves more room
for branches and concurrent recovery. This does not establish an arbitrary
larger-state or lifetime RAM bound.

| Metric at 100,000 names | M12 shared | M13 shared | M12 distinct | M13 distinct |
| --- | ---: | ---: | ---: | ---: |
| Propose total (s) | 19.176 | 12.868 | 27.834 | 16.179 |
| Execute total (s) | 29.563 | 12.723 | 41.201 | 15.709 |
| 100 post-execution root reads (ms) | 949.135 | <0.01 | 1434.849 | <0.01 |
| Eight retained clones (ms) | 83.029 | <0.01 | 98.881 | <0.01 |
| 1,024 name queries (ms) | 556.430 | 4.733 | 425.733 | 2.171 |
| Snapshot serialization (ms) | 39.706 | 8.558 | 8.122 | 10.229 |
| Bare state restore, mean (ms) | 77.159 | 66.247 | 754.781 | 780.334 |
| Certified checkpoint export (s) | 41.670 | 11.514 | 59.989 | 17.137 |
| Checkpoint import (ms) | 116.303 | 81.901 | 1080.822 | 781.312 |
| Compact process reopen, mean (ms) | 105.916 | 77.417 | 1073.471 | 812.652 |

Post-execution root reads use the root already computed for header verification;
this is a cache measurement, not faster cold hashing of a new height. Tiny cache
and clone timings are reported as upper bounds rather than nanosecond precision.
The nonce-update workload remains inside signed proposal/execution timings: shared
cases repeatedly update an existing nonce; distinct cases grow the nonce map.

Bare snapshot decoding is not uniformly faster: rebuilding persistent ordered
indexes can cost more than rebuilding a mutable BTreeMap, particularly with many
accounts. Serialization also varies by layout: the distinct-account case took
10.229 ms versus 8.122 ms in M12 despite eliminating temporary buffers. The table
separates these costs from end-to-end compact recovery. Full
checkpoint construction benefits from faster authenticated replay and fewer
intermediate buffers. New-height hashing remains linear in canonical state bytes.

For these 17-byte labels, state snapshots are exactly `53*N + 130` bytes with one
account and `93*N + 90` with distinct accounts. At 100,000 names they contain
5,300,130 and 9,300,090 bytes; their full certified envelopes contain 5,306,658 and
9,306,618 bytes. All eight roots, tips, snapshot digests **and entire checkpoint
files** match M12 byte-for-byte. Neither revision can export a snapshot at
100,001 names; both execute the same over-limit trace and produce the same root
and tip before refusing export. The byte savings are in working memory, not a
new disk/checkpoint encoding or discarded application records.

These are single controlled campaigns, not repeated hardware trials. Name-query
speed compares the old public iterator scan with the new indexed API. BTreeMap's
existing internal registration lookup was already logarithmic; do not interpret
this as a general BTreeMap-versus-OrdMap lookup benchmark. Small-case RSS has a
runtime/allocator floor; root/copy savings are more apparent at large counts.

## Final validation and recovery

All **552 Rust workspace tests passed**, zero failed, with 10 existing ignores.
Formatting, strict workspace/all-target Clippy, workspace release and benchmark
release builds passed; all six Python tooling tests passed. The production privacy
boundary still rejects all 65,536 financial proof versions. Final source/binary
hashes and log hashes are recorded with the evidence.

The final private campaign passed in 211.00 seconds using
64 registrations/block. M12 and M13 validators agreed before upgrade; M12 also
read the M13 segmented archive clone successfully, as expected for unchanged
storage and consensus formats. A fresh pruned identity imported a height-65 anchor,
then caught up from a segmented source at height 128. Two subsequent restarts
converged at heights 160 and 192; all four nodes ended with 12,288 registrations
and the identical certified state. Migration, compact-base, unlink, export and
import crash hooks passed while signing/evidence safety witnesses were retained.
Existing archive/signing rollback and corruption regressions remain enabled.

A separate container runs four independent readers of the 100,000-account
checkpoint under a **combined** two-CPU/two-GiB limit. It interrupts import after
durable base publication, resumes using the independently retained M12 anchor,
and starts four fresh processes concurrently for each of three recovery rounds.
Every result matches the pinned height, block and root; committed files remain
unchanged. A corrupted snapshot is refused before publishing a compact base.
Sampled combined peak RSS was 151,715,840 bytes;
container peak memory including page cache was 189,972,480
bytes. Round wall times were 1.992, 1.908, 1.925
seconds. No in-memory index survives between these processes.

Six real tmpfs cases also passed: block and inode ENOSPC during archive append,
interrupted migration and pending-intent recovery. Original records/safety bytes
were retained and exact recovery resumed after capacity was restored. These check
that application-state changes preserve the M12 storage safety paths; no history,
reservation, evidence or WAL is deleted to make space.

The remaining limits are concrete: v1 snapshot entry/byte caps, linear work for a
new flat state root, public-key and ownership validation during restore, monotonically
growing registration state, archive replay volume, and existing signing/WAL/evidence
limits. The approved change improves work and memory per operation without claiming
unbounded capacity or enabling the separately proposed codec/root upgrades.
No public deployment or change to `main` occurred.
