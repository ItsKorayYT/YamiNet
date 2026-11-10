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

Still requiring software implementation/validation (not claimed complete):

1. Public permissionless peer discovery and admission integrated with the live
   PoS identity directory; transaction propagation and independent multi-machine
   fault/partition campaigns. Current demos use configured private bootstrap peers.
2. Ordinary financial-state incremental persistence, authenticated large-history checkpoint
   bootstrap/pruning and bounded long-term disk growth. M16 activation gates stay
   closed; the current native financial profile retains full certified history.
3. Attestation-bound executor provisioning, confidential transfer, revocation,
   replacement, measured-guest encrypted RPC packaging and witness custody
   changes. In particular, recovery-custodian
   impersonation is not solved by TLS or process supervision.
4. Wallet transaction-history/status UX, automatic secure network discovery and
   further delegation/reward UX beyond the authenticated portfolio details.
5. Production-quality selection randomness, justified delegator reward policy,
   upgrade negotiation and economic policy versioning. The current disposable
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
