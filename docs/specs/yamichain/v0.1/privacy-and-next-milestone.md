# ADR YC-PRIV-001: YPP policy and next milestone proposal

Status: policy enforced; cryptographic construction DEFERRED. Based on the
v0.5 package supplied on 2026-11-01. The only authorized implementation here is
a non-financial foundation. No privacy proof dependency, circuit, spend tag,
note tree, curve, STARK trace, verifying key, wallet format or trusted ceremony
has been selected. No Zcash-specific dependencies or formats are included.

## Current boundary and threat model

`ConfidentialTransfer` and `ConfidentialProtocolAction` describe intent only.
Public fields are network/protocol/intent/anchor/expiry/fee binding context,
opaque backend public input descriptors, encrypted output boundaries, proof
version/bytes and an explicitly public protocol effect. They have no consensus
wire encoding and disclose no plaintext private sender, recipient, amount, memo,
spending secret or viewing secret. They are not a commitment to a future proof
format. Public fees and protocol effects would remain visible under a future
reviewed construction. Proof byte lengths and network timing also leak metadata.

`PrivacyVerifier` is a concrete non-accepting boundary. All 65,536 version values
are refused, including empty or zero-value fixtures. `VerifiedEffects` cannot be
constructed outside the privacy crate. No accepting injectable trait, runtime
flag, test feature or production dummy implementation exists. The executor has
no financial action discriminant, no mint action, no fee debit and no wallet.
Transparent value-conservation and uniqueness models are compiled only as tests;
they are not shielded proof verification or a network acceptance path.

Assume corrupt senders, malformed peer frames, replayed signatures, an equivocating
validator below one third, hostile history providers, malicious registry operators,
and storage interruptions. The prototype checks public non-financial integrity;
it cannot supply private-value soundness/confidentiality, endpoint security,
network anonymity, public-registry correctness or permissionless Sybil resistance.
No external cryptography or consensus audit has been performed.

Future proof acceptance must satisfy the package's complete obligations:

1. **P1 ownership/membership:** authorized, eligible inputs at accepted finalized
   anchors; explicit rules for chaining and historic membership.
2. **P2 unique consumption:** one immutable spent tag per private object, distinct
   multi-input witnesses and exact retained spent history even after pruning.
3. **P3 conservation:** bounded integer values, no counterfeit assets, exact fees,
   burns and all confidential/public liabilities in a single sound relation.
4. **P4 binding:** chain, protocol, full intent, fee epoch, outputs, beneficiaries,
   action type, anchors, expiry and verification-key schedule bind together.
5. **P5 usable recipients:** ciphertext authentication, commitment binding,
   independent spend/view authority, secure entropy, backup and complete rescanning.
6. **P6 public actions:** stake/share backing, auctions/escrow/refunds, governance
   eligibility and treasury effects cannot mint, duplicate or redirect private value.
7. **P7 independent trust:** IP privacy and registry membership proofs cannot
   substitute for cryptographic soundness, finalized headers or owner authorization.

## Unresolved release gates

| Gate | Open decision |
| --- | --- |
| G-01 | Complete proof backend, public parameters, exact statement, keys, version schedule, audit and license |
| G-02 | Membership state, immutable spend tags, historic anchors, witnesses, accumulation and pruning |
| G-03 | Exact confidential/public accounting for fees, burns, stake, shares, escrow and treasury |
| G-04 | Validator cap, qualifying stake, selection, replacement ordering and voting power |
| G-05 | Slashing/evidence policy and security of the preferred seven-day unbonding horizon |
| G-06 | Proposed billion-YAMI public genesis and adaptive 0.5–2% issuance, initially 1.5%; exact equations/rounding |
| G-07 | Dynamic resource fees, variable burning and beneficiary split |
| G-08 | Treasury genesis allocation, vesting, authority and voting concentration |
| G-09 | Auctions, commit/reveal, renewals, settlement and challenge rules |
| G-10 | Registry transition validity, owner anti-hijack protection and timed recovery |
| G-11 | Governance snapshot/eligibility/quorum/privacy and upgrade authorization |
| G-12 | Audits, dependency maturity, independent operators, adversarial soak and measured resources |

## Separate proposal: YC-R1 comparison, not approved implementation

Evaluate current **FCMP++/CARROT**, **corrected Spark** and **transparent STARKs**
with identical semantic workloads. Do not select or pin a proof library merely
because one candidate appears first. Record exact source commits, licenses,
audits/advisories, Rust/MSRV/native ARM64 constraints and setup assumptions.
The preparation package's maturity/advisory claims are research leads to recheck,
not completed due diligence for those projects.

Compare complete 1-input/2-output and multi-input transfers; duplicate witnesses,
alternate tags, historic anchors, fee/burn binding, malformed ciphertext/proofs,
private-to-public stake and auction escrow/refund transitions, wallet recovery,
sync/prune/restart, and 4/25/100-validator workloads. Mark unsupported operations
UNSUPPORTED; primitive timings cannot stand in for complete payment verification.
Reproduce Spark's disclosed multi-input forgery failure as a negative semantic
case when the precise construction and attack are independently reviewed.

Measure full verifier CPU/peak RSS, proof bytes, state/witness growth and wallet
scan/proving costs on actual 2-core/2-GB x86_64 and ARM64 hardware. No estimated
numbers or borrowed upstream benchmarks. FCMP++ membership is not a complete
YamiChain payment protocol, a deployed Spark variant is not an audit of custom
public conversions, and a transparent STARK is not inherently a private coin.
Archived FCMP++ AGPL licensing, Spark's C++ boundary and research STARK audit
status require explicit investigation before integration.

The output is a reviewable cryptography ADR defining equations, assumptions,
public disclosure, rejected alternatives, source/license evidence, parameters,
valid anchors, version/migration rules, measured resources and independent review.
Only after that ADR and **separate explicit authorization**, propose an isolated
real-proof experiment with an independent audit plan. This PR grants no permission
to add a spend-accepting verifier, including on a public testnet. Mainnet additionally
requires independent audits, adversarial network/economic testing, authentic genesis,
wallet recovery and all associated gates above.
