# Milestone 1 validation — completed 2026-11-03

Baseline: `129c56d51530d6e94a36cc459ca50d0b9596dae3` on the existing
`development/yamichain` branch. The shared branch was fetched before implementation
and again before committing. All changes are confined to this new independent
experiment. Main, the root lockfile, production crates/apps and the RingCT lab
remain unchanged. No financial operation, verifier registration or deployment
was activated.

## Passed checks

- Root workspace and experiment formatting checks.
- Experiment `cargo clippy --all-targets --locked -- -D warnings`.
- Experiment `cargo test --release --locked -- --test-threads=1` with
  `RAYON_NUM_THREADS=2`: **5 passed** (four unit/adversarial tests and one
  integration test). Final run completed in 82.43 seconds for the four unit tests.
  The integration test checks every one of 65,536 production proof versions for
  both payments and protocol actions, through the non-test production library.
- Experiment locked release build and the real Alice-to-Bob demo: three payments,
  six real proofs, offline inbox discovery, absorption, replay rejection, backup
  restoration and complete authenticated history replay.
- Existing root `yami-chain` and `yami-chain-privacy` tests: **9 passed**.
- Existing root `yami-chain-storage`, `yami-chain-consensus` and
  `yami-chain-naming` tests: **25 passed, one existing ignored scale test**.
  These include storage/recovery, consensus and naming regression coverage.
- Existing `python tools/check-chain-independence.py` passed. An additional Cargo
  metadata check confirms YPP is absent from production workspace membership and
  every production package dependency list. Existing production/RingCT files and
  the root Cargo.lock were compared against the baseline and are unchanged.

Test assertions exercise genuine Groth16 verification, not mocked validity.
Direct R1CS checks additionally cover malicious witness assignments for ownership,
inflation, zero credits, version rollover, arithmetic overflow and changed
commitments. Real public packet mutations cover root, slot, mode, both ciphertexts,
old/new/credit commitments and proof. Tests also cover malformed lengths/trailing
bytes, bounded verifying-key decode, conflicting valid proofs, failed record
rollback, prefix resumption, encrypted backup corruption, restored-wallet spending,
and the observable linkability graph.

A deliberately malicious but valid proof demonstrates that ciphertext binding
alone cannot guarantee encrypted delivery or recovery availability. The ledger
accepts the monetary transition; the recipient cannot discover the undecryptable
credit, and the sender's scanner rejects the withheld next state without advancing
its cursor. This negative result is part of the delivered prototype, not a solved
security property. [spec.md](spec.md) describes every remaining blocker.

## Actual native measurements

Measured on **2026-11-02**, Linux x86_64, AMD EPYC 9V74 VM, Rust 1.90.0 release
build, `RAYON_NUM_THREADS=2`. Cgroup CPU quota was 200000/100000 (two CPU-equivalents),
with an 8 GiB environment memory ceiling. This is a virtual machine, not physical
low-end validator hardware. There are no native ARM64 measurements or claims.
Raw samples, timestamp and limits are in
[measurements-x86_64.json](measurements-x86_64.json).

| Measurement | Actual result |
| --- | ---: |
| Relation constraints | 405,110 |
| One local trusted setup | 22.513 s |
| Prove one debit or absorption, median of six | 8.317 s |
| Proving range | 7.964–8.899 s |
| Compressed proof | 192 bytes |
| Fresh verifier process, median of 20 verifications | 3.805 ms |
| Verification range in that process | 3.284–5.788 ms |
| Verifying-key load/preparation and packet decode | 21.590 ms |
| Scan one authenticated record, median of six | 3.722 ms |
| Full six-record wallet recovery | 26.166 ms |
| Encrypted wallet backup restoration | 0.140 ms |
| Prover/setup/recovery process peak RSS | 1,425,256 KiB (1.359 GiB) |
| Separate verifier-only process peak RSS | 3,124 KiB |

Each payment requires **two transitions/proofs** (debit and recipient absorption).
The six proving samples alternate these operations; they are a small, sequential
run, not a statistical study across machines or independent setups. Proving
includes honest wallet construction/encryption and R1CS/proof generation.
Verification includes public context hashing and checked compressed proof decode,
using a prepared key. Scanner timings include replay verification and local age
decryption. The full demo has additional verification outside the timed regions.
Memory comes from Linux `/proc/self/status` VmHWM; the prover figure includes
setup, proving, verification and recovery in one process. It is **not** validator
memory. The verifier ran in a fresh process with an enforced **2 GiB virtual
address-space limit** and the same two-CPU cgroup quota. It did not load the
proving key or execute setup. No unrelated compilation/tests ran during measurement.

Measured logical state after three payments/six accepted transitions:

- Two current vault commitments: **64 bytes**; computed root: **32 bytes**.
- Permanent seen credit markers: **96 bytes** (three hashes); pending inbox empty.
- Historical transaction encodings: **6,811 bytes**, including proofs and both
  encrypted payloads. Per-record root/head fields add **384 bytes**.
- Encrypted genesis payloads: **887 bytes** total, in addition to the leaves.

These byte counters exclude container/allocator overhead, verifier/prover keys,
wallet private state, repeated history held by scanners, length framing for a
future history storage format and transport. Age framing is randomized, so packet
sizes vary. History and seen-marker storage grow; no pruning implementation or
large-history scalability is established. Transactional scanner cloning currently
causes quadratic cumulative copying. Exact-root acceptance globally invalidates
concurrent proofs and remains a throughput/liveness blocker regardless of fast
verification. The target of a complete validator operating on two low-end CPU
cores and 2 GB RAM remains unproven.

Reproduce the raw report after the locked release build:

```sh
python experiments/yamichain-ypp-vaults/measure.py > /tmp/ypp-measurements.json
```

No independent review, setup ceremony, complete dependency advisory audit,
side-channel assessment, durable wallet/database recovery, network/data availability
or privacy-preserving monetary module has been delivered. Public slots and credit
markers expose transaction links. This remains an isolated research prototype.
