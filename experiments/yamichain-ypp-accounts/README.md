# Private accounts: executable research, no production backend

This crate implements persistent ElGamal balances and a real Groth16 proof of an
Anonymous Zether-style transfer statement. Every selected account receives a
homomorphic ciphertext update. The recipient does not participate in proving or
application. The ledger contains ciphertexts, public keys and debit tags, never
plaintext balances. Tests use plaintext witnesses to initialize a **trusted test
genesis** and to construct proofs; the verifier never uses those witnesses.

This is a partial reproduction of the established **account transfer semantics**,
not a port of Firn's many-out-of-many/inner-product proof. We use arkworks Groth16
on BLS12-381, with Jubjub account encryption inside its scalar field. Setup is
local, per cohort size, and requires trusted destruction of setup randomness.
There is no ceremony, audit, or production security claim. Measurements must not
be attributed to Firn's proof system. No notes, ring signatures, enclaves,
custodians, production dependencies, or consensus registration are introduced.

## Run

Use Rust 1.90 or later. From this directory:

```sh
cargo test --release --locked
cargo build --release --locked
for n in 16 64 128; do
  target/release/yamichain-ypp-accounts "$n" > "benchmark-$n.json"
done
python3 adversaries.py
python3 paillier.py  # Python cryptography package required; native integer pow
```

The executable writes public proof, verifying key, statement and ciphertext-only
before/after snapshots to the system temporary directory `ypp-accounts-N` (normally
`/tmp/ypp-accounts-N`). No private keys or proving setup trapdoors are saved.
A separate process can verify and replay them without a wallet:

```sh
cargo run --release --locked --example public_verify -- /tmp/ypp-accounts-16
```

Recovery timing includes reading/deserializing the after-state snapshot, root
verification and bounded decryption. Canonical serialization/deserialization and
subsequent receiving/spending are covered in the real-proof test. Benchmarks are
single observations, not distributions. Recovery uses a full **32-bit** search
bound even for small balances. RSS includes circuit synthesis, setup, proving,
verification and recovery; it is a process high-water mark, not per-stage memory.
Transaction bytes include the complete settled-state witness as well as keys,
updates, epoch, tag and proof; no network framing or ledger transport is counted.
Verifier timings include preparation of the public verifying key. Setup is
reported separately and never hidden in the proving time.

## Exact statement

For ordered public keys Y_i, settled ciphertexts (L_i,R_i), common D, updates C_i,
epoch e and tag U, prove knowledge of x,r,a,b and private one-hot sender and
recipient selectors s_i,t_i such that:

- Y_sender = x G, x != 0, r != 0.
- s and t each select exactly one distinct account.
- 0 < a < 2^32 and 0 <= b < 2^32.
- L_sender = x R_sender + (a+b) G.
- D = r G; C_i = r Y_i + (t_i-s_i) a G for **every** account.
- U = x H(domain,e), with a cofactor-cleared try-and-increment epoch base whose
  discrete logarithm to G is unknown. This is a research hash-to-curve, not an
  RFC 9380 ciphersuite. Using H(e)=hash_scalar(e)*G would expose Y_sender and is
  deliberately avoided.

The verifier obtains all public inputs from the statement and the epoch base
recomputed locally. `verify` requires only a public verifying key. `apply` also
checks the exact epoch, cohort, settled ciphertexts and absence of the tag.
Updates accumulate in pending state, leaving settled state unchanged throughout
an epoch. Two different senders' proofs against that same state can both commit;
two debits from the same key/epoch cannot. Rollover adds pending to settled and
clears tags. Received funds become spendable next epoch; at most one debit per
account per epoch. Applying/rejecting a transaction is atomic in this single
process; no network concurrency or consensus engine is supplied.

Initialization is a closed test genesis (1000 units/account). Schnorr possession
proofs, subgroup/identity checks and duplicate-key rejection protect registration.
Registration does **not** prove initial encrypted funding. There is no public
mint/deposit API. Conservation is inductive from that trusted genesis. A global
supply bound is necessary to guarantee 32-bit recovery after arbitrarily many
credits: these fixtures have at most 128,000 units. There is no claim that a
recipient's final balance is range-proved on every credit, or that arbitrary
untrusted genesis ciphertexts are valid funding.

## Set selection and attacks

A proof hides positions and values **within a public cohort**; it cannot hide
cohort membership. This experiment fixes the entire cohort before transactions
and requires exactly that cohort for every transfer. Thus set membership and
ordering do not depend on the actual sender/recipient **within that cohort**.
Both participants must already be in it. Cross-cohort transfers and privacy
between cohorts are unsupported. The fixed research domain is not a production
chain identifier; reusing the same secret in separate instances links their
epoch tags. Independent instance keys are required for this experiment. This is a deliberately constrained experimental
selection policy, not a claim to have solved global private account selection.

