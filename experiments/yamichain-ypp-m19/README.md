# YPP Milestone 19 — ORAM composition rejected before implementation

**Verdict: NO-GO for ConcurORAM or MVP-ORAM plus public verification as a
zero-trust private-account construction under the requested restrictions.**
No complete replacement for the forbidden shared metadata secret was found.
This is a rejection of the examined composition, not an impossibility theorem
for every multi-client ORAM, cryptographic RAM or private account encoding.

Base: `42cf602`; branch `development/yamichain`. This directory is research only:
no new executable, circuit, proof, simulator, dependency, payment API, trusted
worker or production change. The stop decision was made before writing code.
The existing exhaustive production rejection test was rerun without modifying it.

## Prior rejection reports read first

The following findings are constraints on this investigation, not problems assumed
solved by choosing a different storage primitive. Exact paths and file hashes are
in `prior-reports.sha256`; source and validation details are linked in `SOURCES.md`.

| Earlier investigation | Constraint carried forward |
|---|---|
| Privacy policy/backend gate and RingCT reference | Production remains fail-closed; an offline proof demonstration is not activation authority or an approved account model |
| Vaults / Milestone 1 | Slots and old/new commitments expose activity; binding ciphertext bytes does not prove usable delivery |
| Hidden records / Milestone 2 | Append-only version lineage/inbox is note-like; not an allowed substitute for private account mutation |
| Feasibility / Milestone 3 | Historical-root replacement loses unrelated writes; Merkle-root differences do not merge; public KZG refresh hints expose update positions; old backups cannot reconstruct undistributed fresh openings |
| Oblivious storage / Milestone 4 | Dense two-server shares and normal recovery snapshots expose support when combined; shape proofs do not authorize conflicts or offline credits |
| Encrypted accounts / `5b9f43b` | Genuine additive account updates and epoch tags work in bounded experiments; public cohorts, 32-bit recovery and a local trusted setup remain limitations |
| Milestone 18 / `42cf602` | Sparse *logical* public support fails global privacy; Merkle authentication, correct cross-cohort accounting and faster proofs do not hide it |

Milestone 18's bound does not forbid ORAM: public **physical** paths can be
compatible with privacy when their association with logical accounts remains
hidden. The question here is who can interpret and maintain that association.

## What the requested payment must mean

Alice and Bob have independent wallet secrets sk_A and sk_B. In the requested
example, private amount v=100. A successful transition must establish:

```
Alice owns the debited account; Alice_balance >= v > 0
Alice_balance' = Alice_balance - v
Bob_balance'   = Bob_balance + v <= 2^64 - 1
all balances, before and after, are canonical unsigned 64-bit integers
all other logical balances are unchanged
both effects commit exactly once, or neither effect commits
```

Physical relocations, re-encryption, map changes, stash updates and version
maintenance must preserve all other logical records. Their correctness and
availability also need verification. The displayed equations are a specification,
not an implemented proof or an accounting simulation. A proved global supply cap
at most 2^64-1 could discharge recipient overflow without opening Bob's balance;
no claim that every safe credit must disclose that balance is made. Alice must
not need Bob's plaintext or secret to produce the credit. A proof system's ability to express
these equations does not give Alice their unavailable witness.

Public state would need an authenticated commitment to *all* live data and
metadata, a finality/freshness mechanism, publicly available recovery material,
and replay/conflict history or a proven safe replacement for it. Authentication
can rely on explicitly stated consensus assumptions; it cannot rely on an RPC
supplying a root and asserting that its own root is authentic. No consensus
assumption may be repurposed as permission to give validators privacy secrets.

## Paper models: concise source findings

### ConcurORAM — NDSS 2019, sections IV-A/B, V and VII-B

ConcurORAM assumes honest clients sharing credentials and a semi-honest server.
Its stateless clients retain secrets; persistent maps/stashes reside server-side.
The map uses PD-ORAM. Encrypted query/result logs and locks coordinate access,
and the query log records logical addresses. Query cost is O(log N+c) blocks;
temporary client storage is O(log N) blocks under its block-size assumptions.
The malicious-server extension targets fork consistency. None of this proves
privacy between mutually hostile clients.

### MVP-ORAM — NDSS 2026 extended version, sections III, VI–VIII

