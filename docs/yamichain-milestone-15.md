# YamiChain Milestone 15: large signed histories

Base: `1394f43`, branch `development/yamichain`. This milestone adds real-history
stress tooling, a bounded volatile execution cache and an opt-in distinct-account
naming workload. It changes no block, transaction, certificate, snapshot-v1 or
state-root encoding; raises no production limit; and does not activate v2 or
connect it to production recovery/pruning. YPP, payments, staking and production
elections remain disabled. All 65,536 financial proof versions remain rejected.

## What was actually exercised

There are two deliberately separate kinds of evidence:

- **Live Malachite consensus:** four independent private-network validator
  processes, separate randomly provisioned identities and original safety stores,
  produce 1,640 blocks containing 104,960 genuinely signed registrations with
  distinct accounts. Certificates come from the real engine. The campaign includes
  a quorum-losing network partition, SIGKILL, original-identity restart and
  authenticated peer-history catch-up, followed by full replay of all four stores.
- **Signed/quorum-certified history fixtures:** independent 100k, 250k and 500k
  name/account cases start from genesis and sign, propose, execute, certify, verify
  and persist every block. These are complete actual transaction histories, not
  synthesized maps/snapshots. Four fixture signers reserve their preimages before
  signing valid precommits, and every certificate is verified. They are constructed
  by a controlled fixture author, **not finalized by a running Malachite engine**.
  Their construction throughput must not be read as network finality throughput.

The fixture contexts have distinct, pinned genesis IDs using existing future
committee schedules (not reached during the experiment). This avoids signing
conflicting case histories in one consensus domain. Fixture signers never become
live identities and have no engine WAL. Observer archives use the existing valid
identity binding for index 99, which is absent from every epoch in these fixture
contexts. Both current and previous validators refuse rebinding them to a scheduled
identity. Signer journals themselves retain their reservations and are not a
substitute for a live validator's WAL. Historical prefix copies are observers too.

No 250k/500k **live-engine finality** result is claimed. The larger fixture cases
exercise signed execution, valid quorum certificates, archives, forks and recovery;
extended live-engine runs at those capacities remain a production-readiness gate.

## Live consensus and authenticated synchronization

Each validator has an enforced two-CPU quota and 2-GiB memory limit, with no
additional swap. After initial startup all four share host CPUs 0–1; validation
builds use CPU 2. The restarted container's affinity is reapplied. Private Docker
networks expose no public service, and each validator sees only its own private
identity plus public roster/bootstrap material.

The original campaign reached an identical height-1,640 tip/root on all four nodes.
From first to last observed finalization, throughput is **150.7 registrations/s**,
including the fault window. Median inter-finalization interval is approximately
**404 ms**, with p95 about **415 ms**. These are finalized-event observation
intervals, not transaction-submission-to-finality latencies.

At heights 1,563–1,564, disconnecting two validators removed quorum. The maximum
observed height did not advance during the 12-second observation window. Healing
restored progress. Node 0 was then actually SIGKILLed around height 1,567, with its
irrevocable signing minimum retained outside the volume. Restart used the same
identity, original WAL and original signing/archive data. Old signing prefixes
remained intact. It authenticated and applied 58 peer records at heights
1,570–1,627; first-to-last observed acceptance spanned 4.30 seconds, about **849
registrations/s** between records. That span excludes the first-record cost and
cold archive replay and is not a general WAN synchronization throughput claim.

Completed live processes peak around 44–46 MiB RSS; the pre-SIGKILL cgroup witness
records a 69.4-MiB memory peak. Three uninterrupted validators consume approximately
58–69 CPU seconds each over their approximately 730-second run/linger intervals.
The killed attempt lacks final `getrusage`; its separate pre-crash cgroup witness
is retained instead. Each stopped store occupies about **21.1 MB allocated**, with
19–20 files, roughly 1 MB of signing data and about 2 KB of terminal WAL. All archive
history remains present. Host-side final inspections are explicitly distinguished
from cgroup-constrained recovery measurements.

The final delivery binary then restarted all four original stores with independently
retained signing and archive minima, reaching height 1,672 and **107,008 distinct
accounts/names**. Direct, opt-in timing observes matching local value-builder start
or authenticated proposal admission through the engine's durable finalized event.
Local-builder samples have median **401 ms**, p95 **407 ms** and maximum **42.4 s**;
the maximum includes a staggered cold restart while other validators were still
recovering. These local observations do not claim a public transaction-submission
API or a WAN latency SLA. The untrusted disk/proposal paths still execute or reuse
only an exact already-verified input; timing cannot authorize state.

