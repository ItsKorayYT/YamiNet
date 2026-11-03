# Decision: reject the tested distributed storage composition

This is a bounded technical rejection of **dense two-server additive private
writes plus authenticated complete snapshot download**. It is not a rejection
of all distributed oblivious storage, PIR, MPC, ORAM, FHE, encrypted witness
updates, or private in-place state. No originality or production privacy claim
is made. The mechanisms below are established building blocks; their particular
composition is an unaudited experimental implementation.

## Candidate and exact proof statement

There are N fixed physical slots, each with a 16-byte synthetic plaintext
`(version, counter)`. These counters are test data, never monetary balances.
ChaCha20-Poly1305 encrypts each slot with a different wallet-only key, fresh nonce
and index associated data. Each 44-byte nonce/ciphertext/tag is padded to 62 bytes
and encoded into two canonical 31-byte limbs in the Ristretto scalar field.
Nothing is spent, appended or recreated as an output note: this is a fixed-slot
storage update experiment, and its physical-slot privacy fails.

Storage server s holds vector S_s such that S_0 + S_1 = E, the vector of encrypted
cells. Neither server is given wallet encryption keys. To replace slot k's old
ciphertext E_k by E'_k, the client makes a two-limb point vector D with
D_k = E'_k - E_k and zero elsewhere. It chooses uniform A and sends:

```
server 0: A
server 1: D - A
each server: S_s <- S_s + its received vector
```

These are dense additive secret shares, not an implementation of compressed DPF
keys or the complete Riposte protocol. Each individual vector is uniform and
independent of k. Private authenticated channels and separation of server views
are assumptions of this private-write boundary. No network anonymity, traffic
analysis resistance, Byzantine storage threshold, or real distributed deployment
is implemented. Tests execute independent logical servers in one process; the
fixture creator knowing all keys is test setup, not a proposed coordinator.

For each slot i each server message includes a random blind r_si. The public
packet contains C_si = a_si,0 G_0 + a_si,1 G_1 + r_si H. Independent hash-to-group
generators have no setup trapdoor. The public aggregate is D_i^C = C_0i + C_1i.
The shape proof is the established Schnorr AND/OR relation:

```
OR over k: AND over j != k: know r_j with D_j^C = r_j H
```

Simulated branches use independent challenges/responses; the genuine branch
closes their sum to the Fiat-Shamir challenge hashing the domain, dimensions,
fresh operation nonce, all commitments, and all announcements. Canonical scalar
and point parsing and exact dimensions are checked. Server messages must open
the public commitments. Under standard discrete-log/representation knowledge
and random-oracle assumptions, all but at most one cell open to zero. We make no
claim of a separate security proof/audit for this handwritten implementation.

The proof has N branch challenges and N(N-1) responses, each 32 bytes. Its O(N²)
cost is intentionally explicit. It is a **shape proof**, not owner authorization
or a state-transition proof. A random operation nonce only rejects exact packet
replay. Neither nonce nor proof is an authenticated old-version conflict marker.

## Independent updates work; conflicts destroy state

For two different slots A and B:

```
E_final = E + D_A + D_B
```

Both ciphertexts survive and order does not matter. Both clients prepare from the
same snapshot. No stale-root retry, historical-root replacement, field addition
of Merkle roots, or witness-refresh disclosure is used.

For two replacements x and y of the same old encrypted cell e:

```
e_final = e + (x - e) + (y - e) = x + y - e
```

This is neither replacement. The two real shape proofs pass, both updates apply,
and the wallet rejects the resulting bytes through encoding/AEAD authentication.
The test first confirms the first replacement was decryptable. Two different
operation nonces evade the exact replay filter. This is a conflict-authorization
failure, not a failure of AEAD or of additive sharing. Adding a public stable
per-slot compare-and-swap key would immediately disclose a stable identifier.
A rotating marker would need a proven linkage to the correct current hidden
version; that relation and authenticated hidden-state access are absent here.
We do not claim such a relation cannot be built by other protocols.

