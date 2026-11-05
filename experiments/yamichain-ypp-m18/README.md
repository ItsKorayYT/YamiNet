# Milestone 18 — sparse cross-cohort candidate REJECTED

Base: `5b9f43b`, branch `development/yamichain`. No accepted payment architecture,
new financial backend, or new proof protocol is delivered. This directory contains
an exact rejected candidate, an access-support bound, reproducible attacks,
standard SHA-256 authentication primitives, and test-only real cryptographic
replays. Milestone 16 and all earlier experiments remain unchanged.

**Verdict:** the existing directly indexed account-update construction cannot
provide the requested global sender/recipient anonymity with sparse public
updates. Correct cross-cohort accounting and small authentication paths do not
repair the exposed write support. Neither increasing cohort size nor replacing
Groth16 is accepted as a fix. This result is scoped to this state-access model,
not an impossibility theorem for every possible cryptographic payment system.

## One candidate, specified completely enough to reject

Parameters: registry of N immutable, distinct, prime-subgroup keys Y_i=x_i G;
cohort width c; exactly q selected cohorts, q>=2. C=ceil(N/c). The scalability
experiment uses N=500,000,c=64,q=4. The final cohort has 32 real accounts; it is
never padded with invented participants. A selected set has m<=qc accounts.
The registry is frozen before each experiment's challenge history. Adversaries
may supply arbitrary valid registrations before that freeze; live admission,
registry resizing and transfers overlapping admission are not implemented or
claimed. This restriction does not explain away the failure: attacks need no
new registrations after genesis. Cohorts share **one consensus/finality domain**;
these are not independent shards.
There are no bridge accounts, relayers holding funds, notes or custodians.

Registration uses the base experiment's Schnorr proof of possession, bound to its
fixed research registration domain. Reject identity, off-curve, non-subgroup and
duplicate keys and invalid possession proofs. The closed experimental genesis
has a known bounded supply and valid encrypted balances; permissionless minting,
unproven encrypted deposits and supply creation are not introduced. Funding and
account discovery metadata remain public leakage. Possession proves knowledge of
a scalar, not independence of its owner. A deployable protocol would also require
chain/deployment separation; the inherited research domains are not deployable
cross-chain replay protection.

Stored record i: (Y_i,A_i,P_i,e_i), with settled and pending ElGamal pairs
A=(L,R)=(bG+rY,rG), and last materialization epoch e_i. Record serialization is
168 bytes on this curve: 32+64+64+8. State includes a global epoch e, a **global**
set U_e of used debit tags, an authenticated account table, and the fixed key
registry. The registry is bound through immutable keys in the record leaves.
Account leaves commit the global index and domain-separated record hash; internal
nodes use domain-separated SHA-256 with length-delimited encodings. The root binds
N. The public head commits e, the account root, and a canonical sorted tag-set
digest. Authenticating/freshening this head is a consensus/finality requirement,
not supplied by the hash function or by an RPC returning its own root.

### Selection

For real s!=t, include cohort floor(s/c) and floor(t/c). Uniformly sample enough
other cohorts without replacement to reach exactly q. Expand all real members of
those cohorts, sorted by global index. Cover coins are fresh local CSPRNG coins;
no validator-chosen beacon or identity oracle is required. `select` implements
this distribution using sampling ranks outside the endpoint cohorts, without a
registry-sized shuffle. Test seeds make experiments reproducible, not production
entropy. Same-cohort transactions sample q-1 covers; different-cohort ones q-2.
The public shape alone need not distinguish those two cases, but membership does.

Validators can check that the published indices form q complete cohorts. They
cannot check uniformity or unpredictability of the sender's private sampling
coins using the inherited proof. Even if an ideal oracle enforces honest uniform
selection, the privacy attack below succeeds. Proving fair sampling would not
remove the mandatory endpoint inclusion. Sender entropy grinding and adversarial
registration placement only make selection less trustworthy.

### Authorization and exact ciphertext transition

Read the logical epoch-start settled ciphertext without mutating storage:

