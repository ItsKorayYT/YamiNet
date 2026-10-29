# YamiNet 0.1.0

An experimental Rust MVP for an independent document web. YLang is content,
YSL is presentation, YNP carries documents, and Yami Render performs native
layout and software rasterization. Yami Browser now includes a native desktop
interface with antialiased typography, tabs and an aqua glass theme.

**Not secure for sensitive use.** Networking is restricted to numeric loopback
IP addresses, plus private IPv4 LAN addresses for the authenticated transport
when a node explicitly opts in with `--lan`. Privacy is a design goal, not a
security guarantee. Nodes can have persistent Ed25519 identities that sign
site metadata. An experimental TLS 1.3 transport encrypts YNP and
authenticates a node against a node ID the client trusts in advance; the
browser uses it for `ynps://` addresses. The original `ynp://` transport
remains plaintext, unauthenticated and loopback-only. There is no anonymity.

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
`ynps://` addresses and localhost `ynp://` URLs. Tabs, Back, Forward, Reload, document links and
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

## Node identities

Each node can have a persistent Ed25519 identity. Its node ID, such as
`yn1-vitzizw2y2vq...`, is derived from the public key and stays the same across
restarts. A node can sign its site so others can check that the pages are
exactly what that node published:

```sh
cargo run -p yami-node -- identity new
cargo run -p yami-node -- identity sign-site sites/alice
cargo run -p yami-node -- identity verify-site sites/alice <node-id>
cargo run -p yami-node -- serve-dir 127.0.0.1:7301 sites/alice --identity <identity-directory>
```

Signatures only attribute content to a key. They do not encrypt or anonymize
anything, and fetched documents are not yet verified by the browser. The
secret key stays in a per-user directory and is never printed or sent. See the
[identity format and security limitations](docs/specs/v0.1/identity.md),
including the Windows file-permission limitation.

## Authenticated, encrypted connections (TLS 1.3)

A node with an identity can serve its site over TLS 1.3 instead of plaintext.
The node proves its persistent node identity in the handshake, and a client
connects only to the node ID it was given in the `ynps://` URL:

```sh
cargo run -p yami-node -- tls-demo ./demo-ids/one ./demo-ids/two
cargo run -p yami-node -- serve-dir 127.0.0.1:7443 sites/alice --identity ./demo-ids/alice --tls
cargo run -p yami-node -- fetch ynps://<node-id>@127.0.0.1:7443/home
```

`tls-demo` runs two nodes with different persistent identities that fetch each
other's document over TLS, and shows that a client expecting one node refuses
the other. A `fetch` with the wrong node ID fails without printing anything.
YNP frames are unchanged inside TLS and there is no plaintext fallback.
Keep demo identity directories out of commits; they hold secret keys. See the
[TLS transport, threat model and limitations](docs/specs/v0.1/ynp-tls.md).

### On a private LAN, in Yami Browser

With `--lan`, a TLS node listens on one private IPv4 address of its computer
(never `0.0.0.0`, never plaintext) and prints its full `ynps://` address:

```sh
cargo run -p yami-node -- serve-dir 192.168.1.50:7443 sites/alice --identity ./node-identity --tls --lan
```

On another computer, open the printed `Open in Yami Browser:` address in the
address field, or:

```sh
cargo run -p yami-browser -- --desktop ynps://<node-id>@192.168.1.50:7443/home
```

The browser connects only if the node proves exactly that node ID, then shows
a padlock, **Verified node · TLS 1.3** and the node ID under the address field.
Links on the page to the node's own address stay on the same verified node.
A different node ID fails with `Authentication failed`, and plaintext pages
are marked **Not encrypted · not verified**. The steps for two Windows
computers, including the firewall, are in [LAN testing](docs/lan-testing.md);
that two-machine procedure has not yet been run as part of development.

To find nodes without entering their IP addresses, start the node with
`--tls --lan --discoverable` (and its `--identity` directory). In the browser,
click **Find local nodes**, choose your private LAN interface, then click
**Search selected LAN**. Select a result to inspect its full claimed node ID,
then **Open homepage (ynps://)** to verify that key over TLS 1.3 and load its
homepage. Results stay marked untrusted; discovery does not recognize a node's
owner or add it to saved identities. Sites never open automatically.

Discovery uses one UDP multicast query per search to `239.255.89.78:39817`,
TTL 1, and unicast responses. It has no periodic announcements. Nodes without
`--discoverable` never answer. Configure `YAMINET_DISCOVERY_GROUP` and
`YAMINET_DISCOVERY_PORT` identically on both computers if needed. Discovery
metadata is visible to LAN observers and provides no anonymity. See
[Windows discovery/firewall testing](docs/discovery-testing.md) and the
[versioned discovery protocol](docs/specs/v0.1/lan-discovery.md).

## Workspace

| Package | Responsibility |
| --- | --- |
| `ylang` | Bounded document parser, typed AST, byte spans and line/column errors |
| `ysl` | Bounded original style parser and typed presentation settings |
| `ynp` | Binary frame codec, strict limits, transport traits, legacy plaintext loopback TCP client/listener |
| `ynp-tls` | YNP over TLS 1.3 (rustls), authenticated against a pinned node ID, loopback or opt-in private LAN |
| `ynp-discovery` | Opt-in IPv4 LAN multicast queries, bounded untrusted advertisements and rate-limited responses |
| `yami-render` | Original terminal/bitmap renderer plus shaped native document layout and antialiased text |
| `yami-browser` | Native desktop tabs/navigation and preserved terminal/PPM document tools |
| `yami-identity` | Persistent Ed25519 node identities, node IDs, domain-separated metadata signatures |
| `yami-node` | Directory-backed multi-page sites, in-memory document server, fetch client, identity CLI and two-node demos |

Versioned normative drafts: [YLang](docs/specs/v0.1/ylang.md),
[YSL](docs/specs/v0.1/ysl.md), [YNP](docs/specs/v0.1/ynp.md),
[node identity](docs/specs/v0.1/identity.md),
[YNP over TLS](docs/specs/v0.1/ynp-tls.md)
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
Identity tests cover persistence across restarts, refusal to overwrite or
silently replace keys, corrupted and mismatched key files, node ID checksums,
valid signatures, modified content, wrong keys, purpose separation, site
signing through the CLI, and that secret keys never appear in output.
TLS tests cover pinned fetches, wrong identities, expired and altered
certificates, copied certificates, tampering in transit, maximum-size
documents, no plaintext fallback, failed handshakes, reconnection and restarts,
and the two-node CLI demonstration. LAN tests bind a real private IPv4 address
of the test machine (client and node on that one machine) for encrypted
fetches, wrong identities, refused plaintext and refused addresses, and the
browser tests navigate verified sites and recover from failed authentication.
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

Peer discovery is limited to explicitly selected private IPv4 LAN interfaces.
There is no distributed lookup, anonymous routing, internet or
production networking, NAT traversal, protocol negotiation, multiplexing,
compression or cache. The TLS transport reaches loopback and private IPv4 LAN
addresses only, and has no revocation or client authentication: anyone on the
LAN who can reach a node can read its pages. The plaintext transport has no
authentication, encryption or replay protection and stays on loopback.
See the versioned specs before extending them.

## Project vision

The [original project vision and roadmap](docs/vision.md) is preserved from the
initial GitHub README. The workspace, document/style parser, native renderer
and loopback YNP demo implement its first three milestones. The security and
routing milestones remain future work. This MVP uses `.ylang` sample files;
the original vision used the provisional extension `.yl`.
