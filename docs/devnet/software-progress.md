# Remaining software engineering — implementation ledger

Base assignment: `c1e749f8a84139bc61a9bec2b483a609a8012b4d` on
`development/yamichain`. This is a progress ledger, not a production-readiness
claim or a substitute for the remaining implementations.

Completed before this assignment: encrypted wallets, private native payments,
locked staking lifecycle, live stake-selected committees, fee accounting,
objective slashing and a real validator join/earn/exit/recovery demonstration.

Implemented next:

- Authenticated HTTPS gateway/executor/validator RPC, per-role credentials,
  certificate/hostname verification, redirect refusal and bounded admission.
- Pinned witness TLS with signed application authentication, bounded connection
  workers, absolute handshake/frame deadlines and refusal of remote plaintext.
  Operator-signed TDX provisioning covers the witness TLS trust configuration.
- Independent development TLS configuration with distinct ports/data, retaining
  existing devnets; wallet configurations need no backend credentials.
- Bounded service restarts, Linux child-lifetime enforcement, deadline-bounded
  worker pipes, periodic authenticated executor recovery and monitoring.
- A unified reproducible TLS software demonstration using the existing PoS and
  encrypted-wallet implementations, plus measured idle resource observations.
- Resumable private-executor history synchronization beyond the former
  256-transition gap: bounded authenticated pages, durable incremental witness
  progress, full-prefix verification before state installation and live TLS PoS
  interruption/restart validation. See [recovery contracts](recovery-pages.md).

- Private quorum-authenticated wallet portfolio replies now include per-validator
  delegations and explicit fee-recipient eligibility/reward terms. These query
  additions do not change state encoding, supply, fee allocation or selection.
  Validation: six economics tests and strict all-target/all-feature Clippy in
  `experiments/yamichain-tee-m20` passed on 2026-11-10.

- Explicit economic policy and signed fee-authorization versioning, with legacy
  version 1 canonical bytes retained and every unsupported 16-bit policy version
  rejected. No automatic policy upgrade or activation is provided. Seven economics
  tests and strict all-target/all-feature Clippy passed; see
  [policy versions](economic-policy-versions.md).

- Encrypted network-bound local outgoing transaction history, durable submission
  recording before transmission, native-finality status verification, bounded
  pagination, offline history and explicit byte-identical retry. A timeout never
  creates a replacement payment. The live TLS test checked offline history,
  restart preservation and idempotent retry with private balances 898/100.
- Bounded wallet HTTPS seed discovery with exact independently provisioned policy
  pins and a least-privilege profile exporter. The live wallet used an unavailable
  seed followed by the pinned gateway, without backend credentials or server keys.
  See [wallet validation](wallet-history-validation.json) and
  [wallet operations](economics.md). Strict Clippy passed for the experiment and
  core chain/storage/consensus/validator targets; 23 wallet/RPC/supervision Python
  checks passed. The complete release-mode all-feature experiment suite passed
  (including 273-transition recovery); native Intel/QVL-dependent tests remain
  explicitly ignored rather than treated as hardware validation.

- Live PoS peer proofs now enforce finalized consensus-to-transport bindings even
  in the software profile. Unknown candidates may authenticate for synchronization,
  but cannot borrow reserved transport identities or obtain voting power. Focused
  binding tests, the core consensus regressions and strict Clippy passed.

Still requiring software implementation/validation (not claimed complete):

1. Cross-operator/WAN validation, seed diversity, IPv6/NAT handling and stronger
   Sybil/eclipse/partition campaigns. Signed permissionless admission, live PoS
   identity binding, discovery, synchronization and single-ingress certificate
   propagation are implemented in the native software profile. No public
   production deployment has been authorized.
2. Authenticated large-history checkpoint bootstrap/pruning and archival growth
   management beyond the finite authority budget. Ordinary private financial state
   now persists incrementally; M16 activation gates remain closed and native
   financial history/sign-once records are retained.
3. Attestation-bound executor provisioning, confidential transfer, revocation,
   replacement, measured-guest encrypted RPC packaging and witness custody
   changes. In particular, recovery-custodian
   impersonation is not solved by TLS or process supervision.
