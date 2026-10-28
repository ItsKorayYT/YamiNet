# YamiNet 0.1.0

An experimental Rust MVP for an independent document web. YLang is content,
YSL is presentation, YNP carries documents, and Yami Render performs native
layout and software rasterization. There are no third-party dependencies.

**Not secure for sensitive use.** Networking is plaintext, unauthenticated and
restricted to numeric loopback IP addresses. Privacy is a design goal, not a
security guarantee. There is no cryptographic identity, encryption or anonymity.

## Run

Install Rust 1.90.0 with rustfmt and Clippy (the toolchain is pinned). From this
checkout:

```sh
cargo run -p yami-browser
cargo run -p yami-browser -- examples/welcome.ylang --style examples/default.ysl --ppm /tmp/yami.ppm
cargo run -p yami-node -- demo
```

In the prepared cloud environment, first activate the retained toolchain in
each shell (ordinary local Rust installations do not need this):

```sh
export CARGO_HOME=/workspace/.yami-tools/cargo
export RUSTUP_HOME=/workspace/.yami-tools/rustup
export PATH="$CARGO_HOME/bin:$PATH"
cd /workspace/YamiNet
```

The browser is a terminal document viewer with a native bitmap export, not yet
a desktop GUI. `--ppm` writes a P6 bitmap using the built-in software glyph
renderer. No HTML, CSS, JavaScript, Chromium, WebView or web rendering engine is
used. The viewer does not execute content or automatically fetch links, fonts,
images, stylesheets or other assets. A fetched YNP document includes its style
as inert text in the same frame; a local `--style` explicitly overrides it.

`demo` starts two loopback nodes on OS-assigned ports. Each node serves one
sample document and fetches the other over YNP. It compares the received
source and style with the expected documents, prints the result and exits.
The links inside these unmodified samples point at the persistent demo ports
below; the ephemeral demo does not rewrite documents or follow links.

To run two persistent sample node processes, use separate terminals:

```sh
cargo run -p yami-node -- serve 127.0.0.1:7301 welcome examples/welcome.ylang examples/default.ysl
```

```sh
cargo run -p yami-node -- serve 127.0.0.1:7302 about examples/about.ylang examples/default.ysl
```

Then request either document, and explicitly open a displayed link by giving
its YNP URL to the viewer:

```sh
cargo run -p yami-browser -- ynp://127.0.0.1:7301/welcome
cargo run -p yami-browser -- ynp://127.0.0.1:7302/about --ppm /tmp/yami-about.ppm
cargo run -p yami-node -- fetch 127.0.0.1:7302 about
```

Stop persistent nodes with Ctrl-C. All application network messages use YNP,
including missing-document replies. There is no HTTP endpoint or fallback.
The sample CLI node serves one in-memory document sequentially; malformed
connections are rejected without terminating the process.

## Workspace

| Package | Responsibility |
| --- | --- |
| `ylang` | Bounded document parser, typed AST, byte spans and line/column errors |
| `ysl` | Bounded original style parser and typed presentation settings |
| `ynp` | Binary frame codec, strict limits, transport traits, loopback TCP client/listener |
| `yami-render` | Word wrapping, indentation, colors, spacing, link hit-testing, terminal and bitmap output |
| `yami-browser` | Local or explicitly fetched document viewer |
| `yami-node` | In-memory document server, fetch client and bidirectional two-node demo |

Versioned normative drafts: [YLang](docs/specs/v0.1/ylang.md),
[YSL](docs/specs/v0.1/ysl.md), [YNP](docs/specs/v0.1/ynp.md),
and [rendering and architecture](docs/specs/v0.1/architecture.md).
See [SECURITY.md](SECURITY.md) for the experimental trust boundary.

## Validate

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

No registry access is needed after installing Rust; `--offline` also works.
The integration tests require permission to bind and connect loopback sockets.
They use ephemeral ports and include real subprocesses, two-node exchange,
malformed-client recovery, missing documents, and fetch/parse/style/raster output.
Unit tests cover parser locations and limits, nested ASTs, wrapping and pixels,
golden wire bytes, all truncated prefixes of representative frames, malformed
headers and payload lengths, Unicode, size boundaries and response correlation.

## MVP limitations

No desktop window, interactive navigation, image embedding, rich typography,
Unicode shaping, accessibility tree, editing or persistent document storage.
Layout counts Unicode scalars as fixed-width cells, not graphemes; the bitmap
font is 5x7 ASCII with lowercase rendered as uppercase and a box for unsupported
characters. Terminal text retains Unicode. Headings share one text style.
Oversized layouts and bitmaps fail with an error instead of scrolling or paging.

No peer discovery, distributed lookup, authenticated identity, encrypted channel,
anonymous routing, production networking, protocol negotiation, multiplexing,
compression, cache or replay protection. Transport seams are API boundaries,
not implemented security features. See the versioned specs before extending them.

## Project vision

The [original project vision and roadmap](docs/vision.md) is preserved from the
initial GitHub README. The workspace, document/style parser, native renderer
and loopback YNP demo implement its first three milestones. The security and
routing milestones remain future work. This MVP uses `.ylang` sample files;
the original vision used the provisional extension `.yl`.
