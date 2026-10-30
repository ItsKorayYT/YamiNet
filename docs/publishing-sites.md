# Creating and publishing a YamiNet site

**EXPERIMENTAL. Not secure for sensitive use.** Publishing means serving your
own directory through a node whose persistent identity you control. Preview
is loopback-only. LAN, public listeners and outbound relay hosting are separate
explicit choices; there is no YamiNet-operated upload service.

## Create, validate, preview and build a 0.2 site

After building once with `cargo build --release --workspace --locked`, use
`target/release/yami-node` directly. No Rust knowledge is needed to edit the
plain text files. On Windows use the `.exe` names in the walkthrough below.

```sh
target/release/yami-node site new my-site personal
target/release/yami-node site check my-site
target/release/yami-node site info my-site
target/release/yami-node site preview my-site --identity ./my-node-identity
```

`new DIRECTORY [personal|blog|portfolio]` defaults to `personal` and refuses an
existing destination. Each template creates `home.ylang`, `about.ylang`,
`default.ysl` and `assets/cover.png`, with real 0.2 syntax, rich text, lists,
cards and responsive rows. No identity is created in the generated site.

`check` validates every page and shared style with the production parsers,
checks local page links, full external identity IDs, PNG references, decoder
limits and per-page resource budgets. It performs no network requests. `info`
validates too and displays page keys, PNG keys and total bytes. Both accept
0.1 sites subject to the browser's existing link policies. Symlink assets,
special filesystem names, unsafe keys and inventories over 256 pages/assets
are refused. Unknown unrelated files are omitted from export.

`preview` validates before binding a random loopback TCP port with TLS 1.3. Copy
its complete `ynps://FULL-ID@127.0.0.1:PORT/home` line into Yami Browser, or run:

```sh
target/release/yami-browser --desktop 'ynps://FULL-ID@127.0.0.1:PORT/home'
```

Use the actual output, including the full ID and chosen port. Preview never
opens a browser automatically, enables discovery/DHT, listens publicly or
reconfigures a router. Omit `--identity` to use the existing default identity
location documented in [configuration](configuration.md); an explicit directory
is easier to back up. Existing keys are loaded, never overwritten. Protect this
directory and keep it **outside the site**. Ctrl+C stops the preview and drains
the bounded workers. Edit the site, run `check`, and reload; the existing node
reads files on each request. The checker is a snapshot, not a filesystem watcher.

```sh
target/release/yami-node site build my-site published-site
```

`build` requires a new output directory outside the source. It exports only
validated `.ylang` files, `default.ysl` and PNG assets, then validates the export.
It omits private keys, DHT state, manifests and unrelated files. It never changes
the source identity. The site directory must be locally trusted: do not allow
untrusted local users to mutate it during validation/export/serving.

## Publish through your existing node

Validate/build first, then use the existing commands. For loopback TLS:

```sh
target/release/yami-node serve-dir 127.0.0.1:7443 published-site --tls --identity ./my-node-identity
```

For a home host behind NAT, load the same identity and explicitly trust your
independently operated relay and DHT bootstrap peers:

```sh
target/release/yami-node host-relay published-site --identity ./my-node-identity --relay RELAY-ID@PUBLIC-IPv4:7444 --dht-dir ./my-node-state
```

Configure bootstrap peers and the browser's relay opt-in as described in the
[public relay guide](public-relay.md) and [configuration guide](configuration.md).
The stable address is `yami://YOUR-FULL-NODE-ID/home`. `/about` links keep that
identity automatically. Assets use that same destination pin, including through
the reverse relay; there is no HTTP asset server or additional exposed port.
Public direct serving still requires `--tls --public` and a deliberately
selected address. No command changes firewall rules or router configuration.

Preserve the identity and DHT state directories during upgrades and updates;
never replace them with template/build output. Back them up privately and stop
all processes using them before copying. Replacing the node key changes every
identity address/bookmark. Restoring old sequence state can cause DHT rollback
rejections: follow [safe restart/upgrade guidance](configuration.md). Replace
published content from a validated export while the node is stopped, then
restart with the same identity/state and options. Private keys never belong in
a public site directory. Site signature manifests retain their original scope
(page/style metadata); TLS authenticates each asset connection, not a new
per-asset signing scheme.

## Windows walkthrough (PowerShell)

Install/build as described in [desktop-browser.md](desktop-browser.md). From
an extracted checkout, build once, then run the native executable commands:

```powershell
cargo build --release --workspace --locked
$node = ".\target\release\yami-node.exe"
$browser = ".\target\release\yami-browser.exe"
& $node site new "$HOME\YamiSite" portfolio
& $node site check "$HOME\YamiSite"
& $node site preview "$HOME\YamiSite" --identity "$HOME\YamiIdentity"
# In a second PowerShell window, paste the actual preview URL:
& $browser --desktop 'ynps://FULL-ID@127.0.0.1:PORT/home'
```

