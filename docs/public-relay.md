# Host behind a router and browse from another network

This is an **experimental public deployment procedure**, not a report of a
successful physical cross-network test. The automated four-process test runs
on loopback. Public VPS routing, provider NAT, home-router behavior and Windows
runtime operation still require validation on real machines. Do not publish
sensitive documents. Public relay security needs independent review.

You need three machines/connections:

- A VPS with a reachable public IPv4, acting as relay R and an ordinary DHT peer B.
- A website host A behind a home router. It makes outbound connections only.
- A browser C on another connection, such as a laptop using a mobile hotspot.

R and B use different identities and TCP ports even when on the same VPS.
You may run B on a separate machine or use another operator's ordinary peer.
There is no mandatory YamiNet-operated server or central directory. A relay is
an explicitly trusted transport provider; DHT peers supply signed locations,
not destination trust. Learn the website's full ID from its owner.

## Obtain the executables

Build once with the pinned toolchain following [installation](desktop-browser.md),
then use the resulting executables without Cargo. No Rust programming is needed.
If transferring binaries, build for the destination operating system/architecture
and verify their provenance. No prebuilt release availability is assumed here.

On the VPS, in the repository root:

```sh
cargo build --release --locked -p yami-node -p yami-relay-cli
```

The commands below use `target/release/yami-node` and `target/release/yami-relay`.
Keep state and keys outside the checkout. Run as a dedicated unprivileged user.

## Prepare the public VPS

Find its **specific local interface IPv4** with `ip -4 address`. Ask the provider
for the actual public IPv4. They may differ with provider NAT. Replace both
values below; documentation/reserved IPs such as 198.51.100.x are refused.
Never use 0.0.0.0. This feature currently supports public IPv4, not remote IPv6
or hostname endpoints.

```sh
bind_ip=YOUR_LOCAL_INTERFACE_IPV4
public_ip=YOUR_REAL_PUBLIC_IPV4
yami_data="$HOME/.local/share/yaminet-relay-trial"
mkdir -p "$yami_data/relay" "$yami_data/dht" "$yami_data/peers" "$yami_data/site"
chmod 700 "$yami_data" "$yami_data/relay" "$yami_data/dht"
target/release/yami-node identity new --dir "$yami_data/relay"
target/release/yami-node identity new --dir "$yami_data/dht"
printf '@page 0.1\n  title DHT bootstrap\n= Ordinary peer\n' > "$yami_data/site/home.ylang"
target/release/yami-node identity show --dir "$yami_data/relay"
target/release/yami-node identity show --dir "$yami_data/dht"
```

Create identities once. Use `identity show` on subsequent runs. Copy only each
**Node ID**, never `node.key`. Verify IDs with the VPS operator over a separate
trusted channel (for your own VPS, an authenticated SSH session). A valid TLS
connection proves the chosen key, not who owns it.

In the VPS provider's firewall/security group, allow inbound TCP 17550 (relay)
and 17444 (DHT peer). If possible, restrict source IPs to your test networks.
Allow outbound established traffic. For an Ubuntu host using UFW, first preserve
access to the actual SSH port; only then enable rules:

```sh
sudo ufw allow OpenSSH
sudo ufw allow 17550/tcp
sudo ufw allow 17444/tcp
sudo ufw enable
sudo ufw status
```

If SSH uses a different port/profile, replace `OpenSSH` with that rule before
enabling UFW. Do not disable your firewall to diagnose an application failure.
If the provider uses NAT, configure its public TCP mappings for both ports.
This setup does not configure the **home host's router**.

Start R in one terminal, substituting the persistent path/addresses as above:

```sh
target/release/yami-relay serve "$bind_ip:17550" \
  --identity "$yami_data/relay" --public --advertise "$public_ip:17550"
```

`--advertise` is separate from the local bind: it validates and prints the
operator-selected endpoint. It does not discover an IP, configure NAT, verify
reachability or publish the relay itself into the DHT. Hosts explicitly select
that public endpoint and pin R's ID.

Start B in another terminal (set the same variables there):

```sh
target/release/yami-node serve-dir "$bind_ip:17444" "$yami_data/site" \
  --identity "$yami_data/dht" --tls --public --dht --dht-maintain \
  --dht-dir "$yami_data/peers" --dht-endpoint "$public_ip:17444"
```

