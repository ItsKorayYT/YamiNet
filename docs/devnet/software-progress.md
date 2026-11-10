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

Still requiring software implementation/validation (not claimed complete):

1. Public permissionless peer discovery and admission integrated with the live
   PoS identity directory; transaction propagation and independent multi-machine
   fault/partition campaigns. Current demos use configured private bootstrap peers.
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

This session did **not** finish all remaining software. In particular, ordinary
financial-state incremental persistence and permissionless P2P integration are
still code work, not completed external validation. Do not reinterpret the
external blockers below as completion of those implementations.

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

Concrete next source work:

1. Implement ordinary private financial incremental persistence, starting with
   `Replica::{prepare_with_hook,commit_with_hook,checkpoint,restore_cache}` in
   `experiments/yamichain-tee-m20/src/quorum.rs` and their witnessed callers in
   `src/witness.rs`. Today they still rewrite full encrypted snapshots. Preserve
   exact snapshot authentication, durable witness acknowledgment, poison-on-
   uncertain-write behavior and all crash/rollback tests. Do not substitute a
   final certificate for complete-prefix verification or activate M16 pruning.
2. Integrate bounded signed endpoint discovery and transaction propagation with
   live identities in `crates/yami-chain-consensus/src/{network,node,signing}.rs`.
   Current public-address refusal remains a security gate. Endpoint hints must
   never grant voting authority, overwrite finalized identities or expose private
   owner/stake information. Configured private seeds are the only live P2P profile.
3. Complete confidential executor provisioning/replacement/revocation and witness
   custody changes in `src/{tdx_guest,attestation,witness}.rs` and `deployment/tdx`.
   The recovery-custodian impersonation issue remains unresolved. Software TLS,
   process isolation, copied measurements or ignored QVL tests are not substitutes.
4. Implement incoming/restored-wallet history through an authenticated confidential
   retention/query protocol. Existing public blinded transaction commitments do
   not disclose historical transfers; the new local journal cannot reconstruct
   incoming payments or submissions recorded on another wallet installation.
5. Implement reviewed beacon/randomness and upgrade negotiation. Existing economic
   policy versioning deliberately accepts only version 1; changing a JSON version
   cannot activate new delegator rewards or monetary rules. Do not silently change
   the current deterministic selection seed, fee shares or canonical state roots.

External resources still absent in this environment: `/dev/tdx_guest`,
`/dev/tdx-guest` and `/dev/kvm`; configured outbound identities/secret bindings;
and independently operated validator/executor/witness machines. Hardware/QVL
trials, independent partition campaigns and production authorization therefore
remain blocked. Resource references were requested without asking for credentials
in chat. No such resources are inferred from the local TLS software demonstration.
Independent custody/checkpoint/randomness/economic review is still required before
opening any corresponding production or public-network gate.


## Incremental financial persistence — implemented 2026-11-10

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

Priority 2 P2P implementation continues next; this checkpoint is not a completion
claim for the networking requirement.