Edit `.ylang` / `.ysl` files using a UTF-8 text editor; keep the exact two-space
indentation. Copy PNGs to `assets` with safe names such as `portrait.png`, then
reference `! portrait | A portrait of the author`. Re-run `site check`. The
preview is local and needs no inbound firewall exception. Keep the identity
folder private using Windows account ACLs; Unix permission checks do not apply.
Stop preview with Ctrl+C, export to a new directory, then use the same relay
or deliberately selected LAN/public TLS hosting options on the exported site.
Windows runtime and real home-router validation remain unverified; see the
[executable cross-network plan](cross-network-validation.md).

## Troubleshooting

- Parser errors name the file and line/column: check the version header,
  two-space indentation and the [0.2 examples/property limits](websites-0.2.md).
- A missing local link/image fails `site check`. Image keys omit `.png`, paths
  and URLs; PNGs must satisfy compressed, dimension, pixel and page budgets.
- Images need an authenticated parent. A legacy `ynp://` preview shows alternative
  text; restart with `site preview` or `--tls` and open the printed pinned URL.
- Old nodes/browsers can refuse 0.2 documents/assets. Upgrade both; no plaintext
  downgrade is attempted. APNG, JPEG, GIF, SVG and remote image URLs are unsupported.
- For failed `yami://` lookup or relays, inspect the node/relay status and bootstrap
  settings using [network diagnostics](cross-network-validation.md). A familiar
  title or saved bookmark is never proof of identity.

## Legacy 0.1 directory workflow

A YamiNet site is a directory of YLang pages plus one shared YSL stylesheet.
`yami-node serve-dir` serves every page in that directory over YNP.

## 1. Lay out the directory

```text
sites/
  alice/
    home.ylang
    about.ylang
    projects.ylang
    default.ysl
```

| File | Becomes |
| --- | --- |
| `<key>.ylang` | The page `ynp://<address>/<key>` |
| `default.ysl` | The stylesheet sent with every page of the site (optional) |

A page key is 1 to 128 ASCII letters, digits, `_` or `-`, exactly as in the
[YNP draft](specs/v0.1/ynp.md). The file name must match the key exactly,
including letter case, even on case-insensitive file systems. Other files,
subdirectories, symlinks and names such as `con` or `com1` are never served.
Without `default.ysl` the browser uses its built-in default style.

## 2. Write pages

Legacy pages use the unchanged [YLang 0.1](specs/v0.1/ylang.md) syntax:

```text
@page 0.1
  title Alice's Home
  language en

= Alice on YamiNet
A small multi-page site served from one directory by one local node.
:: Pages on this node
  > ynp://127.0.0.1:7301/about | About Alice
:: Neighbours
  > ynp://127.0.0.1:7302/home | Visit Bob's home page
```

Links are always absolute `ynp://` addresses. Use the node's own address to
link within the site, and another node's address to link across sites. The
browser fetches a linked page only when the reader opens it.

Check a page before publishing it; parse errors include line and column:

```sh
cargo run -p yami-browser -- --terminal sites/alice/home.ylang --style sites/alice/default.ysl
```

## 3. Serve it

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7301 sites/alice
```

The node lists the page addresses it found and logs each request. The address
must be a numeric loopback IP (`127.0.0.1` or `[::1]`); anything else is
refused. Port `0` picks a free port and prints it. `--workers N` (1 to 32,
default 4) sets how many connections are handled at once; further clients wait
in the operating system's queue. Stop the node with Ctrl-C.

Open the site in the desktop browser, or fetch a page's source:

```sh
cargo run -p yami-browser -- --desktop ynp://127.0.0.1:7301/home
cargo run -p yami-node -- fetch 127.0.0.1:7301 about
```

## 4. Update it

The node reads the page and `default.ysl` again on every request, so edits,
new pages and deletions take effect without a restart: save the file and press
Reload in the browser. A page that is half-written at the moment of a request
may fail to parse; Reload again, or save through a temporary file and rename it.

## 5. Sign it (optional)

A node identity lets readers check that a site's files are exactly what you
published. Create an identity once, then sign the site after every change:

```sh
cargo run -p yami-node -- identity new
cargo run -p yami-node -- identity sign-site sites/alice
```

This writes `sites/alice/site.sig`, which covers every page and `default.ysl`.
Someone with a copy of the directory checks it against the node ID you gave
them yourself:

```sh
cargo run -p yami-node -- identity verify-site sites/alice yn1-...
```

Any edited, added or removed page makes verification fail until you sign again.
`serve-dir ... --identity <identity-directory>` prints the node's ID at startup
and reports whether `site.sig` still matches. Signing does not encrypt the site
or hide who published it, and the browser does not check signatures yet. See
[node identity](specs/v0.1/identity.md) for the format and limitations. Never
copy `node.key` into a site directory or repository.

To serve the site over encrypted, authenticated TLS 1.3 instead, add `--tls`.
Readers then use the printed `ynps://<node-id>@<address>/<key>` URLs, and a
fetch succeeds only from the node holding that ID's key:

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7443 sites/alice --identity <identity-directory> --tls
cargo run -p yami-node -- fetch ynps://yn1-...@127.0.0.1:7443/home
```

Open these URLs in Yami Browser's address field, or with
`cargo run -p yami-browser -- --desktop ynps://yn1-...@127.0.0.1:7443/home`.

