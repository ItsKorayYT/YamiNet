# Milestone 20: isolated TEE payment execution prototype

M22 adds [real Intel QVL verification and the attestation-gated wallet](M22.md).
Hardware/production activation remains closed; the M20/M21 reports are historical.

M21 extends this working crate with a four-process 3-of-4 quorum. See
[M21 implementation and security verdict](M21.md). The original M20 report below
and its measurements remain the historical baseline.

**SOFTWARE ONLY. NO-GO for production, real funds, zero-trust custody or financial
safety.** Base `22a57cf`, branch `development/yamichain`. No production module,
consensus rule, Milestone 16 implementation or rejected proof version is changed.
Intel TDX is the intended hardware target. This machine exposes no TDX guest device.
There is no enclave image, genuine quote acquisition or DCAP verifier in this work.
`verify_tdx_quote` always rejects. Do not interpret the software policy tests as
hardware attestation verification.

## Working path

The standalone Rust crate has its own workspace and lockfile. Ed25519 wallet
signatures bind sender, recipient, unsigned 64-bit amount, nonce and chain ID with
a protocol domain. Wallet private keys stay in wallet/test code; Engine receives
only signed transfers. X25519 + HKDF-SHA256 + ChaCha20-Poly1305 encrypt payloads;
noncontributory public keys are rejected. Signing, encryption and storage keys
are separate. Each envelope uses a fresh ephemeral key and random 96-bit nonce.
Engine verifies signatures and uses checked arithmetic, with exact sender nonce,
chain separation, positive amounts and distinct sender/recipient. An unknown
recipient is created on credit without needing its wallet online.

A process lock and exclusive mutable Engine access serialize account updates.
The encrypted database is a complete ordered account snapshot, authenticated with
a storage-specific domain. Its SHA-256 commitment includes nonce and ciphertext.
A signed receipt binds previous head, next head, chain, epoch and encrypted-request
hash. A validator harness checks a pinned receipt public key and exact previous
head. This is an authenticated execution assertion, **not a financial proof**.
Production consensus is deliberately not connected to it.

Persistence writes a pending snapshot, fsyncs it, renames over the database and
fsyncs the directory before returning the receipt. Debit and credit share one
snapshot. Failed persistence poisons the instance until reload. Tests inject
interruptions at synced-pending, renamed and synced-directory boundaries and
check reload behavior; these are deterministic error injections, not physical
power-loss or kill-process experiments. Torn pending files are ignored.

Restart requires caller-supplied storage, receipt and transport secrets plus a
trusted external head. Keys are not embedded or stored in the database. Key
provisioning, TDX sealing, key rotation and secure backup are unimplemented.
Benchmark fixed storage keys are disposable fixtures only. An exact trusted head
rejects old snapshots, wrong keys and tampering. Rewinding both database and trusted
head defeats detection, demonstrated in tests. A crash after rename but before
external finality can leave an unanchored successor: loading against the old head
fails closed. Distributed reconciliation and availability recovery for that window
are not implemented. Never adopt the head merely because it appears in a file.

Recipients sign a query binding their account, reply encryption key, trusted head
and challenge. Responses encrypt account balance/nonce and include an execution
signature bound to the entire query. The client must verify that signature with
the pinned receipt key, use a fresh challenge, and authenticate its current head.
A host can replay queries or deny service; substituted reply keys, unauthorized
queries and modified/replayed responses fail authentication. Query routing metadata
is not hidden. No network service is launched by this crate.

## Security dependencies and gates

1. **Genuine TDX attestation:** integrate Intel DCAP quote verification with trusted
   root/collateral validation, revocation and collateral expiry checks, acceptable
   TCB/QE status and rejection of debug TDs. Allowlist MRTD and relevant RTMRs,
   guest kernel/application/configuration identity and security version. Bind both
   transport and receipt public keys, protocol/chain and wallet challenge into
   REPORTDATA via a domain-separated digest. Verify challenge freshness using a
   wallet-held challenge and trusted time; prevent challenge reuse. Wallet must
   encrypt only after this succeeds. `SoftwareEvidence` is deliberately unsigned,
   caller-supplied policy data, checks only equality/time windows, and has no trust
   value against a host. `encrypt` is a raw primitive, not an attestation-gated wallet
   API. The end-to-end verified-attestation architecture is therefore incomplete.
