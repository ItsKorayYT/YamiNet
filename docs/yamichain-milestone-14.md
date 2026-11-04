# YamiChain Milestone 14: experimental state capacity

Base: `2942bd1`, branch `development/yamichain`. This milestone implements an
**offline experimental codec**, not an activated checkpoint protocol. Default
builds do not expose it. Enabling the `yami-chain/experimental-snapshot-v2` feature
exposes an opaque laboratory state; it provides no conversion into a production
`State`, consensus checkpoint, journal base or pruning authorization.

The existing v1 state root, snapshot bytes, 100,000-entry limits for each map,
16-MiB snapshot limit and 17-MiB compact envelope remain unchanged. Segmented
archives, replay, independently anchored rollback markers, certificates, WAL,
signing reservations and evidence retain their existing behavior. YPP, staking,
financial functionality and production elections remain disabled. Every one of
the 65,536 financial proof versions remains rejected.

## Format and bounded verification

`snapshot_v2` contains separate streaming export/import and immutable-directory
storage APIs. Its laboratory bounds are deliberately separate from v1 constants:

| Bound | Experimental value |
| --- | ---: |
| Accounts and names, independently | 500,000 each |
| Aggregate record payload | 80 MiB |
| Chunk payload | 256 KiB |
| Records per chunk | 4,096 |
| Chunks per manifest | 512 |
| Manifest input | 64 KiB |
| Height | 1–1,000,000 |

These are decoder allocation/work bounds, not approved protocol limits. The
in-memory ordered indexes still grow with the number of entries; bounded chunks
do not make the whole application state constant-memory. The caller must also
enforce memory, disk and concurrency quotas.

The manifest starts with `YC-LAB-STATE2-1`, then genesis ID, height, tip, existing
flat state root, account/name counts, total payload bytes and chunk count. Each
45-byte descriptor contains kind, first record offset, record count, payload
length and digest. Integers use the existing big-endian framing. Account records
are unchanged key/nonce bytes; name records are unchanged length/label/owner bytes.
Accounts precede names, with strict ordering within each map across chunk
boundaries. Records never straddle chunks.

This experimental domain/layout is provisional. A future production format must
use a separately approved version; experimental files must never be reinterpreted
as production checkpoints.

Chunk hashes use a separate domain and include chunk index, kind, offset, count,
length and payload. The manifest has its own domain-separated commitment. Import
requires an **independently supplied exact anchor** containing genesis, height,
tip, root and manifest commitment. It caps and authenticates the manifest before
allocating descriptors or state indexes, then authenticates each bounded chunk
before parsing records. Missing records, reordered chunks, extra bytes, duplicate
keys/labels, weak or reserved keys, unknown owners, invalid labels and inconsistent
nonce/registration counts fail closed. The final reconstructed state must match
the independently pinned, unchanged flat root. No cache or index is read from an
unauthenticated sidecar.

Manifest and chunk hashes authenticate transport integrity; **they do not prove
BFT finality, execution history or ancestry**. A self-supplied root/manifest is
not checkpoint authority. The codec deliberately cannot install its result in
production consensus, even if the feature is enabled elsewhere in the workspace.

## Streaming persistence, interruption and rollback

The store accepts only dedicated lab directories with a fixed experimental
guard and bounded namespace. It refuses validator directories before modifying
their contents. Production journals also refuse the lab guard or lock before
creating their own lock. Symlinks, special files and Unix hardlinks are rejected.
As with the existing local safety stores, an adversary able to replace path
components concurrently is outside this API's local-filesystem trust model.

Every snapshot directory is immutable and specific to one state anchor. The
state pin is synced before any record publication; transfers additionally pin
the exact manifest before copying chunks. New files use bounded `.pending-*`
files, file sync, atomic rename and directory sync. Restart can extend only an
exact prefix of the expected authenticated bytes. Conflicting bytes and unknown
tails are retained and refused, never truncated or silently repaired. Existing
final files must match exactly. A fully reread, reconstructed, root-verified
snapshot is required before publishing `complete.bin` last.

Export buffers one chunk. Transfer verifies and copies one chunk at a time, then
performs complete destination verification. Recovery takes a shared OS lock,
performs no writes and rebuilds all state indexes; writers take an exclusive
lock. Incomplete stores are never returned as recovered states. A crash after
the final rename can already leave a complete, verifiable snapshot, so retry is
idempotent. ENOSPC can leave an exact partial pending file; freeing capacity and
retrying the same anchor completes it without modifying the source.

Recovery always compares the exact external anchor with local pins. Old data
cannot satisfy a newer anchor. This lab store does not maintain a monotonic
checkpoint authority: restoring both an old directory and an old caller anchor
cannot be detected locally. A future integration must retain the existing
independent rollback/finality protections outside the replaced snapshot directory.
No pruning decision uses this codec. No finalized history is discarded.