4. Incoming and restored-wallet historical transaction reconstruction, public
   network discovery beyond independently configured pinned HTTPS seeds, and
   further delegation/reward UX. Local outgoing history and pinned seed discovery
   are implemented; neither claims permissionless network discovery.
5. Production-quality selection randomness, justified delegator reward policy,
   upgrade negotiation and activation of future economic policy versions. The current disposable
   deterministic seed is explicitly not a production randomness beacon.

External validation dependencies:

- Genuine TDX hosts, firmware and collateral/QVL verification for actual guest
  measurements and hardware recovery trials. No software-attestation fallback.
- Independently operated machines and credentials for cross-operator deployment,
  partition and adversarial recovery campaigns.
- Independent review of financial custody, rollback/revocation, randomness,
  checkpoints and monetary-policy transitions before production authorization.

Production financial proofs remain rejected for all 65,536 versions. No real
money, public deployment or production checkpoint writer/recovery/pruning is
activated by these commits. `main` is not modified or merged.

## Continuation — 2026-11-10

Validated and pushed on the shared branch:

- `be1356e`: private delegation details and explicit existing reward terms.
- `fb31c14`: economic policy/authorization version refusal, preserving version 1.
- `8b4fdda`: encrypted outgoing history, verified status/retry, pinned HTTPS seed
  discovery and least-privilege wallet profile export.

At `303be4c`, incremental financial persistence and permissionless P2P integration
were still unimplemented. Both priorities are now implemented and validated in
this branch, as recorded below. Other release requirements remain unfinished;
external dependencies are not completion claims for that remaining work.

Validation completed:

- Release experiment suite, all features: **51 passed, 0 failed, 11 ignored**.
  This includes the 273-transition interrupted/restarted recovery test. The
  debug run was intentionally stopped at sequence 116 to replace its expensive
  history verification with the same complete release-mode suite; no assertion
  or recovery test was removed.
- **23** wallet/configuration/discovery/RPC/supervision/worker-I/O Python tests.
- Strict Clippy for all experiment targets/features and for core chain, storage,
  consensus and validator targets with development input enabled.
- Production privacy-boundary test: all 65,536 proof versions and public value
  actions still refuse. Native wire-boundary tests passed with the development
  feature both disabled and enabled.
- Fresh unified TLS PoS and encrypted-wallet demonstration:
  [wallet-history-validation.json](wallet-history-validation.json).

Resume checkout without resetting or force-pushing:

```sh
git fetch origin development/yamichain:refs/remotes/origin/development/yamichain
git switch development/yamichain
git merge --ff-only refs/remotes/origin/development/yamichain
export CARGO_HOME=/workspace/.yami-tools/cargo
export RUSTUP_HOME=/workspace/.yami-tools/rustup
export PATH=/workspace/.yami-tools/cargo/bin:$PATH
cargo test --release --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml --all-features
cargo clippy --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml --all-targets --all-features -- -D warnings
cargo clippy --locked -p yami-chain -p yami-chain-consensus -p yami-chain-storage -p yami-validator --all-targets --features insecure-devnet-input -- -D warnings
PYTHONPATH=tools python3 -m unittest tools/test_yamichain_wallet_config.py tools/test_yamichain_wallet_history.py tools/test_yamichain_wallet_discovery.py tools/test_yamichain_rpc.py tools/test_yamichain_services.py tools/test_yamichain_worker_io.py
cargo test --locked -p yami-chain-privacy --test non_test_boundary
cargo test --locked -p yami-chain --test native_devnet
cargo test --locked -p yami-chain --features insecure-devnet-input --test native_devnet
python3 tools/yamichain-unified-smoke.py --out work/next-wallet-tls --port-offset 4500
```

Use a new demonstration directory and free port offset on every reproduction;
never overwrite a retained ledger. `work/continuation-wallet-tls` contains this
session's disposable private configuration, wallets and retained network state.
Do not commit its credentials or use its seven-day localhost certificates for
public deployment. Build/test logs are under `work/continuation-*.log` and are
local diagnostics, not durable evidence after an environment replacement.

Remaining release-candidate work after the two implemented priorities:

