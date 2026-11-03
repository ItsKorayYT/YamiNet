# Milestone 9: compact checkpoints, pruning and fast recovery

This extends the existing journal, certified-history transport and snapshot code.
Archive storage remains the default. Compact recovery and pruning are explicit
options; no public deployment, staking, issuance, payment or production YPP
verification is activated. Existing YPP research and validator experiments remain
unchanged. The original `run_node(NodeOptions)` API still selects archive mode.

## Trust and verification

A compact checkpoint carries the complete canonical naming/nonce state, its
height and block ID, the last finalized block and BFT certificate, the genesis
commitment and next committee commitment. State bytes are limited to 16 MiB and
100,000 nonce/name entries each; the complete checkpoint is limited to 17 MiB.
The decoder rejects noncanonical ordering, duplicate keys/names, inconsistent
nonce/name counts, unsupported domains, truncation, padding and incorrect roots.

Fresh import requires **an independently obtained `HEIGHT:BLOCK:STATE` anchor**
and a separately pinned complete genesis/committee schedule. A peer's advertised
hash, filename, height, certificate or accompanying anchor does not provide that
independence. Obtain a recent finalized anchor from an authenticated operator
channel or an independently verified archive, independently of the snapshot
supplier. The CLI deliberately has no “trust whatever the peer sent” mode.
Software cannot establish the provenance of an anchor an operator pastes in.

Verification checks the exact anchor, genesis ID, certificate quorum and strict
signatures, height-specific committee, certified block ID/state root, next
committee and recomputed canonical state root before installation. Incoming
snapshots never set committee membership, powers, protocol features or validator
keys. Historical execution before an imported anchor is explicitly trusted, not
silently claimed to have been re-executed. Rotating committees remain pinned by
the locally authenticated genesis; snapshot-supplied committee history is refused.

Local checkpoint export first authenticates sequential history, or the previously
authorized local base plus the complete certified/executed suffix. Once installed,
the local compact base is trusted checkpoint metadata on restart, like the local
genesis file. It is rechecked cryptographically before replaying the suffix.
Privileged rollback/replacement of the entire store or trust configuration is
outside the remote-peer threat model. Preserve trusted metadata and backups.

## Archive and pruned validators

```sh
# Full independently executable/certified history, unchanged default.
yami-validator run --dir work/archive --index 0 --storage-mode archive

# Explicit bounded recent history; same consensus and signing rules.
yami-validator run --dir work/pruned --index 1 \
  --storage-mode pruned --checkpoint-interval 64

# Offline authenticated export from a stopped validator; never overwrites FILE.
yami-validator export-checkpoint --dir work/archive --out work/checkpoint.bin

# Fresh never-used identity/observer store. Obtain ANCHOR independently.
yami-validator import-checkpoint --dir work/fresh --snapshot work/checkpoint.bin \
  --anchor HEIGHT:BLOCK:STATE

# Authenticate pressure/history-floor information while stopped.
yami-validator storage-info --dir work/pruned
```

Supply the same `--committee-plan` for development schedules or
`--isolated-network CONFIG --index N` for Milestone 8 independent identities.
Status/inspection infer an initialized validator's index; freshly imported stores
without a validator index require an explicit `--index` for isolated profiles.

Automatic pruning runs every N finalized heights and checkpoints at head minus N.
The first height N is retained entirely. Thereafter the certified suffix contains
N through 2N−1 records between checkpoints, plus the single compact base. Short
partitions and ordinary P2P catch-up use that suffix. Sync advertises the actual
history floor, never history the node has deleted. A separate archive can serve
older history. A node older than every available history floor must explicitly
import an independently anchored checkpoint; it does not silently trust a peer.

The offline `prune --dir PATH` command explicitly checkpoints/prunes through the
current head, leaving no ordinary historical prefix. It is intended for stopped,
operator-controlled maintenance. Use automatic pruning for a retained catch-up
window. `status`, `inspect` and offline maintenance recognize compact stores;
`run` requires explicit pruned mode to reopen one, rather than claiming it is an
archive. The previous full-history snapshot APIs and their verification policy
remain available for archive stores.

A compact checkpoint is **not a validator identity backup**. Import refuses stores
with validator-index metadata, signing reservations, evidence or a WAL. Never
reuse a previously active key in a fresh directory to bypass that refusal; retain
its entire original safety store. The process lab uses a scheduled validator key
that has never voted before import. Existing validators restart with their own
original WAL and all signing reservations.

## Crash and deletion policy