B can start without upstream bootstrap peers. It stores A's validated records
and answers lookup requests. Its startup may report zero upstream publications;
this does not prevent it from serving DHT RPCs. Other peers can be added later.
Check `ss -ltn` and the startup IDs/addresses. Listening on a local interface
alone does not establish public reachability.

## Windows website host behind a home router

Use Windows 10/11 and the [one-time build prerequisites](desktop-browser.md).
Build in the checkout, then keep the identity/site under your user profile:

```powershell
cargo build --release --locked -p yami-node
$YamiData = Join-Path $env:LOCALAPPDATA 'YamiNetRelayTrial'
$HostKeys = Join-Path $YamiData 'identity'
$PeerDir = Join-Path $YamiData 'peers'
$SiteDir = Join-Path $YamiData 'site'
New-Item -ItemType Directory -Force $SiteDir | Out-Null
.\target\release\yami-node.exe identity new --dir $HostKeys
$HostId = (Get-Content (Join-Path $HostKeys 'node.id')).Trim()
```

Run `identity new` only once; on later starts use `identity show --dir $HostKeys`.
Keep that directory private using Windows account/ACL controls. The key is
unencrypted at rest; copying or losing it affects the stable website identity.
Back it up securely and separately from public site files. Do not commit it.

Make two UTF-8 documents without a byte-order mark (or edit these in Notepad):

```powershell
$Utf8 = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText((Join-Path $SiteDir 'home.ylang'), "@page 0.1`n  title My Yami site`n= Hello from home`n:: Links`n  > yami://$HostId/about | About`n", $Utf8)
[System.IO.File]::WriteAllText((Join-Path $SiteDir 'about.ylang'), "@page 0.1`n  title About my site`n= Another page behind the router`n", $Utf8)
Copy-Item .\examples\default.ysl (Join-Path $SiteDir 'default.ysl')
```

Set the actual full verified IDs and public IP from the VPS setup:

```powershell
$RelayId = 'FULL_RELAY_NODE_ID'
$DhtId = 'FULL_DHT_PEER_NODE_ID'
$VpsIp = 'ACTUAL_PUBLIC_IPV4'
.\target\release\yami-node.exe dht bootstrap $DhtId "${VpsIp}:17444" --dir $PeerDir
.\target\release\yami-node.exe host-relay $SiteDir --identity $HostKeys --relay "${RelayId}@${VpsIp}:17550" --dht-dir $PeerDir
```

Wait for `Registered outbound host ... at relay ...` and an authenticated DHT
publication. Share `yami://<HostId>/home` with the browser user. No inbound
Windows firewall rule, LAN listener, home-router port forwarding or automatic
router configuration is needed. If outbound connections are restricted, permit
this executable to connect to the VPS's two TCP ports.

For a second explicitly trusted relay, add another `--relay FULL-ID@IPv4:port`.
The maximum is two. Each has one outbound slot; a busy slot can refuse additional
clients. The host makes bounded jittered retries after outage and re-registers
idle leases. The existing DHT worker publishes current routes, withdraws routes
from new publications on disconnection, renews before the 24-hour expiry and
persists increasing sequences. Changes may take several seconds to propagate.
There is no global availability or replication guarantee.

On Linux the equivalent host command is:

```sh
target/release/yami-node dht bootstrap "$dht_id" "$public_ip:17444" --dir "$peer_dir"
target/release/yami-node host-relay "$site_dir" --identity "$host_keys" \
  --relay "$relay_id@$public_ip:17550" --dht-dir "$peer_dir"
```

## Browser on a separate connection

Build `cargo build --release --locked -p yami-browser`. On Windows PowerShell:

```powershell
$env:YAMINET_PEER_DIR = Join-Path $env:LOCALAPPDATA 'YamiNetBrowserPeers'
.\target\release\yami-browser.exe --bootstrap "${DhtId}@${VpsIp}:17444" --enable-relays --desktop "yami://$HostId/home"
```

Supply the IDs/IP/HostId on this machine too. `--bootstrap` persists the pin;
use it once for each ID. To change an existing pin, remove it explicitly with
`yami-node peer remove ID --dir DIRECTORY` before adding the replacement.
Alternatively use **Add bootstrap peer** in the browser sidebar and paste the
full `ID@IPv4:port`, then enable **Relay fallback** and type the native address.
Adding a pin does not fetch or automatically open discovered content.

