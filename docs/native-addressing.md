# Native identity addresses in Yami Browser

Enter `yami://<full-node-id>/home` in the existing desktop address bar. The
node ID is the complete `yn1-...` Ed25519 identity, including its checksum.
The canonical address contains an identity and document key, never an IP.
Omitting the key (`yami://<id>` or `yami://<id>/`) selects `home`. Keys use the
existing YNP rule: 1–128 ASCII letters, digits, underscores or hyphens.
Case changes to an ID, shortened IDs, aliases, ports, user information,
queries, fragments, percent escapes, whitespace and extra path segments are
rejected. There is no human-readable naming service.

The browser calls the existing `yami-dht` crate directly. Its two bounded
loader workers perform cache access, lookup, TLS, YNP and parsing off the UI
thread. A valid cached location is tried first. Missing locations and failed
cached connections trigger a bounded DHT lookup. Every learned record passes
the DHT's existing signature, endpoint-policy, expiration and monotonic
sequence checks. A failed cached location is replaced only by a higher-sequence
signed record. A reachable old endpoint remains usable until its record expires;
Reload does not force a network-wide search while a cached location works.
During refresh, an equal/older reply cannot end iteration before another peer
can provide a newer record; the same global lookup budget still applies.

Each document connection uses TLS 1.3 pinned to the exact ID in the original
address. No YNP request is sent until that ID is proved by the handshake.
The browser rechecks record expiration before each endpoint attempt and before
requesting a document. A signature on a location is never proof of the
connection's identity or of availability. The address field, history and tabs
retain the `yami://` address. Existing explicit `ynps://` URLs still work.

## Configure ordinary peers and persistent state

No bootstrap peers are bundled. Obtain a full ID and numeric endpoint for an
ordinary compatible DHT node through a channel you choose, then use the existing
peer configuration command. The browser uses `YAMINET_PEER_DIR`, or the same
per-user peer directory as `yami-node peer` when the variable is absent.
For a public experiment with reachable nodes you control:

```sh
cargo build --workspace --locked
export YAMINET_PEER_DIR=/absolute/path/to/browser-public-peers
export YAMINET_BROWSER_DHT_NETWORK=public
target/debug/yami-node dht bootstrap '<B-full-ID>' '<B-public-IPv4>:7443' --dir "$YAMINET_PEER_DIR"
target/debug/yami-browser --desktop 'yami://<A-full-ID>/home'
```

Replace the placeholders. B must have opted into DHT service; A must publish
its signed location into the reachable DHT. A server uses
`serve-dir ... --identity <dir> --tls --public --dht`; see
[DHT setup](dht-testing.md) for publishing and advertised endpoints.
The browser does not start a listener or publish a self record.

The browser reads the existing `peers.tsv` manual pins and filters them through
the selected DHT network policy. It does not change pins or write authentication
status to the manual store. Pins outside that policy are not queried. Configure
or remove them through `yami-node dht bootstrap` / `yami-node peer remove`.
Pins are sampled on each required lookup, so a bootstrap configuration added
after an error can be used by Reload. Network/directory settings are sampled
on the first native request; restart the browser to change those settings.

Persistent browser state is `dht-public-browser.cache` or
`dht-local-browser.cache` inside the peer directory. These are separate from
the node server and CLI client caches, with separate lifetime writer locks.
Public and local snapshots also have different encoded policy tags, so using
a snapshot under the wrong policy fails closed. Use separate directories for
public and local manual pins as well. One browser process owns each browser
cache; additional processes should select a separate peer directory. Tabs and
workers within one process share the cache. A locked, corrupt or unavailable
state file produces a browser error without exposing its path or contents.

Lookup does **not** require a browser private key. The current DHT authenticates
servers and allows identity-free clients. The existing `Service` needs a public
ID only as its local routing origin/cache owner. `Service::open_lookup` generates
that origin through the existing identity implementation, immediately discards
the unused private key, and retains only the public origin in signed-cache
snapshots. Reopening a snapshot reuses that public origin. No `node.key`, client
certificate, persistent signing identity or new cryptography is required.

Keep snapshots across normal restarts: they retain signed sequence high-water
marks even after records expire. Deleting a snapshot deliberately discards that
rollback history. The cache accepts at most 256 IDs and fails closed for new
IDs at capacity. Bootstrap pins are local configuration; protect that directory
against unauthorized modification.

## Local three-node reproduction

