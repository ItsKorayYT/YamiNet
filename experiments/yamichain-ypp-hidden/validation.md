# Milestone 2 validation — 2026-11-03

Baseline: `c8634c2764e4b43c41d6942b05fa4807cfefa039` on
`development/yamichain`. Changes are confined to this independent experiment.
Milestone 1, RingCT, production crates/apps, root manifest/lockfile and main are
unchanged. The experiment is absent from every production workspace dependency
list. No production verifier, YAMI operation or financial module was enabled.

## Passed

- Experiment formatting, Clippy `--all-targets --locked -- -D warnings`, and locked
  release build, on Rust 1.90.0.
- Full experiment release suite: **5 passed**, zero failures/ignored. Four unit
  tests finished in 202.71 seconds; the separate integration test checks all
  **65,536 production proof versions**, for payments and protocol actions, using
  the production library without its test configuration.
- The real-proof test constructs eight actual Groth16 proofs: four independent
  same-root payments, a conflicting spend, an unspent proof whose root expires,
  credit absorption plus onward payment, and a repeated Alice payment. Every proof
  is independently verified before exercising ledger acceptance/rejection.
- Adversarial R1CS witnesses cover wrong hidden path/index, forged authority,
  inflation with correctly re-encrypted outputs, dummy-credit issuance with
  recomputed marker/output, wrong recipient encryption key, wrong encrypted salt
  or spending authority, reused ElGamal masks, and a noncanonical scalar encoding
  with **recomputed alternate HMAC markers**. Native code rejects overspending.
- Packet mutations cover root, nullifier, output commitment, encrypted amount,
  encrypted salt, encrypted spending authority, recovery ciphertext and proof.
  Exact-size/tag, truncation, trailing-data and verifying-key allocation bounds
  are tested. Wallet tests include u16 boundary decoding, wrong viewing key,
  misaddressed plaintext detection, corrupted/wrong backup key, interrupted
  authenticated replay, prefix/head rejection and view-only authority isolation.
- The executable stale in-place root example shows the second root overwriting
  an unrelated first update. It is explicitly a data-structure counterexample,
  not a claim to have generated a proof for that hypothetical protocol.
- Existing chain, privacy, storage, consensus and naming regressions: **34 passed,
  one existing ignored scale test**. Existing chain-independence script and an
  additional production Cargo dependency/membership check passed.

The successful proof tests use real cryptography; rejected malformed packets and
the structural counterexample are not presented as evidence of proof validity.

## Actual native measurements

Raw timestamp/hardware/quota metadata, samples and **complete public packet hex**:
[measurements-x86_64.json](measurements-x86_64.json). Measured Linux x86_64 on an
AMD EPYC 9V74 VM, two CPU-equivalents of cgroup quota, `RAYON_NUM_THREADS=2`, and
an 8 GiB environment memory ceiling. No native ARM64 measurements were made.
No unrelated compilation or tests ran during the measured processes.

| Measurement | Actual result |
| --- | ---: |
| Relation constraints | 715,751 |
| Local trusted setup | 37.226 s |
| Prove transition, median of six | 16.235 s |
| Proving range | 15.797–16.643 s |
| Fresh verifier, median of 20 verifications | 8.590 ms |
| Fresh verifier range | 8.109–9.088 ms |
| Verifying-key preparation + packet decode | 15.921 ms |
| Compressed Groth16 proof | 192 bytes |
| Complete fixed packet | 740 bytes |
| Scan one record for all four wallets, median | 65.621 ms |
| Four-wallet scan range | 61.723–70.269 ms |
| Bounded-amount decoder setup | 1.513 ms |
| Four-wallet genesis scanning | 68.606 ms |
| Full one-wallet six-record recovery | 113.456 ms |
| Encrypted backup restoration | 0.129 ms |
| Setup/prover/demo/recovery peak RSS | 2,431,160 KiB (2.319 GiB) |
| Separate verifier-only peak RSS | 3,260 KiB |

The verifier-only process had an enforced **2 GiB virtual address-space limit**
and did not load a proving key or generate parameters. Verification includes
native subgroup checks, public input construction and pairing verification
with a prepared key. Scanning includes proof replay, local decryption, bounded
amount decoding, root/history validation and scanner copying. Proving includes
wallet construction, encryption and proof generation. RSS is Linux VmHWM.
These are small sequential samples, not a validator-throughput, side-channel or
large-history scaling study. Prover memory exceeds 2 GiB and is reported separately.
A complete low-end validator meeting the 2-core/2-GB target remains unproven.

Logical state after four genesis vaults and six transitions: **16 leaves / 512
commitment bytes**, four retained roots / **128 bytes**, twelve spent markers /
**384 bytes**, **4,440 bytes** of historical packets plus **384 bytes** of per-record
root/head fields. Container overhead, duplicate commitment index, tree parameters,
proving/verifying keys, wallet secrets and scanner copies are excluded from those
logical byte counts. Leaves and markers grow; no pruning is implemented. Capacity
is 32 leaves, balances are u16, and cumulative scanner copying is quadratic.

## Observable transcript result and limits

The measured scenario has four independently generated wallets and six transfers.
Synthetic harness ground truth (not wire fields): Alice→Bob, Carol→Dave, Bob→Carol;
then Bob absorbs Alice's credit while paying Carol, Alice pays Bob again, and
Carol absorbs Bob's earlier credit while paying Dave. Initial three proofs use
one root and all apply unchanged. Final vault balances plus pending credits equal
the original synthetic 400 units. Full Bob backup recovery reaches the pinned head.

Across the six accepted public packets, equality analysis found:

- **0** output-commitment / nullifier matches;
- **0** repeated accepted nullifiers and **0** repeated ephemeral keys;
- **4** distinct anchors; the first three packets deliberately share one anchor;
- no input slot, input index, old commitment or consumed-credit ID field.

This removes Milestone 1's direct slot, old/new-pair and debit/absorption identifier
graph. It does not prove cryptographic anonymity of the composition. Output and
marker roles remain positional; roots/age, insertion positions, timing, counts,
reproving/conflicting-input markers and transport/auxiliary observations remain.
See [spec.md](spec.md) for the full public surface and key/privacy assumptions.

The accepted ciphertext relation now proves recovery of **all** committed opening
fields under the committed viewing key; arbitrary undecodable payloads cannot be
substituted while retaining a valid proof. Out-of-band address authentication,
retained receiver secrets, archival data availability and a trusted final head
remain necessary. Deliberately paying a different composite address is not proof
of paying the intended human. A decryptable mismatched spending authority is
surfaced as misaddressed, not silently accepted as the wallet's balance.

This is an append-only **shielded-record prototype with a vault lineage and
note-like inbox**, not a novel unlinkable in-place vault system. The lost-update
counterexample and its scope are documented and tested. Independent review,
reviewed setup/parameter provenance, scalable witnesses/storage, unbounded amounts,
network privacy and a safe in-place update architecture remain unresolved. No
security approval or production acceptance follows from these results.
