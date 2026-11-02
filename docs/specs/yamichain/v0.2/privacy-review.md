# Privacy backend gate — 2026-11-02

Follow-up: the user's Milestone A instruction separately authorizes an **offline**
real-proof experiment. [The isolated RingCT lab](../../../../experiments/yamichain-privacy-lab/README.md)
pins monero-oxide's complete CLSAG/Bulletproofs+ wallet path and records evidence,
formats, tests and limitations. This selects an experimental baseline only;
consensus backend selection/activation remains deferred. Orchard is excluded at
the user's request. The original comparison below is historical, not an
instruction to block the newly authorized offline work.

Original decision: **DEFER selection and proof integration**. Ordinary payments must remain
shielded; all payment/protocol proof versions remain rejected in debug and release.
No transparent fallback, new proof dependency, wallet, mint or private-value wire
format was introduced. This is a source/licensing assessment, not a cryptographic
audit or measured comparison of working YamiChain payment backends.

| Candidate and inspected source | Evidence and remaining blocker |
| --- | --- |
| [FCMP++ current source](https://github.com/monero-oxide/monero-oxide/tree/77788c368145127f2dde2ac3e2ddce919f3ddd01), with [CARROT specification](https://github.com/jeffro256/carrot/tree/08094f7c70fb0a70f322c50ba85589c64436d554) | The archived FCMP repository explicitly points to monero-oxide. Its current `monero-fcmp-plus-plus` manifest declares MIT and Rust 1.81; CARROT's specification license is BSD-3-Clause. The older archive's licensing concern cannot simply be applied to the current project. The complete transitive license set, exact deployed construction, audit coverage/remediation, CARROT integration, wallet recovery, private stake conversions and YamiChain domain binding still require review. |
| [Spark V2 source](https://github.com/firoorg/firo/tree/eea3c22d0c8411279e6f70ecc8ccfc621e6ca991/src/libspark) | The old `firoorg/spark` repository is a Python research implementation, not the corrected node implementation. Current C++ `spend_transaction.cpp` distinguishes V1/V2 and binds V2 outputs, fees, transparent value, extensions and cover-set references. [Versioning change](https://github.com/firoorg/firo/commit/0e3c41a90efc091e2ea3f78a34201781724af6d8) supplies correction/review leads, not proof of soundness. Require independent review of the precise multi-input correction, license inventory including native dependencies, safe Rust/native boundary, recovery and binding of all YamiChain actions. An upstream transparent-output capability must not become a YamiChain fallback. |
| [Orchard 0.16.0](https://github.com/zcash/orchard/tree/616a669df8c9a59c33d47064d3ce04b25d026b2e) | An established shielded-protocol candidate; manifest/README declare MIT OR Apache-2.0 and Rust 1.88. Uses Halo 2/Pasta and Zcash-specific note/key/encoding dependencies. Compatible MSRV and a permissive top-level license do not establish safe YamiChain adaptation. Require protocol/audit applicability, dependency and parameter review, wallet/recovery and shielded stake/withdrawal statements. No Orchard dependency was added. |

No complete transfer, multi-input forgery reproduction, proof-size, verifier/prover
CPU/RSS, wallet scan or ARM64 benchmark was run for these candidates. No security,
license-completeness or resource-target attestation follows from repository metadata.
Generic STARK primitives alone do not supply a reviewed private payment relation.

Next decision requires a reviewer-approved, version-pinned cryptographic design
covering ownership/membership, immutable spent tags, exact conservation, fees and
burns, complete intent binding, authenticated recipient ciphertexts, separated
spend/view keys and backups, historical anchors, private staking/withdrawals and
spent-tag retention after pruning. Require a correction-specific adversarial test
plan, complete dependency/license inventory, actual 2-core/2-GB and native ARM64
measurements, and an independent security review plan. Only then seek separate
explicit authorization for a real-proof experiment. This work grants no payment
activation or payment-capable deployment authorization.