Follow the existing [three-node local simulation](dht-testing.md#manual-three-node-windows-session)
to start A, B and C with `--tls --dht --dht-local`, with A publishing to B.
Configure a separate browser directory with only B's full ID and endpoint.
For the example ports from that guide, in PowerShell:

```powershell
$env:YAMINET_PEER_DIR = Join-Path $PWD 'target\dht-demo\browser-local'
$env:YAMINET_BROWSER_DHT_NETWORK = 'local'
.\target\dht-validation\debug\yami-node.exe dht bootstrap $bId 127.0.0.1:17442 --local --dir $env:YAMINET_PEER_DIR
cargo run -p yami-browser -- --desktop "yami://$aId/home"
```

On Linux/macOS, use `export YAMINET_PEER_DIR=...` and
`export YAMINET_BROWSER_DHT_NETWORK=local`, then the equivalent node command
and `cargo run`. Public is the default; local requires explicit selection.
The local DHT policy allows only IPv4 loopback endpoints, and public mode
refuses private/LAN/loopback/special-use endpoints. Neither mode does DNS.
The browser needs B's endpoint in configuration and A's **ID** in its address
bar; it does not need A's endpoint. To restore public defaults, remove the
local environment setting and select the public peer directory.

The terminal/PPM viewer also accepts `yami://` using the same resolver and
environment settings, for example `cargo run -p yami-browser --
'yami://<A-ID>/home' --ppm target/native-page.ppm`. Its explicit terminal
command waits for the bounded fetch; the desktop uses background workers.

## Document links and navigation

The browser recognizes native targets in existing inert link nodes:

```text
> yami://<full-node-id>/about | About this node
> yami://<another-full-node-id>/home | Another node
```

This is a browser address adapter; the YLang and YSL crates and language
specifications are unchanged. It preserves byte lengths and source spans while
passing the document through the existing YLang parser, then restores native
targets in the parsed link nodes. Standalone YLang tools still apply the 0.1
grammar. For compatibility with those tools, `> ynp://<full-node-id>/about |
About` is also resolved as a native identity target when followed from a
verified native page. An old numeric `ynp://IP:port/key` link matching that
page's authenticated endpoint becomes `yami://<same-id>/key`. Other plaintext
links from native pages are refused; use an explicit identity address for the
destination. Links never run lookups until clicked. Back, Forward, Reload,
tabs and scrolling use the existing browser model.

## Errors, tests and current limits

The browser distinguishes no configured bootstraps, unknown nodes, expired or
invalid records, unavailable nodes, TLS identity mismatch and DHT timeout.
An expired cached target gets an expiry error if no fresh record is learned.
A peer that omits an unknown/expired record cannot prove whether that node ever
existed; a clean reply without a target is reported as unknown. Invalid record
diagnostics may describe a rejected response batch, since DHT batch validation
is atomic. No keys or sensitive local configuration appear in these errors.

```sh
cargo test -p yami-browser --test native_dht --locked -- --nocapture
cargo test -p yami-node --test dht --locked -- --nocapture
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

`native_dht` runs actual browser-model jobs against three real loopback TLS/DHT
nodes using the existing node dispatcher and A/B/C publication topology. It
checks rendered YLang, native same-node/cross-node links, old numeric links,
history, reload, tabs/scrolling, persisted cache reopening without a private
key, bootstrap-offline cache use, higher-sequence endpoint changes, rejected
rollback, wrong TLS identity before any document request, offline/expired
nodes, forged records, unknown nodes, missing bootstrap and timeout errors.
A stalled bootstrap also verifies that another tab can load while lookup waits.
The existing three-process node simulation remains part of workspace tests.

On 2026-10-30, Rust 1.90.0 on Linux passed formatting, all-target workspace
Clippy with warnings denied, the locked workspace build, and the locked
workspace suite (150 passed, 8 existing opt-in tests ignored). The native
browser-model/terminal integration and three-process node simulation ran;
owned public endpoints, selected-interface multicast and manual visual capture
tests remain opt-in. This run does not establish public-network availability
or native desktop visual correctness on another platform.

DHT RPCs have a three-second total connection/I/O budget; lookup is bounded to
15 seconds and 16 queries. Each fetch pass tries at most four signed endpoints
with a three-second budget each. A failed cache pass, lookup and refreshed
fetch pass can therefore take roughly 39 seconds plus bounded local work.
At most two obsolete requests can remain in flight; queued same-tab jobs are
replaced and stale completions cannot change the current page. There is no
unbounded retry, background crawl or periodic maintenance.

This remains an experimental, small DHT, without guaranteed availability.
Owners must publish fresh records; they expire within 24 hours. Serving nodes
can enable [automatic maintenance](dht-maintenance.md) for bounded renewal and
peer recovery; the browser itself remains an on-demand lookup client. Correct local
time matters. Cold clients cannot know the globally newest sequence. Malicious
peers can omit records, fill caches, observe lookups or attempt Sybil/eclipse
attacks. Peers see source IPs and requested IDs; neither lookup nor TLS provides
anonymity. No NAT traversal, Tor/I2P, aliases, central naming service, revocation,
independent security review or production safety is provided. The validation
here is a one-computer loopback simulation, not evidence of public internet
reachability. See [the DHT security specification](specs/v0.1/dht.md) and
[SECURITY.md](../SECURITY.md).

## Relay fallback

Relay use is off by default. Enable it with the sidebar's **Relay fallback**
toggle, `yami-browser --enable-relays --desktop yami://NODE-ID/home`, or
`YAMINET_BROWSER_RELAYS=1`. `--disable-relays` overrides the startup setting.
The toggle affects subsequent attempts; reload a failed page after changing it.

Use **Add bootstrap peer** in the sidebar, paste the complete `NODE-ID@IPv4:port`
and press Enter. This saves an unverified pin and performs no connection. It
leaves the current page/address in place. The same setting is available at
startup with `--bootstrap NODE-ID@IPv4:port`, or through `yami-node dht bootstrap`.
Pins are stored in `YAMINET_PEER_DIR` (the existing default peer directory if
unset). `YAMINET_BROWSER_DHT_NETWORK=local` selects isolated loopback testing;
the default public policy refuses relay/direct probes to private addresses.

The existing two loader workers resolve and validate DHT records, attempt up to
four direct endpoints first, then up to two signed relay routes if enabled.
Each direct endpoint has a three-second budget; each relay route permits up to
two attempts sharing a three-second budget, with a 400 ms renewal-gap delay. Lookup, cached attempts and refreshed
record attempts share a thirty-second total deadline. There is no plaintext
fallback. The canonical yami:// identity, navigation, tabs, history and reload
are unchanged. Numeric links naming a relay are not treated as host links.

Only a successfully fetched authenticated page displays **Verified node · TLS
1.3** or **Verified node · TLS 1.3 · Relayed**. For relayed pages the sidebar
shows the complete verified destination and relay IDs separately. A signature
or an available relay alone never earns a verified indicator. Relays see client
and host IPs, IDs, traffic sizes and timing, and can refuse service; document
traffic is encrypted by a separate destination-pinned TLS 1.3 session.