On Linux with a desktop session:

```sh
export YAMINET_PEER_DIR="$HOME/.local/share/yaminet-browser-trial"
target/release/yami-browser --bootstrap "$dht_id@$public_ip:17444" \
  --enable-relays --desktop "yami://$host_id/home"
```

Check **Verified node · TLS 1.3 · Relayed**, compare the full destination ID
with the website owner and the separate relay ID with the VPS operator. Follow
the About link: the address should stay `yami://<same-ID>/about`. Test Back,
Reload and another tab. A signed advertisement or open TCP port alone is not
a successful end-to-end test. Direct sites show **Verified node · TLS 1.3**.

For a headless verification use the actual Rust terminal renderer:

```sh
target/release/yami-browser --enable-relays --terminal "yami://$host_id/home"
```

## Failure diagnostics and controlled outage tests

| Symptom | Check |
| --- | --- |
| Host cannot register | R is running; compare full relay ID; verify advertised public IP/port, provider NAT, both firewalls and outbound host policy |
| Registration succeeds but node unknown | A has B's correct pin in the same `--dht-dir`; B is running with `--tls --public --dht`; check publication count and clocks |
| Browser says no bootstrap peers | Set the browser's `YAMINET_PEER_DIR` or add a pin with the sidebar/`--bootstrap`; do not use isolated `local` mode on the Internet |
| Node has relay routes but cannot open | Enable relay fallback, confirm A remains online, retry after slot renewal; busy or quota errors never imply authentication |
| Identity mismatch | Stop and compare the exact destination and relay pins separately; do not replace a pin with a discovered key to suppress the error |
| Expired/rolled-back record | Synchronize clocks; retain the host's identity and DHT cache; inspect `dht peers` and `dht status`; do not delete sequence history as a routine fix |
| Lock or state error | Stop duplicate processes using the same directory; preserve caches and keys; repair storage permissions/full disks before restart |
| Large page disconnects | Relay permits 2 MiB per tunnel and 256 KiB per second window; reduce the page/style or test direct TLS; no automatic transport downgrade |

Host diagnostics, in another terminal with the same peer directory:

```powershell
.\target\release\yami-node.exe dht status --dir $PeerDir
.\target\release\yami-node.exe dht peers --dir $PeerDir
```

`dht status` reports local maintenance/publication diagnostics, not a global
reachability proof. `dht lookup ID --dir DIRECTORY` inspects validated records;
the browser is the relay-aware fetching/rendering path. Existing `dht fetch`
remains a direct-endpoint diagnostic.

Stop R with Ctrl-C. A should report bounded reconnect attempts and later publish
without that route. Browsing must fail with no verified indicator. Restart R
using the **same identity** and address; verify A re-registers, a newer sequence
appears at B, and Reload recovers. Stop A and confirm clients cannot fetch its
previously advertised route. Test incorrect relay/destination pins with disposable
client configuration; authentication must fail. Retain any packet captures only
under your normal privacy rules: TLS hides document contents, not IPs/IDs/timing.

## Stop and retain state

Use Ctrl-C on A, R and B and close the browser. On Unix SIGTERM is also supported
for maintained nodes and the relay. Wait for exit before restarting. Keep key
and DHT state directories to preserve identity and sequence history. Remove the
two public firewall rules when the trial is over (`ufw delete allow 17550/tcp`
and `ufw delete allow 17444/tcp`) and remove provider mappings/rules if dedicated
to the trial. Keep the SSH rule. Older signed advertisements can remain cached
until replaced or expired; neither they nor an available relay can impersonate
a disconnected host.

## Limits and test report

Relays observe host/client IPs, IDs, timing and volumes, and can deny or correlate
service. They cannot decrypt the independent destination-pinned TLS document
session. There is no anonymity, client authorization, key revocation, Sybil
resistance, reliable global replication or protection against volumetric DoS.
Bounded slots can still be exhausted by distributed or malicious clients; use
provider protections and narrowly scoped firewall rules. See [SECURITY.md](../SECURITY.md)
and the [protocol limits](specs/v0.1/relay.md).

Record OS/build revision, VPS provider and public/bind addresses, home-router
model, separate client connection type, observed IDs, publication sequences,
startup/recovery times and both navigation results when conducting the physical
test. Do not mark it passed from automated loopback results. Windows instructions
have not been runtime-tested in this Linux environment.