```
View_e(i) = A_i + P_i    if e_i < e
            A_i          if e_i = e
```

For selected S, a sender proves knowledge of canonical nonzero x,r, distinct
one-hot private selectors s_i,t_i, a positive amount a and remaining balance b:

```
Y_sender = x G
0 < a < 2^32; 0 <= b < 2^32
View_e(sender).L = x View_e(sender).R + (a+b) G
D = r G
C_i = r Y_i + (t_i-s_i) a G       for EVERY i in S
u = x H_epoch(e)
```

H_epoch is the base prototype's cofactor-cleared research hash-to-curve with
unknown discrete log to G, not hash_scalar(e)*G. The tag depends on the global
epoch, never the cohort or selected set. Otherwise a sender could debit the same
settled balance using different sets. Aggregate plaintext change is exactly zero;
all other accounts receive re-encryptions of zero. Conservation is inductive from
the valid bounded genesis; recipient balance ranges are not proved per credit.

This is the **existing** complete Groth16 relation in `yamichain-ypp-accounts`.
It is used unchanged for accounting falsification tests, with its local trusted
setup explicitly retained. It is not our zero-trust solution. No new unproved
many-out-of-many, membership, Paillier or private-access proof is substituted.

Preflight a transaction (e,S,keys,View_e(S),C,D,u,proof): validate canonical unique
indices/whole cohorts and registered keys, exact epoch and logical settled
ciphertexts; reject u already in U_e; run the real proof verifier on exactly these
public inputs. Native lookup binds global indices to immutable keys and state.
It is not a hidden Merkle lookup inside the proof. All checks precede mutation.
For each selected i, stage:

```
if e_i < e:
    A_i := A_i + P_i; P_i := (0,0); e_i := e
P_i := P_i + (C_i,D)
```

Commit all staged records and u to the one global transition, updating only their
Merkle paths. Failure commits neither debit nor credit nor tag. A transition
crossing cohorts is a **single** proof and commit, never separate debit and credit
proofs whose amounts might differ. A crash-safe database batch and production
consensus implementation are not supplied. The test harness checks in-memory
atomic rejection and both application orders.

### Concurrency and offline recovery

Same-epoch proofs bind View_e, which excludes same-epoch pending updates. Different
senders can therefore concurrently credit an offline recipient and then serialize
in either order. Pending ciphertext sums commute. A second proof from the same
sender/epoch has the same u, even with different support, and is rejected globally.
At most one debit per key per epoch is possible. Credits become spendable next
epoch. Advance e and replace the empty current tag set without scanning accounts;
materialize each record only when next selected. Deleting all old tags can be
pruned asynchronously: logical epoch advancement is O(1); tag reclamation is
O(number of tags), not magically free. Old-epoch proofs are rejected.

An offline owner obtains its current raw record, global index, Merkle path and
public head components. Recompute the head against a separately authenticated,
fresh finalized checkpoint, verify membership and ownership Y_i=x_i G, and decode
(A_i.L+P_i.L)-x_i(A_i.R+P_i.R). Lazy epoch flags do not affect total owned balance.
Receiving uses no recipient secret or recipient interaction. The test recipient
receives two credits, recovers from an authenticated record and later proves a
debit of those credits. Tampered record/path/epoch fails against the trusted
checkpoint; an old authentic checkpoint still needs an external freshness policy.
A malicious server supplying both an invented record/root and its own checkpoint
is not authenticated. The public lookup index also leaks which account a wallet
is reading; authenticated lookup does not mean private lookup.

## Realistic security games and the failure

