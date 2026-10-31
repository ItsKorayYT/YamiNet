# Yami Browser desktop

Yami Browser opens a native desktop window by default. Its aqua glass chrome,
rounded tabs, sky gradients and original new-tab artwork are drawn in Rust.
The glass reflections are software effects inside an opaque native window;
they do not imply operating-system background blur or transparency. The system
still provides the window title bar and its window controls.

The UI uses `winit` for windows and input, `softbuffer` for presentation,
`tiny-skia` for antialiased graphics, and `cosmic-text` for shaped, antialiased
text. A bundled sans-serif font makes basic typography available without an
Internet font download. No Chromium, WebView, HTML, CSS or JavaScript is involved.
The original YLang, YSL and YNP architecture remains intact.

**Experimental: not secure for sensitive use.** `ynp://` is plaintext,
unauthenticated and limited to numeric loopback addresses. `ynps://` addresses
use TLS 1.3 to the node ID named in the address and may point at a numeric
loopback, private IPv4 LAN or known public IPv4 address. Opt-in LAN discovery is available through
**Find local nodes** in the sidebar; select a private interface, search, inspect
the full untrusted ID, and explicitly open its `ynps://` homepage. The existing
TLS connection must prove that ID before requesting the page. Discovery never
opens sites automatically or saves trust. There is no anonymity, HTTP support
or DNS resolution. See [the security notes](../SECURITY.md) and
[Windows discovery/firewall testing](discovery-testing.md).

## Discover websites and directory subscriptions

Select **Discover** in the sidebar. **Websites** searches
only saved directory snapshots, using the existing bounded title, description,
category and address matching. Type a query and press Enter or **Search**. No
query leaves the browser. Starting the browser or opening the panel never
refreshes a remote directory. Select a result to enable **Open selected website ·
verify identity**. Previous/Next, Page Up/Page Down and the mouse wheel browse
pages. Escape closes the panel without closing tabs.

Each result shows its operator-supplied title, description, category, canonical
address, directory label and provenance. **Fresh** means a successful read at the
last explicit refresh, not a promise of current availability. **Cached** means
a refresh failed and the last validated snapshot was retained. Directory rows
include the reason. Loading, empty and error states leave browser navigation
available.

Under **Directories**, enter a local label and a canonical
`yami://FULL-NODE-ID/directory-key`, then **Add subscription (no fetch)**. An
omitted/empty key canonicalizes to `home`. New subscriptions stay inactive until
refreshed. Select a row for **Refresh selected directory** or **Remove selected**.
Removal deletes its local cached contribution, without deleting the remote
directory or other sources.

Remote refresh requires network browsing in **Settings**.
Local-only mode blocks new remote refreshes; offline search and local-file
refreshes remain available. Refresh reuses the browser's network policy and
bootstrap pins, signed DHT lookup and TLS pinned to the directory's exact
identity. **Relay fallback: On** permits existing signed relay routes after
direct endpoints fail, independently pinning relay and destination identities.
Storage and refresh work run off the UI thread, one operation at a time. An
already running refresh finishes using the transport's bounded timeouts;
closing the panel does not cancel it or start another refresh.

**Every listing remains untrusted, including listings in a freshly authenticated
directory.** A title, label, subscription or cache status never authenticates a
listed website. Opening a result uses normal browser navigation and independently
verifies that website's identity over TLS. Tabs, Back/Forward, bookmarks, history
opt-in and privacy preferences retain their existing behavior. Discover does not
enable history, save search queries or mark destinations verified.

The default registry/snapshot directory is `<browser-library-directory>/discover`.
Set `YAMINET_DISCOVER_DIR` to an absolute directory before launching to share an
existing `yami-discover` CLI store. The CLI's default `.yami-discover` directory
is not imported automatically. For example:

```sh
export YAMINET_DISCOVER_DIR=/absolute/path/to/discover-state
cargo run -p yami-discover -- source add community /absolute/path/to/community.ylang --state "$YAMINET_DISCOVER_DIR"
cargo run -p yami-discover -- source refresh community --state "$YAMINET_DISCOVER_DIR"
cargo run -p yami-browser
```

Local-file sources registered through the CLI also appear in the panel. Opening
it loads snapshots without rereading source files. Close and reopen the panel to
reload saved CLI changes. The store is independent of browser bookmarks/history;
see [directory authoring and the CLI](discover.md) for source commands.

## Rich sites, bookmarks and local history

Open a YLang 0.2 site using its canonical `yami://FULL-NODE-ID/home` or its
explicit pinned `ynps://` URL. The original native renderer now lays out rich
inline text, lists, cards, responsive rows and PNGs. Images appear in document
flow, wrap/scale within the viewport, preserve aspect ratio and scroll with the
page. Their same-node requests/decoding run on the existing loader workers;
the document becomes readable before slow images complete. Missing, invalid,
unsupported or oversized images show alternative text. Plaintext `ynp://` pages
never load assets. See [website limits](websites-0.2.md) and [site creation](publishing-sites.md).