1. **TDX custody:** complete attestation-bound enrollment and confidential state
   transfer in `src/{tdx_guest,attestation,witness}.rs` and `deployment/tdx`.
   Recovery-custodian impersonation remains unresolved. Genuine guest devices,
   firmware/collateral and independently pinned native QVL verification are still
   required; TLS, encrypted chunks, copied measurements and ignored tests do not
   establish hardware security. Rebuild guest images and approve their genuine
   new measurements for the changed execution/storage code.
2. **Executor replacement/revocation:** implement attestation-bound recipient
   authorization, generation/key revocation and rollback-resistant witness custody
   changes. Preserve 3-of-4 financial authorization and sign-once/CAS guarantees.
   A copied executor/custodian secret must not become a replacement capability.
3. **Wallet recovery/history:** add authenticated confidential retention/query
   support for incoming and restored-wallet historical activity. Existing public
   blinded commitments cannot reconstruct private transfers. Local outgoing
   sidecars remain separate backups; password loss has no recovery bypass.
4. **Security validation:** independent review of the chunk format/manifest and
   legacy migration, crash/quota/corruption behavior, custody and anti-rollback;
   independent operator credentials/machines for partition, adversarial recovery,
   Sybil/eclipse and WAN/loss campaigns. The five-process P2P test is one machine,
   not five independent trust domains. New peers still gain no voting power from
   advertisements or synchronization. Keep all production financial gates closed.
5. **Scale and archival state:** checkpoint/root serialization and full witness
   checkpoint RPCs remain linear in financial state size. Wire checkpoints retain
   the 4 MiB limit, cache databases the 12 MiB page budget and authorities the
   1 GiB page budget. Exhaustion refuses new work. Add scalable authenticated
   large-history bootstrap/archival management before any reviewed pruning;
   certified history, nonce reservations and vote locks are never deleted to
   reclaim financial chunks. M16 writers/recovery/pruning remain inactive.
6. **Production policy:** reviewed selection randomness, delegator reward policy
   and upgrade negotiation/activation remain open. Only economic version 1 is
   supported. Do not replace deterministic seeds, fee allocations or canonical
   roots through configuration, or turn peer availability into validator weight.

External resources still absent in this environment: `/dev/tdx_guest`,
`/dev/tdx-guest` and `/dev/kvm`; configured outbound identities/secret bindings;
and independently operated validator/executor/witness machines. Hardware/QVL
trials, independent partition campaigns and production authorization therefore
remain blocked. Resource references were requested without asking for credentials
in chat. No such resources are inferred from the local TLS software demonstration.
Independent custody/checkpoint/randomness/economic review is still required before
opening any corresponding production or public-network gate.


## Incremental financial persistence — implemented 2026-11-10

Implementation commit: `3bc916d`.

Executors now persist authenticated content-defined encrypted chunks and a small
anchored manifest, reusing unchanged ciphertext. Witnesses persist those chunks
in the same SQLite transaction as revision CAS, nonce reservations, vote locks,
certificates and small authority metadata. There is no account identifier,
plaintext balance, amount or plaintext content hash in the chunk database.
Canonical financial roots, signatures and monetary policy are unchanged.

Legacy full encrypted snapshots and legacy witness rows are readable and migrate
on the next successful mutation; old binaries fail closed on the new format.
The existing durability hooks, poison-on-uncertain-write behavior and all sign-once,
rollback, crash, replay and recipient-bound recovery tests pass. Keep the new
`.chunks.sqlite` executor sidecar with standalone backups. Witnessed caches can
be rebuilt from their authenticated authority checkpoint.

The four-executor/four-witness process test transferred funds with 10,009 accounts,
restarted every process and recovered the lagging executor. Measured checkpoint
and inserted-ciphertext byte counts are in
[incremental-persistence-validation.json](incremental-persistence-validation.json).
The full all-feature release experiment suite and strict all-target/all-feature
Clippy passed. Eleven Intel/QVL-only tests remain explicitly ignored.

Only unreachable encrypted state chunks are reclaimed. Certified history, nonce
reservations and sign-once records are never pruned. Each wire checkpoint remains
bounded at 4 MiB; executor chunk databases have a 12 MiB page budget and authority
databases a 1 GiB page budget, with bounded WAL checkpoint intervals. Exhaustion
fails closed. These are software limits, not production capacity authorization.
Full snapshot serialization/root hashing and witness RPC checkpoint transfer are
still linear in state size; incremental network transfer and scalable authenticated
financial-history pruning remain unfinished. Incremental storage exposes encrypted
chunk sizes/reuse and I/O patterns; it is not ORAM or hardware confidentiality.