## Capacity measurements

Results are recorded in [the capacity evidence](evidence/yamichain-m14-capacity.json).
The benchmark runs in isolated containers with a two-CPU quota, 2-GiB memory limit,
no additional swap, no network and no concurrent build/test load. Each case uses
the requested number of **both** accounts and names, distinct valid Ed25519
account keys, nonce one and maximum-length 63-byte labels. Independently encoded
v1 root preimages supply expected roots. The large fixtures are synthetic
application states, **not signed, BFT-finalized histories**; this measures storage
capacity and recovery, not extended consensus admission.

| Names + accounts | Snapshot allocated MB | Chunks | Save + verify s | Transfer + verify s | Recovery s | Peak RSS MiB | Full root ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100,000 + 100,000 | 13.93 | 63 | 0.92 | 0.89 | 0.79 | 81.6 | 16.3 |
| 250,000 + 250,000 | 34.79 | 157 | 2.36 | 2.41 | 2.26 | 166.0 | 40.9 |
| 500,000 + 500,000 | 69.55 | 312 | 4.95 | 4.72 | 4.65 | 329.2 | 76.5 |

At 500,000 pairs the manifest is 14,179 bytes and logical snapshot size is
69,514,529 bytes. The complete build/fixture/export/transfer/recovery campaign
uses 38.12 CPU seconds (38.56 seconds elapsed); its cgroup memory peak is
476.0 MiB including page cache. A new empty proposal needs 75.9 ms, essentially
the full 76.5-ms root computation. Hashing dominates this empty-block operation,
but is only about 1.6% of bulk recovery time; key validation and index rebuilding
still dominate import. The measured full hash is nearly linear across these
capacities. This is a per-node laboratory measurement, not a consensus throughput
guarantee.

Save and transfer timings include full destination readback, key validation,
index construction, nonce-count checks and root verification. Single-process
recovery is the mean of three fresh imports; the harness also checks v1 export
refusal (and serializes v1 at 100,000 entries) inside that timed loop. Fresh-process
concurrency runs four independent read-only imports together, repeated three
times, sharing one combined two-CPU/2-GiB limit. OS page cache is not dropped:
these are fresh process/index recoveries, not cold-disk measurements. RSS is
reported separately from cgroup memory peak, which also includes page cache.

| Concurrent fresh imports | Mean round s | Combined sampled RSS MiB | Cgroup memory peak MiB |
| --- | ---: | ---: | ---: |
| 4 × 100,000 pairs | 1.98 | 95.7 | 97.0 |
| 4 × 250,000 pairs | 4.93 | 227.9 | 229.1 |
| 4 × 500,000 pairs | 11.16 | 471.2 | 474.8 |

The 100,000-entry state round-trips through the existing v1 snapshot decoder.
At 250,000 and 500,000 entries, v1 serialization still returns `Error::Limit`.
Chunking does not authorize old pruned nodes to accept larger checkpoints.

The unchanged flat root includes height near the beginning of its SHA-256 input.
Every new height therefore still needs a full state hash, including an empty
proposal. A snapshot format cannot remove that cost while preserving the digest.
Root timings and empty-proposal timings isolate this limitation from key/index
validation during bulk recovery. Full-history replay at growing state size still
accumulates that per-block cost; these fixtures are not a replay-latency benchmark
of 500,000 signed registrations.

Single snapshots require about 139 bytes per name/account pair with these labels,
plus small manifests and file rounding. Keeping both export and received copies
doubles that footprint; retained immutable generations, filesystem allocation,
archives, WAL, evidence and signing logs require additional disk. This establishes
that a 500,000-pair snapshot itself fits comfortably below 20 GB; it does **not**
establish indefinite total-node operation within 20 GB.

## Validation and security limits

Formatting, strict workspace Clippy with the feature both disabled and enabled,
and release workspace/example builds passed. Default Rust regressions passed
554 tests with 10 existing ignores; feature-enabled all-target tests passed 25,
and the final feature-unit rerun passed 17 after strengthening the ENOSPC test to
require errno 28 from the interrupted transfer itself. The six existing Python
tooling tests passed. Log and source/binary hashes are recorded in
[the validation evidence](evidence/yamichain-m14-checks.json).

The default regression suite preserves the fixed consensus vectors and independent
legacy state-root/snapshot/execution oracle. A real signed 64-registration state
also round-trips through v2 with exactly equal v1 bytes. Additional tests cover
manifest truncations, limits/overflow, forged roots, rehashed malformed records,
duplicates across chunks, corruption, missing/extra chunks, rollback, conflicting
pending bytes, symlink/hardlink redirection and unchanged safety namespaces.