Use **Bookmarks & history** in the existing sidebar. **Save current page** (or
Ctrl+D) edits a custom title in the address field; press Enter to save. Bookmark
rows open explicitly; **Title**, **Folder** and **Remove** organize entries.
Previous/Next pages through bookmarks. Folders are plain local labels sorted
with title/address. A `yami://` page retains its complete canonical identity;
an explicitly entered `ynps://` page retains its full identity and endpoint.
Saved titles never confer verification; every reopened page must prove its key.
**Use current address as Home** sets the existing Home shortcut (also Alt+Home).
Use a new tab and that button to restore the blank default Home.

History is **off by default**. Toggle it in the same panel to retain up to 512
successful network-page visits locally; the panel offers the last three visits.
Disabling stops recording, and **Clear history** removes all retained entries.
Back/Forward's existing per-tab in-memory navigation remains available whether
persistent history is enabled or not. History and bookmarks are never uploaded.
The full library is capped at 256 bookmarks and 256 KiB. It stores plaintext
addresses, titles, folder labels, visit times and preferences; anyone with access
to your account/backups can read them. Clearing is not secure disk erasure.

Default library location is `<peer-directory>/browser-library/library.tsv`;
`YAMINET_PEER_DIR` selects the usual peer root. For separate browser profiles,
set `YAMINET_BROWSER_DATA_DIR` to an **absolute** directory before launch. On
Windows, the default identity/peer roots follow the existing LOCALAPPDATA-based
locations; on Linux they follow XDG_DATA_HOME/HOME (see [identity storage](specs/v0.1/identity.md)).
Only one browser may write a given library at a time. Writes use a flushed
private temporary file and atomic rename; new Unix directories/files use
0700/0600, Windows inherits ACLs. A malformed/locked library is reported and
left intact while browsing remains available. Correct the file/path or restore
a trusted backup with the browser closed. Configuration-file DHT state paths
remain independent from this local library; use the environment override to
choose its directory explicitly.

Address editing supports Unicode-safe selection, Shift+arrows/Home/End,
Ctrl+A/C/X/V and horizontal caret scrolling. Internal `/about` links preserve
the current identity/transport. Explicit `yami://` links trigger a fresh bounded
lookup and exact identity authentication. Hover and pressed link colors use the
same shaped glyph rectangles as hit testing. Security indicators still come
only from the successful document connection, displaying direct or relayed TLS
with destination and relay identities separately.

## Launch on Windows

Use Windows 10 or Windows 11 with a native desktop session. These are build
instructions, not a claim that the application has been tested on Windows.

1. Install Git for Windows and Rust using the official Rust installer at
   <https://rustup.rs/>. Choose the default MSVC host toolchain.
2. Install Visual Studio Build Tools with **Desktop development with C++**,
   including the MSVC compiler tools and Windows SDK. Rust needs the native
   Microsoft linker when building this application.
3. Open a new PowerShell window after installation, then run:

```powershell
git clone https://github.com/ItsKorayYT/YamiNet.git
cd YamiNet
rustup toolchain install 1.90.0 --profile minimal --component rustfmt --component clippy
cargo run --release -p yami-browser
```

The repository pins Rust 1.90.0. Cargo downloads the native Rust dependencies
on the first build. Subsequent builds can use `--offline` once those dependencies
have been cached. No browser engine or JavaScript tooling needs to be installed.

After a release build, the executable can also be launched directly:

```powershell
.\target\release\yami-browser.exe
```

The initial page works without a node process. Choose the built-in Welcome or
About shortcut to read a real YLang document locally. The address bar understands
`local:welcome` and `local:about`; these are explicit built-in page identifiers,
not a new network protocol. The new-tab page is an internal UI page.

## Read documents over YNP

Open two more PowerShell windows in the repository directory. Start one sample
node in each, leaving both running:

```powershell
cargo run --release -p yami-node -- serve 127.0.0.1:7301 welcome examples/welcome.ylang examples/default.ysl
```

```powershell
cargo run --release -p yami-node -- serve 127.0.0.1:7302 about examples/about.ylang examples/default.ysl
```

In the desktop address bar, enter `ynp://127.0.0.1:7301/welcome` and press Enter.
The document's link opens `ynp://127.0.0.1:7302/about`. The homepage's sample-node
bookmarks also require these node processes; they show an error if no node is
listening. They do not pretend to be a public search or cloud service.

A desktop window can start directly at a YNP address:

```powershell
cargo run --release -p yami-browser -- --desktop ynp://127.0.0.1:7301/welcome
```

For two multi-page sites with links between them, run
`cargo run --release -p yami-node -- serve-dir 127.0.0.1:7301 sites/alice` and
`cargo run --release -p yami-node -- serve-dir 127.0.0.1:7302 sites/bob`
instead, then open `ynp://127.0.0.1:7301/home`. See
[Creating and publishing a YamiNet site](publishing-sites.md).

### Native identity addresses (`yami://`)

Enter `yami://<full-yn1-node-id>/home` without a server endpoint. The existing
DHT supplies signed locations; the browser then verifies the exact requested ID
over TLS 1.3 before fetching. Omitting the document key selects `home`.
There are no bundled bootstrap peers or aliases. Configure ordinary peers using
the existing peer store; see [native addressing](native-addressing.md) for
public/local settings, persistent state, links, tests and current security limits.
Native lookup runs in the existing background loader and retains the native URL
in history and the address bar. No browser private key is required.

### Verified nodes (`ynps://`)

Enter an address such as `ynps://yn1-...@192.168.1.50:7443/home`, as printed
by `yami-node serve-dir ... --tls` (add `--lan` for another computer on the
LAN). While connecting, the page area shows `Verifying node identity…`. The
page is requested only after the node proved exactly that node ID. Under the
address field the sidebar then shows a padlock, **Verified node · TLS 1.3**
and the full node ID; compare it with the ID the node printed. Plaintext
`ynp://` pages show **Not encrypted · not verified** instead.

Links on a verified page that name the node's own address and port open as
`ynps://` with the same node ID. If the node at the address has another
identity, the page shows `Authentication failed at <address>: ...` with both
IDs, and nothing is requested from it. Reload verifies the node again. See
[LAN testing](lan-testing.md) for two Windows computers.

Known remote nodes use the same address field, tabs, renderer, errors and
verification path: enter `ynps://<known-node-id>@<public-IPv4>:<port>/home`.
The remote node must explicitly use `--tls --public`; see
[VPS/forwarded-node testing](internet-testing.md). There is no DNS, automatic
public discovery or replacement of a configured pin. Manual bootstrap peers
can be managed through `yami-node peer`; browser navigation never imports
advertised peers into that list.

Stop each sample node with Ctrl-C in its terminal. Keep the plaintext sample
nodes local; remote experiments require the separate public TLS mode.
All peer application messages, including missing-document replies, use the existing binary YNP
protocol.

## Tabs, navigation and scrolling

The desktop browser has a new-tab button, close buttons, an editable address
bar, Back, Forward and Reload. Each tab keeps separate history and scroll state.
Opening another page after going Back discards the old forward branch. At most
12 tabs can be open. Closing the final tab leaves a fresh new-tab page.

YLang hyperlinks are clickable. Documents are laid out by the native extension
of `yami-render`, using the typed YLang AST and the document's YSL presentation.
The browser theme controls chrome and the new-tab page; it does not replace a
document's YSL. Bundled YSL arrives with a fetched document in the same YNP
response. Links do not fetch anything until explicitly activated.

Use the mouse wheel or the scrollbar to move through a long document. Resizing
reflows the document to the available viewport; window scale changes update
the framebuffer and text rendering for high-DPI displays.

| Shortcut | Action |
| --- | --- |
| Ctrl+L | Focus/select the address bar |
| Ctrl+D | Save current page with a custom bookmark title |
| Alt+Home | Open saved Home or the blank start page |
| Shift+arrows / Home / End | Extend selection in an editing field |
| Enter | Open the entered address |
| Ctrl+T | Open a new tab |
| Ctrl+W | Close the active tab |
| Ctrl+Tab | Select the next tab |
| Alt+Left / Alt+Right | Back / Forward |
| Ctrl+R | Reload the current page |
| Ctrl+A / Ctrl+C / Ctrl+V | Select all, copy or paste in the focused address field |
| Escape | Leave address editing |

Loading and failure states are visible in the active tab. Network access and
source parsing run on two background workers with bounded pending work.
Completions carry the stable tab ID and navigation generation; a completion
from an older navigation or a closed tab cannot overwrite the current page.
Cancellation removes queued work and stops subsequent image requests; an
in-flight socket may finish or reach its strict deadline before releasing its
worker. Generation checks discard all obsolete results.

## Linux and headless tools

The Linux desktop build uses X11, including XWayland in a Wayland session.
A native Wayland backend is not enabled. A working graphical display is required
to open the window. No GPU or web engine is required for the software rendering
path.

The terminal and original PPM renderer remain available for debugging and
regression checks. A positional file or YNP URL retains the original terminal
behavior; `--desktop` explicitly chooses a window for an initial URL:

