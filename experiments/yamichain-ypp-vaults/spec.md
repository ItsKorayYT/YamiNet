# State-transition specification (experimental v1)

## Primitives, domains and representation

`H` is SHA-256; `||` is fixed-width concatenation unless a length is stated.
Digest fields are 32 bytes. Integers are unsigned little-endian u64. Spending
secret `s` and each state blinding `r` / credit salt `t` are independent OS-random
32-byte values. The viewing authority is a separately generated age X25519
identity. Address `(P,V)` contains `P=H(0x00||s)` and the age public recipient `V`;
it is exchanged out of band. `Q=H(0x03||ASCII(canonical_age_recipient(V)))`.

Vault state `S=(P,Q,b,v,r)` is 112 bytes. Its commitment is
`C=H(0x01||P||Q||LE64(b)||LE64(v)||r)`. Credit `I=(P_r,Q_r,a,t)` is 104 bytes.
Encrypt its encoding to `V_r` using the established age v1 X25519 format, getting
ciphertext `E`. Credit commitment is
`N=H(0x02||P_r||Q_r||LE64(a)||t||H(E))`.
The authenticated current root is `R=H(0x04||C_0||C_1)`; these two public leaves
are kept in the local ledger. The setup is scoped to this single relation.
All hash algorithms and age encryption are unchanged established primitives;
new domain-separated encodings and the proof statement are experimental.

The complete public packet consists of one-byte mode `d` (1 debit, 0 absorb),
one-byte slot `j` (0 or 1), `R`, `C_old`, `C_new`, `N`, a 32-byte proof-context
binding `B`, credit ciphertext `E`, recovery ciphertext `F`, and a compressed
192-byte Groth16 proof. Both ciphertexts are capped at 2,048 bytes. Variable
fields have u32 little-endian lengths; parsing rejects invalid mode/slot, excessive
lengths, noncanonical proof points, trailing data, truncation and wrong proof size.

`X=H("YPP-context-v1"||d||j||R||C_old||C_new||N||LE32(len(E))||E||LE32(len(F))||F)`.
`B=H(0x05||s||X)` is an additional proof relation binding the complete packet
context to the spending witness. This is a constrained SHA-256 preimage equation,
not a separately exported signature/authentication scheme. `F` is age encryption
of `S_new` to its owner's viewing recipient. `N` binds exactly the original credit
ciphertext, including during absorption. Credit plaintexts do not contain vault
balances, and Alice does not require Bob's pre-state witness.

## Exact proof relation and ledger checks

Public Groth16 inputs, in order, are `d`, followed by one field per byte of
`C_old,C_new,N,H(E),X,B`. There are 193 field elements, plus the constant input.
Individual bytes and the mode are range constrained; no digest-to-field reduction
ambiguity is used. Private witnesses are `s`, old/new balances and versions,
old viewing hash, old/new blindings and the credit opening. Owner is computed
in-circuit as `H(0x00||s)`, and reused unchanged in both state commitments. The
viewing hash also stays unchanged. Prove:

1. The old and new state hashes equal the public commitments.
2. `v_new=v_old+1`, with both versions range constrained to 64 bits.
3. The credit hash equals `N` and `a>0`, with `a,b_old,b_new` range constrained to
   64 bits. The BLS12-381 scalar field is much larger than these sums; modular
   wraparound cannot masquerade as u64 conservation.
4. In debit mode, `b_old=b_new+a`. In absorb mode, `b_new=b_old+a`, and credit
   owner/viewing hashes equal the vault owner/viewing hashes.
5. `B=H(0x05||s||X)`. The verifier independently hashes the actual packet to `X`.

Membership is checked against the ledger's existing authenticated leaves outside
the circuit: require `R=current_root` and `C_old=leaves[j]`, not a caller-supplied
membership assertion. Because leaves/slot are public, no hidden membership path
is needed or claimed. On debit, require `N` never previously appeared, then add
it to pending and permanent seen sets. On absorb, require `N` in pending, then
remove it. Verify the proof before making any changes. Replace only `leaves[j]`
with `C_new`; hash the new root and full packet into the history head.

Changing a ciphertext, anchor, slot, mode or commitment invalidates the context
binding/proof. Two simultaneous updates against the same root cannot both apply,
even if they target different slots. Old roots are accepted for **zero** further
updates; clients reprove after unrelated updates. A reused old leaf cannot spend
again, and versions cannot wrap. Seen credit markers remain forever. This is
sequential in-memory state, not crash-atomic database or distributed consensus.

Inductively, synthetic sum of live vault balances plus amounts of pending credits
is invariant: debit moves value from a vault to the inbox, and absorb moves it back.
The initial balances/openings and unique synthetic fixture are trusted, not an
issuance proof. No value is issued by either transition. A malicious owner can
strand value through invalid encryption; conservation alone does not ensure that
value is recoverable or that a recipient received it.

## Scanning, authenticated history and recovery

Trusted genesis contains both public leaves and age-encrypted openings. Its pinned
checkpoint hashes `YPP-synthetic-genesis-v1`, both leaves,
`LE64(len(E_0))||E_0||LE64(len(E_1))||E_1`. Lengths bind ciphertext boundaries.
Every accepted record carries the packet, resulting root and
`head_i=H("YPP-record-v1"||head_(i-1)||packet||root_i)`.
Recovery requires a trusted genesis checkpoint, exact verifier key, and externally
trusted final head. It replays proof verification and all state checks, validates
each root/head, tries local age decryption of each record, and checks recovered
openings against public commitments and the viewing authority's owner/view hashes.
The scanner has only viewing authority and cannot spend. It neither queries a
validator by address nor publishes a detection tag; scanning is linear in history.

