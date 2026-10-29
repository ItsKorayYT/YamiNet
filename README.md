# YamiNet 0.1.0

An experimental Rust MVP for an independent document web. YLang is content,
YSL is presentation, YNP carries documents, and Yami Render performs native
layout and software rasterization. Yami Browser now includes a native desktop
interface with antialiased typography, tabs and an aqua glass theme.

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
cargo run -p yami-node -- serve-dir 127.0.0.1:7301 sites/alice
```

In the prepared cloud environment, first activate the retained toolchain in
each shell (ordinary local Rust installations do not need this):

```sh
export CARGO_HOME=/workspace/.yami-tools/cargo
export RUSTUP_HOME=/workspace/.yami-tools/rustup
export PATH="$CARGO_HOME/bin:$PATH"
cd /workspace/YamiNet
```

The default command opens the native desktop browser. Its new-tab page has
local document shortcuts and sample-node bookmarks; the address bar accepts
existing localhost YNP URLs. Tabs, Back, Forward, Reload, document links and
scrolling operate on actual YLang documents. Network work runs in bounded
background workers, with stale-response protection.

See [Desktop browser and Windows launch instructions](docs/desktop-browser.md)
for installation, controls, native rendering details and platform limitations.
The terminal and PPM paths remain available: use `--terminal` for the built-in
sample, or retain the original positional file/URL commands. `--desktop` opens
a supplied YNP URL in the window. `--ppm` writes the original P6 bitmap output.
No HTML, CSS, JavaScript, Chromium, WebView or web rendering engine is used.
The viewer does not execute content or automatically fetch links, fonts, images
or other assets. A fetched YNP document includes its style as inert text in
the same frame; the terminal viewer supports a local `--style` override.

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
cargo run -p yami-browser -- --desktop ynp://127.0.0.1:7301/welcome
cargo run -p yami-browser -- ynp://127.0.0.1:7302/about --ppm /tmp/yami-about.ppm
cargo run -p yami-node -- fetch 127.0.0.1:7302 about
```

Stop persistent nodes with Ctrl-C. All application network messages use YNP,
including missing-document replies. There is no HTTP endpoint or fallback.
The single-document `serve` command keeps one in-memory document and handles
connections sequentially; `serve-dir` (below) serves a whole site concurrently.
Malformed connections are rejected without terminating the process.

## Multi-page sites

A node can serve a whole site from a directory: each `<key>.ylang` file is the
page `ynp://<address>/<key>`, and `default.ysl` is the site's shared style.
Files are read on each request, so edits appear on Reload without restarting
the node. A fixed worker pool (default 4) serves concurrent requests. Keys never
become arbitrary paths; traversal, symlink escapes, case aliases and device
names are refused. Two linked sample sites are included; in two terminals run:

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7301 sites/alice
```

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7302 sites/bob
```

Then open `ynp://127.0.0.1:7301/home` in the browser and follow the links
between both nodes:

```sh
cargo run -p yami-browser -- --desktop ynp://127.0.0.1:7301/home
```

See [Creating and publishing a YamiNet site](docs/publishing-sites.md) for the
directory layout, linking, updating and exactly what a node will serve.

## Workspace

| Package | Responsibility |
| --- | --- |
| `ylang` | Bounded document parser, typed AST, byte spans and line/column errors |
| `ysl` | Bounded original style parser and typed presentation settings |
| `ynp` | Binary frame codec, strict limits, transport traits, loopback TCP client/listener |
| `yami-render` | Original terminal/bitmap renderer plus shaped native document layout and antialiased text |
| `yami-browser` | Native desktop tabs/navigation and preserved terminal/PPM document tools |
| `yami-node` | Directory-backed multi-page sites, in-memory document server, fetch client and two-node demo |

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

See the [current desktop validation status](docs/desktop-browser.md#current-cloud-validation-status)
for tested Linux window interactions and remaining platform checks.

The first desktop build downloads native Rust library dependencies. After
those dependencies have been cached, `--offline` also works.
The integration tests require permission to bind and connect loopback sockets.
They use ephemeral ports and include real subprocesses, two-node exchange,
malformed-client recovery, missing documents, and fetch/parse/style/raster output.
Site tests cover multi-page serving, live updates, NotFound replies, invalid
keys, traversal and symlink escapes, concurrent clients, and browser
navigation across two directory-backed nodes.
Unit tests cover parser locations and limits, nested ASTs, wrapping and pixels,
golden wire bytes, all truncated prefixes of representative frames, malformed
headers and payload lengths, Unicode, size boundaries and response correlation.

## MVP limitations

The desktop interface is a software-rendered native window with tabs,
navigation, shaped text and vertical scrolling. Its glass effect is drawn
inside the window; it does not blur other applications behind it. Linux uses
X11/XWayland. Windows build instructions are documented separately.

There is no image embedding, full accessibility tree or document editing.
Sites are plain directories; there is no cache, index or upload mechanism. The debugging terminal/PPM backend still counts
Unicode scalars as fixed-width cells. Its original 5x7 bitmap font maps lowercase
to uppercase and unsupported characters to a box; this limitation does not
describe the desktop font renderer. Oversized layouts and bitmaps fail with
an error rather than allocating without bounds.

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
