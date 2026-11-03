# Relation and privacy boundary

## Records, authentication and conservation

Use Jubjub's prime-order subgroup with standard generator G. A wallet generates
independent nonzero canonical scalar authorities s (spend) and v (view); its full
address is (S=sG,V=vG). A viewing capability contains S and v, not s. Spending
requires both secrets. A record is (kind,S,V,b,R), with random nonidentity subgroup
point R and u16 balance b. Vault kind is 0, credit kind 1. Its commitment is
SHA256("YPP2-record-v1" || kind_u8 || x(S) || x(V) || LE16(b) || x(R)).
Coordinates have canonical 32-byte little-endian field encoding. The upstream
TECompressor injective map is used only on prime-order points; full points are
also checked and encrypted. Entropy in R blinds the commitment.

An append-only depth-five tree authenticates full 32-byte commitments. Internal
nodes use upstream PedersenCRHCompressor with TECompressor: hash
0x09 || left_32 || right_32, zero-padded by the library to five 128-bit windows.
Bases come from upstream OS-random setup; their discrete-log relationships must
remain unknown. This parameter trust is additional to Groth16 setup trust. Leaves
start empty (32 zero bytes); accepted outputs append, never overwrite old leaves.
There are at most 32 leaves. The prototype does not prove a large anonymity set.

A transition privately opens one vault and an optional credit, proving paths to
the same public historical root. Input owner/view keys must equal (sG,vG). If no
credit is consumed, a fresh dummy credit opening has amount zero and does not need
membership; its marker still has the same fixed public format. The live/dummy bit
is private. Prove b_old + b_credit = b_new + a, with every amount constrained to
16 bits, and keep (S,V) unchanged in the new vault. Field arithmetic cannot wrap
these sums. Output 0 is the next vault version; output 1 is a temporary credit to
the privately witnessed recipient address. Zero-value payments are allowed to
avoid publicly identifying absorption-only operations.

Each input marker is RFC 2104 HMAC-SHA256 keyed by canonical LE32(v), over
"YPP2-nullifier-v1" || commitment. The circuit implements that standard HMAC and
matches native library results. A critical bound proves 0 < v < subgroup_order:
otherwise v and v+order yield the same EC public key but different HMAC markers.
Tests recompute the alternate markers and show that the canonicality constraint
rejects this double-spend route. Spending scalar canonicality is checked too.
Viewing authority can track spends; knowing only v cannot satisfy sG=S.

## Verifiable offline delivery

For **each** output, independently encrypt bG, R and S with standard ElGamal:
(U_i,W_i)=(r_i G, M_i+r_i V). The proof computes the upstream encryption gadget
and equates all public ciphertext coordinates to its result, using exactly the
V and plaintext points committed in the output. It enforces nonzero, pairwise
distinct ephemeral keys across all six ciphertexts. Reusing a mask for bG and R
would expose their difference and allow small-amount guessing against the record
hash; the adversarial test demonstrates that this witness is rejected.

An offline recipient with v decrypts R and S directly, and decodes bG over the
complete u16 domain using standard baby-step/giant-step. V, kind and the recovered
fields reconstruct the exact commitment. Thus an accepting proof cannot bind an
arbitrary undecryptable payload in place of the committed opening, as Milestone 1
could. The full spending authority is delivered, avoiding hidden unrecoverable
metadata. Keys/addresses must still be authenticated out of band: a malicious
payer can choose a different composite (S,V) address. If a decryptable opening has
an unexpected S, the scanner records and surfaces it as misaddressed rather than
silently crediting it. The chain does not prove a human's intended payee or that
a receiver retained its secrets. Raw ElGamal is CPA encryption, not an AEAD API;
only proof-verified envelopes are scanned. This composition is not independently
reviewed or a complete chosen-ciphertext security proof.

## Public transcript and validation

The exact 740-byte packet is: "YPP2", one field root, two 32-byte nullifiers,
two outputs (32-byte commitment plus three pairs of compressed Jubjub points),
and a 192-byte Groth16 proof. The circuit has 153 public field inputs: root,
individual bytes of both nullifiers, then each output commitment's bytes and its
ciphertext x/y coordinates. Native input ordering matches circuit allocation.
Checked decode rejects wrong length/tag, noncanonical fields, malformed/off-curve/
non-subgroup points and invalid proof points. Verifier-key vector lengths are
checked before allocation. Verification is deterministic and uses no random RNG.