`adversaries.py` demonstrates intersections when an adversary knows that repeated
transactions have the same pair (e.g. from external timing/application metadata).
The population is 4096; select the true pair plus fresh random decoys. With the
fixed seed, 16/64 sets intersect to the pair after two observations; 128 after
three. The same fixed cohort remains N throughout, but timestamps, amounts known
to counterparties and activity patterns can still provide outside evidence.
The script measures set leakage only; cryptographic tests separately decrypt
actual observer-controlled update ciphertexts for 16,64,128 accounts and verify
that every controlled decoy's change is zero. An observer owning N-2 accounts
reduces the candidates to the remaining two. If it knows one participant or the
transaction direction externally, the other is exposed. Possession proofs reject
forged/identity/duplicate registrations; they cannot reject independently valid
Sybil keys. There is no Sybil defense in this experiment.

## Offline recovery and malicious validators

`recover` accepts a separately authenticated finalized SHA-256 state root,
recomputes it over canonical public state, and decrypts only with the owner's
key. Changed/replayed state fails against the expected root. No transaction scan
or online recipient interaction is needed. The prototype downloads the **whole
cohort state**, not a constant-size authenticated account path. The test's root
is a local checkpoint. No production finality integration authenticates that
checkpoint: obtaining a fresh, authentic root in the presence of malicious
validators is an explicit integration blocker, not supplied by hashing alone.
A malicious RPC supplying both a forged state and its root defeats authentication
if the caller incorrectly trusts that root. A stale but genuine root also needs
an external freshness policy.

Under DDH for Jubjub and Groth16 soundness/zero knowledge with honest setup,
validators with only public transcripts learn the cohort, epoch, tags and timing,
not its hidden selectors/amount. They cannot make an overspend pass the honest
verifier. A validator owning accounts has the same exclusion and Sybil attacks as
any key owner. Validators can censor, delay, request distinguishing sets, observe
IP/query patterns or collude with counterparties. Fixed cohorts reject altered
sets but cannot fix censorship, registration concentration or network metadata.
Compromised setup defeats proof soundness. A consensus majority can also lie
about finality. **Meaningful privacy against arbitrary malicious validators is
not established.** Conditional cryptographic privacy is much narrower than that
claim. No production change is justified by this experiment.

## Full 64-bit recovery / Paillier blocker

Message-in-exponent ElGamal needs a discrete log. BSGS for a 32-bit range needs
roughly 2^16 baby entries and giant steps; for a full 64-bit range it needs roughly
2^32 each (over 128 GiB of raw compressed baby points alone, excluding table
metadata). Our working prototype is 32-bit and makes no full-u64 recovery claim.

`paillier.py` uses real 2048-bit Paillier arithmetic with g=n+1, fresh random units,
local RSA-prime generation, and decrypts u64::MAX exactly. It tests homomorphic
addition and records actual arithmetic timings and 512-byte ciphertexts. Python
`pow` is not constant-time. The 2048-bit modulus is an arithmetic experiment,
not a 128-bit security target; 3072-bit parameters would enlarge ciphertexts to
768 bytes. Paillier key generation reuses RSA prime generation only; its encryption
and decryption formulas are implemented directly.

It has **no transfer proof**. In different per-owner Paillier groups, Firn's shared
D and prime-order linear equations do not carry over. A valid substitution needs
well-formed modulus/secret-key knowledge, ciphertext/randomness membership,
range/negative encoding, hidden sender ownership, and equality/conservation across
different moduli, all bound to the same hidden selectors. Giving everyone one
shared decryptable key would violate owner-only balance privacy. Generic
non-native modular exponentiation in R1CS is not a measured or justified port of
the specialized protocol. Proof bytes/time are intentionally `null`, not zero.
We therefore **do not adopt Paillier** or claim its transfer proof cost is known.

## Source investigation

Inspected 2026-11-05:

- https://docs.firn.io/overview/cryptographic-explainer (account ciphertexts and
  offline state retrieval; its lightweight-client claims are not our benchmarks).
- https://github.com/firnprotocol/contracts at
  `fa6dd7dde6e782008a6048e3ef725f175706731e`:
  `src/Firn.sol` transfer/rollOver/register, `src/TransferVerifier.sol`,
  `src/Utils.sol`. Confirms pending updates, settled-balance statement, epoch
  nonce, and registration possession. Its current balance/epoch tree machinery
  and custom proof were not ported.
- https://github.com/Consensys-inc-archive/anonymous-zether at
  `be12ba6336607b26f235730a0c792a00d79625b5`:
  `packages/protocol/contracts/ZSC.sol` and
  `packages/anonymous.js/src/client.js`. Confirms sender/recipient plus supplied
  decoys, public key shuffle, shared update randomness, pending rollover and
  nonce-set semantics. Shuffle itself does not guarantee safe selection.
- https://eprint.iacr.org/2020/293: Diamond, Many-out-of-Many Proofs and Applications
  to Anonymous Zether (2020; IEEE S&P 2021). The abstract establishes logarithmic
  proof growth. We inspected the landing page and upstream implementation, not
  a full formal derivation of its many-out-of-many protocol.
- https://firn.cash/whitepaper.pdf was inaccessible through the research tool;
  the documentation's https://firn.io/whitepaper.pdf also timed out. Neither is
  represented as reviewed in full. This limits any claim of exact reproduction.

Essential unfinished work is precise: a reviewed port of the specialized
many-out-of-many proof, production-grade setup/hash-to-curve decisions, authentic
finality integration, and a justified full-u64 encrypted account/proof system.
The implemented account statement and tests function without pretending these
blockers are solved.
