# Windows LAN discovery testing

Discovery is opt-in and exposes public node IDs and endpoint metadata to LAN
observers. Use a trusted private LAN. It does not provide anonymity. See the
[protocol and security boundary](specs/v0.1/lan-discovery.md).

## Automated tests versus physical LAN validation

`cargo test --workspace` runs packet validation, malformed/oversized input,
response correlation, duplicates, rate limits, interface/subnet policy,
CLI opt-in checks and browser selection tests, alongside the existing TLS
identity-mismatch and networking regressions. Tests never weaken TLS checks.

The two opt-in tests below use real multicast and TLS sockets on **one Windows
computer**. They verify two discoverable CLI processes, an undiscoverable TLS
process, correct IDs/endpoints, timeout behavior, discovery-to-browser
navigation, history/tabs and rejection of forged IDs. They do not demonstrate
packets crossing a router/switch, Wi-Fi, or the firewall between two computers.
They are explicitly ignored in ordinary CI rather than silently reported as
passing when an interface is unavailable.

On 2026-10-29, the regular workspace suite, both opt-in discovery tests and a
rendered flyout visual check passed on one Windows computer using its selected
WLAN private IPv4 interface. The browser test runs its actual three-second
search worker before selecting and opening a result. These results do not
claim physical two-computer or firewall validation. The initial blocking
Windows receive call was found to ignore its configured timeout; discovery
now uses nonblocking reads and bounded polling instead.

In a normal PowerShell window at the repository root, choose an actual local
Wi-Fi/Ethernet private address (not a VPN or virtual adapter):

```powershell
Get-NetIPAddress -AddressFamily IPv4
$env:YAMINET_TEST_LAN_ADDRESS = "192.168.1.50"
cargo test -p yami-node --test discovery -- --ignored --nocapture
cargo test -p yami-browser discovered_node_opens -- --ignored --nocapture
```

Use your computer's address in place of `192.168.1.50`. The tests choose a
temporary UDP port; firewall policies may require a temporary rule scoped to
the test executable and that interface. Do not disable the firewall. A failed
multicast test must be investigated, not counted as a pass. If an executable
in `target/debug` cannot be replaced, append
`--target-dir target/discovery-validation` before `--` in Cargo commands.

**Physical validation:** The two-computer procedure below has not been run
as part of this implementation. Record Windows versions, adapter/profile,
firewall rules, selected interface, IDs, results and failures when running it.

## Prepare two computers

Use Windows 10/11, Rust 1.90+, and this repository on both computers. Connect
both to the same trusted home/office subnet. Guest Wi-Fi/client isolation and
multicast filtering can prevent discovery. No port forwarding is needed.
Build on each computer:

```powershell
cargo build --workspace
Get-NetIPAddress -AddressFamily IPv4
Get-NetConnectionProfile
```

The chosen network profile must be **Private**. Change the profile in Windows
Settings if appropriate for that network. Select the physical Wi-Fi/Ethernet
interface deliberately; an RFC 1918 VPN address does not identify your LAN.
Computer A's example address below is `192.168.1.50`; replace it with A's
actual address. Computer B never has to type A's IP in the browser.

## Private-network firewall rules

Run the following in an **administrator PowerShell**, from the repository
root, on Computer A. Substitute its actual address and interface alias
(for example `Wi-Fi`, `WLAN` or `Ethernet`). Rules are scoped to the executable,
interface, local subnet and Private profile. Check existing broad app rules
if Windows previously offered to allow Public networks.

```powershell
$nodeProgram = (Resolve-Path .\target\debug\yami-node.exe).Path
$lanAlias = "Wi-Fi"
$lanAddress = "192.168.1.50"
New-NetFirewallRule -DisplayName "YamiNet TLS Private" -Direction Inbound -Action Allow -Profile Private -Program $nodeProgram -InterfaceAlias $lanAlias -RemoteAddress LocalSubnet4 -LocalAddress $lanAddress -Protocol TCP -LocalPort 7443
New-NetFirewallRule -DisplayName "YamiNet Discovery Private" -Direction Inbound -Action Allow -Profile Private -Program $nodeProgram -InterfaceAlias $lanAlias -RemoteAddress LocalSubnet4 -Protocol UDP -LocalPort 39817
```

On Computer B, in an **administrator PowerShell** at its repository root:

