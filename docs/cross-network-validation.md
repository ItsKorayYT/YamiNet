# Executable cross-network validation plan

**Execution status: NOT RUN on physical networks.** This plan is prepared for a
Linux VPS, a Windows host behind a home router, and a native browser using an
independent Internet connection. Local process tests are recorded separately.
No external server was deployed and no live firewall/router rule was changed
while developing this milestone. Windows runtime and true NAT behavior remain
unverified. The optional direct NAT transport was not implemented; do not open
UDP ports or claim hole punching from any result in this plan.

Use disposable public content and independently compare complete node IDs.
TLS pinning is mandatory and does not imply anonymity, private sites or a
security audit. Keep secret keys and backup files off shared terminals/logs.

## 1. Record the test environment and build

Copy [the report template](validation-report-template.md) for the actual trial.
Record `git rev-parse HEAD`, OS/build versions, VPS provider/local/public IPv4,
router model and separate client connection type. Record public IDs only.

On the VPS, follow [the existing setup](public-relay.md) to install Rust and build:

```sh
cargo build --release --locked -p yami-node -p yami-relay-cli
```

On Windows follow [MSVC prerequisites](desktop-browser.md) and build:

```powershell
cargo build --release --locked -p yami-node -p yami-browser
```

Copy/install trusted executables to your deployment directories. Users need no
Rust programming after this one-time build. Use persistent keys and state
outside the checkout. Never overwrite an existing identity to repeat a test.

## 2. Explicit VPS identity, configuration and firewall

Create a dedicated unprivileged service account and private identity/state/site
directories as described in [configuration and service setup](configuration.md).
Create R's identity and B's separate identity once:

```sh
/opt/yaminet/yami-node identity new --dir /var/lib/yaminet/relay/identity
/opt/yaminet/yami-node identity new --dir /var/lib/yaminet/dht/identity
/opt/yaminet/yami-node identity show --dir /var/lib/yaminet/relay/identity
/opt/yaminet/yami-node identity show --dir /var/lib/yaminet/dht/identity
```

Run as the service owner or use an appropriate administrative installation step;
keys must be readable only by their owner. On later runs use `identity show`,
not `identity new`. Prepare a harmless `home.ylang` in B's site directory.

Edit [relay](../deploy/relay-public.conf.example) and
[DHT node](../deploy/dht-public.conf.example) templates into `/etc/yaminet/relay.conf`
and `/etc/yaminet/dht.conf`. Replace every placeholder with the actual numeric
addresses/absolute paths; files are deliberately not runnable unedited. For
provider NAT, `listen` is the local interface and `advertise` is the public
address/port. Neither setting establishes reachability or configures NAT.

Explicitly allow inbound **TCP 17550** (relay) and **TCP 17444** (DHT) in both
provider and OS firewalls, restricted to test source networks where practical.
Preserve your actual SSH rule before enabling any firewall. The UFW commands,
provider NAT mappings and safe SSH caveats are in [the VPS guide](public-relay.md).
Do not disable the firewall. Do not open any inbound port on the Windows host
or its home router. No UPnP, ICE/STUN, NAT-PMP or PCP is involved.

Check, then start in separate VPS terminals:

```sh
/opt/yaminet/yami-relay check-config /etc/yaminet/relay.conf
/opt/yaminet/yami-node check-config /etc/yaminet/dht.conf
/opt/yaminet/yami-relay --config /etc/yaminet/relay.conf
```

```sh
/opt/yaminet/yami-node --config /etc/yaminet/dht.conf
```

Expected: R prints `Relay listening on ... as ...; explicit public mode true`
and its selected advertisement/state directory. B prints its persistent ID,
TLS listener and DHT sequence/maintenance diagnostics. B may have zero upstream
publications when it is the first ordinary peer; it still stores validated
records and answers pinned TLS RPCs. Check `ss -ltn` for the two local listeners.
This alone is not an external connection test. Optional systemd startup must be
explicitly selected using [the example service](../deploy/yaminet-relay.service).

## 3. Windows host behind the router

Follow [the Windows host walkthrough](public-relay.md) to create an identity and
a two-page site under your account's LocalAppData directory. The home page must
link to `yami://FULL_HOST_ID/about`. Verify the VPS relay/DHT IDs using a trusted
channel such as your authenticated SSH session.

Edit a node config with `role node`, `network public`, existing absolute
identity/state/site paths, R's full `relay ID@IPv4:17550` and B's full
`bootstrap ID@IPv4:17444`. Do not add `listen`, `advertise`, `public-listen`
or `dht` fields to this outbound-only profile. See [the Windows config sample](configuration.md).
Use a second explicit independent relay pin if testing redundancy.

```powershell
.\yami-node.exe check-config C:\Users\YOUR_USER\yaminet-host.conf
.\yami-node.exe diagnose --config C:\Users\YOUR_USER\yaminet-host.conf
.\yami-node.exe diagnose --config C:\Users\YOUR_USER\yaminet-host.conf --connect
.\yami-node.exe --config C:\Users\YOUR_USER\yaminet-host.conf
```

The Rust diagnostic command without `--connect` reads local configuration and
identity metadata only. `--connect` authenticates at most eight explicitly
configured TLS pins under a fifteen-second total budget (three seconds each).
It does not query discovered endpoints, register the host or fetch documents.
An authenticated relay pin is not proof that the website is available. A failed
check returns a nonzero exit status; it never falls back to plaintext.

Expected host output includes `Outbound-only hosting as ...; no direct listener`,
`Registered outbound host ... at relay ...`, then an authenticated DHT startup
publication count. With restricted outbound firewall policy, permit the node to
connect to the VPS's two TCP ports. No inbound host rule or router mapping is
required. Record A's public ID and starting sequence. In another terminal:

