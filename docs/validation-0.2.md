# YamiNet 0.2 validation record

Validated in the Linux development environment with the pinned Rust 1.90.0
toolchain. This is implementation testing, not an independent security audit.

| Required command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed, no warnings |
| `cargo test --workspace --locked` | 225 passed, 0 failed, 8 ignored |
| `cargo build --workspace --locked` | Passed |

The ignored tests are four manual native screenshot captures, two tests needing
an explicitly selected private interface/multicast environment, and two tests
requiring an owned public remote endpoint. They were not counted as passed.

All three checked-in 0.2 templates pass `yami-node site check`. The CLI subprocess
test creates/checks/exports a template, launches a real loopback TLS preview,
fetches its document and PNG, and verifies the persistent identity. Browser
integration tests use real direct and reverse-relay TLS connections, the
production DHT/browser loader model and the native renderer. They cover local
and cross-identity navigation, responsive rendering, bookmarks across restart,
optional/cleared history, corrupt/missing images, exact destination changes,
slow assets, per-page attempt limits and cancellation on navigation/tab close.
Parser, native rendering, decoder, codec, filesystem and library regressions
exercise version compatibility, bounds and malformed data. All pre-existing
DHT, TLS, relay, maintenance and resilience tests remain enabled.

A 1000-pixel-wide native content capture of the generated portfolio was rendered
with the production native renderer and visually inspected. Its rich text,
image, lists, cards, link placement and responsive rows rendered. CJK glyphs
without an installed fallback font displayed missing-glyph boxes, as documented.
This was an offscreen content render, not an interactive window test.

No Windows interactive runtime, physical home-router/CGNAT, VPS firewall or
independent cross-network asset-delivery test was executed. No external
infrastructure was deployed or firewall changed. Follow
[the cross-network plan](cross-network-validation.md), including its 0.2 checks,
and record results separately before claiming public-network validation.

Known implementation limits are in [websites-0.2.md](websites-0.2.md): static
same-node PNG only, bounded simple layouts, bundled/system glyph availability,
nearest-neighbor image scaling, no persistent image/document cache, and text-only
rich-content fallbacks in the debugging terminal/P6 backends. Assets reuse their
parent's selected authenticated route; a route failure produces alternative
text until reload rather than independent asset discovery. Bookmarks/history
are local plaintext files. Existing network trust, privacy, denial-of-service
and independent-review limitations still apply.