Define global participant indistinguishability G_global under ideal anonymous
transport and authentic, available checkpoints first, to avoid blaming the result
on IP leakage or broken consensus. The adversary sees the registry, all public
transcripts and state access indices, acts as validators, schedules transactions
and epochs, supplies arbitrary keys with valid possession proofs before the
registry freeze, corrupts non-challenge owners adaptively, and obtains their secret keys and decrypted views. It can
adapt its actions to earlier transcripts. For a challenge it supplies two
feasible honest-account histories with the same initial state, timings, amounts,
fees, transaction counts and size bounds. Plaintext changes and disclosed values
at every corrupt account must be identical in both worlds. Challenge endpoints
remain uncorrupted through the experiment; otherwise exposure is trivial.
The challenger chooses a hidden bit and executes that history with independent
coins and honest verification. The adversary guesses the bit. Required advantage
|Pr[correct]-1/2| is negligible, including after polynomially many epochs.
Single-transfer challenges with different honest endpoints are included; the
public selected set is an observation, **not** a condition required to coincide.
Aborts/order and lookup leakage are part of the view; security cannot assume a
malicious validator never uses them. The measured attack works with no censoring,
no corrupt accounts and no cryptographic break.

A weaker G_local conditions both histories on the same public support, equivalent
public leakage and the same corrupt plaintext views. The base encrypted relation
is intended to hide positions/amounts inside that set, under DDH, correct subgroup
checks and honest Groth16 setup. No new formal security proof of the composed
sparse harness is claimed. Passing G_local cannot establish G_global. Known
counterparties know their own transfer amount and peer. All-corrupt accounts,
endpoint key exposure, network-origin observation, distinguishing wallet queries,
and distinguishable application behavior cannot be made secret by this relation.
Honest finality/availability assumptions remain necessary: a wholly malicious
validator set can censor or forge its version of finality. Zero trust is not a
promise of liveness without any consensus assumptions. Even the idealized weaker
network/checkpoint environment already fails G_global.

### Public-support bound (specific to directly addressed account updates)

Assume each positive, distinct-endpoint transfer immediately publishes support W
of at most k account slots and only these slots can change. Correctness requires
both endpoints in W. Fix a reference pair (a,b). Among its N-2 other accounts,
one j has inclusion probability p_j <= (k-2)/(N-2), because the sum of their
inclusion probabilities is at most k-2. Challenge (a,b) against (j,b), with the
same feasible amount. Guess the second world exactly when j is written. Success
is 1-p_j/2, hence at least 1-(k-2)/(2(N-2)). This does not depend on encryption,
proof size, sampling secrecy, or corruption. For indistinguishability advantage
at most epsilon, support must be at least N-2*epsilon*(N-2) in this bound.
Negligible epsilon therefore requires essentially dense support in this model.
`attacks.py` also checks exact support-family intersections on a small universe:
only the full support can be valid with identical distributions for every pair.

The bound is NOT a theorem against hidden-address RAM, alternative encodings,
private deferred logs, or every form of batching. But invoking their names does
not supply a private-account algorithm. Batching still admits the low-activity
challenge with one real transfer, unless it adds a substantive access-oblivious
mechanism or changes the stated liveness/recovery requirements.

For the concrete cohort candidate, C=7813,q=4 and two distinct real cohorts, a
chosen other real cohort appears only as cover with probability (q-2)/(C-2).
The disjoint-pair support classifier therefore succeeds with probability
1-(q-2)/(2(C-2)) = 0.9998719754. See measured trials in `attacks.json`.
Repeated linked pair transfers intersect to the two endpoint cohorts. A fixed
sender with changing recipients localizes the sender cohort, and vice versa.
Refreshing the public partition with ideal unpredictable coins can instead
intersect down to the actual two accounts. Linkage is explicitly assumed/given
by those challenge histories; arbitrary unrelated transactions must not simply
be intersected. No automatic real-world linkage classifier is claimed.

### Malicious registrations and colluding owners

With shared D, an unregistered rogue key Y*=alpha Y and zero update C*=rY* lets
an attacker compute C-alpha^-1 C*=delta G for the target. This is reproduced with
real curve operations and a recovered amount. Subgroup checks alone do not stop
it. Registration possession proofs reject that forged registration without the
unknown secret, under Schnorr's knowledge assumptions and discrete-log hardness.

