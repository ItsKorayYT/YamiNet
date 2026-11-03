# Milestone 7 — isolated validator election and discovery lab

**No live PoS, real backing, public validator network, funds, rewards or issuance.**
Base: `096c63ae399db9984f069ca71b997e2ab3fce02e`, existing
`development/yamichain`. This independent Cargo workspace consumes the existing
Malachite runner. No production application depends on this lab. The production
`ElectionsDisabled` selector and all 65,536 payment/action proof rejections remain
unchanged. Previous YPP experiments are untouched.

The implemented subset is signed candidate discovery infrastructure plus a
deterministic **simulated election compiler**. It derives arbitrary selected
committees rather than manually enumerating their members. It compiles them into
the existing externally pinned genesis schedule before the chain starts. It does
**not** implement live consensus-driven elections or authorization of epoch
transitions from real staking state. That distinction is a deployment gate.

## Operational non-financial library

`yami_chain::validators` implements canonical signed announcements, registration,
deregistration and candidate-directory paging/snapshot restoration. These are
off-chain discovery hints, not on-chain actions or voting authority. Each record
binds network scope, authenticated epoch, monotonically increasing epoch-local
sequence, consensus key, transport key, numeric IPv4 endpoint and active status.
Both keys sign the entire canonical record with existing strict Ed25519
verification. Announcements contain **no weight field**.

The directory:

* Requires the caller's current authenticated epoch/scope; no peer announcement
  can advance them. Old-epoch records fail. Exact retransmissions are idempotent;
  conflicting or lower sequence numbers fail.
* Retains deregistration tombstones until the trusted epoch changes, including in
  snapshots, so restart cannot resurrect an older registration under the pinned
  snapshot. Snapshot hashes require an independently trusted freshness source.
  A digest supplied with an untrusted download establishes no such freshness.
* Rejects key aliases across candidates and consensus/transport roles. This proves
  key possession, **not** one person, ownership of an IP address or unique stake.
* Bounds the cache to 128 total records including tombstones, pages to 16, records
  to 320 bytes and snapshot decoding to 64 KiB. The public policy limits active
  entries per IPv4 address to four and excludes private, loopback, reserved,
  documentation, multicast and unspecified ranges. It performs no DNS lookup.
  These are conservative resource/address rules, not economic Sybil resistance.
* Rejects the runner's known public development consensus/transport keys in
  public-address mode. This is defense in depth, not a general test for weak or
  publicly known secrets. A real public service must provision fresh private keys.

There is no new public listener or automatic candidate-to-validator admission.
The explicit loopback policy is for hermetic testing. Quotas can be exhausted by
an attacker controlling enough keys/addresses, including NAT-related availability
tradeoffs. A peer may omit or withhold candidates. None of these hints changes
the authenticated election roster. Public deployment still needs authenticated
transport challenges, seed diversity, connection/CPU budgeting and an admission/
retention policy that protects honest discovery under sustained Sybil floods.

## Authenticated simulated eligibility and randomness

The `fixture` command creates fresh ephemeral Ed25519 signing authorities for
simulated eligibility and entropy, using OS randomness. Their **private keys are
not serialized or retained**. The plan contains public verification keys, signed
allocation certificates, signed candidate announcements, revealed entropy and
binding commitments. The two authority keys belong to one fixture generator;
they are not independent operators or a distributed beacon.

An allocation binds a unique nonzero simulated backing ID, a development validator
index and both derived public keys, positive u64 weight and eligible epoch
interval. Duplicate IDs/indices, modified weights, key substitution, zero weights,
invalid signatures and arithmetic overflow fail. A valid self-registration with
no allocation cannot enter the committee pool. These signatures authenticate a
test issuer's assertions; they do not prove deposits, delegation conservation,
unbonding, slashing or economic scarcity.

For each epoch, the compiler checks the complete frozen set of signed eligible
registrations, computes a canonical ID-ordered input root, checks activation
height/epoch ordering and the preceding election transcript, verifies an entropy
commitment and authority signature, and derives a domain-separated sampling seed.
All operations are deterministic integers. Input arrays can be reordered without
changing the resulting genesis.

Sampling is weighted without replacement over the unique authorized backing IDs.
For total weight `W`, a 64-bit hash-derived draw is accepted only when
`draw >= (2^64 mod W)` and then reduced modulo `W`. This removes modulo reduction
bias under uniform draws. Hash inputs include the authenticated seed and a checked
counter; the rejection loop is capped and fails closed, without a biased fallback.
Weight sums are checked and bounded by `u64::MAX/3`. Zero or excessive committee
sizes and insufficient candidates halt compilation; there is no quorum reduction
or fallback roster.

Each selected seat has unit BFT voting power. Input weights affect selection,
not an additional multiplication of voting power. This is an explicit simulation
policy, not an approved economic PoS design. Resistance to stake splitting,
delegation concentration and adaptive corruption needs a reviewed live policy.