## Observer attacks, with distinct input boundaries

1. **Private-channel public packets only.** Hiding commitments and the standard
   shape-proof composition are intended to hide the selected branch under their
   assumptions. The experiment does not prove privacy empirically or claim that
   lack of a byte-equality match establishes unlinkability. Packet timing,
   transport identity, and external correlations are out of scope.
2. **Both write channels available.** The observer adds the two vectors and lists
   nonzero cells. It learns k exactly and links repeated writes. It gets no wallet
   key, plaintext, or target index. Publishing the raw messages for public
   validation or operating both storage servers creates this transcript.
   Commitment blinds in the exposed fixture only permit independent opening
   checks; the attack does not need them.
3. **Only normal recovery downloads available.** A wallet downloads both complete
   latest snapshots to authenticate and recover its own cell. It can also combine
   every other encrypted cell. Comparing two epochs reveals exactly the changed
   ciphertext positions. An ordinary wallet can do this for every epoch; no
   colluding servers or private write channels are needed. The independent Python
   observer consumes only these downloadable snapshots and reconstructs all
   update positions and same-slot pairs. Ciphertext confidentiality survives;
   update-location privacy does not.

Stable array positions become stable vault handles through these observations.
There is no need to identify a human owner to violate the stated requirement.
The eight-update trace has a single actual write per epoch, including repeated
updates at different intervals. Random guessing of a slot succeeds with 1/N;
both exposed-view attacks succeed on all trace entries. This deterministic
algebraic distinguisher is stronger evidence than a small empirical classifier.
The public-commitment-only transcript must not be conflated with either exposed
view. The rejection applies to the complete recovery/validation composition.

## Wallet witness refresh, offline recovery and availability

A witness here consists of both complete ciphertext-share snapshots and an
independently pinned current head `(epoch, N, hash(snapshot0), hash(snapshot1))`.
The snapshot hash binds the domain, epoch, length and all canonical scalar bytes.
The wallet verifies both complete snapshots before decrypting only its slot.
This is the trivial full-download private retrieval baseline: the request is
identical for every vault and reveals no selected index to a storage server.
It provides neither selective-disclosure privacy to other wallets nor succinct
membership witnesses. The recovery test covers eight unrelated updates while a
wallet is offline, as well as decrypting another wallet's repeatedly rotated
cell using its original key/index. There is no fixed history window and the
latest complete snapshots suffice; no old per-epoch backup is needed.

Changed, truncated, old-epoch and wrong-key recovery attempts fail. Returning an
old head together with old snapshots remains a rollback unless the wallet pins a
new head from independently finalized history. Hash consistency is not finality.
The lab computes heads from honest in-memory server states; no proof binds those
hashes to the accepted message shares and prior head. Consequently the public
verifier cannot agree on correct state changes from this proof alone. Replacing
this missing link by trust in two receipts would assume storage honesty. The
single-process atomic prechecks are not a distributed atomic-commit protocol.

All encrypted cells reside in the two storage shares. Reconstructing any cell
needs both. One unavailable server makes recovery impossible; hashes detect this
but cannot reconstruct missing data. Redundant independent copies of each share
can improve availability, while requiring explicit placement/collusion and
retention assumptions. Availability certificates, erasure-coded authenticated
retrieval, archival redundancy, Byzantine agreement and censorship resistance
are not implemented. No timeout or hash is presented as a data-availability
proof. A public-copy availability repair also enables the snapshot observer.

Offline receiving is not a successful feature of this candidate. The sender has
neither the recipient's symmetric key nor a recipient-authorized credit relation.
A sender can retrieve ciphertexts and generate a shape-valid overwrite encrypted
under its own unrelated key; validators' shape checks accept it and the recipient
later fails authentication. The test names this corruption, not a credit. An
asynchronous encrypted note/inbox fallback would be a different state model and
was not added. Homomorphic account credit would need a different encryption and
authorization relation, plus the unresolved private-access/availability layer.

