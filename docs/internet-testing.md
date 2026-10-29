# Experimental internet connectivity testing

**Not security-reviewed or production-safe. No anonymity.** Public mode lets
any reachable client read every served page. TLS authenticates the node to
the client; clients are not authenticated. Use only a disposable test site and
a node you own. There is no DHT, DNS, HTTP, mandatory server, UPnP, hole punching,
automatic firewall setup, automatic port forwarding, Tor or I2P.

## Validation scope

Automated tests run locally on one Windows computer. They exercise public-mode
listeners on two different loopback addresses (`127.0.0.1`, `127.0.0.2`), exact
key pinning, TLS 1.3 protection, malformed/plaintext/idle clients, bounded
admission and socket deadlines, persisted manual peers, unavailable endpoints,
an impostor at a saved endpoint, reconnection with the original identity, peer
CLI commands, and browser navigation/history/tabs/error recovery. Existing
certificate-tampering, expired/copy/wrong-key, document/frame limit and LAN tests
remain in the suite. Parser tests accept public numeric addresses without
contacting those addresses. Public-mode loopback tests are not internet tests.

On 2026-10-29, formatting, Clippy with warnings denied, workspace tests/build,
and both existing opt-in LAN discovery integration tests passed on Windows.
The new public-mode node/CLI/browser and peer persistence tests passed locally.

**Not performed during development:** a VPS-to-client connection across
different internet providers; router port forwarding or CGNAT behavior;
provider/OS firewall traversal; external packet capture; a graphical browser
session on a second network; Linux/VPS execution; sustained hostile internet
load; or an independent security review. No owned reachable remote test endpoint
was supplied. The ignored tests below require one; they are not counted as
passing automatically. Completing this guide on separate networks is still
required to validate actual internet reachability.

Run local checks from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

If an existing debug executable is in use, add
`--target-dir target/internet-validation` to the Cargo build/test/Clippy
commands. Stop your own running application before replacing its executable.

## Choose a reachable test endpoint

Use a disposable VPS with a directly assigned public IPv4 address, or a test
node behind a router on which **you manually configure** a TCP forward. Obtain
the node's public endpoint and full ID from its operator through a trusted
independent channel. No YamiNet service resolves or registers it for you.