Separate child processes exit at publication boundaries during both export and
transfer. The capacity lab repeats transfer interruption at larger size, runs
concurrent fresh recovery processes, rejects hostile copied directories and
checks that a production validator refuses a lab directory without changing any
file. Actual tmpfs block and inode exhaustion exercise errno 28, partial-write
recovery and retry. These are process-crash and real filesystem-pressure tests;
they do not simulate storage hardware lying about fsync or physical power loss.

The additional [live v1 interoperability campaign](evidence/yamichain-m14-v1-interop.json)
passed with the frozen M13 validator and default M14 validator: four archive/pruned
nodes converged at height 128, then restarted to the same height-160 state with
10,240 signed registrations. Old readers accepted the unchanged segmented archive;
compact bootstrap, historical synchronization, five archive-migration crash
boundaries and four compact/pruning/import crash boundaries preserved authenticated
state and safety records. An initial attempt stopped safely at the existing
filesystem reserve after all nodes reached the same height-128 state: temporary
build/container artifacts had consumed the workspace headroom. Removing rebuildable
build artifacts and rerunning resolved it; no reserve or safety check was weakened.

The generic streaming `Read` interface enforces byte bounds, but network deadlines,
peer admission and aggregate concurrent-transfer quotas belong to a future
transport integration. This laboratory includes no production network receiver,
checkpoint signature envelope or large-state protocol activation. Recovery
requires complete in-memory indexes and a full root hash. The format has not had
an independent security audit.

## Required compatibility and activation work

1. Separately approve capacity, memory/time budgets, snapshot/compact envelope
   versions and activation policy. Do not edit the v1 entry or byte constants.
   Approve any execution-state limits explicitly; no new limit is silently
   introduced by this experiment.
2. Specify a production versioned checkpoint envelope binding the exact manifest
   and state context to independently verified finality. Define ancestry,
   committee transitions, trusted genesis, rollback anchors and manifest retention.
   Preserve old certificate/history verification and the existing pruning rules.
3. Add explicit version negotiation and reader capability checks. Deploy compatible
   readers and operator recovery/backup tooling before crossing v1 capacity.
   Older pruned nodes cannot import an oversized v1 state; an archive fallback
   does not make the old compact reader compatible. Plan quorum availability and
   a rollback policy that never discards finalized history.
4. Run real signed, finalized large-state histories and replay with maximum account
   diversity, key/label distributions, retained forks, adversarial peers, bounded
   transport deadlines and simultaneous live consensus load. Establish latency,
   memory and disk admission budgets and independently audit the implementation.
5. Require approved, certificate-verified v2 checkpoints and independent rollback
   anchors before allowing any future pruning integration. Keep this lab namespace
   and opaque API separate; do not reinterpret experimental bytes as production
   checkpoints or auto-migrate stores.
6. If full-state hashing exceeds the chosen per-block budget, propose a separate
   consensus root version with deterministic authenticated-tree semantics and a
   verified bridge from the old root. Changing the snapshot codec alone preserves
   the existing linear hashing cost. Updates/deletions also require separately
   approved transaction semantics and nonce invariants.

No part of this activation plan is enabled by M14.

## Reproduction

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy -p yami-chain --all-targets --features experimental-snapshot-v2 --locked -- -D warnings
cargo test --workspace --locked -- --test-threads=1
cargo test -p yami-chain --all-targets --features experimental-snapshot-v2 --locked -- --test-threads=1
cargo build --workspace --release --locked
cargo build --release --locked -p yami-chain --features experimental-snapshot-v2 --example snapshot-v2-bench
cargo test -p yami-chain --features experimental-snapshot-v2 --lib --no-run --message-format=json
python3 -m unittest discover -s tools -p 'test_yamichain*.py'
python3 tools/yamichain-v2-lab.py --binary SNAPSHOT_V2_BENCH \
  --test-binary FEATURE_UNIT_TEST_EXECUTABLE --production-validator DEFAULT_VALIDATOR \
  --output NEW_DIRECTORY --sizes 100000 250000 500000
python3 tools/yamichain-storage-lab.py --segmented-archive --transactions 64 \
  --binary DEFAULT_VALIDATOR --legacy-binary M13_VALIDATOR --output NEW_PROCESS_DIR \
  --height 128 --restart-count 1 --restart-step 32
```

Use the executable reported by Cargo for the feature-enabled library unit tests,
not an integration-test executable. The Docker harness checks that each real
ENOSPC invocation executed exactly one passing test. Pin/freeze executables before
measurement; run capacity cases without concurrent Rust builds.
