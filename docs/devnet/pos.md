# Live private-stake Malachite devnet

This opt-in **software-only disposable-YAMI network** selects the running
Malachite committee from the existing encrypted staking ledger. It is separate
from the fixed economics devnet. Production financial activation remains closed.
M16's checkpoint/recovery activation restrictions and financial rejection checks
are unchanged.

```sh
./scripts/yamichain-pos-devnet start
./scripts/yami-wallet --network pos create work/operator.secure
./scripts/yami-wallet --network pos validator-key work/operator.validator
./scripts/yamichain-pos-devnet faucet work/operator.secure 1000
./scripts/yami-wallet --network pos register work/operator.secure work/operator.validator
# Use the consensus public key printed by validator-key:
./scripts/yami-wallet --network pos stake work/operator.secure VALIDATOR_PUBLIC_KEY 200
./scripts/yamichain-pos-devnet join-validator work/operator.validator 4
curl http://127.0.0.1:30780/committee
./scripts/yami-wallet --network pos balance work/operator.secure
./scripts/yami-wallet --network pos verify FINALIZED_HEIGHT
```

`join-validator` starts a separate Malachite node with the supplied independent
consensus and transport keys. It does not copy a validator's state or grant a
vote. The new node synchronizes certified history through the network. A
registered candidate with sufficient locked self-stake can be selected at the
next epoch boundary; selection is permissionless and independent of endpoint
hints. The local supervisor supplies a fifth endpoint slot for this demonstration.
Additional private endpoint slots can be configured before genesis.

The example uses six finalized transitions per epoch, minimum self-stake 100,
four active members (up to eight supported within the certificate byte budget), a six-block evidence window and fifteen-block unbonding.
Four public development bootstrap identities have real locked allocations from
the finite treasury; no stake or rewards are minted. Each epoch samples eligible
validators **without replacement, weighted by total locked stake**. Members have
equal BFT voting power after selection, keeping individual stake amounts private.
The seed is deterministic and predictable in this disposable devnet; it is not a
production randomness beacon or a claim of production economic security.

Every executor independently computes the next selection. Its witnessed signed
transition contains only the next consensus/transport public-key pairs. Account
owners, stake amounts, payment fields and rewards stay inside encrypted execution.
The preceding committee must finalize the containing block before the signer,
proposal verifier and Malachite height parameters can use the next committee.
Speculative proposal execution never installs committee authority. Selected
consensus and transport keys remain reserved from application signing, including
after they leave. Idle proposal waits are bounded so history/status requests
remain available during catch-up.

Consensus signatures and archives bind the network's genesis allocation/policy
scope, committee, height, round and step. Unknown future committees fail closed.
Synchronization requests one record at a time and sequentially reconstructs the
same authority. Restart replays the complete certified archive before opening
consensus; missing/corrupt records, contradictory schedules, identity replacement
and missing signing WALs refuse startup. Native snapshot/pruning shortcuts remain
unavailable. Candidate peer authentication grants connectivity, never voting power.
Public peer endpoints are still rejected by this development runner.

Fees go to the committee that finalized the operation, including on boundary
blocks. Each owner receives an equal integer share, with the remainder rotated
by finalized height. There is no minting or delegator reward-sharing activation.

```sh
./scripts/yami-wallet --network pos exit work/operator.secure VALIDATOR_PUBLIC_KEY
./scripts/yami-wallet --network pos unstake work/operator.secure VALIDATOR_PUBLIC_KEY 200
# Keep the validator online until /committee no longer contains its key.
# After the encrypted portfolio's ticket unlock height is finalized:
./scripts/yami-wallet --network pos withdraw work/operator.secure TICKET_ID
```

Exit/unbonding affect selection at the next boundary. Funds remain slashable
past the last possible active slot and evidence window. `slash WALLET LEFT RIGHT`
submits two actual encoded conflicting consensus votes through an encrypted,
fee-authorized operation. Executors reconstruct the historical committee and use
the existing objective double-vote verifier. Forged, nonconflicting, cross-network,
future, expired and repeated-slot evidence is rejected. An accepted offense
removes half of each remaining bond and withdrawal ticket (rounded up), credits
the original treasury and excludes the validator from subsequent selections.
Reversing the evidence or supplying a third conflict cannot charge the same slot
again. A committee shortage refuses the boundary operation instead of silently
reducing the safety threshold or retaining unbacked members.

```sh
./scripts/yamichain-pos-smoke
./scripts/yamichain-pos-devnet stop
./scripts/yamichain-pos-devnet start
```

The smoke requires fresh dedicated data and wallet paths. It checks registration,
real network synchronization, selection, signed Malachite commits, fee revenue,
eight missed blocks across an epoch, crash recovery with the original WAL,
objective slashing against an unbonding ticket, replay rejection, delayed withdrawal,
full network restart and independent history verification. The checked-in
`pos-smoke.json` contains the executed public result. Preserve the entire data
and signing directory as described in [the native devnet guide](README.md).

Encrypted wallets, staking lifecycle and objective evidence verification reuse
the existing implementations. The fixed economics profile remains available
unchanged. Hardware custody, attestation measurements, production randomness,
public-network deployment and production financial activation are not enabled
by this software integration; changed executor images still need their separate
hardware build and measurement approval process.

## Executed validation

The recorded run selected the new validator at height 13, paid its first fee at
14, recovered it across eight missed blocks, verified its subsequent commit at
23, slashed 100 from the unbonding ticket and completed withdrawal after unlock
40. All five nodes preserved the same head across restart; independent replay
verified 41 finalized blocks. The positive speculative-proposal check also
confirmed that future committee authority remains unavailable until finality.

[Lifecycle evidence](pos-smoke.json), [encrypted wallet verification](pos-wallet-smoke.json)
and [validation checks](pos-validation.json) record the executed results. The
existing encrypted wallet successfully imported the disposable operator key,
created/restored an authenticated encrypted backup and independently verified the
entire dynamic-committee history. The example uses disposable identities only.

```sh
cargo test -p yami-chain-consensus --features insecure-devnet-input --lib
cargo test -p yami-chain --features insecure-devnet-input --test native_devnet
cargo test -p yamichain-devnet-cert
cargo test --manifest-path experiments/yamichain-tee-m20/Cargo.toml --tests
cargo test --release -p yami-chain -p yami-chain-privacy -p yami-chain-consensus \
  --features yami-chain-consensus/large-state-lab,yami-chain-consensus/inactive-checkpoint-v2 \
  -- --test-threads=1
cargo clippy -p yami-validator -p yami-chain-consensus -p yami-chain \
  -p yamichain-devnet-cert --all-targets --features insecure-devnet-input -- -D warnings
cargo clippy --manifest-path experiments/yamichain-tee-m20/Cargo.toml --all-targets -- -D warnings
```