The authenticated replacement base is written to one bounded pending file,
fsynced, atomically renamed, and its directory fsynced. Only after that succeeds
is the minimum trusted height/block/state marker durably installed and old records/receipts unlinked.
Every deletion derives its floor from re-verification of the exact installed base.
A crash during deletion can leave any mixture of old records present or absent;
all are below the verified base, and suffix replay remains valid. Repeated pruning
resumes safely. A missing/corrupt base, an older otherwise valid base, or a conflicting marker
fails closed; generic
archive replay explicitly refuses compact stores and cannot fall back to genesis.

Only canonical finalized records/receipts below the authorized floor and
reproducible finalized candidate/proposal payloads are deleted. **No signing
reservation, equivocation evidence, unresolved candidate, genesis, identity or
WAL is deleted by pruning.** Malachite's existing normal per-height WAL lifecycle
is unchanged; its bytes naturally change when new heights are processed.

External exports use a bounded pending file and atomic no-overwrite publication;
an incomplete export is never published as the requested output. Uncommitted
pending files in a journal are cleaned under its exclusive OS lock on restart.
External export directories remain operator-managed; stale pending exports there
require cleanup or a fresh output filename. No peer import copies signing or WAL
material from another validator.

Local process fault injection is available only through explicit offline options:
`--storage-crash base|unlink` for prune/import, and `--storage-crash export-temp`
for export. Matching boundaries exit 86. Tests also inject I/O errors before file
sync, after rename and before directory sync. These are private lab hooks, not
network APIs.

## Storage pressure and resource bounds

Journal writes check a default **18,000,000,000-byte logical store refusal budget**,
a **64-MiB reserve**, the directory-entry bound, and available filesystem blocks
on Unix. `run --max-store-bytes N` can lower/raise the explicit refusal budget.
The process periodically checks pressure as well. Existing committed records may
be read/idempotently synced at pressure; new signatures are not released when
their reservation cannot be durably written. ENOSPC and sync errors propagate;
there is no safety-record eviction or partially verified success.

The byte check covers history, retained safety data, WAL, snapshots and temporary
files visible in the store. Replacement needs room for the old base plus one
pending new base; only one active base is retained. Operator-created external
exports are individually bounded, not a globally quota-managed export service.
WAL writes and Milestone 8's bounded discovery cache use their existing writers;
the periodic pressure check is not a filesystem-enforced hard quota. Use an OS
filesystem quota/isolated volume to enforce an absolute 20-GB allocation and
protect against external writers or concurrent WAL growth. Inode exhaustion also
fails through normal I/O errors. Logical bytes do not measure inode/block overhead.

The implementation deliberately performs bounded directory pressure scans rather
than trusting an undercounted cache. That cost grows with retained signing files;
benchmarks must not be extrapolated to millions of entries. Signing reservations
and unresolved evidence still grow indefinitely and eventually force a safe stop.
Pruning live registry/nonce state, signing-reservation compaction, immutable safety
log segmentation, automatic checkpoint anchor distribution/renewal, snapshot
network download negotiation and hard filesystem quota provisioning are not
implemented. The existing demonstration height/round limits remain in force.
No claim of indefinite operation within 20 GB or Internet deployment readiness
is made.

## Reproduction

```sh
cargo build --release -p yami-validator
cargo build --release -p yami-chain-consensus --example storage-bench
python3 tools/yamichain-storage-lab.py --output work/m9-process
python3 tools/yamichain-storage-bench.py --output work/m9-bench \
  --process-lab-output work/m9-process
```

The retained Milestone 8 adversarial campaign exposed a finite forged-page
backlog delaying one endpoint update. The receiver now drains at most 64
datagrams cheaply per tick while retaining the four-page signature budget; it
still cannot promise discovery availability against continuous flooding.

The process lab runs four real validators in separate network namespaces on an
internal-only Docker bridge, with independent consensus/transport keys, no public
ports, two-CPU/2-GiB per-validator limits, capability removal and separate stores.
Three validators establish history; the fourth imports a pinned compact state
without prefix history and joins authenticated P2P suffix synchronization. Archive
and pruned nodes converge, then all restart and advance with their original safety
stores. Offline processes are interrupted during export, base publication, pruning
and import, then authenticated recovery and safety-file preservation are checked.