Clients are mutually trusted and fail by crashing; base security excludes adversarial
query injection. Replicas hold secret shares of ORAM encryption keys; accessing
clients reconstruct key material. Base obliviousness depends on workload and
concurrency. Strong MVP-ORAM adds sigma dummy accesses per real access, requires
c<=sigma+1 and relative-speed/synchrony assumptions; it does not establish hostile
wallet isolation. Published costs are O((R+c)c log N) communication and
O(N+c^2 log N) storage per replica, in its model, with three service invocations
per access. R counts replicas, not wallets. The paper leaves integration of
verifiable computation against fully malicious clients to future work.

Those models differ from the requested adversary even before financial proof
composition. Public verification of an ORAM step and confidentiality between its
clients are separate obligations.

## Exact placement of state, secrets and synchronization

The MVP artifact findings below are independently corroborated by public Java
source at `6d51057bf3ccd50c9374703586be1a88bb88b08f`, not inferred from its name.
Pinned file/line references and SHA-256 hashes are in the source record.

| State | ConcurORAM model above | Inspected MVP artifact | Required YPP boundary |
|---|---|---|---|
| Account spending/view secrets | Not an account-payment authorization system | Not an account-payment authorization system | Only the relevant wallet; never a common ORAM credential |
| ORAM metadata encryption authority | Shared client credentials | Creator generates a password; replicas store confidential shares; `getORAM` retrieves confidential material and derives the same encryption key | No common client secret and no privacy-critical replica shares |
| Position information | Server-resident private map accessed through PD-ORAM | Encrypted path-map history on server; decrypted full `PositionMap` arrays in client memory | Selective hidden lookup/update with independent keys, including when Bob is offline |
| Stashes / block headers | Interpreted by trusted clients | `EncryptionManager` decrypts stashes, maps and bucket records for clients | A payer cannot gain another owner's routing secrets by being a valid payer |
| Synchronization | Server logs, ordering and locking | `latestAccess`, `missingTriples`, four position-map arrays, versioned path maps, per-client outstanding contexts and ordered eviction processing | Authenticated, crash-safe and privacy-preserving payment commits, with bounded recovery |
| Trusted current head | Fork-consistency issue in server model | Provided through the replicated service model | Explicit finality and availability; public validation must not disclose metadata secrets |

In the artifact, `ORAMManager.getORAM` consumes `getConfidentialData()[0]`, derives
an encryption key and passes the `EncryptionManager` to `ORAMObject`.
`ORAMServer` returns the stored encryption-key share on `GET_ORAM`.
`EncryptionManager` uses its abstraction to decrypt path maps, stashes and
buckets. This is shared **ORAM-layer** secrecy, even if a hypothetical inner
account ciphertext has a separate owner-only spending key. Splitting those
layers does not make the outer key harmless or satisfy the no-shared-secret rule.

`ORAMObject` allocates a position map for the entire tree's logical address space,
merges received map histories by version/access counters, translates an address
to a physical bucket/path, reads paths/stashes, modifies a block and sends an
eviction. `PositionMap` has four integer arrays: locations, accesses, versions,
and location-update accesses. The server retains encrypted path-map versions and
outstanding paths; clients maintain progress cursors and missing-version sets.
This is real synchronization state, not state removed by describing a client as
stateless. A freshly joining client must obtain enough data to reconstruct it.
The artifact's `cleanPositionMaps` body is empty at this revision, so it cannot
be cited as an implemented bounded-history recovery mechanism. This observation
is not a claim that the paper's abstract protocol lacks a compaction strategy.

The paper describes key delivery during access, whereas this inspected artifact
bootstraps confidential material through `GET_ORAM`; these are not claimed to be
identical API traces. Both require privacy-critical key material outside one
independent account wallet.

## Attempted algorithms and the first undefined operation

The candidate was: store owner-encrypted accounts in a shared multi-client ORAM,
authenticate the whole service state, and require a public proof for each atomic
100-YAMI transfer and its storage rearrangement. The following is its *rejection
trace*, not a deployable algorithm:

1. **Authenticate current state.** Alice obtains a fresh finalized state commitment
   and public storage data. A hash alone does not authenticate finality.