An initial follow-up harness used the wrong private archive metadata field as its
minimum root. Validators refused it before WAL/consensus. The corrected harness
queries the existing archive API and the rerun passed with the protections intact.
The full live-engine archive was then replayed and exported/imported only through
the experimental v2 lab codec: save/readback **0.965 s**, recovery **0.889 s**,
21.25 seconds for combined full replay and lab audit, **60.0 MiB peak RSS**. Every
original archive, WAL and signing byte was checked unchanged by that audit. This
is not a production recovery path. Raw measurements are retained in
[the live evidence](evidence/yamichain-m15-live.json).

## Capacity profiles on complete signed histories

Sequential isolated containers enforce two CPUs and 2 GiB, no additional swap and
no network. Profiles use distinct valid Ed25519 owners, nonce zero per new account,
17-byte labels, at most 64 transactions per block and a partial terminal block
where necessary. They execute from genesis and replay the complete durable
history. The eight retained branches contain genuinely different next-block
registrations; they are not merely serialized synthetic states. OS page cache is
not dropped, so these are fresh-index/process recoveries, not cold-disk tests.

| Names and accounts each | Certified construction s | Construction registrations/s | Full replay s | New-height hash ms | V2 recovery s | Peak RSS MiB | Archive allocated MB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 100,000 | 79.36 | 1260.0 | 16.04 | 13.8 | 0.80 | 57.8 | 18.98 |
| 250,000 | 227.34 | 1099.7 | 82.66 | 34.4 | 2.45 | 139.3 | 47.42 |
| 500,000 | 742.09 | 673.8 | 296.95 | 63.6 | 4.38 | 274.7 | 94.81 |

At 500k, proposal construction consumes 294.30 seconds, execution 290.55 seconds,
and certificate/signing-log/archive persistence 134.53 seconds over 7,813 blocks.
The complete profile consumes 946.29 CPU seconds over 1,054.89 elapsed seconds,
with a 477.0-MiB cgroup peak. The sequential flat hash cannot use the second core
to eliminate its dependency on all preceding bytes. The eight divergent retained
forks take 1.03 seconds to construct/execute; their immutable maps share storage.
The v2 snapshot occupies about 46.5 MB; the complete case with archive, two v2
copies and fixture signing/profiling journals occupies about 196.4 MB allocated.

Four fresh 500k v2 recoveries together finish in 9.88 seconds, with 625.2 MiB
sampled combined RSS and 626.4 MiB cgroup peak under one shared two-CPU/2-GiB
limit. Four independent full 100k archive replays together take 44.31 seconds
under that same combined limit. See [the history evidence](evidence/yamichain-m15-history.json).


Proposal/execution totals cover the complete growing trace. Certificate/persistence
totals include four durable signing reservations per block, certificate creation
and verification and archive append. Empty-proposal time averages three hashes at
the final state size, with unchanged height framing. Cached-root timings measure
the existing immutable root cache and must not be mistaken for full-state hashing.
RSS includes retained forks, snapshot verification and replay; cgroup peak also
includes page cache. Archive allocation excludes the separate fixture signer
journals and v2 export/received copies; evidence also records full case allocation.

The 100k v1 state serializes to 9,300,090 bytes. At 250k and 500k, v1 still returns
`Error::Limit`. No unapproved v2 output is installed as a compact base. The same
experimental codec is evaluated on the live-engine state separately, through full
authenticated archive replay and opaque lab export/import, without production
recovery integration.

## Safe optimization and its limits

`Candidates` keeps **one** volatile validated result, keyed by the complete parent
context (genesis, height, tip and existing root) and exact complete block equality.
It is populated only after successful ordinary `State::execute`. Identical
retransmissions and a matching decision can clone that verified immutable result.
A different block, parent, context or root executes normally. Failed inputs do
not replace a good entry; finalization evicts it; restart begins empty.

Proposal authentication still runs for every admission, including disk-backed
retransmissions. Candidate payload identity/encoding checks and every decision's
certificate verification remain mandatory. Neither disk cache membership nor a
sidecar can supply a trusted state. All other execution, signatures, error
precedence, nonce/ownership checks, signing reservations, WAL and history verification
retain their old paths. The complete block comparison prevents a changed proof or
transaction from hitting the cache.

At 100k entries, paired authenticated-retransmission admission drops from 17.12 ms
to 0.166 ms; at 250k, from 39.10 ms to 0.180 ms; at 500k, from 67.28 ms
to 0.133 ms. The 63.6-ms new-height hash accounts for most of that uncached
500k admission cost. The paired profile performs the same
disk decoding and proposal authentication, clears only the volatile execution
entry for the control, and verifies results against ordinary execution. These are
cache-hit measurements, not a claimed 100× consensus throughput improvement.
Invalid cached metadata/payloads, signatures, alternate parents/contexts, changed
headers and restart are adversarially tested.

The unchanged flat root includes height near the beginning of the input. Every
new height therefore still requires a full canonical hash. Cached execution
removes avoidable repeated work but cannot make new-height hashing incremental.
Growing full-history replay remains approximately quadratic in application-state
growth; this, rather than archive file overhead, is a central remaining limit.

