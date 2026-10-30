# YamiNet 0.3 alpha implementation and validation record

Base: upstream main `3e2e86a` (YamiNet 0.2). Linux managed Debian 13 x86_64
workspace, Rust 1.90.0, 2026-10-30. This record distinguishes implementation,
local validation, cross-compilation and physical-network checks.

Implemented:

- Windows ZIP and Linux tar.gz staging with all three binaries, licensed fonts,
  examples, documentation, version, file manifest and archive SHA-256. Per-user
  NSIS installer with Start Menu/uninstall support; Linux launcher lifecycle.
  Windows/Linux CI validates locked builds and uploads artifacts; alpha tags
  must match the version. No automatic stable or GitHub release publication.
- Native first-run explanation and Settings, local-only default, explicit
  networking and relay switches, existing trusted-bootstrap entry, persistent
  preferences and safe reset preserving bookmarks/keys. Legacy libraries load
  safely; history remains off until opted in. No startup external request.
- Local exact Unicode find in visible shaped text, match navigation/highlight/
  scrolling, keyboard shortcuts, editor selection/caret, Escape dismissal,
  visible-link keyboard traversal and high-contrast focus outlines. Native
  accessibility API investigation is documented; no screen-reader claim.
- Bounded owner/resource/encoded-content PNG decode reuse, per-page budgets on
  hits, stale active-layout cleanup and fewer hover-triggered full redraws.
  CPU benchmarks for simple, long and image-heavy documents with observed
  environment/results in performance-0.3.md. No persistent document/image cache.
- Offline `site setup` validates/creates a site, creates or reuses a persistent
  identity, validates/writes a new config and prints a canonical address for an
  existing page, including sites without a home page.
  Optional preview or host start is explicit. Existing authoring and network
  implementations are reused. Host diagnostics include site inventory, local/
  public intent, relay registration and DHT snapshots, timestamps and errors.
- Explicit browser `--probe` authenticates document and optional PNG delivery
  using the existing direct/DHT/relay transport, parses/decode-checks results,
  prints a versioned report and fails nonzero on failure. It does not establish
  physical independence or native GUI rendering. Validation topology, firewall
  requirements, checklist and repeatable report template are documented.

Actual final Linux checks:

| Command/check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS; warnings denied |
| `cargo test --workspace --locked` | PASS: 234 passed, 0 failed, 8 ignored |
| `cargo build --workspace --locked` | PASS |
| `cargo build --workspace --release --locked` | PASS |
| Windows GNU x86_64 release cross-build, workspace, locked | PASS; no Windows execution |
| NSIS 3.11 installer compilation on Linux | PASS; unsigned, not installed/run |
| Python packaging/lifecycle regressions | PASS: 4 tests |
| Staged Linux manifest, three executable versions, five site validations | PASS |
| Staged Windows manifest and PE import inspection | PASS; only OS DLL imports observed |
| Offscreen native onboarding/find capture and visual inspection | PASS; separate from interactive GUI testing |
| GitHub Actions workflow | Configured and YAML parsed; remote jobs NOT RUN here |

The eight default ignored tests are four screenshot captures, two explicitly
selected private-interface/multicast tests and two owned-public-endpoint tests.
The screenshot capture test was explicitly run separately and its onboarding
and find output inspected; it is not counted in the 234 workspace passes.
Existing DHT, TLS identity rejection, relay resilience, direct/relayed rich-site
navigation, slow/unavailable nodes, resource cancellation, parser bounds and
0.1/0.2 compatibility coverage remains enabled. Added regressions cover safe
legacy settings/reset, blocked remote fetch submission in local-only mode,
keyboard navigation/find/scroll/layout cleanup, Unicode glyph rectangles,
cache ownership/budget/eviction, offline setup with identity reuse and no config
overwrite, and explicit direct plus nested-relay document/PNG probes. A relay
renewal-gap regression drove reuse of the existing bounded PNG retry loader.

Packaging tests exercise fresh layouts/checksums, Windows ZIP completeness,
staging upgrades retaining a separate identity/library fixture, missing binary
failure and actual Linux launcher install/uninstall with paths containing spaces.
They do not substitute for Windows installer upgrade testing. Benchmarks and
exact local CPU results are recorded separately in performance-0.3.md.

Unverified/release limits:

- Native Windows interactive runtime, MSVC CI execution, installer lifecycle,
  actual upgrade/uninstall data survival and Windows Task Scheduler operation
  require execution on Windows. Cross-compilation alone does not prove them.
- Native Linux desktop/IME/high-contrast interaction and screen readers require
  separate manual checks. Offscreen raster tests are not an interactive session.
- VPS + Windows NAT host + independently connected client trial NOT RUN. No
  public reachability, NAT traversal, service deployment or firewall change is
  claimed. Local integration and stream impairment tests remain simulations.
- All packages are unsigned. No independent security audit; not anonymous,
  production-secure or appropriate for sensitive activity. Limited glyphs,
  exact case-sensitive search, no document selection/semantic accessibility,
  bounded simple layouts, initial long-page layout on UI thread, software full
  window drawing and nearest-neighbor images remain alpha limitations.
- Linux binaries require compatible host libraries/glibc. CI produces native
  MSVC Windows packages; locally cross-built Windows GNU artifacts have different
  toolchain provenance and still need native execution validation.