The public endpoint must be an ordinary public unicast IPv4 address. The
conservative address policy refuses special-use ranges including 0/8,
100.64/10, link-local, 192.0.0/24, documentation blocks, deprecated 192.88.99/24,
198.18/15, multicast and reserved space. It also accepts existing loopback and
RFC 1918 LAN endpoints for local use. IPv6 remains loopback-only. Routability
is not guaranteed by address validation. See the
[IANA special-purpose registry](https://www.iana.org/assignments/iana-ipv4-special-registry/iana-ipv4-special-registry.xhtml).

## Start the node with a deliberate public opt-in

Build on the server, then bind **one specific local IPv4 address**:

```text
yami-node serve-dir <LOCAL-IPv4>:7443 <test-site-directory> --identity <persistent-key-directory> --tls --public --workers 4
```

Replace all angle-bracket values before running. On a directly addressed VPS,
LOCAL-IPv4 is its public address. If the VPS has provider NAT, or your own router
forwards to a private node, LOCAL-IPv4 is the actual local RFC 1918 address and
the client uses the separately known public address/forwarded port. Never bind
an address that is not assigned to the server. `0.0.0.0`, wildcard IPv6 and
special-use endpoints are refused. Loopback is accepted for isolated testing.

`--public` requires `--tls --identity`, is mutually exclusive with `--lan`, and
cannot be combined with `--discoverable`. Without either opt-in, listening
stays loopback-only. `--lan` still drops public-source connections. Plaintext
YNP remains loopback-only in every mode. Nothing changes your router or firewall.

Keep the identity directory private and persistent. Record the full printed
ID. If NAT is involved, the printed URLs name the **bind** address, not your
external forward; construct the external `ynps://` URL using the same ID and
your known public IP/port. No address advertisement is inferred from a remote
service.

Prepare `home.ylang` and `default.ysl` using the existing YLang/YSL syntax. For
internal navigation, write links as `ynp://<PUBLIC-IP>:<EXTERNAL-PORT>/about`.
When the current page came from that exact endpoint over `ynps://`, the browser
keeps its pinned ID and upgrades those same-endpoint links to `ynps://`.
Links naming a private bind address or different external port will not match
the current public endpoint. A different node requires an explicit `ynps://`
address containing its separately known ID; there is no identity lookup or
automatic plaintext fallback. No language changes are required.

## Firewall and forwarding

Use your VPS provider's firewall/security group to permit TCP 7443 from the
test client's public IP only. Apply the same restriction in the OS firewall.
Do not add discovery UDP rules or forward UDP for internet testing. The node
does not need a mandatory bootstrap server or external discovery endpoint.

On a Windows test server, an administrator PowerShell rule can be scoped to
the built executable, chosen local address, actual network profile and client:

```powershell
$nodeProgram = (Resolve-Path .\target\debug\yami-node.exe).Path
$localAddress = "REPLACE_WITH_LOCAL_IPV4"
$clientAddress = "REPLACE_WITH_TEST_CLIENT_PUBLIC_IPV4"
New-NetFirewallRule -DisplayName "YamiNet temporary remote TLS test" -Direction Inbound -Action Allow -Profile Public -Program $nodeProgram -LocalAddress $localAddress -RemoteAddress $clientAddress -Protocol TCP -LocalPort 7443
```

Replace the strings first. `-Profile Public` here means the Windows network
profile of the selected server connection, not the IP's address category. Use
its actual profile as shown by `Get-NetConnectionProfile`; do not change the
profile just to make a rule apply. For a manually forwarded private Windows
node, the adapter may use `Private`. Rules do not open provider firewalls or
routers. Parameter semantics are in Microsoft's
[New-NetFirewallRule documentation](https://learn.microsoft.com/en-us/powershell/module/netsecurity/new-netfirewallrule).

On a Linux VPS, use its existing firewall manager and provider controls to
allow only the test client to TCP 7443; preserve SSH/admin access. These
instructions do not install a firewall manager or change rules automatically.
For a router test, manually forward exactly the external TCP port to the chosen
node IPv4 and TCP port. CGNAT or a provider ingress block may make forwarding
impossible; YamiNet has no NAT traversal workaround. Test from a genuinely
different internet connection, not a second computer using the same router.

## Add and authenticate a manual bootstrap peer

On the client, replace the placeholders with the operator's exact full ID and
known public endpoint:

```text
yami-node peer add <NODE-ID> <PUBLIC-IP>:7443
yami-node peer list
yami-node peer connect <NODE-ID>
yami-node peer list
```

Add/list must not connect. The new record says `unverified` and has no success
time. Connect performs a fresh pinned TLS 1.3 handshake, closes it and records
`authenticated` with Unix seconds. Failure records `failed` without replacing
the ID/address or erasing a previous success time. This is historical status,
not a live connection. `peer remove <NODE-ID>` deletes the manual record.
Use `--dir <peer-directory>` after a command for an isolated test configuration;
see [bootstrap peer storage](specs/v0.1/bootstrap-peers.md).

## Browse from the other network

Open Yami Browser and enter:

```text
ynps://<NODE-ID>@<PUBLIC-IP>:7443/home
```

Confirm the existing **Verified node · TLS 1.3** status and full node ID, then
follow an internal link and use Back, Forward, Reload and another tab. Existing
tabs, address entry and rendering are preserved. Try an incorrect valid node
ID at the same address: authentication must fail clearly before any document
request. An unknown key is never substituted automatically. Testing a plain
`ynp://` public URL must fail locally before connecting.

Optional integration checks against **only your owned endpoint**:

```powershell
$env:YAMINET_TEST_REMOTE_URL = "ynps://REPLACE_WITH_FULL_ID@REPLACE_WITH_PUBLIC_IPV4:7443/home"
cargo test -p yami-node --test internet owned_remote_node -- --ignored --nocapture
cargo test -p yami-browser browser_opens_owned_remote_site -- --ignored --nocapture
```

The node check authenticates, fetches/parses the page and tries a wrong pin.
The browser check exercises the actual loader, tab state and mismatch error;
it does not open a desktop window. Also run the manual graphical steps above.
To observe encryption independently, capture traffic on the external path:
expect TLS 1.3 records, not readable YLang bodies. Automated TLS protection and
tampering tests are not a substitute for an external packet capture.

Stop/restart the node with the **same identity** and repeat peer connect and
browsing. While stopped, connect must fail within its connection timeout and
record failure; restarting must authenticate again. A different key at the old
endpoint must fail even after a recorded success. Wait a second if a rapid
test sequence reaches the per-IP admission rate limit.

## Limits and remaining risks

Public listeners request a kernel backlog of 128 (OS may clamp it), allow at
most 32 accepted connections and two per source IP concurrently, and admit
at most 32 connection starts per sliding second globally/eight per source IP.
State holds at most 32 recent starts and 32 active addresses, including failed
handshakes until dropped; attacker addresses are not kept in an unbounded ban
map. Clients behind one NAT share the per-IP budget. Limits apply per listener,
not globally across processes. The CLI still uses a fixed pool of 1..32 workers
(default four), one request per accepted connection. No per-client thread is
spawned. Public CLI outcome logs are sampled to at most 20 lines/second.

TCP connect timeout is three seconds. TLS handshake deadline is five seconds
from accept/client connection construction, and the total TLS connection budget
is ten seconds from that point. Individual socket calls use the smaller of
three seconds and the remaining deadline. TCP connect precedes the TLS budget.
Scheduling or system load can add delay. The node's existing ten-second YNP
request deadline and bounded document/style/frame/parser budgets remain.
Rustls outgoing application buffers are explicitly limited to 64 KiB; this is
not a bound on all TLS/process memory. Its record/handshake validation and the
existing YNP limits remain in force. There is no plaintext fallback, TLS 1.2,
resumption or early-data path.

These are basic application protections, not DDoS resistance. SYN queues,
bandwidth, CPU, memory, filesystem access and logging still need operational
monitoring and firewall controls. No client authorization, key rotation,
revocation or independent security review is provided. Treat public support
as an experimental connectivity foundation, not a production-safe network.

After testing, stop public mode, remove your temporary provider/router rules,
and remove the Windows rule if created:

```powershell
Remove-NetFirewallRule -DisplayName "YamiNet temporary remote TLS test"
```