Permissionless software P2P implementation followed in the next component below.


## Permissionless software P2P — implemented 2026-11-10

Transport-proof binding commit: `b9f679e`; integrated discovery/propagation,
operator controls and final evidence are in the commit containing this entry.

The native live-PoS software profile now discovers signed endpoints, authenticates
candidate peers, synchronizes new nodes and propagates canonical encrypted payment
certificates through peers. Enable `p2p_discovery` in the devnet configuration;
old fixed and isolated modes retain their gates. A gateway uses one successful
proposal ingress rather than posting to every proposer. A joining node supplies
only its own identity, independently pinned public policy, listening endpoint and
up to eight seeds; no manually configured validator/RPC slot is required.

Two key-possession signatures bind every advertisement to its network scope,
endpoint and bounded lifetime. Finalized transport bindings always prevail.
Unknown peers may synchronize but have zero voting authority until the existing
registration/stake/committee protocol selects them. Endpoint hints never publish
account owners, balances, amounts, stake weights or wallet secrets. Hint memory,
peer dials, receive/signature work, packets, connections and per-IP admission are
bounded. Uncertain SDK additions remain tracked until reconciliation; a delayed
acknowledgment cannot escape the 16-entry dial cap. A full hint cache can admit
new nonvoting candidates while protecting
current finalized identities. The existing isolated private-address gate is
unchanged; public development fixture keys remain loopback-only.

The final five-process demonstration used an extra endpoint absent from the
configured inventory and only a public policy/own key/one seed. It synchronized
before registration, registered/staked, signed native finality at height 7,
survived a restart/catch-up and an invalid discovery burst, and finalized through
height 11. Each node discovered the other four. Evidence:
[permissionless-p2p-validation.json](permissionless-p2p-validation.json).
Operator controls and exact bounds: [PoS networking](pos.md).

Validation: 51 core consensus tests passed (one manual scale campaign remains
ignored); focused discovery, cache capacity, transport-binding and original
isolated regressions passed. Strict Clippy passed with the development feature
both enabled and disabled. All 65,536 production proof versions still reject;
native wire gates passed with the feature enabled and disabled. The 23 existing
wallet/RPC/supervision Python checks passed. Priority 1 additionally passed the
complete 55-test all-feature release experiment suite and 10,009-account recovery
measurement. No expensive hardware/scale tests were silently substituted.

Reproduce only the relevant checks for a follow-up change:

```sh
cargo test --release --locked -p yami-chain-consensus --features insecure-devnet-input
cargo clippy --locked -p yami-chain-consensus -p yami-validator --all-targets --features insecure-devnet-input -- -D warnings
cargo clippy --locked -p yami-chain-consensus -p yami-validator --all-targets -- -D warnings
cargo test --locked -p yami-chain-privacy --test non_test_boundary
cargo test --locked -p yami-chain --test native_devnet
cargo test --locked -p yami-chain --features insecure-devnet-input --test native_devnet
python3 tools/yamichain-p2p-smoke.py --out work/next-p2p-check --port-offset 6000
cargo test --release --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml --test witness_e2e large_financial_state -- --nocapture
```

Use the Rust environment setup earlier in this ledger and a fresh output path/free
ports. This session's process demonstrations were stopped cleanly, retaining
`work/permissionless-p2p`, `work/permissionless-p2p-final` and
`work/permissionless-p2p-ack` for local diagnostics;
private identities/configuration in those directories must never be committed.
Durable evidence is the linked JSON, not ephemeral work logs. Continue with the
TDX custody/replacement, wallet recovery and security-validation work above;
do not restart the implemented persistence or peer discovery components.
`main`, M16 activation and production financial verification remain unchanged.

## Executor lifecycle implementation — in progress

The executor, wallet and native-validator development paths now share one
certificate/witness verifier (`yamichain-devnet-cert`), eliminating divergent
copies before adding generation and replacement authorization. The complete
55-test release experiment suite and strict all-target/all-feature Clippy passed.
This verifier consolidation does not itself resolve custody or activate recovery.
