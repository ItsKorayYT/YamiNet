# Dependency provenance and security review — 2026-11-01

This is a source/manifest/advisory review, **not an audit or a clean-security
attestation**. Public networking and real-value use remain blocked.

## Pins and provenance

| Component | Exact selection | Evidence |
| --- | --- | --- |
| Rust | 1.90.0, edition 2021 | Existing `rust-toolchain.toml`; workspace MSRV unchanged |
| Malachite direct packages | `arc-malachitebft-app-channel`, `core-consensus`, `signing`, `sync` = **0.8.2** | Exact manifest pins and registry checksums in `Cargo.lock` |
| Malachite upstream | tag `v0.8.2`, commit `a453b32155f4ba9f959724279b52f601bae92e09` | Published app-channel `.cargo_vcs_info.json` matches checked-out tag |
| Consensus signatures | `ed25519-dalek` **3.0.0** | Reuses existing dependency version, separate consensus/registry role keys |
| Public hashing | `sha2` **0.11.1**, SHA-256 | Reuses existing dependency version; independently encoded vectors |
| Transport | `libp2p` **0.56.0**, `libp2p-identity` **0.2.14** | Transitive through Malachite; separate Ed25519 transport identity |
| Async/runtime | `tokio` **1.54.1** | Exact resolved version in lockfile; executable uses two worker threads |
| Privacy proof backend | **NONE** | No proof dependencies or accepted wire discriminant |

All pre-existing `(crate, version)` entries remain in the lockfile: no existing
package version was removed/upgraded to add this work. The change adds 298 registry
package versions plus six workspace packages. The high-level engine brings a
substantial networking/metrics dependency graph; the ordinary browser/hosted-node
dependency closures do not include it. `tools/check-chain-independence.py` enforces
that separation in CI.

Resolved package metadata has no new missing declared license and no new declared
MSRV above 1.90. Malachite declares Apache-2.0 and MSRV 1.88. Other added license
expressions select MIT/Apache-2.0, BSD, permissive alternatives, Zlib,
Unicode-3.0, or MPL-2.0 (`attohttpc` 0.30.1). Some expressions offer LGPL as one
alternative to MIT/Apache. Review distribution notices/source obligations before
shipping; metadata inspection is not a legal opinion. No AGPL-only proof code or
Zcash/Sapling/Orchard/Halo 2 dependency was added. Our six crates inherit
GPL-3.0-only and workspace `unsafe_code = forbid`; transitive upstream code is
not claimed free of unsafe blocks.

## Reviewed upstream notices and remaining blockers

Malachite's [README at the pinned commit](https://github.com/circlefin/malachite/blob/a453b32155f4ba9f959724279b52f601bae92e09/README.md)
labels it **alpha, under heavy development, not externally audited**.
Its [release notes](https://github.com/circlefin/malachite/blob/a453b32155f4ba9f959724279b52f601bae92e09/RELEASE_NOTES.md)
record security-relevant WAL error propagation, replay-signature consistency,
certificate/vote-extension validation and bounded-input fixes in recent releases.
0.8.2 fixes proposal `pol_round` validation, a polka/prevote timeout issue and sync
certificate-round handling. It also notes a restarted peer can temporarily lack
validator proof recognition while a dead connection remains; consensus/sync are
reported unaffected upstream. Those statements are upstream claims, not independent
YamiChain verification. Our interface excludes ProposalOnly's automatic-validity
path and independently authenticates all durable commit certificates.

The RustSec database was inspected at commit
`2295466c4e9df26647fd67d8fa1c705ac2fc7f81`, including matching advisory version
ranges for newly introduced packages. Known unresolved matches:

| Dependency | Notice | Current treatment |
| --- | --- | --- |
| `hickory-proto` 0.25.2 | [RUSTSEC-2026-0118](https://rustsec.org/advisories/RUSTSEC-2026-0118.html): DNSSEC NSEC3 validation loop/OOM | Transitive via `libp2p-dns`/resolver. No DNS endpoints are accepted by this local CLI; DNSSEC feature exploitability was not established. Still a dependency blocker. |
| `hickory-proto` 0.25.2 | [RUSTSEC-2026-0119](https://rustsec.org/advisories/RUSTSEC-2026-0119.html): quadratic DNS message compression CPU cost | Fixed upstream in a newer incompatible minor. Loopback literal-IP configuration does not exercise DNS; do not enable arbitrary peers without resolving it. |
| `crypto-bigint` 0.5.5 | [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387.html): secret-dependent timing in intended constant-time operations | Via `elliptic-curve`/P-256/libp2p identity support. Our selected consensus and transport keys are Ed25519 with public dev seeds; no claim about general peer-curve safety. Upgrade/removal review is required before real secrets/public exposure. |
| `paste` 1.0.15 | [RUSTSEC-2024-0436](https://rustsec.org/advisories/RUSTSEC-2024-0436.html): unmaintained | Transitive build-time macro through netlink/if-watch. Track upstream replacement; not suppressed or represented as audited. |

Other reviewed matching ranges include patched `quinn-proto` 0.11.19,
`crossbeam-epoch` 0.9.21, `h2` 0.4.20, `snow` 0.9.6 and `anyhow` 1.0.104.
The full `cargo audit` command was **NOT RUN**; this review was a registry metadata,
upstream-release and RustSec advisory-source scan. Advisory freshness and
feature/target reachability need a maintained automated check before any public
network milestone. We did not vendor or silently patch upstream consensus/crypto.

The executable's transport is intentionally restricted to four fixed loopback
addresses and known peer IDs. This containment does not make these dependencies
production-safe or audited, and local processes must not hold real funds/secrets.