2. **Private metadata access.** Alice must resolve her logical account and Bob's
   address into their current records, without disclosing the association, using
   only sk_A and public data. Call this `LocateAndPrepare(sk_A, pk_B, head)`.
   **No construction for this operation has been supplied by the examined ORAMs
   under the allowed key model. Stop here.** Giving Alice the outer shared key or
   retrieving threshold shares changes the model; publishing the map discloses
   routing; putting it only under Bob's key prevents offline writers from using it.
3. **Blind credit and eviction.** Even if step 2 were granted, Alice would need to
   credit Bob and relocate/re-encrypt unrelated records without learning their
   plaintexts or map secrets. Generic opaque ciphertext copying is not a complete
   stash-placement, duplicate-version resolution and metadata-update algorithm.
4. **Public authorization.** Prove the monetary specification, correct record
   selection, preservation of all other records, proper randomness, exact metadata
   evolution, global replay/conflict binding and encrypted recovery correctness.
   There is no implemented composed statement/witness generator for steps 2–3.
5. **Atomic concurrent commit.** Accept both logical account effects and all storage
   changes together, resolving conflicts without stale-root rollback. No protocol
   for doing so with the missing private metadata operation is specified.
6. **Offline recovery.** Bob independently locates, authenticates and decodes the
   latest full-range u64 state using sk_B and public data. A common outer key,
   secret-share service, latest random opening delivered privately to a worker,
   or an unbounded history scan is not this algorithm.

None of steps 3–6 is marked implemented just because a specification can name it.
The rejection at step 2 already triggers the user's before-coding decision rule.

### Why the obvious key substitutions fail this candidate

- Publishing the map makes it possible to associate logical accounts with physical
  storage changes, reviving the Milestone 18 leakage. Encrypting it with a key
  known to every account owner leaves the same information available to a corrupt
  owner. Publicly posting all decryption shares has the same effect.
- Encrypting each map entry only to its owner prevents Alice from using Bob's
  private current locator when Bob is offline. A selective, independently keyed
  private lookup/update capability would be new cryptography/protocol design;
  neither a hash proof nor an owner signature constructs it.
- A separate ORAM per wallet does not solve cross-wallet credit: Alice still needs
  a safe way to modify Bob's ORAM, and public per-wallet ORAM identifiers can expose
  the account. Shared routing capabilities would violate the explicit restriction.
- A server/proxy holding the map secret or threshold-distributing it to writers
  reintroduces the prohibited secret-bearing service, even if the inner payload
  uses another encryption key. Ordinary consensus quorum assumptions do not
  authorize this additional confidentiality assumption.
- A simple FHE implementation of a hidden-index array update can evaluate every
  entry. That particular full-array approach has O(N) work and a dense resulting
  ciphertext array unless another representation/update protocol is specified.
  Compact proofs do not supply a compact available state update. This is not a
  lower bound on every FHE/ORAM construction; no such alternative is validated here.

## Publicly verifiable computation: what a proof would and would not establish

A necessary proof relation would bind (chain/domain, old authenticated state,
new authenticated state, operation commitment, conflict identifier, encrypted
storage transcript) to a valid atomic transition. Its witness would cover both
account authorization and complete private storage metadata evolution. It must
show exact ranges, conservation, valid old/current versions, preservation of
unrelated records and recoverable owner-encrypted outputs. Correctness of a
single ORAM eviction alone is insufficient.

A ZK proof can restrict which modifications are accepted. It cannot revoke a
key already learned by a client, stop that client from decrypting saved public
metadata locally, or synthesize unknown map entries for a prover. Even a proof
that all accepted writers followed the protocol would not hide a query log from
someone authorized to decrypt that log. Preventing malicious updates and hiding
other users' accesses are different security properties.

Likewise, authenticated ciphertext storage proves byte consistency, not valid
account arithmetic or delivery. An externally verifiable ORAM dispute-resolution
protocol is not automatically a publicly verifiable currency transition; the
2017 construction inspected uses a trusted arbitrator. No arbitrator is adopted.
No claims of succinctness, setup assumptions, soundness or runtime are made for
an uninstantiated composed proof. The earlier Groth16 circuit is not silently
reused as if it verified these ORAM operations.

## Indistinguishability under repeated transactions and malicious clients