To let other computers on your private network read the site, serve it on
this computer's private IPv4 address with `--lan` (which requires `--tls`):

```sh
cargo run -p yami-node -- serve-dir 192.168.1.50:7443 sites/alice --identity <identity-directory> --tls --lan
```

Links inside pages stay `ynp://` addresses. On a page opened over `ynps://`,
the browser opens links to the node's own address and port as `ynps://` with
the same node ID, so a LAN site links its pages with that address, for example
`> ynp://192.168.1.50:7443/about | About`. See [LAN testing](lan-testing.md)
for the firewall and a two-computer walkthrough, and
[YNP over TLS](specs/v0.1/ynp-tls.md) for the threat model.

## Two sample sites

The repository includes two linked sample sites. Run each node in its own
terminal:

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7301 sites/alice
```

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7302 sites/bob
```

Then open either home page and follow links between the two nodes:

```sh
cargo run -p yami-browser -- --desktop ynp://127.0.0.1:7301/home
```

`sites/alice` also contains `welcome`, so the browser's pinned node tile
(`ynp://127.0.0.1:7301/welcome`) opens a page on Alice's node.

![Alice's home page served by serve-dir](images/site-alice-home.png)

![Bob's garden page on the second node](images/site-bob-garden.png)

To regenerate these screenshots, which serve both samples on ports 7301 and
7302 and follow rendered links across the nodes:

```sh
cargo test -p yami-browser --bin yami-browser -- --ignored capture_sites
```

The images are written to `target/screenshots/`.

## What the node will and will not serve

- Only `<root>/<key>.ylang` files whose canonical path is exactly inside the
  site root under that name, and `<root>/default.ysl`.
- Keys with `/`, `\`, `.`, `:`, `%`, spaces, non-ASCII or control bytes are
  rejected by the YNP decoder before the file system is touched.
- Symlinks that point outside the site or to another name, directories, and
  non-regular files are refused, as are files over the YNP limits (256 KiB per
  page, 16 KiB of style) or with invalid UTF-8. The connection is closed and the
  reason is logged on the node.
- A missing page gets a YNP NotFound reply.
- A leading UTF-8 byte-order mark is removed; otherwise files are sent as is.

Anyone who can write to the site directory controls what it serves, including
through hard links. Keep the directory private to you.

## Guided alpha setup (no development tools required)

From the extracted package, run (quote actual paths with spaces):

```
yami-node site setup /path/to/new-site --new personal --identity /private/yaminet/my-identity --output /private/yaminet/host.conf
```

Windows example, in PowerShell:

```powershell
.\yami-node.exe site setup "$env:USERPROFILE\Documents\MyYamiSite" --new personal --identity "$env:LOCALAPPDATA\YamiNet\my-host-key" --output "$env:LOCALAPPDATA\YamiNet\my-host.conf"
```

Omit `--new personal` to select an existing site. `blog` and `portfolio` are also
available templates. Setup uses the production validator, creates or loads the
persistent identity, validates the networking profile, saves a **new** config
without overwriting one, and prints the canonical address. It performs no network
requests or hosting by default. Keys must be outside the site/application tree.
Local mode listens on loopback port 7443 with a local DHT advertisement when
explicitly started; the printed canonical address is not proof of resolution.
For a local browser use an explicit local config/bootstrap, or the direct
`ynps://ID@127.0.0.1:7443/home` address. Port conflicts require editing the saved
listen/advertise ports together before starting.

Add `--preview` for a loopback TLS preview using the existing preview command.
Add `--start` to start the selected host explicitly. Otherwise run
`yami-node --config /private/yaminet/host.conf` later. Preview and start are
mutually exclusive. Add a full trusted `--relay ID@public-IPv4:port` pin and one
or more `--bootstrap ID@public-IPv4:port` pins for outbound relay hosting.
This creates a public-network profile without opening an inbound home listener.
Choose and verify those services yourself; none are bundled. Use
`yami-node diagnose --config FILE` for offline preflight and snapshots, and
`--connect` for bounded explicit pin checks. A registration/publication snapshot
may be stale; prove document and PNG delivery from an independent connection.

### Optional persistent Windows hosting

Use Task Scheduler's **Create Task** under your ordinary user account. Select
**Run only when user is logged on** initially; leave highest privileges disabled.
Program: the absolute packaged `yami-node.exe`; arguments:
`--config "C:\Users\you\AppData\Local\YamiNet\my-host.conf"`; set **Start in**
to the application directory. Choose a manual trigger first and inspect host
and DHT status with the diagnostic CLI. The node does not require an open
development terminal. Enabling a logon/startup trigger is a separate opt-in action.
Task Scheduler history and node health snapshots help diagnose failures; protect
any redirected logs because they may contain operator filesystem paths.
Disable and remove the task before uninstalling or moving the application.
No task/service or firewall rule is created by the package or setup command.
