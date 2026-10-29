# Testing YamiNet between two Windows computers

**EXPERIMENTAL. TLS 1.3 to a pinned node ID on a private LAN. Not
independently reviewed. No anonymity.** Read the threat model in
[YNP over TLS](specs/v0.1/ynp-tls.md#threat-model) before serving anything over
a network you share with other people.

Computer A runs a node that serves a multi-page site on its private LAN
address. Computer B opens that site in Yami Browser with a `ynps://` address
that names A's node ID. The browser verifies A's identity in a TLS 1.3
handshake before it requests anything, then downloads, renders and navigates
the YLang pages.

## Validation status

The automated tests (`apps/yami-node/tests/lan.rs` and the browser tests) bind
a real private IPv4 address of the machine that runs them, with the node and
the client on that one computer. They cover the LAN listener, address policy,
identity checks and browser navigation, but traffic never crosses a physical
network and the Windows firewall does not filter it. The procedure below,
which needs two computers, has **not** been run as part of developing this
milestone. Treat the first run on two physical machines as the real LAN
validation.

## Requirements

- Two Windows 10 or 11 computers on the same private network (the same home or
  office router), both with a private IPv4 address: `192.168.x.x`, `10.x.x.x`
  or `172.16.x.x`–`172.31.x.x`. Guest Wi-Fi often isolates clients from each
  other; use the main network.
- The Windows network profile set to **Private** on both computers.
- Rust 1.90.0 and a copy of this repository on both computers. See
  [Desktop browser and Windows launch instructions](desktop-browser.md).

The examples use `192.168.1.50` for computer A and port `7443`. Replace them
with your own values everywhere.

## 1. Computer A: find its LAN address

```powershell
ipconfig
```

Use the `IPv4 Address` of the adapter that connects to your router (Wi-Fi or
Ethernet). Ignore `169.254.x.x` (no DHCP lease; refused by YamiNet) and
virtual adapters such as `vEthernet (WSL)` or `vEthernet (Default Switch)`.

Check that the network is Private:

```powershell
Get-NetConnectionProfile
```

`NetworkCategory` must be `Private`. If it is `Public`, open Settings →
Network & internet → Wi-Fi (or Ethernet) → your network, and set
**Network profile type** to **Private**.

## 2. Computer A: prepare a site for the LAN

YLang links are always written `ynp://<address>/<key>`. When a page was opened
from `ynps://<node-id>@<address>/...`, the browser opens a link to that same
`<address>` as `ynps://<node-id>@<address>/<key>`: the same node, the same
pinned identity, still encrypted. Links to any other LAN address are refused,
never fetched in plaintext.

The sample sites link to `127.0.0.1:7301`, which on computer B means B itself.
Copy Alice's site and point its own links at A's LAN address and port:

```powershell
$site = "$env:USERPROFILE\yaminet-lan-site"
Copy-Item -Recurse sites\alice $site
Get-ChildItem "$site\*.ylang" | ForEach-Object {
    $text = [IO.File]::ReadAllText($_.FullName) -replace '127\.0\.0\.1:7301', '192.168.1.50:7443'
    [IO.File]::WriteAllText($_.FullName, $text)
}
```

(`WriteAllText` writes UTF-8 without a byte order mark.) The links to Bob's
site still name `127.0.0.1:7302` and will fail on B unless B runs Bob's node
itself; that is expected.

## 3. Computer A: start the node

```powershell
cargo run -p yami-node -- serve-dir 192.168.1.50:7443 $env:USERPROFILE\yaminet-lan-site --identity $env:USERPROFILE\yaminet-ids\alice --tls --lan
```

`--lan` is required for any non-loopback address, works only together with
`--tls` and `--identity`, and accepts exactly one private IPv4 address of this
computer. `0.0.0.0`, public addresses and plaintext LAN serving are refused.
The identity directory holds the node's secret key; keep it out of the
repository (`/node-identity/` and `/demo-ids/` inside the checkout are ignored
by Git).

The node prints, among other lines:

```text
Serving site ... via YNP over TLS 1.3 on 192.168.1.50:7443 as yn1-...
LAN: computers on this private network can connect to 192.168.1.50:7443; nothing listens on other interfaces. Allow inbound TCP port 7443 for Private networks only.
  ynps://yn1-...@192.168.1.50:7443/about
  ynps://yn1-...@192.168.1.50:7443/home
Open in Yami Browser: ynps://yn1-...@192.168.1.50:7443/home
```

The node ID (`yn1-...`) stays the same across restarts with the same
`--identity` directory. Transfer the `Open in Yami Browser` address to
computer B in a way you trust, for example by reading it off A's screen. The
node ID in that address is the only thing that decides which node B trusts.

## 4. Computer A: allow the port in Windows Defender Firewall

Computer B needs no firewall change; outbound connections are allowed by
default. On computer A, allow inbound TCP on the node's port, from the local
subnet, on Private networks only. In an **administrator** PowerShell:

```powershell
New-NetFirewallRule -DisplayName "YamiNet node (TLS, LAN)" -Direction Inbound -Protocol TCP -LocalPort 7443 -LocalAddress 192.168.1.50 -RemoteAddress LocalSubnet -Profile Private -Action Allow
```

If Windows showed a "Windows Defender Firewall has blocked some features of
this app" dialog when the node started, allow **Private networks** only and
never Public. If that dialog was cancelled or Public was refused, Windows may
have created *block* rules for `yami-node.exe`, and block rules win over allow
rules. List them:

```powershell
Get-NetFirewallApplicationFilter | Where-Object Program -like "*yami-node.exe" | Get-NetFirewallRule | Format-Table DisplayName, Enabled, Profile, Direction, Action
```

Remove a blocking rule shown there with `Remove-NetFirewallRule -DisplayName
"<its name>"` (administrator), or disable it in "Windows Defender Firewall with
Advanced Security". After testing, remove the rule you added:

```powershell
Remove-NetFirewallRule -DisplayName "YamiNet node (TLS, LAN)"
```

Do not forward this port on your router. YamiNet refuses public addresses, and
a LAN node drops connections from non-private source addresses, but the
transport has not been reviewed for internet exposure.

## 5. Computer B: check that A is reachable

```powershell
Test-NetConnection 192.168.1.50 -Port 7443
```

`TcpTestSucceeded : True` means the firewall and network allow the
connection. (A's node logs this probe as a rejected connection, which is
harmless.) If it is `False`, see [Troubleshooting](#troubleshooting).

## 6. Computer B: open the site in Yami Browser

```powershell
cargo run -p yami-browser -- --desktop "ynps://yn1-...@192.168.1.50:7443/home"
```

Or start `cargo run -p yami-browser`, click the address field, paste the
address and press Enter. Expected:

- While connecting, the page area shows `Verifying node identity…`.
- Alice's home page renders with its own style.
- Under the address field the sidebar shows a padlock, **Verified node · TLS
  1.3**, and the full node ID. Compare it with the ID printed on computer A.
- Clicking **About Alice** and **Projects** opens them from the same node; the
  address stays `ynps://` and the verified marking stays.
- Back, Forward, Reload, new tabs and scrolling work as for local pages.
  Reload connects and verifies the node again.

From a terminal on B, the same document can be fetched without the window:

```powershell
cargo run -p yami-node -- fetch "ynps://yn1-...@192.168.1.50:7443/home"
cargo run -p yami-browser -- "ynps://yn1-...@192.168.1.50:7443/projects"
```

Both print `Verified node yn1-... over TLS 1.3` on standard error.

## 7. Computer B: check that the wrong node is refused

Create a second identity on B only to obtain another valid node ID:

```powershell
cargo run -p yami-node -- identity new --dir $env:TEMP\yaminet-other-id
```

Replace the node ID in the address with the printed one and open it. The
browser shows **Could not open page** with:

```text
Authentication failed at 192.168.1.50:7443: server is not the trusted node yn1-<other>: it presented node identity yn1-<A>. Nothing was requested from it.
```

No document is shown and no padlock appears. Changing a single character of
the real ID instead usually breaks the ID's checksum, which the browser
rejects before connecting (`Cannot open this address: trusted node ID: ...`).
Both outcomes refuse the page. Delete `$env:TEMP\yaminet-other-id` afterwards.

Further checks:

- `ynp://192.168.1.50:7443/home` (plaintext) is refused by the browser: plain
  YNP never leaves loopback. `yami-node fetch 192.168.1.50:7443 home` fails too.
- Stop the node on A (Ctrl+C) and press Reload on B: the page becomes an error
  that says nothing was requested and nothing fell back to plaintext. Other
  tabs keep working; start the node again and Reload recovers.
- On A, `serve-dir 192.168.1.50:7443 ...` without `--lan`, or with
  `0.0.0.0:7443`, is refused.

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| `TcpTestSucceeded : False` | Firewall rule missing or for the wrong profile; a block rule for `yami-node.exe`; wrong address; guest Wi-Fi client isolation; a VPN on either computer. |
| `a LAN listener needs one specific private IPv4 address` / `--lan needs one specific private IPv4 address` | The address given to `serve-dir` is not a private IPv4 address of A. |
| `The requested address is not valid in its context` (os error 10049) | The address is private but does not belong to computer A. Check `ipconfig`. |
| `Authentication failed ... presented node identity ...` | The node at that address is not the one in the URL: wrong ID, or another node on that address. |
| `invalid peer certificate` saying the certificate is expired or not yet valid | The clocks of A and B differ by too much. Enable automatic time on both. |
| The address stops working after a router restart | A received a new DHCP address. Restart the node on the new address; the node ID is unchanged. A DHCP reservation keeps the address fixed. |

## What this does and does not protect

Over the LAN, the content of pages and requests is encrypted, tampering is
detected, and B only accepts the node whose ID is in the address. It does not
hide that the two computers talk YNP, when, or how much. Clients are not
authenticated: anyone on the LAN who can reach the port can read every page
the node serves. A stolen `node.key` lets its holder impersonate the node. See
the [threat model](specs/v0.1/ynp-tls.md#threat-model).
