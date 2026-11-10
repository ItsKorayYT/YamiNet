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

## Signed discovery and certificate propagation

The native software PoS profile supports permissionless peer admission with
`"p2p_discovery": true` in the trusted network configuration. The existing fixed
and isolated profiles retain their address and financial gates. Discovery does
not authorize public deployment or real-money activity.

Each node initially knows at most eight seeds (`discovery_seeds`, defaulting to
the first consensus endpoint). It learns signed endpoint advertisements through
other peers and dynamically establishes authenticated Malachite connections.
The gateway submits an encrypted payment certificate to one reachable validator;
nodes propagate and independently validate it. Other proposers no longer require
separate gateway submissions. Only canonical witnessed private certificates are
relayed: wallet keys, owners, balances, amounts, stake data and query payloads are
not part of discovery or transaction gossip.

A standalone joining process sets `YAMI_NATIVE_POLICY` to the independently pinned
public policy, `YAMI_NATIVE_KEYS` to its own 64-byte consensus/transport secret-key
file, `YAMI_NATIVE_DISCOVERY=1`, `YAMI_NATIVE_DISCOVERY_LISTEN=IP:PORT`, and
`YAMI_NATIVE_DISCOVERY_SEEDS=IP:PORT,...`. Protect the identity file as mode 0600.
It needs no validator RPC slot, backend credential or manually supplied peer
identity. `--index` is a local store/fixture selector, not validator authority.
The same explicit numeric IPv4 port serves consensus TCP and discovery UDP.
Persist the whole node journal, WAL, identity and signed hint checkpoint when
restarting. Candidate peers can synchronize before registration; voting requires
the existing private registration, locked stake and finalized committee selection.

Advertisements bind both key-possession signatures, the network scope, address
and expiry. Known consensus identities must match the finalized transport key;
unknown peer proofs cannot borrow reserved identities. Membership and voting
power never come from endpoint hints. Discovery contains no owner-to-validator
mapping, monetary weights or private financial state. Public development fixture
keys cannot advertise or bind outside loopback. A distinct private-key node may
participate in this software profile without changing that gate.

Bounds: 40 retained peer hints, 16 dial targets, four dial changes per two-second
window with 100 ms request deadlines, including uncertain updates in the 16-entry
tracked dial cap. SDK target counts are eight inbound/16 outbound peers; its
fourfold transport headroom gives hard caps of 32 incoming/64 outgoing TCP
connections and four per peer, with eight per IP. Discovery pages contain at most eight
advertisements. Certificates are at most 30,000 bytes; datagrams are bounded at
32 KiB. Each window drains at most 128 packets and verifies at most 32 packets
(four per non-loopback IP; 32 for a loopback process campaign). Advertisements
expire after ten minutes, with 30 seconds of receive clock skew allowance.
Nonvoting hints can be evicted from a full cache; active finalized identities
are protected and prioritized. Expiry and stored endpoint freshness bound replay;
certificate height/head validation and immutable admission prevent transaction
reapplication or conflicting queue replacement.

Reproduce the executed five-process demonstration on a fresh disposable network:

```sh
python3 tools/yamichain-p2p-smoke.py --out work/my-p2p-check --port-offset 5000
```

The fifth endpoint is absent from the configured inventory. It uses only its
public policy, own identity and one seed, synchronizes before registration,
registers/stakes, is selected, signs native finality, and catches up after a
restart. The test also injects an invalid discovery burst and verifies private
fields are absent from peer hints and relayed certificates. Results are in
[permissionless-p2p-validation.json](permissionless-p2p-validation.json).
These are independently running processes on one machine, not independently
administered trust domains. WAN loss/fragmentation, NAT/IPv6 support, seed diversity,
Sybil/eclipse resistance and cross-operator partition/adversarial campaigns still
need release validation. UDP retransmission is periodic; this is not a claim of
production networking readiness or a production randomness beacon.
