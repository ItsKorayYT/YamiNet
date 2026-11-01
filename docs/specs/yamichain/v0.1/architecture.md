# ADR YC-001–008: YamiChain 0.1 local foundation

Status: implementation proposals for review, 2026-11-01. Based on the supplied
v0.5 preparation package, particularly its first implementation prompt. The user's
implementation request authorizes this branch and draft PR; earlier planning-only
restrictions in the package describe that earlier phase.

## YC-001: identity, genesis and activation

Only `yamichain-local-0.1`, protocol 1, feature mask 0 is supported. Genesis has
four distinct, equal-power consensus identities and four separate transport keys,
derived from explicitly public role-labelled development seeds. Keys are never
YamiNet NodeId keys or spending identities. These keys offer no security outside
an isolated test. Genesis has zero issued/spendable value. The proposed billion
YAMI and inflation/treasury parameters are future gates, never allocations here.
There is no stake election or mutable membership at any height.

## YC-002: canonical encoding

All hashes use SHA-256 with explicit length-delimited domain and payload. Canonical
bytes use fixed-width big-endian unsigned integers, fixed-size keys/digests and
u32 length-prefixed byte strings. UTF-8 is checked; chain ID is exact ASCII;
labels are lowercase ASCII. Reject unknown discriminants, trailing bytes,
unsupported versions/features and lengths before allocation. Block cap 64 KiB,
64 transactions/block, 1 KiB/transaction, label cap 63 bytes. Versioned format is
only the non-financial prototype format, not a YPP proof/commitment/note format.

## YC-003: signed envelopes and execution

The only wire action is a test naming registration. Independent registry-owner
Ed25519 signatures bind chain ID, protocol, features, expiry height, owner nonce,
action tag and complete label. No fees, coins, allocations or public account
balances. Naming dispatch accepts only `test-` labels; `.yami` is immutable reserved
syntax, including attempts with leading dot/case/noncanonical labels. Sequential
nonces prevent replay. Stage on a clone; any failure leaves original state intact.
State roots commit genesis, height, sorted nonces and sorted test registrations.
Headers commit chain/version, height, parent, committee, transaction and state
roots. No wall clock, floating point or map iteration nondeterminism.

## YC-004: Malachite

Pin all direct Malachite packages to 0.8.2, Apache-2.0, declared MSRV 1.88,
upstream tag commit `a453b32155f4ba9f959724279b52f601bae92e09`.
Use the high-level channel engine with ProposalAndParts, genuine Ed25519 votes,
WAL, fixed membership and application validation before voting. ProposalOnly is
excluded: upstream's path currently marks received values valid without consulting
the application. Votes/proposals bind chain ID and committee, height, round,
step, value and signer. Certificates require three distinct valid precommits.
This is alpha/unaudited consensus, not production PoS or audited finality.

## YC-005: durability and recovery

A process holds an exclusive OS file lock for its entire store lifetime. Finalized
records are one file per height: write create-new temporary, fsync, rename to a
new immutable destination, fsync directory on Unix. On restart replay from pinned
genesis, verify every certificate and block, refuse gaps/corruption/conflicts.
A leftover temporary record is uncommitted and can be removed under the lock.
The signer independently persists the entire signing preimage before returning
any signature and refuses another message at that height/round/step. Keep signing
history, WAL and finalized records together; manual deletion/rollback of safety
records invalidates these guarantees. Filesystem snapshots and Byzantine disk
rollback are not trusted. Windows directory-fsync is unavailable via this safe
std-only implementation: power-loss durability on Windows remains unverified.

## YC-006: sync, checkpoints and pruning

Serve bounded certified history and validate synced blocks with the same executor.
Verify height, chain, committee and signatures from pinned genesis. No arbitrary
snapshot import; an interface requires an independently authenticated checkpoint.
Pruning is deliberately disabled until checkpoint trust, evidence retention and
future spent-tag retention have reviewed semantics. No public RPC or mempool.

## YC-007: YNS independence

No dependency from browser, hosted node, identity, DHT, games or YNS to these crates.
Test labels have no authority in YNS; existing `.yami` trust and self-certifying
identity names remain unchanged. Chain bootstrap never depends on YNS.

## YC-008: YPP rejection boundary

Mandatory shielded ordinary payments, with no transparent transfer alternative.
Intent structs only describe public binding context, opaque backend public inputs,
encrypted outputs and opaque proof. No spend/view keys, plaintext payer/payee/value,
notes, curves, spent-tag math or cryptographic backend is selected. Every production
verification path rejects all proofs and all value-bearing protocol actions.
No generic injectable accepting verifier in the executor, no runtime dummy switch.
Value-conservation and uniqueness models live only in hermetic tests.

Open gates: proof backend/parameters/audits, supply/private-public accounting,
PoS selection/slashing/evidence/unbonding, issuance, fees/burn, vested treasury,
auctions/registry-root validity/recovery, governance and audited upgrade rules.