The required game allows an adversary to observe all public storage snapshots,
physical paths, reads/writes, operation counters, proofs, heads, message sizes,
ordering and aborts. It may control storage endpoints/validators for privacy
analysis, create arbitrary valid account keys, corrupt non-challenge owners and
schedule their permitted accesses adaptively. Integrity/finality still needs its
explicit consensus assumptions. Network identity/timing leakage must either be
included or replaced by an explicit ideal anonymous-transport assumption; we grant
that idealization here and still reject the candidate.

Challenge histories start at the same authenticated state, have the same public
length/amount/fee schedule, feasible balances and equal disclosures at corrupt
accounts. Honest endpoints remain uncorrupted. The challenger executes one history
under a hidden bit. The adversary's whole-history guessing advantage must be
negligible, including for repeated payments and concurrent valid probing accesses.
The public map/log is an observation, not something the game conditions away.

Consider the proposed composition granting a corrupt but valid wallet the same
metadata decryption authority as other writers. It obtains/decrypts the available
logical-address log or labelled position metadata while a challenge is executing.
For ConcurORAM's query log, selecting different challenge logical addresses then
reading that decrypted entry directly distinguishes the histories, up to ordinary
cryptographic correctness error. This is a deductive attack on the proposed trust
boundary, not a measured attack on a deployed system or an empirical proof of
security. Repeated honest random coins do not hide plaintext metadata from its
key holder. A validity proof cannot prevent this passive local computation.

For the inspected MVP client path, a writer similarly receives authority to read
and consolidate global labelled routing metadata. That already violates hostile
wallet isolation and the no-shared-secret rule; no universal claim that every
observed physical path uniquely identifies its queried block is needed. A path
can contain many blocks. Correct ORAM privacy must hide associations, not merely
avoid publishing a literal account-index field in the payment packet.

The examined security theorems do not establish this game. In particular, a
workload distribution or a bounded/dummy schedule cannot be substituted for
privacy under the requested malicious-client scheduler without proving that
scheduler restriction enforceable under the allowed trust model.

## Synchronization: memory operations versus payments

Linearizable individual reads and writes can legally produce:

```
read_A(Bob) = 1000; read_C(Bob) = 1000
write_A(Bob, 1100); write_C(Bob, 1070)
```

The final value 1070 loses Alice's credit although each memory operation was
linearizable. This is a simple specification counterexample, not a new payment
simulator. A timestamp/last-writer merge does not implement addition or an atomic
transfer. Reversing the writes instead loses Carol's credit. Debit and credit
cannot be separate commits, either, since a crash could leave only one effect.

A solution needs an authenticated atomic RMW/transaction, or proven commutative
credit deltas plus private debit conflict prevention and safe metadata merging.
Ordering requests through consensus can serialize commits but does not by itself
refresh private client witnesses or make a stale-root proof apply safely. Replacing
the current state with a valid old-root-derived state repeats Milestone 3's failure.
The existing encrypted pending-credit/epoch-tag algebra helps the monetary layer;
it does not automatically merge ORAM maps, stashes and duplicate record versions.
Successful independent *memory operations* are not evidence that these payments
can execute concurrently without lost updates.

## AFT 2026: exact scope of the synchronization trade-off

Cachin–Lehnherr–Villacis–Wicht study abstract linearizable shared-memory objects,
not Byzantine ORAM implementations. CUAT consumes/replaces every masking-set
member; overlapping invocations conflict. With masking size lambda, weak
untraceability has unbounded consensus power; the strong variant's consensus
number is lambda^2. One-round protocols have upper bound lambda^2-lambda+1.
Their incidence characterization uses symmetric priors and an independent-inclusion
model; it is not general cryptographic transcript indistinguishability.
For a fixed eligible workload without interference, coloring/independent-set
numbers characterize rounds/parallel success. CUAT can starve under an unfair
asynchronous scheduler despite terminating attempts. The full-version LUAT model
retains representations/nullifiers, has consensus number 2 for unshared accounts
and pays in growing state. Epoch expiry introduces an additional progress
assumption. These results do not prove all encrypted account protocols impossible,
nor specify a number of blockchain validators or a transactions-per-second limit.

For YPP, define the invalidation and progress rules explicitly. These object
bounds do not decide feasibility for a different private-access interface.

## Complexity and the 500,000-account requirement