The generator freezes an epoch's registrations before drawing its fresh entropy.
Nevertheless, signatures and hash commitments do **not** prove public chronology,
unbiasability or availability. A malicious fixture authority can grind, issue
false backing or withhold a plan before the external pin is chosen. Future
production needs a finalized eligibility cutoff and an independently reviewed
unpredictable/unbiasable randomness protocol; this lab supplies neither.

## Transition and restart safety

The default fixture has eight potential candidates, four selected seats and three
epochs at heights 1, 5 and 9. Two candidates become eligible later. A selected
first-epoch member signs a future deregistration, guaranteeing a real membership
change at height 5. Offline status and discovery responses never alter selection.

The complete plan must verify before constructing a genesis or signing anything.
A shortage in a future planned epoch rejects the entire plan at startup. There
is no live starvation/fallback policy. The `parent` field links election
transcripts, **not** finalized chain checkpoints. Integrating live activation
requires an on-chain authenticated transition relation and explicit finality and
availability rules; it is not smuggled into the existing block protocol.

The plan's full-file digest is pinned independently for each process and stored
immutably as `simulation-plan.bin`. The existing genesis binds the entire
selected committee schedule; the separate plan pin also distinguishes different
input histories yielding equivalent committee schedules. Changing a plan on
restart fails before listeners or signing. Retain the public plan and pin as well
as the full node directory; ephemeral fixture authorities cannot regenerate lost
plans. No secret authority material is needed to replay a retained plan.

Cached discovery snapshots are checked against the pinned plan before networking.
The executable then fetches signed catalogs from other processes and requires the
fixture's expected catalog commitment. This completeness check works because this
**closed lab already has the frozen catalog**. Generic public discovery cannot
infer completeness from self-signed records. Discovery gates lab startup but the
Malachite transport still uses its existing pinned loopback peer topology; it is
not dynamic Internet routing or permissionless admission.
The command's catalog epoch selects a pre-pinned fixture catalog; it is not
automatically advanced from live finalized height. Production integration must
derive freshness from authenticated consensus state, never a peer's claimed epoch.

All consensus keys remain public development fixtures, and every listener binds
explicitly to `127.0.0.1`. No public address, key import or deployment option is
offered. Catalog service has eight in-flight handlers, one-second request
deadlines, 64 accepted requests/second, bounded pages and an 8192-byte response
cap checked before allocation. Catalog traffic carries no consensus values.

The existing Malachite certificate verification, >2/3 quorum, height-specific
membership, signing reservations, WAL, durable receipts and crash recovery are
unchanged. Real processes use the computed genesis through the existing generic
runner API, including selected non-prefix committees and retired observers.

## Build and test

From the repository root (Rust 1.90):

```sh
CARGO_TARGET_DIR=target cargo fmt --manifest-path experiments/yamichain-validator-lab/Cargo.toml -- --check
CARGO_TARGET_DIR=target cargo clippy --manifest-path experiments/yamichain-validator-lab/Cargo.toml --all-targets --locked -- -D warnings
CARGO_TARGET_DIR=target cargo test --manifest-path experiments/yamichain-validator-lab/Cargo.toml --release --locked
CARGO_TARGET_DIR=target cargo build --manifest-path experiments/yamichain-validator-lab/Cargo.toml --release --locked
python3 tools/yamichain-election-lab.py --directory work/election-normal --report work/election-normal.json
python3 tools/yamichain-election-lab.py --directory work/election-faults --report work/election-faults.json --adversarial
```

Campaign directories must be fresh. The driver launches every scheduled identity
under Linux `prlimit --as=2147483648`, including restart and malformed-response
probe processes. This limits virtual address space, not a per-node cgroup/RSS
budget. It checks authenticated discovery for every node, consensus roots at every
reported height, independent final replay and financial-disable flags.

The adversarial campaign splits the active first committee 2+2 across encrypted
TCP proxy banks, checks no quorum progress, heals it, SIGKILLs the retiring member
and restarts its entire store. It also checks oversized and forged discovery
replies, 16 idle clients followed by service recovery, a conflicting valid plan on
restart and a corrupt discovery snapshot. Invalid restarts must leave store bytes
unchanged. The observer compares all finalized roots; successful exit alone is
not the acceptance criterion.

Unit tests additionally cover registration replay/epoch/tombstone recovery,
key aliases, per-address/cache/page limits, public endpoint policy, forged
allocation weights/authority signatures, randomness and frozen-root tampering,
conflicting epochs/parents, permutation independence, unauthenticated Sybil
registrations, integer bounds and authenticated candidate shortages. A non-test
production-boundary integration test exhausts all proof versions for payments and
actions and confirms the production election selector remains disabled.

Measurements and workspace regression results are recorded in
[`docs/specs/yamichain/milestone-7-validator-decentralization.md`](../../docs/specs/yamichain/milestone-7-validator-decentralization.md).
