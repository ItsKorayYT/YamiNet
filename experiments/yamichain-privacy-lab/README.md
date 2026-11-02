# Offline RingCT experiment — Milestone A

Real cryptographic transfers over disposable synthetic outputs. **No YAMI, real
funds, RPC, broadcast, issuance, financial module, or consensus acceptance.** This
separate Cargo workspace is excluded from the chain workspace; no validator or
YamiNet application depends on it. The root lockfile and sealed verifier are
unchanged. The only dependency on chain code is a lab integration-test dependency.

## Experimental selection and evidence

Use the complete Monero CLSAG + Bulletproofs+ RingCT wallet path in
[`monero-wallet` 0.2.0 / `monero-oxide` 0.1.0](https://github.com/monero-oxide/monero-oxide/tree/a6b46701fb9b331006d7f2f2278bf51f123b1447),
pinned to **a6b46701fb9b331006d7f2f2278bf51f123b1447**. Pure Rust, upstream
MSRV at most 1.89 for the selected wallet, MIT top-level and component licenses.
Our adapter inherits GPL-3.0-only, Rust 1.90 and forbids unsafe code. Full resolved
versions/checksums are in `Cargo.lock`; `dependency-licenses.tsv` inventories
declared licenses for all 168 resolved packages, including dev/target packages.
Metadata is not a complete legal review or distribution notice bundle.

[Cypher Stack's report](https://github.com/monero-oxide/monero-oxide/blob/a6b46701fb9b331006d7f2f2278bf51f123b1447/audits/Cypher%20Stack%20May%202025/Audit.pdf)
reviews Serai `networks/monero` at **48db06f**, excluding `verify-chain` (PDF dated
2025-08-14, provenance release `cypherstack/monero_wallet_audit` 1.1). It covers
the predecessor wallet/RingCT implementation, documents transcript responsibilities,
and recommends bounds and adversarial testing. **It does not audit this pinned
revision, adapter, wallet derivation/backup integration, or YamiChain.** No claim
of complete remediation/diff review or audit coverage for the experiment follows.
`crypto-bigint` 0.5.5 remains in the closure; the repository's existing
RUSTSEC-2026-0387 timing notice also requires review here. The inspected Monero
hash-to-point call processes public output keys; this is not a general timing
clearance. A targeted RustSec source check at
`263a4d951a0867d4bddc29dddd6d59427c78f9a7` also found unmaintained build dependencies
`basic-toml` (RUSTSEC-2026-0374) and `proc-macro-error2` (RUSTSEC-2026-0173).
The age plugin execution notice RUSTSEC-2024-0433 is fixed in the selected 0.11.1;
plugin support is disabled here. A complete automated advisory audit was not run.

This is an established complete confidential-transfer baseline, rather than
FCMP++ membership alone. FCMP++/CARROT's complete wallet composition and review
applicability remain unresolved; corrected Spark adds an unreviewed native/Rust
boundary and correction-specific review work. The earlier comparison is in
`docs/specs/yamichain/v0.2/privacy-review.md`. Zcash/Orchard/Halo 2 are excluded.
RingCT is selected **only for this experiment**, not as the final chain backend.
Rings of 16 do not provide full-set membership or an anonymity guarantee.

## Implemented formats and checks

- Wallet entropy comes from `OsRng`. RFC 5869 HKDF-SHA256 under an offline-only
  salt separates spend and outgoing authority. The incoming view key uses the
  upstream Monero spend-to-view derivation. Public addresses use upstream staging
  network encoding. No secret Debug/CLI export, production key reuse, or saved keys.
- Upstream stealth output keys, Pedersen commitments, compact encrypted amounts,
  CLSAG key images/signatures and Bulletproofs+ formats are used unchanged. The
  upstream builder performs encryption, range proving and spend authorization;
  our verifier consumes public commitments, never private amounts/openings.
- A fixed offline payment/version/chain domain, intent, fixture anchor, expiry,
  fee epoch and actual public fee have fixed-width canonical encoding. Its
  length-delimited SHA256 digest is placed in upstream signed transaction extra.
  Upstream `signature_hash` binds the complete prefix (including output keys and
  extra), encrypted amounts, fee, commitments and range proof. No arbitrary
  protocol actions or unsigned financial effects exist.
- At most 2 inputs, 16 outputs and 16 members per ring; packets at most 16 KiB.
  Decode rejects noncanonical/trailing bytes and bounds the outer input first.
  Ring members come from the local trusted public set, not caller-provided rings.
  Verification checks range proofs, every CLSAG, prime-order keys/commitments,
  exact commitment conservation including fee, expiry, binding, and unique images.
  Only a fully verified packet changes the in-memory set and spent history.
- Wallet discovery uses the upstream scanner and checks decrypted openings against
  commitments. Monero compact amounts are **not an AEAD note format**: transaction
  authorization binds ciphertext, and scanning validates the commitment opening.
  No custom encryption/proof/signature scheme or arbitrary encrypted memo is added.
- Backups encrypt an offline version tag, birthday and entropy with **age 0.11.1
  X25519**, using its standard authenticated format. Restore is bounded to 4 KiB
  encrypted input and the exact expected plaintext length. Keep the independent
  age identity separately; losing it loses recovery. Tests rescan and spend with
  the recovered wallet. Secret buffers use zeroization; this is not protection
  against compromised endpoints, memory copies, swap or filesystem rollback.
- `funded_fixture` creates trusted random initial ownership/commitment openings,
  which are serialized through the pinned upstream wallet storage format. Those
  openings are not packet fields and are not used by verification. Initial supply
  is a synthetic test assumption, not a mint/proof/issuance implementation.

## Run

Local results and hardware limits are recorded in [validation.md](validation.md).

```sh
manifest=experiments/yamichain-privacy-lab/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo clippy --manifest-path "$manifest" --all-targets --locked -- -D warnings
cargo test --manifest-path "$manifest" --locked
cargo test --manifest-path "$manifest" --release --locked
cargo build --manifest-path "$manifest" --release --locked --example offline
cargo run --manifest-path "$manifest" --release --locked --example offline -- 2
python experiments/yamichain-privacy-lab/measure.py
```

Tests cover 1/2-input transfers, viewing/change, a recipient's onward spend,
encrypted recovery, wrong backup identity, corruption/truncation, wrong spending
key, forged CLSAG/range proof, inflation/overspending, altered outputs/fees/context,
malformed ciphertext, duplicate inputs/images, alternate spends and invalid rings.
The separate chain test first verifies a real packet offline, then confirms every
one of 65,536 proof versions fails the actual chain library boundary in both
debug/release and neither raw RingCT nor its envelope decodes as a chain action.

## Remaining gates / next coding milestone

Milestone A supplies a complete offline transaction demonstration and recoverable
wallet foundation. No production wallet format, network anonymity, complete
shielded state, synchronization, financial integration or consensus backend has
been established. Ring indices, key images, fees, input/output counts, packet sizes
and timings are public. Recipients learn their received amounts; a view key exposes
incoming notes but does not reliably identify outgoing spends. A small trusted
fixture has no meaningful anonymity set.

**Milestone B next:** reviewed authenticated commitment state, historical anchors,
retained spent history across restart/pruning, authenticated wallet synchronization
with duplicate-output detection, and bounded deterministic verification. The lab
currently trusts one in-memory membership set, accepts only its current anchor,
and uses upstream randomized range verification (`OsRng`); do not replace this
with a fixed verification RNG and claim consensus soundness. Scanner inputs are
locally verified fixtures, not authenticated remote history. Birthday is recovery
metadata, not an implemented network sync cursor. No durable wallet balance/spent
database exists. Multi-module liabilities and all staking/issuance/treasury/auction/
governance proof statements remain inactive, unimplemented Milestone C work.

Independent review of the exact upstream revision and complete adapter, dependency
advisories/licenses, native ARM64 testing and validator-level resource measurements
remain open. Any accepting consensus backend requires separate explicit approval
and a reviewed upgrade; this experiment does not grant it.
