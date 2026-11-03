# Architectural decision: no approved Private Vault construction

**Reject the tested root-replacement and public-refresh compositions as intended
YPP. Keep production and additional monetary modules on hold.** We have concrete
cryptographic counterexamples, not a proof that private persistent state is
universally infeasible. We do not substitute another shielded-note implementation
or claim originality for an existing private-record architecture.

## 1. Hidden writes are insufficient for concurrent composition

The actual Groth16 relation proves knowledge of an owner's SHA-256 preimage, a
blinded old vault state, a hidden two-level Merkle path, and a new state at the
same private location. Owner/balance stay unchanged; a range-constrained u64
version increments without wrap. HMAC-SHA256(secret, domain || old_commitment)
is a public conflict marker. Neither leaf commitment nor location is published.
The only public inputs are old root, new root and marker. Standard arkworks
Pedersen CRH compresses Merkle nodes; SHA-256 commits vault fields. The test uses
full-width balances, including a value near u64::MAX.

Let Alice and Bob start at R0. They independently produce **valid, independently
verified proofs** R0→RA and R0→RB. Let RAB be the root of both private changes.

| Acceptance rule | Result |
| --- | --- |
| Require the current root | Alice applies; Bob's independent proof is stale |
| Accept any known historical root; replace the current root | Both apply; final root RB restores Alice's old state, RB ≠ RAB |
| Keep global conflict markers as well | Rejects two Alice branches, but the unrelated rollback still occurs |
| Add (RB−R0) to RA as a field element | Deterministic root, but different from RAB |

Replicas replaying the unsafe history agree on exactly the same wrong root.
Even with unchanged balances, Alice's current private version is absent while her
restored old version's conflict marker is already spent: usable state was lost,
not merely rerandomized. Consensus agreement is not state correctness. All proofs in this counterexample
are genuine; adversarial packet mutations and fully rebound inflation witnesses
fail verification. The unsafe acceptance policies exist solely to reproduce this
failure and must not be registered as chain verifiers.

A fresh Bob proof against RA succeeds if Bob receives the updated Merkle witness.
Publishing only Alice's new commitment is already enough to break the tested
privacy model: an observer who knows the previous commitment tree substitutes it
at every position and matches RA. Repeating this with Alice's next update returns
the same position. No old/new pair or explicit index is needed. This attack's
assumption—the observer knows the previous leaf commitments—is stated, not hidden.
An encrypted tree/private-witness service would be a different construction and
needs its own availability and trust analysis.

## 2. Linear commitments compose; the tested refresh channel leaks

The second experiment uses upstream **KZG10 over BLS12-381**, not a new polynomial
commitment. Interpolate an evaluation vector as p. A private single-position delta
is d. Actual group commitments satisfy C(p+dA+dB)=C(p)+C(dA)+C(dB), in either order.
An old opening for an unaffected position does not verify under the new root.

The tested repair publishes KZG quotient-opening hints Qi for d at every public
domain point zi. Wallet i refreshes its witness by adding Qi; upstream KZG checks
confirm it works. But an observer receives only C(d), the hints and public
parameters and runs check(C(d),zi,0,Qi). All unchanged positions pass; the changed
position fails. The attack does not receive the index, delta value, owner keys or
setup trapdoor. Repeated writes reveal the same position. This remains true even
though the hints contain no explicit claimed evaluation values.

Thus **this ordinary non-hiding KZG/public-hint composition is rejected**. It is
not a theorem against all vector commitments, hiding variants, encrypted refresh
broadcasts or zero-knowledge sets. The experiment also shows that blindly adding
two deltas based on one old version is not either authorized replacement state;
version/conflict authorization still has to be proven. These KZG checks are data-
structure experiments, not a financial authorization or range-proof implementation.

## 3. Anchor lifetime and state-witness availability are different

A standard SHA-256 append-only Merkle archive is implemented with cached nodes,
O(log n) append work and O(log n) public membership refresh. It authenticates an
old anchor through 100,000 entries. There is no fixed four-root cutoff in this
archive. Its checkpoint must come from independently validated/finalized history;
a caller's self-supplied checkpoint is not authority. Global conflict/spent markers
must remain available. Archive membership alone never licenses root replacement.

The event simulator uses actual Milestone 2 proof/verification samples, independent
remote prover latencies, a serialized verifier queue and external background roots.
At one background root per second, a four-root window expires before a roughly
16-second proof arrives. Larger windows or archived anchors improve that admission
model; they **do not** fix either concurrency counterexample. It is a simulation,
not measured distributed liveness or a native parallel-prover throughput claim.
The separate demo does measure two actual concurrent proof-generation threads.

## 4. Offline receiving and recovery are not solved

The hidden-write candidate authorizes only owner-controlled, value-preserving
updates. It cannot directly credit an offline Bob: Alice lacks Bob's opening and
spending authority, and her attempted Bob update fails. This is a concrete missing
property of the candidate, not a proof that all offline account crediting is
impossible. No inbox/note fallback or unproven cross-account credit relation was
added to conceal the gap. Homomorphic account encryption, as in prior art, needs
a different complete authorization and private-access composition.

An old authenticated age backup restores the old opening correctly. After a
valid rotation, the new root/proof/marker transcript supplies no new random nonce,
so that opening is no longer a member. Under the commitment-preimage and ZK
assumptions, replaying the authenticated history does
not reconstruct it. An updated encrypted private backup restores the new opening;
corruption and wrong backup keys fail. This distinguishes missing recovery data
from a cipher failure. No online data-availability, witness-refresh or durable
wallet-journal protocol has been established. Interrupted replay rolls back its
cursor and rejects a missing/altered suffix against a pinned final head.

## 5. What is, and is not, decided

The current YPP compositions have **not** achieved private persistent vaults under
all requested properties. The tested compositions are no-go; production remains
inactive. Native performance and longer anchor retention cannot repair their
semantic/privacy failures. No universal impossibility, secure construction,
novelty, low-end validator capacity or production parameter claim is established.

Established research gives reasons not to generalize these failures:

- [KZG polynomial commitments](https://cacr.uwaterloo.ca/techreports/2010/cacr2010-10.pdf)
  provide the linear commitment/opening operations actually exercised here.
- [Vector Commitments with Efficient Updates](https://eprint.iacr.org/2023/1830)
  studies update-information/runtime trade-offs. Efficient refresh alone does
  not establish update-location privacy for this composition.
- [Path ORAM](https://pages.cs.wisc.edu/~yxy/pubs/PathORam.pdf) hides access patterns
  with client position-map/stash state. Treating one global ORAM client as a
  secret-bearing coordinator would violate this task's model; distributed/client
  variants are not implemented or ruled out by the counterexamples.
- [Updatable Zero-Knowledge Sets](https://dspace.mit.edu/entities/publication/b72a2dc4-9181-4bef-b0b2-f026269607eb)
  explicitly studies hidden updates and transparent witness updates by a set
  owner. Its existence precludes an inference that these properties are inherently
  incompatible. Independent untrusted writers, stale-proof composition, monetary
  invariants and offline receiver availability need a separate complete design.
- [Zether](https://eprint.iacr.org/2019/191) is account-based encrypted-balance prior
  art. Renaming it or a shielded-record system does not establish YPP originality.

A further feasibility proposal must specify its authenticated write algebra,
private witness-refresh/delivery protocol, conflict rule, multi-writer trust model
and recovery data before another payment implementation is justified. Dense
verifiably encrypted refresh, reviewed updatable zero-knowledge structures or
client-side distributed computation remain unimplemented research possibilities,
not approved solutions. We do not put secrets in validators or delegate the
unresolved problem to a trusted central server.