## Why other established blocks do not repair this implementation automatically

| Building block | What it supplies | What would still need implementation and review |
| --- | --- | --- |
| Riposte / distributed point functions / function secret sharing | Compact private point writes under a defined server/collusion model | Owner/current-version validity, correct public state heads, recovery privacy and Byzantine availability; compression leaves the support-reconstruction attack intact |
| Replicated-database PIR | Hide selected read position from separated responders holding the same database | Authentication, current-head consistency, access control, and compatibility with secret-shared rather than replicated storage |
| Distributed oblivious storage / multi-client ORAM | Hide read/write access traces under a defined client/server model | Decentralized authenticated multiwriter control, shared metadata synchronization, crash recovery, offline receiving, and a model without a secret-bearing central ORAM client |
| Encrypted witness updates | Deliver refresh material confidentially to the right wallet | Public proof of correct encrypted delivery, sender encryption authority without wallet secrets, offline discoverability, and durable authenticated availability |
| MPC / FHE plus validity proofs | Compute hidden state transitions without reconstructing locations | A concrete reviewed protocol, malicious-security/threshold assumptions, distributed execution and recovery; not implemented or ruled out by these counterexamples |

The tests include an exact algebraic counterexample to simply placing ordinary
two-replica linear PIR atop two additive storage shares. For replicated database
E, q0 + q1 = e_k gives q0·E + q1·E = E_k. For E = S0 + S1,
q0·S0 + q1·S1 generally differs from E_k. More replicas, suitable secret-shared
PIR or MPC might work; relabeling the two storage shares as PIR replicas does not.

No operator is trusted with wallet decryption keys. Nevertheless, making
validators keep private storage views, assuming non-collusion of unspecified
operators, trusting unproven state heads, or granting a central ORAM client all
wallet secrets would change the task's trust model. This experiment stops before
making those substitutions. The normal-retrieval leak already suffices to reject
the tested composition even if storage operators remain separate and honest.

## Established references and scope

- Corrigan-Gibbs, Boneh and Mazières, **Riposte: An Anonymous Messaging System
  Handling Millions of Users**, IEEE S&P 2015,
  [project and paper](https://crypto.stanford.edu/riposte/). Its anonymous messaging
  setting and server assumptions are not a private-vault state-transition proof.
- Boyle, Gilboa and Ishai, **Function Secret Sharing: Improvements and Extensions**,
  ACM CCS 2016, [author publications](https://www.cs.technion.ac.il/~gilboa/).
  Dense additive shares here are a baseline, not their optimized FSS construction.
- Cramer, Damgård and Schoenmakers, **Proofs of Partial Knowledge and Simplified
  Design of Witness Hiding Protocols**, CRYPTO 1994: standard AND/OR composition
  of Sigma protocols; Pedersen, **Non-Interactive and Information-Theoretic Secure
  Verifiable Secret Sharing**, CRYPTO 1991: hiding commitments.
- Chor, Goldreich, Kushilevitz and Sudan, **Private Information Retrieval**, FOCS
  1995 / JACM 1998: replicated-database PIR. The full-download recovery implemented
  here is the trivial bandwidth baseline, not an efficient PIR implementation.
- Stefanov et al., **Path ORAM**, CCS 2013,
  [paper](https://pages.cs.wisc.edu/~yxy/pubs/PathORam.pdf).
- [RFC 9496](https://www.rfc-editor.org/rfc/rfc9496.html) describes Ristretto255;
  [RFC 8439](https://www.rfc-editor.org/rfc/rfc8439.html) describes
  ChaCha20-Poly1305. The actual implementations are pinned upstream Rust crates.

These are prior-art pointers and design boundaries, not a claim to implement,
audit, benchmark or disprove those full protocols. This milestone's result is a
reproducible rejection of the specified storage composition. A replacement
candidate must supply a complete authenticated private read/write/conflict and
availability protocol before payment features are justified.
