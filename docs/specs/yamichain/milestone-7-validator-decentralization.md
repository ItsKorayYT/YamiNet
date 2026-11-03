# Milestone 7 — validator decentralization foundations

Base: `096c63ae399db9984f069ca71b997e2ab3fce02e`, branch
`development/yamichain`. No block/certificate format, quorum threshold, signing
domain, financial action, production election selector or YPP experiment changes.

## What is operational and what remains simulated

| Component | Implemented scope |
|---|---|
| Signed registration/deregistration | Non-financial discovery announcements with consensus and transport key possession; epoch/sequence replay checks, tombstones and authenticated snapshot restoration |
| Candidate discovery | Bounded canonical library with public IPv4 policy, page/cache/address quotas; actual catalog exchange between loopback processes |
| Eligibility and weights | Fresh-authority signed **simulated** allocation certificates; unique backing IDs/key bindings and checked weights; no real stake authentication |
| Committee selection | Deterministic integer weighted sampling without replacement in an isolated lab; selected BFT seats have unit power |
| Epoch transitions | Elected schedules compiled into an ordinary externally pinned genesis, then exercised by real Malachite processes at heights 1, 5 and 9 |
| Randomness | Fresh OS-generated fixture entropy, binding commitments and signed frozen inputs; **not** an unbiasable/available public beacon |
| Public deployment | Disabled: the executable binds only loopback and accepts only public development validator identities; no public key/listen deployment path added |
| Permissionless economic PoS | **Not operational**; production `ElectionsDisabled` still rejects activation |

The previous infrastructure already supplied authenticated height-specific
committees, quorum certificates, crash-safe signing reservations, WAL, certified
sync and durable record receipts. This milestone reuses them instead of replacing
consensus. The independent lab has no production consumers or wire action. It
selects arbitrary member subsets from authenticated simulated inputs rather than
manually enumerating members, but it deliberately **does not** turn discovery
responses into membership or change committee selection in a running production
chain. Live activation remains a separate security gate.

## Protocol and safety details

See the [complete format, algorithm, threat boundaries and reproduction guide](../../../experiments/yamichain-validator-lab/README.md).

Discovery records bind scope, epoch, sequence, both public keys, numeric endpoint
and registration status. Both Ed25519 keys sign the entire canonical record.
There is no self-asserted stake field. A candidate must separately have an
authenticated simulated allocation to be eligible. Unallocated Sybil registrations
do not change the elected committee. Duplicate backing IDs, duplicate candidate
indices, forged weights, altered frozen roots and invalid signatures fail.

The public directory policy rejects private/reserved endpoints and the runner's
known development keys. Candidate counts, record lengths, paging, snapshots and
per-address active entries are capped. Tombstones retain sequence authority within
the caller's externally authenticated epoch. Full snapshots require an independent
trusted commitment; a peer cannot establish its own freshness. Directory hints
do not prove endpoint ownership, one person or economic scarcity. Many IPs/keys
can still exhaust the bounded cache; quotas are not a Sybil consensus mechanism.

The sampler uses canonical backing-ID order, checked u64 weight sums, a
domain-separated hash stream and rejection sampling before integer reduction.
It avoids modulo bias under uniform draws. A supplied random value is not treated
as sufficient: the plan binds the entropy commitment, frozen roster, epoch,
activation height and previous election transcript under an authenticated test
authority. Nevertheless, this authority can grind/withhold/lie before the external
plan is pinned. Separate signing keys in one fixture process are not independent
operators. No distributed randomness or economic security is claimed.

The complete plan verifies before constructing a genesis or signing. An
authenticated shortage rejects the whole simulation plan; no partial election,
quorum reduction, online-only resampling or fallback committee is used. Offline
status never changes the selected roster. The existing Malachite >2/3 rule alone
governs progress. One selected member retires after the first epoch, and later
candidates become eligible; later sampling may legitimately repeat a committee.

An immutable full-file plan pin is stored before networking/signing. This also
distinguishes different input histories that happen to select equivalent
committees. On restart, a different valid plan or corrupted discovery snapshot
fails before signing and leaves store bytes unchanged. All certified history,
WAL, signing reservations and receipts are retained. Plan parent links connect
election transcripts, **not** finalized chain checkpoints. The plan and all future
committees/entropy are already known at bootstrap; live unpredictable elections
have not been implemented.

The catalog protocol is exercised over TCP between real processes, with eight
in-flight handlers, one-second deadlines, 64 accepted requests/second, pages of at
most 16 records and an 8192-byte response cap checked before allocation. The lab
knows its entire frozen catalog and verifies the expected commitment. This is not
a claim that generic public discovery proves complete membership. Malachite
peering remains the existing pinned loopback topology, not dynamic Internet
routing. No development keys were exposed on public networks.

## Adversarial coverage

New directory tests cover signature/endpoint mutation, missing transport key
possession, same-sequence conflicts, epoch replay, deregistration resurrection,
snapshot restoration, malformed encodings, key aliases, page/cache exhaustion,
per-address limits and public/private address policies. Failed updates preserve
the previous directory.

Lab tests cover permutation-independent selection, unallocated signed Sybils,
forged allocation/authority signatures, altered weights, duplicate backing,
entropy/frozen-root tampering, conflicting epochs/heights/parents, integer limits,
unique selected members and shortages with correctly authorized frozen inputs.
A separate integration test uses the production privacy library without
`cfg(test)`: all 65,536 payment and action proof versions still fail. The
production election selector still returns `Unsupported`.