The ledger accepts only its four most recent roots, rejects every used nullifier
and every repeated output commitment, verifies the proof, then atomically appends
two outputs, records both nullifiers and updates its root/history head. Acceptance
of historical membership is safe because spentness is checked globally, not at
the old root. Independent same-anchor proofs remain usable until expiry. Competing
updates of one vault share a nullifier and only one applies. Root expiry still
requires reproving; this is bounded concurrency, not unlimited witness lifetime.

Remaining observable identifiers and patterns are:

- Shared historical roots and their age, which restrict possible input history
  and can fingerprint witness refresh policy; roots identify a ledger prefix,
  not a vault. Output positions/order and their vault/credit roles remain public.
  Nullifier 0 is the vault marker; nullifier 1 is credit-or-dummy, with the live
  flag hidden. The version tag and verifying parameters are common to the ledger.
- Two public new commitments, their insertion positions, six ephemeral EC keys,
  both nullifiers, proof/packet size, transaction ordering and timing. Honest
  successful repeated spends reveal no equality mapping from these outputs to a
  later input. Conflicts, rebroadcasts and reproving the same input repeat markers
  and are linkable.
- Known peer identities, address exchange, payer/payee knowledge, active probing,
  small candidate sets, self-selected malicious randomness, compromised viewing
  keys and external timing/transport data can still reveal associations. There
  is no network anonymity, batching, mixing, key-privacy composition proof or
  guaranteed sender/recipient anonymity against those observations.

Tests compare the observable packets, not just private amounts. The benchmark
records the full packet hex and equality analysis. Absence of an equality match
is a regression property, not a formal anonymity proof.

## Recovery and the architectural obstacle

Trusted synthetic genesis pins parameters, commitments and encrypted openings.
A record head hashes a domain, prior head, complete packet and resulting root.
Recovery replays every proof and state check from pinned genesis, authenticates
root/head, tries local viewing-key decryption and removes openings whose derived
nullifiers appear. A record failure rolls back the whole in-memory scanner step.
A pinned final head detects incomplete/modified history. Backup/restart can replay
from genesis; no durable incremental cursor, remote history transport, reorg or
data-availability system exists. Spent leaves remain members of later roots, so expiring roots is not permission
to discard spent markers. Records and spent markers cannot safely be pruned
under these guarantees. Scanner history cloning has quadratic cumulative copying.

The executable in-place counterexample starts with root R, computes independent
updates R_A and R_B, and shows that replacing R_A with stale R_B loses A's update.
Hidden membership alone cannot safely compose hidden in-place writes: a stable
public locator leaks the vault, a current-root-only replacement invalidates
unrelated proofs, and blind acceptance of stale replacement roots rolls back
state. This is a counterexample to that approach, **not an impossibility theorem**
for private accounts. Reviewed oblivious update/aggregation or a different state
commitment architecture would be needed to solve it here.

This implementation instead appends new states and uses private consumption
markers. That is structurally a shielded-record design with a constrained
one-vault lineage and temporary delivery claims. It demonstrates a working way
to remove the trivial public graph, not an original private-vault construction.
Offline direct mutation of Bob's hidden existing balance remains unimplemented;
Bob must later consume a temporary credit. No additional financial module exists.

Relevant prior art: [Zether](https://eprint.iacr.org/2019/191) for encrypted accounts
and concurrency, [Zexe](https://eprint.iacr.org/2018/962) for private records and
state transitions, and Bellare et al.'s
[Key-Privacy in Public-Key Encryption](https://www.di.ens.fr/users/pointche/Documents/Papers/2001_asiacryptC.pdf)
for the ElGamal/DDH key-privacy model. Those primitive-level results do not prove
this complete construction secure. The actual relation, wallet adapter, setup,
parameter provenance, cross-use of viewing authority, side channels and dependency
advisories require independent review. No production activation is authorized.
