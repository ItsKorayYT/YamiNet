# YamiChain 0.2 — development status

Continues draft PR #14 without merging it or changing `main`. This remains a
local-only, non-financial Malachite 0.8.2 prototype with public development keys.
No YAMI, live stake balances, wallet, accepting proof verifier or payment network.
Browser, hosting, identity, DHT, games and YNS remain chain-independent.

| Milestone | Implemented | Unfinished / next gate |
| --- | --- | --- |
| 1. Security | Authenticated proposal parts; one durable payload per authenticated proposer/round within Malachite’s existing lookahead; bounded equivocation-pair retention; on-demand payload reads; no 64-value fatal quota; exact round recovery; signer reservations retained; regular-file/dangling-symlink/name checks; streaming history replay. | Public-network flood/soak review, disk pressure during prolonged stalls, Malachite audit and dependency remediation. |
| 2. Committees | Genesis-authenticated activation schedule; 1–100 members/set, up to 64 scheduled epochs; explicit positive powers with bounded total; height-specific Malachite sets, signing domains, certificate thresholds, sync and replay; transport-key binding; crash injection at epoch boundary. | Stake-selected membership, unbiased randomness, elections, live transition authorization/governance and independent transition review. |
| 3. PoS foundations | `staking_sim.rs`, compiled only under `cfg(test)`: signed registration with separate-key possession, nonce authorization, fixture backing, self-stake eligibility, delegation, bounded commissions/reward reserve, unbonding, jail/slash from genuine historical double-vote signatures, evidence replay protection, exact accounting. | No production accounting or economic parameters are approved. Review share/rounding/dust economics, evidence policy, real unbond horizon, elections and shielded backing. |
| 4. Privacy | [Pinned source/licensing comparison](specs/yamichain/v0.2/privacy-review.md) of current FCMP++/CARROT, Spark V2 and Orchard; no backend selected. | Full construction/dependency/audit review and measured end-to-end workloads; explicit backend/design approval. |
| 5. Payments | Isolated existing intents; release/debug rejection of every proof version; all unknown financial action tags refused; fee/burn conservation only in hermetic models. | Real proofs, wallets/keys/recovery and shielded staking/withdrawals require approved design, separate authorization and independent review. |
| 6. Resources/pruning | Locally authenticated checkpoints with replay budget and committed next committee; bounded per-record/per-request reads; prune only reproducible finalized candidate payloads; resource measurement driver. | Snapshot bootstrap and finalized-history/evidence/spent-tag pruning remain disabled. Native ARM64 and x86_64 sustained small-state workloads passed with enforced two-CPU/2-GiB limits; large-state/approximately-5-GB pruned-state validation remains open. |

## Run the non-financial rehearsal

Use a **new store directory**. Chain ID `yamichain-local-0.2`, protocol 2 and
`YC-GENESIS-2` deliberately reject v0.1 stores. Preserve those stores unchanged;
there is no automatic migration. No financial action or epoch change is accepted
through a transaction. Committee plans are externally pinned genesis inputs, not
updates a peer or validator may choose after startup.

```sh
cargo build -p yami-validator --locked
# Launch indices 0..6 in separate terminals, with distinct directories:
target/debug/yami-validator run --dir work/node-0 --index 0 \
  --committee-plan 1:4,4:7,7:5 --height 10 --timeout 180 --linger 8
# Stop a process before inspecting its store; supply the identical plan:
target/debug/yami-validator inspect --dir work/node-0 \
  --committee-plan 1:4,4:7,7:5
python tools/yamichain-local.py --report work/v02-process.json
python tools/yamichain-resources.py --report work/v02-resources.json
```

The local runner accepts only separate public development consensus/transport
identities. The Rust genesis/set infrastructure also supports explicit voting
powers; CLI plan fixtures use equal power and prefix-index membership. Proposers
rotate deterministically over canonical consensus-key order. Quorum is strictly
more than two thirds of authenticated height-specific voting power. Byzantine
safety assumes less than one third Byzantine voting power **in each committee**;
precommitting a schedule does not supply permissionless Sybil resistance.

An epoch takes effect solely from finalized height and the pinned genesis. The
previous height is certified/executed/durable before starting the next height.
Restart derives the same set from authenticated replay; no independent mutable
“current epoch” file can disagree with committed history. Changing the plan
changes genesis, state and signature domains and conflicts with the existing store.
`--crash-after epoch` exits once after a durable record before a scheduled next
committee starts. `--linger` serves terminal certified history without producing
further blocks; its bounded deadline is separate from the consensus deadline.

## Candidate admission and storage limits

Parts bind the complete block ID, height, round, valid round, proposer, genesis
and height-specific committee using an Ed25519 proposal signature. Admission accepts
the current round through Malachite’s existing ten-round lookahead, including before
`StartedRound`; older/out-of-window/undefined rounds cannot allocate. A second authenticated conflicting proposal is preserved as
one evidence pair, then further distinct payloads for that round are rejected
without repeated execution or signature checks. Exact duplicates reuse previously
authenticated data. Admission failure replies `None`; storage corruption/I/O failure
still fails closed. Synced values have already passed Malachite certificate checks
and are independently executed before storage/commit.

Acknowledged values are retained on disk until finality; there is no unsafe
last-in/first-out eviction of locked/reproposable values. Payload reads and round
recovery are bounded, but **disk use is not constant during an arbitrarily long
stalled height**: payloads, signing reservations, evidence and upstream WAL grow
with real rounds. The wire round/height caps and loopback containment are not a
production disk-pressure policy. Do not delete or roll back signing records/WAL.

Checkpoints are constructed only by sequential certificate verification/execution
from the caller's pinned genesis. Stored checkpoint bytes are accepted only after
recomputation. They are not independent snapshot trust anchors. Candidate pruning
never removes certified history, signing reservations, WAL or original evidence.
With no accepted payments there is no live spent-tag set to prune; any future
snapshot/pruner must retain exact spent-tag uniqueness and required history.
Windows directory-fsync/power-loss durability remains unverified; the filesystem
and safety records must not be modified concurrently by an attacker or rolled back.

See [subsequent scale/PoS-interface work](validation-yamichain-scale-pos.md),
[stress and hardware evidence](validation-yamichain-stress.md) and
[earlier validation evidence](validation-yamichain-0.2.md) for actual checks, failures,
measurements and unresolved advisories. No production-readiness claim is made.