Real-process campaigns derive their scheduled identities from the signed plan,
verify authenticated discovery for each, compare every observed finalized root,
and independently replay every final store. The adversarial run additionally:

* Splits the first active committee 2+2 through encrypted TCP proxies and confirms
  no quorum progress during the settled partition.
* Heals the partition, SIGKILLs the retiring member, restarts the entire retained
  store and crosses both epoch boundaries without conflicting finality.
* Rejects an oversized discovery response and forged record signature.
* Opens 16 idle discovery clients, then confirms service recovers.
* Rejects a conflicting valid election plan and a corrupted cached snapshot on
  restart before signing, checking byte-identical retained stores afterward.

## Actual complete-process measurements

Linux x86_64, shared `cpu.max=200000 100000` (two CPU equivalents), host memory
ceiling 8 GiB. Every validator, restart and probe was launched with hard/soft
`RLIMIT_AS=2147483648`. This is a **2-GiB virtual address-space** bound, not a
per-node cgroup/RSS budget. No unrelated compilation or tests ran during these
campaigns. The lab release profile enables overflow checks.

| Campaign | Real validator processes | Epochs / finalized height | Time to all nodes at target | Largest sampled node RSS | Campaign child CPU |
|---|---:|---:|---:|---:|---:|
| Normal discovery + elected rotation | 7 | 3 / 12 | 7.892 s | 18.44 MiB | 1.836 s |
| Partition, crash/restart, malicious discovery and restart rejection | 6 plus restart | 3 / 12 | 17.145 s | 19.42 MiB | 1.824 s |

The largest allocated final node store in either run was **237,568 bytes**.
Time includes a three-second startup barrier and the existing approximately
400-ms finalization delay. Child CPU includes validators, discovery probes,
independent inspections and restart checks after initial fixture generation; it
is not a per-validator CPU rate. The selected union differs between campaigns
because each generates fresh simulated entropy. The adversarial run partitioned
at 3.823 s, held settled height 2, healed/SIGKILLed at 10.853 s and restarted at
12.889 s. Every process reached the same certified height-12 state for its plan.

These small loopback measurements include plan verification/selection, catalog
exchange, Malachite execution, disk persistence and replay. They do not qualify
WAN deployment, long-running churn, large eligibility populations, sustained
authenticated floods, economic attacks or a physical 20-GB SSD. Native ARM64 and
per-node cgroup qualification remain unmeasured.

Raw reports and **public-only** retained plans make the exact committee outcomes
reviewable:

* [Normal report](measurements/milestone7-normal.json), [normal signed plan](measurements/milestone7-normal-plan.json).
* [Adversarial report](measurements/milestone7-adversarial.json), [adversarial signed plan](measurements/milestone7-adversarial-plan.json).

Use `yamichain-validator-lab describe PLAN PIN` with the report's `plan_hash` to
independently verify a retained plan and reproduce its genesis/committee sets.
No authority private key is included in a plan. Validator keys are intentionally
the existing public loopback fixtures.

## Security gates blocking public PoS

1. Real authenticated staking/eligibility, unique backing, delegation,
   unbonding/slashing and supply/accounting invariants. Signatures on an issuer's
   weight assertion are not a substitute for these.
2. An unbiased, unpredictable and available randomness protocol with a finalized
   eligibility cutoff, grinding/withholding and adaptive-corruption analysis.
3. Consensus-authenticated live epoch activation, checkpoint/eligibility binding,
   transition proofs and explicit starvation/availability policy. Bootstrap
   fixture plans must never become an arbitrary production genesis override.
4. Real key custody and rotation, authenticated public transport/endpoint
   challenges, diverse bootstrap sources and robust discovery admission/retention
   under global Sybil and eclipse attacks. No public development-key networking.
5. Review of weighted selection and stake-splitting economics, long-running
   resource/corruption/partition tests, key compromise and large-state recovery.

No payment architecture, issuance, rewards, live stake or deployment is enabled.

## Validation

Rust 1.90.0, locked dependencies:

* Root and independent-lab formatting checks: pass.
* Workspace and independent-lab strict Clippy, all targets, `-D warnings`: pass.
* Full workspace regressions: **496 passed, 10 existing ignored**.
* Release chain/privacy/naming/storage/consensus/validator tests: **46 passed**,
  one existing manual core scale test ignored in the normal suite.
* That core scale test was run explicitly: all **12 schedules** passed for
  4/25/50/100 members with reordering, a malicious participant and partition
  recovery. This is a core schedule harness, not 100 full validator processes.
* Isolated lab release suite: **5 passed**, including the exhaustive non-test
  production proof-version and election-disable boundary test.
* Workspace release build and independent lab release build: pass.
* Python harness regressions: **3 passed**. New campaign driver syntax and both
  complete multi-process campaigns: pass.
* Existing seven-validator epoch-boundary crash/restart process regression:
  pass at height 10. [Raw report](measurements/milestone7-legacy-epoch.json).
* Chain/application independence check: pass. It now also rejects any production
  workspace dependency on this isolated election lab. Root manifest/lockfile,
  production privacy crate and every previous YPP experiment remain unchanged.

The full GUI workspace used the same scratch-extracted Debian ALSA development
metadata as Milestone 6, linking the existing runtime library. No repository build
workaround or system package change was made; no physical audio device was
available. Development/test debug symbols and incremental build output were
disabled locally to limit disk usage; debug assertions remained enabled.