A record failure preserves the prior in-memory cursor, state and inbox. A prefix
can resume from that cursor; a restarted process can rescan from genesis using its
backup. A prefix is not current until its head matches the externally trusted
head. There is no durable cursor format or wallet database in this milestone;
restart recovery is replay, not a claim of crash-safe incremental persistence.
A hash chain without a trusted final head cannot detect withheld suffixes or an
alternative history. No finality source, reorg handling, checkpoint distribution
or authenticated remote history transport has been implemented.

Full backup plaintext contains `YPP-backup-v1`, the spending secret and the
canonical age secret identity string, encrypted with age to an independently
retained backup identity. Restore is size/tag checked and must authenticate the
complete age stream. Losing that independent identity loses recovery. Secret
buffers use zeroization where practical, but SNARK witness/field copies and OS
memory are not guaranteed erased. There is no side-channel hardening claim.

Historical encrypted records must be retained for offline discovery/recovery.
The current ledger retains all records and permanent seen credit markers. No
pruning API exists. Deleting history removes wallet recoverability; deleting seen
markers removes the stated replay protection. A production design needs a
reviewed accumulator/checkpoint scheme, proof of data availability and archival
policy. The scanner currently clones history on each transactional step: it is
intentionally small and has quadratic cumulative copying costs, not a scalable
wallet index or benchmark of large-history operation.

## Exact unresolved privacy and correctness problems

- **Stable locator:** old/new public commitments and fixed slots expose all vault
  updates and their order. Hiding owner inside a commitment does not hide its
  activity graph. Private indexed updates need hidden membership and a reviewed
  oblivious state-update/version scheme; encrypted balances alone cannot supply it.
- **Offline receiving:** an offline Bob cannot supply his current hidden balance
  opening. Alice therefore creates a separate credit claim; Bob later absorbs it.
  This is expressly note-like, publicly links both steps via `N`, and requires an
  extra proof and eventual recipient transaction. It does not solve fully private
  asynchronous direct mutation of Bob's existing account.
- **Encrypted witness availability:** proof verifies the amount/opening hash and
  binds ciphertext bytes but does not prove age encrypts that opening to the
  committed recipient. A payer can submit an undecryptable credit with a valid
  proof; a vault owner can withhold its own recovery opening. Honest sender demo
  works; guaranteed delivery and recovery against malicious senders do not.
  The adversarial test constructs and verifies this exact real proof.
  Addressing it requires verifiable encryption/circuit-compatible reviewed
  encryption and key binding, plus data availability; this milestone has neither.
- **Concurrency/liveness:** exact-root acceptance globally serializes state and
  requires reproving on any change. Fair scheduling, stale witness refresh and
  denial-of-service bounds need design. Local verification is deterministic,
  but no network/consensus liveness or adversarial resource policy exists.
- **Monetary modules:** synthetic conservation is not a protocol for fees, staking,
  delegation, rewards, treasury, supply changes, slashing or monetary policy.
  Those relations are not implemented or activated.
- **Setup and review:** OS-random per-run trusted setup is a development tool.
  Malicious/toxic-waste-retaining setup can forge proofs. No multiparty ceremony,
  setup verification lifecycle, independent security review of this relation,
  complete dependency advisory audit, or production secret hygiene is claimed.

## Prior art and distinctness

- Zether (Bünz et al., 2019), *Zether: Towards Privacy in a Smart Contract World*,
  <https://eprint.iacr.org/2019/191>: encrypted account balances and ZK-authorized
  account updates predate YPP; epoch/concurrency and anonymity trade-offs are
  relevant. This code does not implement Zether or its encryption/range proofs.
- Aztec (Williamson, 2018), <https://github.com/AztecProtocol/AZTEC>, and Zerocash
  (Ben-Sasson et al., 2014), <https://zerocash-project.org/paper>: confidential
  value claims, commitments, consumption markers and encrypted delivery are
  prior art. YPP's inbox is explicitly analogous, not an originality claim.
- Zexe (Bowe et al., 2020), *Enabling Decentralized Private Computation*,
  <https://eprint.iacr.org/2018/962>: private records and proof-authorized state
  transitions are existing ideas. Persistent logical application state is not
  itself evidence of a distinct cryptographic construction.
- Groth (2016), <https://eprint.iacr.org/2016/260>, arkworks
  <https://github.com/arkworks-rs/groth16> and
  <https://github.com/arkworks-rs/crypto-primitives>: established proof system and
  implementation/gadgets used here. age format: <https://age-encryption.org/v1>.
  These references explain provenance; they do not audit this composition.

Monero CLSAG/FCMP++/Spark conceal a spend among possible prior outputs under their
own models; this experiment performs proof-authorized in-place balance/version
updates and uses no ring/decoy machinery. Conventional shielded notes consume
old notes and create new notes, usually using hidden membership and nullifiers.
Here the principal balance remains at the same public slot rather than being
split into payment outputs; temporary delivery credits still behave like claims.
This has compact live vault state but sacrifices unlinkability and serializes
updates. The goal of an original **private** persistent-vault architecture is a
research direction, not a delivered security property or novelty claim.