```powershell
$browserProgram = (Resolve-Path .\target\debug\yami-browser.exe).Path
$lanAlias = "Wi-Fi"
New-NetFirewallRule -DisplayName "YamiNet Discovery Replies Private" -Direction Inbound -Action Allow -Profile Private -Program $browserProgram -InterfaceAlias $lanAlias -RemoteAddress LocalSubnet4 -Protocol UDP
```

The browser receives replies on a temporary UDP port, and the responder sends
from a temporary UDP port, so B's reply rule intentionally omits port filters.
It remains scoped to the browser executable, selected adapter, local subnet
and Private profile. Normal outbound-allow policy is assumed; managed networks
with outbound blocks also need an approved outbound UDP rule to the configured
multicast group/port and node replies to the local subnet. Do not enable
Public-profile exceptions. The application does not install firewall rules.
Parameter semantics are documented by Microsoft in
[New-NetFirewallRule](https://learn.microsoft.com/en-us/powershell/module/netsecurity/new-netfirewallrule).

## Computer A: serve an encrypted discoverable site

```powershell
.\target\debug\yami-node.exe serve-dir 192.168.1.50:7443 sites/alice --identity .\node-identity --tls --lan --discoverable
```

Record the full node ID printed at startup. The node joins the multicast
group only on the interface with that address. Keep the process running.
For two discoverable nodes, start another terminal with `sites/bob`, TCP port
`7444` and a separate identity directory, and add a Private TCP 7444 rule.
Both nodes can share the discovery UDP port.

## Computer B: discover and connect

1. Launch `.\target\debug\yami-browser.exe` without a destination URL.
2. Click **Find local nodes** at the bottom of the existing sidebar.
3. Click **Choose LAN interface** to select B's Wi-Fi/Ethernet adapter. Click
   the interface row again to cycle if several adapters are listed.
4. Click **Search selected LAN**. It sends one query and waits three seconds.
5. Confirm A is listed with its private address and **Untrusted**. With two
   discoverable nodes, confirm both IDs and TCP endpoints are distinct.
6. Select A's result. Compare the **full claimed ID** with A's printed ID.
   Selecting a result must not open a page or alter other tabs.
7. Click **Open homepage (ynps://)**. Confirm Alice's homepage loads and the
   existing connection status says **Verified node · TLS 1.3**, with A's ID.
   The selected key was proved by TLS; verify its owner independently.
8. Follow internal links, use Back/Forward/Reload, switch existing tabs and
   open another tab. Existing navigation and the address bar must still work.

## Negative checks

- Stop A and repeat a search. It must end with no A result after three seconds.
- Restart A without `--discoverable`, preserving its identity and TLS flags.
  It must serve explicit `ynps://` requests but never answer discovery.
- Use a different multicast group or UDP port on B: A must not appear.
- Choose an unrelated interface on B: A must not appear through that interface.
- To check a forged identity without writing a UDP injector, copy a discovered
  `ynps://` address and replace only its ID with a different valid node ID.
  It must fail authentication, request no document and never retry plaintext.
  The automated tests also construct a forged discovery claim and open it
  through the actual browser action/loader.
- Repeated clicks during a search must not create additional workers/queries.
  Duplicated replies must produce one result per `(ID, endpoint)`.
- With a packet capture on the chosen adapter, inspect `udp.port == 39817`.
  One search sends one 192-byte multicast query, TTL 1. Replies are unicast,
  smaller than the query, carry only public metadata, and use ephemeral UDP
  ports (capture all UDP from the node's IP to inspect replies). There must be
  no periodic discovery traffic while idle. A capture is a manual check;
  one-computer tests do not prove an external packet trace.

If no nodes appear, check the selected subnet, Private profile, executable
paths in rules, multicast filtering/client isolation, VPN routing and matching
group/port settings. Test explicit TLS first using [LAN testing](lan-testing.md).
Do not broaden listeners to `0.0.0.0`, enable plaintext LAN or disable pinning.

Remove the rules after testing when no longer needed (administrator shell):

```powershell
Remove-NetFirewallRule -DisplayName "YamiNet TLS Private"
Remove-NetFirewallRule -DisplayName "YamiNet Discovery Private"
Remove-NetFirewallRule -DisplayName "YamiNet Discovery Replies Private"
```

To use another local group/port, set these in the terminal used to start each
application (and adapt firewall rules), then relaunch both:

```powershell
$env:YAMINET_DISCOVERY_GROUP = "239.255.89.79"
$env:YAMINET_DISCOVERY_PORT = "39818"
```