```powershell
.\yami-node.exe dht status --dir C:\Users\YOUR_USER\AppData\Local\YamiNetRelayTrial\peers
.\yami-node.exe dht peers --dir C:\Users\YOUR_USER\AppData\Local\YamiNetRelayTrial\peers
```

Use the exact state directory from the profile. Samples show publication time,
peer counts, error/next-attempt information, relay registration/failure counts
and history occupancy. They are local diagnostics, not remote liveness proofs.

## 4. Browser on another connection

Use a separate machine/network, for example a laptop on a mobile hotspot. Do
not count another window on the home host as a separate-network trial. Prepare
a browser config with `role browser`, `network public`, a separate absolute
state directory, B's independently verified bootstrap pin and `relay-fallback yes`.
There is no browser identity file and no public listener.

```powershell
.\yami-browser.exe check-config C:\Users\YOUR_USER\yaminet-browser.conf
.\yami-node.exe diagnose --config C:\Users\YOUR_USER\yaminet-browser.conf --connect
.\yami-browser.exe --config C:\Users\YOUR_USER\yaminet-browser.conf --desktop yami://FULL_HOST_ID/home
```

Expected: the original canonical yami:// address remains visible; the actual
YLang homepage renders; connection status says `Verified node · TLS 1.3 · Relayed`.
Inspect and compare the full **destination ID** and the separate **relay ID**.
Follow the site's actual About link and verify `yami://SAME_HOST_ID/about`, then
Back, Reload, scrolling and multiple tabs. Every new connection must prove the
same exact destination. Discovery or a signed advertisement alone must not
produce a verified indicator. Record the result/timing of each action.

For a headless companion check using the actual native project's Rust terminal
renderer:

```powershell
.\yami-browser.exe --config C:\Users\YOUR_USER\yaminet-browser.conf --terminal yami://FULL_HOST_ID/home
.\yami-browser.exe --config C:\Users\YOUR_USER\yaminet-browser.conf --network-status
```

## 5. Controlled failures and recovery

| Action | Expected observation and acceptance |
| --- | --- |
| Stop R with Ctrl-C or `systemctl stop yaminet-relay` | A reports bounded reconnect backoff; newly published records exclude the dead route; with one relay new fetches fail without a verified indicator |
| With R/S independently configured, stop the relay actually selected by the browser | C can fetch through the other eligible relay; the verified destination ID stays unchanged and displayed relay ID changes |
| Restore R with its original key/config | A re-registers automatically; DHT sequence advances when routes change; Reload recovers within bounded retry/maintenance delays |
| Stop B while an independently operated second DHT peer is available | Warm cache remains usable; a fresh configured client can resolve through the other peer; after B returns health maintenance republishes without waiting for the normal renewal period |
| Restart A with unchanged identity and state | Same host ID; reserved record sequence is newer; outbound registrations and browsing recover |
| Publish an operator-selected direct endpoint change on a direct-host test site | A newer owner-signed record is accepted; stale/failed addresses refresh; a prior valid lower-sequence record is rejected |
| Open several tabs | UI remains responsive; shared discovery and bounded destination permits avoid duplicate request storms; overload still fails within the total deadline |
| Use a disposable copied profile with a wrong relay/destination ID | Authentication fails; no document request to a substitute identity; no verified indicator or plaintext retry |
| Repeated outages or intentionally slow peers | Attempts stay bounded; cooldown/backoff occurs; unrelated navigation remains responsive; resource counters fall after cleanup |

Do not edit live trust pins to make authentication errors disappear. Signed
endpoint changes are only for an identity the operator actually owns, using
supported configuration/maintenance paths. Original direct `ynps://` testing
remains in [internet-testing.md](internet-testing.md); UDP/direct-NAT behavior
is not part of this plan. Physical packet loss, router mappings, MTU and provider
behavior require real observations and must be recorded separately from the
TCP stream fragmentation/latency/reset tests.

## 6. Diagnose failures without weakening authentication

- Registration fails: compare R's full pin first; check real public IP/port,
  local bind, provider NAT, public-listen opt-in and both firewalls.
- Registration works but lookup fails: verify A's and C's B pins/state paths,
  B's DHT flags, publication diagnostics and synchronized clocks. Diversify
  ordinary bootstrap operators; there is no central directory to repair a partition.
- Busy/quota errors: the host has one serving slot per configured relay; public
  per-IP limits can affect users behind the same NAT. Retry after cooldown or
  use another explicitly configured relay. Do not increase bounds blindly.
- Full cache/rollback error: retain state; untrusted owner history is finite.
  Reserved local/bootstrap capacity is protected, not a solution to Sybil/eclipse
  attacks. Never automatically evict history or delete the cache to suppress the error.
- Stale status: check sample age, process state and logs. A fresh local sample
  still does not prove that the other Internet connection can reach the service.
- Key/config error: correct private directory ownership/ACLs and paths. Do not
  log key files, dump environment variables or substitute a new identity.

## 7. Clean shutdown and final report

Close C. Ctrl-C A and wait for exit; stop R and B (or their explicit services),
then inspect relay status for stopped/zero active resources. A crashed process
can leave a stale snapshot, which must be treated as such. Retain identity and
current matching DHT state for future starts. Follow [safe upgrade procedures](configuration.md)
before replacing binaries. If the trial is complete, explicitly remove the two
trial OS/provider inbound TCP rules/mappings while preserving SSH access.

Complete [the report](validation-report-template.md) with observed success,
failure, timings and limits; include public IDs and revision, no secrets.
Mark unexecuted cases NOT RUN. Do not call automated loopback tests physical
cross-network validation or claim production security from this procedure.