No accepted construction means **no end-to-end YPP complexity or performance
claim**. The paper bounds above apply only under their own trust and workload
models. Communication is not identical to computation, latency or validator
verification work. Strong-mode padding multiplies accesses; bounded concurrency
must not be mistaken for the total number of registered wallets.

Some independently checkable accounting clarifies the size of the missing state:

- For a notional tree with at least 500,000 leaves, ceil(log2 N)=19 and a path has
  20 levels. A leaf-label-only map needs 9,500,000 bits = 1,187,500 bytes before
  stash locations, authentication, versions, recursion or encoding. This is an
  illustrative representation size, not a lower bound for every ORAM encoding.
- The inspected `PositionMap` serialization is exactly `4+16*M` bytes for M
  entries. At M=500,000 it is 8,000,004 bytes. In that implementation a complete
  tree of height 18 has M=524,287 logical map entries and serialized size
  8,388,596 bytes. These are **source-derived formulas**, not measured heap or
  network bytes. Client working storage for those four arrays is Theta(M).
- Reading/writing a full map each operation would be Theta(N) data transfer.
  Recursion/deltas can change this cost, but require the same confidential
  metadata access/control and a bounded authenticated cold-start strategy.
- The M18 account record stores 168 serialized bytes/account, or 84,000,000
  bytes for 500,000, before ORAM padding/metadata, redundancy and authentication.
  This existing record-size calculation does not constitute a 64-bit private
  payment format or an ORAM database benchmark.
- Public verification must account for generation of *all* ciphertext data needed
  by the next reader. A small proof/root is not data availability. A scheme requiring
  Bob to replay unbounded history instead of fetching bounded current state does
  not meet this task's recovery/scale objective.

The unresolved operation cannot be assigned a polylogarithmic cost merely by
naming ORAM. There are no new transaction bytes, proof-generation/verifier times,
ORAM throughput, 500k full-registry measurements or recovery measurements here;
those quantities are **not implemented/not measured**, not zero.

## Full 64-bit offline receiving and recovery

If Alice instead needs Bob's symmetric payload key to rewrite his u64 balance,
the independent-key requirement fails before any decoding question. Owner-key
homomorphic encryption could avoid that particular disclosure, but still needs
private routing plus a sound, publicly verifiable bounded-integer update.
ORAM does not alter the balance encryption's decoding complexity.

The previous single-exponent ElGamal balance supports bounded 32-bit decoding;
full-range u64 BSGS needs roughly 2^32 entries/steps. Splitting into 16-bit limbs
recovers a static u64 but leaves the offline encrypted carry/normalization operation
unresolved. Standalone Paillier decryption supplies neither cross-key conservation
nor a proof of the anonymous account update. No missing full-range transfer or
recovery statement is implemented in this milestone.

Bob would additionally need to reconstruct current placement using only sk_B and
fresh authenticated **public** data. Downloading confidential shares from service
replicas, sharing another writer's outer key or fetching an owner's undistributed
opening is not that recovery procedure. Offline duration must not introduce an
unbounded reconstruction cost with no stated checkpoint/pruning protocol.

## Decision against the eight requirements

| Requirement | Finding for the examined composition |
|---|---|
| Alice's private access | Fails the allowed key model at metadata bootstrap |
| Debit 100 / credit offline Bob | No independently keyed private routing and blind-credit composition |
| Independent public verification | Necessary full storage-plus-payment statement has no witness-generation protocol |
| No revealed account-location association | Shared metadata authority exposes information to a malicious authorized holder; public map repair leaks |
| Concurrent payments, no lost updates | Read/write linearizability is insufficient; safe private transactional composition missing |
| Bob's full u64 recovery | Neither independent placement recovery nor full-range encrypted balance transition supplied |
| Malicious-holder isolation | Outside the trusted-client models; public proofs cannot undo shared decryption authority |
| 500,000 accounts / subdomain transfers | Storage asymptotics under other assumptions do not demonstrate this requirement for YPP |

Reject before coding. Reopening requires a complete **independently keyed,
malicious-client-secure, publicly verifiable private metadata lookup/update and
atomic credit/debit protocol**, with explicit recovery and bounded costs. It must
not hide a common secret or a secret-bearing service in the words “stateless”,
“BFT”, “verifiable”, or “ORAM”. No universal impossibility or approved alternate
architecture follows from this review.