2. **Independent financial validity:** demonstrate a public, private authorization
   and conservation verifier before claiming safety against compromised execution.
   The present receipt can authenticate arbitrarily fabricated state (tested).
3. **Rollback/finality:** establish durable independent finalized checkpoints,
   anti-equivocation, fork selection, snapshot availability and recovery protocol.
   A malicious validator cannot forge honest signatures but can withhold or present
   stale heads to a client that lacks an authenticated current head. No validator
   quorum or Byzantine consensus test is claimed here.
4. **Operational security:** hardware execution, side-channel analysis, secure
   entropy/time/key lifecycle, guest isolation/hardening, dependency audit, bounded
   network framing/rate limits and malicious-host resource exhaustion testing.
   File reads and JSON parsing are not a hardened untrusted network interface.
   Ordinary fs2 locks assume cooperative local writers; a hostile host can ignore
   them. Cryptographic integrity does not give availability or stop host forks.
5. **Scale/durability:** O(accounts) snapshot encryption/write and state cloning per
   transfer is a functional baseline only. No incremental encrypted database,
   physical power-cut testing, multi-machine failover or capacity guarantee.

## Independent authorization/conservation investigation

The attack test has an enclave signing key authenticate a false commitment; receipt
verification accepts it. Wallet signatures prove only that some transfer was
requested. They do not prove that the encrypted successor contains that transfer,
that other accounts were unchanged, or that total supply was conserved. Encryption
and TDX identity do not change this conclusion after enclave compromise.

`audit_disclosed` independently computes the exact complete authorized transition
from the old ledger and wallet signature. Tests accept the valid successor, reject
inflation, and reject a balanced theft which conserves total supply. This demonstrates
the required relation in a disclosure-only oracle, without trusting the enclave's
receipt. It does **not** demonstrate independently verifiable *private* payments.

A candidate next step is a zero-knowledge transition proof bound to finalized old
and new account commitments AND to the actual encrypted-state commitment. It must
prove wallet signature verification, chain/nonce uniqueness, canonical u64 ranges,
checked debit/credit, unchanged other accounts, conservation, and ciphertext/plaintext
consistency and recoverability. Validators must independently verify this proof and
serialize old roots. A proof of aggregate conservation alone permits balanced theft;
a signature proof alone permits unrelated state corruption. The present byte hash
of randomized whole-state ciphertext is not a suitable efficient algebraic account
commitment by itself. Designing a private account commitment, proving encrypted
state consistency and delivering durable offline query access remain open. Existing
M17/M18/M19 experiments do not discharge these requirements and are not reactivated.
A second independent executor/quorum would add trust assumptions, not eliminate
compromised-executor custody risk. No complete private mechanism is claimed.

## Reproduce

Use Rust 1.90.0 (repository toolchain), from repository root:

```sh
cargo test --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml
cargo clippy --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml --all-targets -- -D warnings
cargo build --release --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml
python3 experiments/yamichain-tee-m20/benchmark.py
cargo test --release --locked --manifest-path experiments/yamichain-ypp-m18/Cargo.toml --test production_boundary
git diff --check
```

`measurements.json` records one run per size, with 1,000 sequential payments,
wallet signing/encryption, durable execution and receipt verification together.
Recovery loads/decrypts one snapshot with externally retained secrets/head. Linux
wait4 peak RSS includes the whole benchmark process; storage is final snapshot
bytes, not cumulative writes. Timings depend on this container/filesystem and are
not isolated enclave execution costs. No TDX timing is available.