The benchmark runs independent real-signature chains at 64, 256 and 1,024 heights,
with eight non-financial naming transactions per block and pinned 4→7→4 committee
rotations. Each measurement uses a separate network-disabled two-CPU/2-GiB
container, reports child-process CPU and peak RSS, and measures archive replay,
compact export/import, pruned restart, checkpoint/history bytes and retained
signing bytes. The offline fixture retains one precommit reservation per height;
real validator storage is measured separately by the process lab. A bounded 8-MiB
tmpfs is filled to ENOSPC to verify refusal without deleting committed files.
This tests exhaustion mechanics, not a 20-GB endurance campaign or dedicated SSD
latency. Provisioning identities and raw stores stay in git-ignored `work/`.

## Measured results (2026-11-03)

Formatting, strict workspace/all-target Clippy, the workspace release build and
**512 workspace tests passed** (zero failures; 10 existing manual/environment
ignores). A strengthened optimized rollback regression also passed: after all
suffix history is removed, replacing the base with an older valid checkpoint is
rejected by the durable minimum anchor. The non-test privacy integration test
still rejects all 65,536 proof versions for payments and public protocol actions.

The final Milestone 8 regression passed after the bounded-drain fix: quorum loss,
256 stalled/malformed TCP attempts, 2,048 oversized UDP packets, 512 forged pages
from a known peer IP, forced kill, IP migration, authenticated recovery and durable
replacement-endpoint discovery on all four nodes. Consensus still needed its
pinned quorum; discovery supplied no voting authority.

The final storage process campaign passed in 133.61 seconds. The never-used fourth
identity imported a **14,330-byte checkpoint at height 64 with no prefix record
files**, joined authenticated P2P catch-up, and converged with archive/pruned nodes
at 128. All four then restarted with original safety stores and converged at 160.
Every original signing/evidence file remained unchanged across live advancement.
Offline fault cases preserved the WAL byte-for-byte as well. Process exits during
base publication, first unlink, uncommitted export and import all recovered safely.

At height 160 the archive retained 275,631 logical bytes; pruned validators retained
137,080, 135,077 and 106,227 bytes, including all their respective signing files,
the compact base and 32 recent certified records. The late-joining node had fewer
historical signing reservations because it had never voted before bootstrap.
Peak sampled validator cgroup CPU was 9.77% of one core. Containers shared the
managed host; this is not a geographically distributed capacity claim.

Offline real-signature measurements, eight naming transactions/block, pinned
4→7→4 committee rotations, two CPUs and 2 GiB RAM per separate container:

| Heights | Original store bytes | Snapshot bytes | Pruned store bytes | Archive replay | Import | Compact restart | Pruned restart (8-record tail) | Peak RSS |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 64 | 152,165 | 27,890 | 52,935 | 78.94 ms | 3.64 ms | 1.05 ms | 11.02 ms | 16,420 KiB |
| 256 | 606,925 | 105,954 | 161,335 | 368.81 ms | 5.52 ms | 2.21 ms | 15.51 ms | 16,672 KiB |
| 1,024 | 2,427,285 | 419,506 | 596,231 | 1,957.03 ms | 10.20 ms | 6.32 ms | 27.52 ms | 16,488 KiB |

The 1,024-height fixture retained **161,792 bytes of signing reservations** after
pruning. Its complete generation/export/import/prune/replay campaign consumed
13.70 CPU user seconds and 6.23 CPU system seconds. Generation took 15.65 seconds;
authenticated export took 1.92 seconds and pruning 3.78 seconds. Store-byte figures
include safety/receipt/genesis metadata; they are not packet-exact full-history
transfer sizes or physical SSD allocation measurements.

Filling an isolated **8-MiB tmpfs** to actual ENOSPC made pruning return failure
(`filesystem reserve exhausted`) with every committed file unchanged. A separate
quota test also refuses new reservations while preserving earlier ones and their
idempotent retries. During development, copied Docker roots exhausted build-disk
headroom and correctly stopped a P2P run; only rebuildable debug artifacts were
cleaned before the successful final campaigns. Raw failed experiments remain in
`work/`, alongside final data; no keys or stores are published.

Public, key-free evidence: [check summary](evidence/yamichain-m9-checks.json),
[storage process recovery](evidence/yamichain-m9-process.json),
[P2P regression](evidence/yamichain-m9-p2p.json), and
[CPU/RSS/storage benchmark and ENOSPC](evidence/yamichain-m9-bench.json).
The target 20-GB SSD was not filled for an endurance test. Archive remains the
fail-closed default; secure independent anchors, hard volume quotas, retained
safety-log growth, active-state limits and large-history scan costs remain
production deployment blockers. No public deployment is authorized or performed.