## Faults, validation and remaining gates

The large-history campaign rejects corrupted and missing segments and a
cross-genesis history substitution. A separate **same-chain** rollback test copies
a complete valid certified prefix through height 1,562 (99,968 names), then
requires the independently retained height-1,563 tip/root: recovery rejects it
without returning a partial state. The observer identity fence is checked with
both the previous and final validator binaries; no signing WAL is created.

On the genuine 100k history, bounded tmpfs block and inode exhaustion each produce
actual errno 28 during production v1 checkpoint export. No partial final export
is published, all source archive bytes remain unchanged, and export succeeds after
freeing only the test fillers. Six additional block/inode × append/migration/
recovery archive tests exercise actual storage pressure and process crashes at
intent/publication boundaries. They use small low-level storage fixtures and are
reported separately from the large authenticated-history tests in
[the archive-fault evidence](evidence/yamichain-m15-archive-faults.json). These are
process/filesystem tests, not physical power-loss or faulty-hardware simulations.

Formatting, strict Clippy with and without lab features, release builds and full
workspace regressions pass. Default regressions contain 556 passing tests and 10
existing ignores. The original optimized regression run exposed a browser test
assuming 30 renders consume enough wall time to animate a scroll. The test now
supplies 16-ms animation ticks explicitly; product rendering behavior is unchanged.
Feature tests pass 67 tests with one existing ignore, and the six existing
tooling tests pass; evidence records exact
counts, source/binary hashes and log hashes in
[the validation evidence](evidence/yamichain-m15-checks.json).

Performance binaries were frozen before adding the observer-fencing/prefix CLI
utilities. The hot execution/cache and root algorithms are unchanged. All fixture
archives were explicitly fenced afterward with existing valid identity metadata;
no history bytes changed. Recorded timed archive allocation precedes those two
small metadata files; evidence also records post-fence allocation. The delivery
generator fences before creating history, and its complete small campaign tests
current/previous-reader refusal, pressure and same-chain rollback.

The real live result establishes the exercised 100k-scale workload and faults,
not universal consensus safety or a production capacity guarantee. Remaining work
includes large live runs through 250k/500k and committee transitions, sustained
Byzantine traffic, large competing-round payloads, cold-disk/hardware power-loss
testing, deadline/admission budgets, backup/restore drills and an independent
security review. Larger v1 pruned checkpoints remain incompatible. Execution
already could grow past the v1 snapshot cap; this experiment does not fix that
production recovery boundary by changing a constant.

The snapshot/archive sizes at these finite heights fit well below 20 GB. That is
not an indefinite node-storage guarantee: continuing registrations, retained
history, signing/evidence data, backups and generations continue growing. The
application implements registration, not deletion/garbage collection. CPU cost
from full-state hashing/replay grows even when disk usage remains modest.

The [concrete activation proposal](yamichain-v2-activation-proposal.md) separates
certificate-authenticated state roots from early manifest/chunk authority, orders
reader/writer upgrades, retains independent rollback minima and explicitly gates
all production recovery/pruning activation. No activation is implemented here.

## Reproduction

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --features yami-chain-consensus/large-state-lab --locked -- -D warnings
cargo test --workspace --release --locked -- --test-threads=1
cargo build --workspace --release --locked
cargo test -p yami-chain -p yami-chain-consensus --release --features yami-chain-consensus/large-state-lab --locked -- --test-threads=1
cargo build --release --locked -p yami-chain-consensus --features large-state-lab --example large-state-bench
python3 -m unittest discover -s tools -p 'test_yamichain*.py'
python3 tools/yamichain-large-live.py --binary FROZEN_VALIDATOR \
  --output NEW_LIVE_DIR --height 1640 --fault-height 1563 --cpuset 0,1
python3 tools/yamichain-large-history.py --binary FROZEN_LARGE_STATE_BENCH \
  --validator FROZEN_VALIDATOR --output NEW_HISTORY_DIR \
  --sizes 100000 250000 500000 --cpuset 0,1
python3 tools/yamichain-archive-faults.py --test-binary STORAGE_UNIT_TEST_EXECUTABLE \
  --output NEW_FAULT_REPORT.json --cpuset 2
python3 tools/yamichain-large-followup.py --binary FINAL_VALIDATOR \
  --source LIVE_DIR --anchor HEIGHT:BLOCK:STATE --output NEW_FOLLOWUP_DIR \
  --step 32 --cpuset 0,1
```

CPU affinity examples assume a three-CPU executor: reserve CPU 2 for builds/fault
checks while the measured workload uses 0–1. Do not run competing measured cases
on those CPUs. Freeze binaries and retain their hashes before starting a campaign.
The follow-up deliberately reuses original identity/safety volumes; fixture
signers and observer prefixes are never replacements for them. Bootstrap endpoint
hints must still be fresh and authenticated; do not bypass freshness checks.