```sh
cargo run -p yami-browser -- --terminal
cargo run -p yami-browser -- examples/welcome.ylang --style examples/default.ysl
cargo run -p yami-browser -- ynp://127.0.0.1:7301/welcome
cargo run -p yami-browser -- examples/welcome.ylang --ppm /tmp/yami.ppm
```

The PPM backend intentionally keeps its original tiny bitmap font. It is a
regression and debugging output, not a screenshot of the desktop interface.

## Native window captures

These 1440×900 images are rendered offscreen by the same paint code the window
uses. Regenerate them with
`cargo test -p yami-browser capture_screenshots -- --ignored`, which writes to
`target/screenshots/`. They are documentation only; the application paints its
own interface and does not use these captures as assets.

![Native YamiNet start page](images/desktop-home.png)

![YNP document in the native renderer](images/desktop-document.png)

## Validation and current limits

From the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

Tests that bind loopback sockets need permission to do so. A successful build
is not visual validation: launch the actual window and inspect it at normal
and high-DPI scale, resize it, open multiple tabs, navigate between the two sample
nodes, click a document link, and test scrolling and error recovery.

This is a local document browser, not a conventional public web browser. There
is no search service, weather service, account system, cloud connection,
extension system, download manager or automated remote content loading. The
homepage's scenic elements are decoration. Bookmarks are bundled sample
shortcuts, not a synchronized account feature. There is no document editing,
complete accessibility tree or production network hardening.

### Current cloud validation status

The native dependencies are resolved in `Cargo.lock`. The complete workspace
build and Clippy with warnings denied pass, and 38 tests pass. Tests cover the
navigation model, tab/history behavior, stale-response rejection, input and
resource bounds, small-window hitboxes, native link geometry, clipped raster
output and high-DPI layout, alongside the preserved parser, YNP and terminal/PPM
regression checks.

A real Linux native window was launched under Xvfb. The rendered homepage and
YLang document viewport were visually inspected. Interaction checks passed for
Back, Forward, Reload, opening and switching tabs, built-in document navigation,
localhost YNP retrieval and an error state. This is actual window validation,
in addition to the offscreen homepage inspections at 1180×800 and 720×540.

Further release-window checks passed for mouse-wheel scrolling, minimum-size
resizing, document hyperlinks, toolbar buttons and tab closing. A controlled
YNP peer delayed its reply while the UI opened another tab and navigated to a
new page; the eventual obsolete reply did not replace that page. Reload also
recovered successfully after a previously unavailable sample node was started.

The actual native window was tested with simulated 200% scaling: the framebuffer
size, shortcut hitboxes, Home control and minimum logical resize were verified.
This does not replace testing on physical high-DPI hardware. Windows runtime
behavior has not been tested; the Windows commands above remain launch
instructions rather than Windows validation evidence.


### Relay fallback settings

The existing sidebar includes **Relay fallback: Off/On** and **Add bootstrap
peer**. Relay fallback defaults off; its toggle affects subsequent attempts.
To add an ordinary peer, choose Add bootstrap peer, paste its complete
`yn1-...@IPv4:port` and press Enter. This saves an unverified pin without opening
content. `--bootstrap ID@IPv4:port` and `--enable-relays` provide startup settings.
A verified relayed page shows **Verified node · TLS 1.3 · Relayed** and both full
IDs separately. The destination is still pinned to the original yami:// address.
See [Windows outbound-only hosting and public testing](public-relay.md).

## Desktop alpha 0.3

For installation without Rust, first launch, data locations and upgrades, see
[installation.md](installation.md). The UI retains its existing native design.
Settings provides local-only mode, explicit network browsing, relay preferences
and preference reset. It opens only from the sidebar. Config/bootstrap management remains local.

Ctrl+L focuses/selects the address; Ctrl+T opens a tab; Ctrl+W closes it;
Ctrl+R/F5 reload; Alt+Left/Right navigate; Ctrl+Tab/Shift+Ctrl+Tab switch tabs;
Ctrl+D bookmarks; Ctrl+Shift+C copies the committed current address when no
editor is focused. Ctrl+F opens find; Enter/Shift+Enter or F3/Shift+F3 advances
matches; Escape closes find/dialogs. Editors support Ctrl+A/C/X/V, Shift+arrows,
Home/End, deletion and Unicode/IME. macOS uses Command for command shortcuts,
but macOS packaging is outside this milestone.

Find searches rendered text locally, with exact Unicode matching. It does not
search addresses/source syntax or send queries to peers. The current result is
highlighted and scrolled into view; counts and previous/next controls appear in
the find bar. Tab and Shift+Tab focus native controls and visible document links;
Enter activates the focused control. See [accessibility.md](accessibility.md)
for the tested scope and missing native accessibility tree.