They do not stop a single party from creating 128 valid independent keys; that
experiment succeeds. A colluding owner computes C_i-x_i D for each controlled
slot, recognizes zero, and removes it from the candidates. With all but two
accounts of a support controlled, the honest candidate pair is exposed. A
counterparty or outside direction evidence can resolve roles further. No
permissionless cryptographic test distinguishes many owners from one owner who
knows many independent secrets. Fees, stake, age or work could impose explicit
resource assumptions, but none provides an unconditional honest fraction, and
none is silently added here as a Sybil solution.

## Scaling accounting toward 500,000 slots does not scale anonymity

Selection is O(q) auxiliary memory and O(q log q + m) expansion for fixed small q
(using rand's index sampler). Account lookup and encrypted updates require O(m)
public account data and curve work, independent of N, plus proof verification at
its own measured cost. Authenticated account updates here use O(m log N) hashes;
no optimized multiproof is claimed. The test harness uses a vector of T current
tags: duplicate checks cost O(T), and hashing a sorted tag set into the head costs
O(T log T) sorting plus O(T) hashing. These overheads are not included in the
isolated sparse-tree benchmark. No authenticated O(log T) tag-set implementation
or high-throughput payment pipeline is claimed. Account storage is
O(N); a padded binary tree uses 2*next_power_of_two(N) 32-byte nodes. Initialization
is O(N), epoch advance touches no account records, and a record path is O(log N).
Global tag storage is O(accepted senders per epoch); no per-cohort tag reset.

At 500,000 slots this tree uses 33,554,432 bytes. The serialized ciphertext record
table would require another 84,000,000 bytes before database overhead. A 19-sibling
path is 608 bytes, plus record/index/header transport and an authenticated head.
For full cohorts m=256, the inherited unoptimized statement/proof encoding plus
u64 indices would be 35,104 bytes (128m+288+8m); this is an **encoding formula**,
not a measured or transparently proved 500k transaction. The last partial cohort
can reduce m to 224. Firn's power-of-two/parity restrictions would need separate
handling; fake accounts cannot be counted as honest anonymity.

`scale.json` measures actual SHA-256 construction of 500,000 synthetic record
slots, 256 incremental updates, path verification and selection. These slots are
NOT 500,000 registered curve accounts; no full-registry key generation, transfer
proof, network workload or production database throughput is reported. Real
ciphertext/proof tests instead use 64 registered accounts and 32 selected ones.
Keeping that distinction explicit prevents a sparse Merkle benchmark from being
misrepresented as scalable private payments.

## Firn / Anonymous Zether proof assessment — do not port now

We inspected the repository copy of Diamond's *Many-out-of-Many Proofs and
Applications to Anonymous Zether*, especially sections 4, 5, 6.2, 6.3, 6.7, 6.8,
and the upstream verifier. Its general many-out-of-many commitments are not by
themselves a payment protocol. They combine hidden selections, re-encrypted
balances, range checking, equal randomness, zero changes for every other account,
and ownership. Prime-order groups, binding/hiding commitments with independent
transparent generators, DL/DDH assumptions and Fiat–Shamir/random-oracle reasoning
allow a construction without Groth16's secret setup. Transparent parameter
derivation must still avoid known generator relations and validate subgroups.
Logarithmic proof size does not eliminate the public O(m) account/update list or
make its support private. Public-input processing is not O(log m) merely because
the proof is small; the paper's computation is quasilinear in the list size.

Important restrictions prevent a blind adaptation:

- Section 6.7's ledger-indistinguishability challenge uses a **single fixed public
  set R*** containing both possible histories, and consistent corrupt plaintext
  views. It does not prove that constructing different sparse sets hides their
  participants, nor that possession proofs guarantee independent owners.
- Section 6.3 uses opposite-parity sender/recipient indices and special rotations.
  Our arbitrary-selector Groth16 relation does not have this restriction. A port
  needs a justified ordering/padding policy and the matching security game.
- Section 5 explicitly constructs a ring signature; section 6.3 reuses those
  re-encrypted authorization techniques. We will not rename a prohibited ring
  signature to make it appear compliant. Generic many-out-of-many commitments
  can be assessed separately from a standalone ring-signature API, but extracting
  them with an independently justified ownership/range/conservation composition
  under the no-ring restriction has NOT been completed here.
- The paper explicitly notes that its full interactive payment protocol is not
  zero knowledge; it proves noninteractive ledger indistinguishability under
  DDH. That privacy theorem cannot silently be replaced by a generic ZK claim.
- The inherited hash-to-curve, domain separation, transcript binding, malicious
  key registration, range bounds and late/duplicate epochs all need review for
  any different group or proof composition. No such proof implementation or
  performance claim is supplied in this milestone.

Thus setup-free proof machinery is relevant to a future **justified statement**,
but not a solution to this candidate's architectural failure. No port is started.

## Full 64-bit recovery: exact remaining operations

The current transfer statement and bounded recovery remain 32-bit. A full 64-bit
message in one ElGamal exponent requires about 2^32 baby entries and giant steps
for BSGS, over 128 GiB for compressed point payloads alone. Lower-bounded guesses
or cached balances are not full-range offline recovery.

The crypto test independently encrypts/reconstructs u64::MAX as four 16-bit
ElGamal limbs. This does NOT produce a 64-bit account-transfer relation. A recipient
with low limb 65535, upper limbs zero, receiving 1 needs an encrypted carry to the
next limb. Pure homomorphic addition instead leaves 65536 in the low limb, outside
its 16-bit recovery bound. Repeated credits make deferred limbs unbounded unless
a carry/normalization algorithm or another bound is proved. The sender does not
know the offline recipient's old plaintext; no secret-bearing worker may repair
it. The test demonstrates this blocker on a valid, non-overflowing total 65536.

Paillier could provide owner-only direct integer decryption, but the prior
standalone arithmetic benchmark is not a transfer proof. A future proof statement
would have to simultaneously establish well-formed per-owner moduli/keys and
ciphertexts, secret sender ownership, positive a and nonnegative remainder,
signed delta encodings, equality of the same integer a across different moduli,
zero change elsewhere, global no-wrap supply/range invariants and the epoch tag.
The private-write operation must also hide where those ciphertexts are updated.
For a per-owner modulus n_i, the hidden selectors must bind delta_i to -a, +a or 0
as integers with canonical encodings, not merely unrelated residues mod n_i.
A range/supply bound below all moduli and no-underflow reasoning are essential.
Using a shared decryption key would violate owner-only balances. Public verification
of an encrypted recipient carry or cross-modulus amount equality without revealing
the recipient's state has no implemented composed proof here. No Paillier proof
cost, no secret-free carry routine and no setup-free payment proof is claimed.

## Specific rejection and next operation required

Reject the candidate at **ApplyPublicSupport(S, encrypted updates)**: S must
contain the real endpoints and is observable. Selection distributions then fail
G_global before any proof is considered. The needed replacement is an actually
specified and proved private sparse state access/update operation: authenticate
and change the right owner-encrypted account without revealing its address,
serialize conflicting debits, accept concurrent credits, and let an offline owner
recover from fresh authenticated public state without a secret mapping service.
Neither the sparse Merkle path nor many-out-of-many proof above supplies that
operation. Hiding a root in a ZK proof does not by itself define the encrypted
record lookup, write, data-availability or receiver-recovery algorithms. An
anonymous transport assumption does not hide public account writes. A query-only
PIR primitive would not solve private writes and may still scan O(N) server data.
We do not claim all possible approaches impossible; we report this operation as
unresolved and reject this candidate rather than committing it as a payment design.

## Reproduce

```sh
cargo test --release --locked
cargo clippy --release --all-targets --locked -- -D warnings
cargo run --release --locked > scale.json
M18_CRYPTO_REPORT="$PWD/crypto.json" cargo test --release --locked --test cross_cohort
python3 attacks.py
```

`tests/cross_cohort.rs` is a rejected-candidate accounting harness, not an exported
payment service. Its four Groth16 proofs reuse the existing research backend with
local trusted setup; the new library exports only selection and Merkle primitives.
`attacks.py` tests public metadata distributions, not simulated cryptographic
verification. Test seeds and single-run timings are not security assumptions or
benchmark distributions. See `validation.md` for actual results and source hashes.
