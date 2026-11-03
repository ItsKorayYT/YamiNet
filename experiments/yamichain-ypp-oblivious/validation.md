# Milestone 4 validation — 2026-11-03

Base: `21ab5d193e3c0ad95122d37d76a5ee6287ec4470`, on
`development/yamichain`. Changes are confined to this new independent experiment.
No earlier experiment, production source, production manifest/lockfile,
consensus setting, or financial activation flag is modified.

## Checks

- `cargo fmt --manifest-path experiments/yamichain-ypp-oblivious/Cargo.toml -- --check`: pass.
- `cargo clippy --manifest-path experiments/yamichain-ypp-oblivious/Cargo.toml --all-targets --locked -- -D warnings`: pass.
- `cargo test --manifest-path experiments/yamichain-ypp-oblivious/Cargo.toml --release --locked`: **12 tests pass** (11 storage/crypto/counterexample tests, one production boundary integration test).
- The boundary test compiles the production privacy library without `cfg(test)`
  and confirms rejection for every one of 65,536 proof versions, for payments and
  protocol actions.
- `cargo test -p yami-chain-privacy --release --locked`: **2 existing tests pass**,
  including the independent production fail-closed boundary.
- Locked release build, fresh demo, separate fixture verifier, independent Python
  observer, and Python syntax compilation: pass.
- Checked-in fixture replay: 12 real shape proofs verify; exposed shares open
  the matching commitments; nine snapshot-pair checkpoint hashes and 16
  disclosed-message server snapshot transitions agree.
- New benchmark run: three samples at each of N = 2, 4, 8, 16, 32, 64, 128, 256.
  A separate freshly generated trace is verified and attacked by both Rust and
  Python code. [Raw measurements](measurements-x86_64.json) include all samples,
  CPU/cgroup context, byte counts and observer results.
- [Dependency inventory](dependency-licenses.tsv) is extracted from locked Cargo
  metadata, including target-specific dependencies and the local test-only
  production privacy dependency. No upstream crypto code is vendored or patched.

Counterexample tests passing means the **rejection was reproduced**, not that the
candidate passed the requested privacy/conflict requirements.

## Counterexample evidence

`independent_deltas_commute_without_lost_writes` prepares two encrypted
replacements from one snapshot, applies both in each order, compares final
snapshot hashes, and decrypts both final states successfully.

`reject_candidate_conflicting_valid_writes_both_pass_and_destroy_cell` prepares
two distinct replacements from the same old ciphertext, verifies/applies both
real shape proofs, and observes failed final ciphertext authentication. Exact
replay is rejected, but a distinct conflicting operation is accepted. This
demonstrates why replay IDs and write shape do not establish version authority.

`encrypted_snapshot_refresh_and_offline_recovery_are_authenticated` leaves a
wallet offline through eight unrelated changes. Latest snapshots plus its
original key/index and independently pinned current head restore its state.
The repeatedly modified wallet also decrypts its latest state. Modified,
truncated, old-epoch and wrong-key data fail. A valid old head with old data is
accepted, explicitly documenting the independent-finality requirement.

The trace is `[2, 2, 0, 1, 0, 3, 3, 1]`. A keyless observer using only the recovery
downloads recovers exactly this sequence. All eight slots and all 28 pairwise
same/different classifications are correct, including four repeated-slot pairs:
`(0,1), (2,4), (3,7), (5,6)`. Combining exposed write shares independently gives
the same result. The limited hiding-commitment-only transcript is kept separate;
no classifier result or empirical privacy claim is made about that view.

Additional tests reproduce one-server withholding, unauthorized offline
recipient corruption, the incompatibility of ordinary replicated-database PIR
with two additive database shares, malformed proof/message rejection without
mutating state, and proof failure after rebinding a second nonzero cell.

## Native scaling measurements

x86_64 AMD EPYC 7763, Rust 1.90.0, release optimization, 2 CPU cgroup quota,
8 GiB memory limit. Single-process, sequential execution, three samples per
size; medians below. No remote servers or network round trips are timed.

| Vaults | Write body bytes | Recovery body bytes | Prepare ms | Verify ms | Recovery µs | Harness RSS KiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 672 | 272 | 0.609 | 0.167 | 3 | 12,160 |
| 4 | 1,568 | 528 | 1.708 | 0.921 | 3 | 12,288 |
| 8 | 4,128 | 1,040 | 5.759 | 4.191 | 4 | 12,288 |
| 16 | 12,320 | 2,064 | 21.283 | 18.080 | 5 | 12,288 |
| 32 | 40,992 | 4,112 | 80.141 | 73.973 | 8 | 12,288 |
| 64 | 147,488 | 8,208 | 321.300 | 307.594 | 11 | 12,288 |
| 128 | 557,088 | 16,400 | 1,231.469 | 1,202.020 | 17 | 12,288 |
| 256 | 2,162,720 | 32,784 | 4,948.058 | 4,890.437 | 29 | 15,704 |

**Prepare** includes random shares, Pedersen commitments, and actual shape-proof
generation, not just a simulated proof cost. **Verify** checks that proof;
private share-opening checks are separate raw metrics (55.734 ms median at
N=256). The reference `apply` also repeats these checks and is reported
separately. Recovery times hash complete already-resident snapshots and decrypt
one cell; fetching/snapshot creation/head generation are excluded. Microsecond
samples at small N are clock-resolution/noise limited, not latency guarantees.

Byte formulas count fixed-width payload bodies, with N known from the head;
no compact binary transport encoder is implemented. These formulas exclude
framing, length fields, TLS, checkpoint delivery, receipts, signatures and
consensus replication. Actual compact JSON sizes from the implemented serializer
are also measured separately in every sample:

- Shape proof: `32 N²` bytes.
- Public operation nonce + commitments + proof: `32 + 64 N + 32 N²` bytes.
- Both private channel messages, including blinds: `192 N` bytes.
- Total modeled write body: `32 + 256 N + 32 N²` bytes.
- Both snapshot bodies, including epochs: `16 + 128 N` bytes; head body: 80 bytes.
- At N=256, actual JSON write payloads are 7,777,998–7,779,611 bytes;
  actual JSON snapshot downloads are 118,877–119,001 bytes. No bandwidth/throughput
  claim about a deployed network is inferred from these payload counts.

The wallet's modeled persistent material is **184 bytes** on this 64-bit
platform: 32-byte key, 8-byte index, 80-byte pinned head, 64-byte cached cell.
There is no N-sized permanent client position map. This does not include secure
backup, allocator metadata, implementation objects or wallet journals. Preparing
a write needs **192 N** bytes of dense share/blinding material plus **32 N²**
bytes of proof challenges/responses. Announcement points additionally occupy
`size_of(RistrettoPoint) N(N-1)`, or **10,444,800 bytes at N=256**. Recovery
material is **128 N** bytes of share cells. These are explicit payload/buffer
counts, not a claim of constant working memory. Server share cells use **64 N**
bytes per server, before replay sets, snapshots, redundancy or archival retention.

RSS uses Linux `wait4` independently for each child and covers the **whole
harness process**, including synthetic wallets, both servers, proof buffers and
JSON serialization. It is not isolated client or validator RSS. The shape proof
is quadratic; dense shares and recovery are linear. DPF or proof compression
could change those costs but would not fix the reproduced recovery leak or
missing conflict authorization. Sizes above 256 are unmeasured. No low-end
validator, throughput, payment-proof, distributed-liveness or production claim
is established.
