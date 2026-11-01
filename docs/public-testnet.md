# Run a YamiNet public test network

This guide prepares an **experimental public test network** on a Linux VPS and
shows how to host a demonstration website, share its `yami://` address, and
check it from a separate client.

> **Read this first.** YamiNet is an early experiment, not an anonymity network
> or a production service. Public-network testing is incomplete. This guide does
> not make production-security or anonymity claims. Do not publish sensitive
> documents. Do not change a firewall, router or real server without the
> operator's explicit authorization.

For the current VPS DHT/relay/YNS provider + outbound home website/hosted game +
independent-client topology, use the [first public-internet trial runbook](../deploy/testnet/scenarios/public-internet-e2e.md).
It extends this existing kit, documents the `init --yns` configured-service opt-in,
and records local rehearsal separately from real public-internet results.

## What you will build

```text
                    Internet
                       |
          +------------+-------------+
          |                          |
   VPS (public IPv4)           Client C (separate
   +------------------+        machine/connection)
   | Bootstrap node B |               |
   | Relay R          |<--------------+  knows B's pin,
   | Demo website A   |                  R's pin and A's
   +------------------+                  full yami:// address
          ^
          | outbound only (optional)
   Home website host A (behind a router, no port forwarding)
```

- **Bootstrap node B** answers signed DHT lookups. It is an ordinary YamiNet
  node with an explicit public listener and `dht yes`.
- **Relay R** is optional. A website host behind a router uses it as an
  explicitly trusted transport provider; the relay authenticates itself, and
  the destination authenticates separately.
- **Website A** is optional. It can run on the VPS or on a home computer using
  only outbound connections.
- There is no mandatory YamiNet-operated server, account, directory or telemetry
  service.

### Status labels used in this guide

| Label | Meaning |
| --- | --- |
| **PASS** | An automated check ran on this machine and succeeded |
| **FAIL** | A check ran and failed |
| **NOT RUN** | The check was not possible or not requested. Never treat it as success |

A `PASS` from a one-machine test does **not** prove Internet reachability,
provider routing, firewall state or independent-client behaviour.

---

## 1. Prepare the VPS

You need:

- a Linux VPS with a public IPv4 address (IPv4 only in this kit);
- SSH access and permission to open the approved TCP ports;
- Rust 1.90 or later if you build from source;
- the repository checkout on the VPS, or a way to copy the built binaries.

### 1.1 Create a dedicated unprivileged account

Do not run YamiNet as root.

```sh
sudo useradd --system --create-home --home-dir /var/lib/yaminet \
  --shell /usr/sbin/nologin yaminet
sudo install -d -m 0700 -o yaminet -g yaminet /var/lib/yaminet/testnet
```

Confirm:

```sh
id yaminet
```

### 1.2 Build the binaries

From the repository root:

```sh
cargo build --release --locked \
  -p yami-node -p yami-relay-cli -p yami-testnet
```

Install the executables and the operator wrapper:

```sh
sudo install -d -m 0755 /opt/yaminet
sudo install -m 0755 target/release/yami-node \
  target/release/yami-relay target/release/yami-testnet /opt/yaminet/
sudo install -m 0755 deploy/testnet/yaminet-testnet /opt/yaminet/yaminet-testnet
```

The wrapper finds `/opt/yaminet/yami-testnet` automatically.

### 1.3 Find the two IPv4 addresses

- **Bind IPv4:** the address on the VPS interface, from `ip -4 address`. It may
  be private if the provider uses NAT.
- **Public IPv4:** the address clients actually reach, from the provider
  console. It may differ from the bind address.

Record both. Do not use `0.0.0.0`.

### 1.4 Decide the ports, but do not open them yet

The defaults are:

| Purpose | TCP port |
| --- | --- |
| Bootstrap DHT | 17444 |
| Relay | 17550 |
| Direct demonstration website (only if you choose direct mode) | 17445 |

You must explicitly approve and add firewall rules later. YamiNet never changes
them.

---

## 2. Start a bootstrap node

### 2.1 Create the offline profile

The initializer writes identities, a demo site, configurations and systemd unit
files. It does **not** start anything, publish anything, install a unit or touch
a firewall.

```sh
sudo -u yaminet /opt/yaminet/yaminet-testnet init \
  --profile /var/lib/yaminet/testnet \
  --mode public \
  --bind-ip YOUR_LOCAL_INTERFACE_IPV4 \
  --public-ip YOUR_REAL_PUBLIC_IPV4 \
  --relay --site --client \
  --binary-dir /opt/yaminet
```

`--relay --site --client` are optional components. Leave them out for a
bootstrap-only profile; the relay and website can be added later by re-running
`init` with the extra flags (existing files are never silently replaced).

Example output:

```text
Initialized testnet profile: /var/lib/yaminet/testnet
Mode: public
Bootstrap ID: yn1-................................
Bootstrap endpoint: 203.0.113.10:17444
Relay ID: yn1-................................
Relay endpoint: 203.0.113.10:17550
Website ID: yn1-................................
Website address: yami://yn1-................................/home
Client config: /var/lib/yaminet/testnet/client/client.conf
Files created: 12; existing files kept: 0
Nothing was started, installed, published or allowed through a firewall.
```

The example uses documentation addresses; your real output will show your
addresses.

### 2.2 Review before doing anything privileged

```sh
sudo -u yaminet find /var/lib/yaminet/testnet -maxdepth 3 -type f -print
sudo -u yaminet cat /var/lib/yaminet/testnet/bootstrap/bootstrap.conf
sudo -u yaminet cat /var/lib/yaminet/testnet/addresses.env
sudo -u yaminet cat /var/lib/yaminet/testnet/units/yaminet-testnet-bootstrap.service
```

Check:

- `identity`, `state` and `site` are absolute paths;
- `listen` is one specific bind address, never `0.0.0.0`;
- `advertise` is the public address you intend to publish;
- `public-listen yes` and `network public` are deliberate;
- the unit runs as `yaminet`, restarts on failure and logs to the journal.

`node.key` is secret. Only `node.id` and the full node ID are public. Never
paste `node.key` into a report, chat or ticket.

### 2.3 Print the firewall plan

```sh
sudo /opt/yaminet/yaminet-testnet firewall-plan \
  --profile /var/lib/yaminet/testnet
```

This is a **plan only**. It prints the exact provider, `ufw` and `firewalld`
commands. Nothing is applied.

Before enabling any host firewall, preserve your SSH access. For example, on
Ubuntu with UFW:

```sh
sudo ufw allow OpenSSH          # only if SSH uses the default profile
sudo ufw allow 17444/tcp        # bootstrap
sudo ufw allow 17550/tcp        # relay, if used
sudo ufw enable
sudo ufw status
```

If SSH uses another port, use the correct rule. Also open the same ports in the
**provider's external firewall/security group**. This is an explicit operator
action. Do not disable a firewall to troubleshoot.

### 2.4 Install and start

The installer requires root, shows the units, and asks for confirmation.

```sh
sudo /opt/yaminet/yaminet-testnet install \
  --profile /var/lib/yaminet/testnet \
  --user yaminet --yes --start
```

Without `--start`, units are copied but not enabled or started. With `--start`,
they are enabled now, which is an explicit action.

Check:

```sh
sudo systemctl status yaminet-testnet-bootstrap.service --no-pager
sudo journalctl -u yaminet-testnet-bootstrap.service -n 50 --no-pager
```

Look for:

```text
DHT v1 enabled, Public; signed sequence ... expires ...
Serving site /var/lib/yaminet/testnet/bootstrap/site via YNP over TLS 1.3 ...
Open in Yami Browser: yami://.../home
```

A line like `DHT startup publication failed` is not a reason to disable
authentication or open more ports. The maintenance loop retries; investigate
bootstrap reachability first.

### 2.5 Bootstrap-only checklist

- [ ] Identity created once and backed up privately.
- [ ] `listen` is a specific bind address.
- [ ] `advertise` is the real public address.
- [ ] Provider and host firewalls allow only the approved ports.
- [ ] Unit runs as the unprivileged account.
- [ ] Journal shows the expected listener and node ID.
- [ ] No central service or account was created.

---

## 3. Configure an optional independent relay

The generated profile already contains a relay when `--relay` was used.

Review:

```sh
sudo -u yaminet cat /var/lib/yaminet/testnet/relay/relay.conf
sudo -u yaminet cat /var/lib/yaminet/testnet/units/yaminet-testnet-relay.service
```

Expected shape:

```text
yaminet-config 1
role relay
network public
public-listen yes
identity /var/lib/yaminet/testnet/relay/identity
state /var/lib/yaminet/testnet/relay/state
listen BIND_IPV4:17550
advertise PUBLIC_IPV4:17550
```

The relay has its **own** identity. It is not the website identity. The outer
TLS session authenticates the relay; the inner TLS session authenticates the
destination. Do not replace either pin with a key discovered at runtime.

Start it with the same explicit installer (idempotent; unchanged units are
reported as unchanged):

```sh
sudo /opt/yaminet/yaminet-testnet install \
  --profile /var/lib/yaminet/testnet \
  --user yaminet --yes --start
```

If you use a separate relay machine, build the same binaries and run the same
steps there with that machine's bind/public IPv4. There is no central relay
registry; hosts and clients learn relay pins through a trusted channel.

---

## 4. Host a test website

### Option A: demonstration website on the VPS (relay path)

The profile created with `--site --relay` contains:

```text
/var/lib/yaminet/testnet/host/site/
  home.ylang
  about.ylang
  default.ysl
  assets/cover.png
```

Edit the pages with any UTF-8 text editor. The node reads them on each request,
so no restart is needed after a page edit. Keep the identity directory separate
from the website directory.

Start the host unit:

```sh
sudo systemctl enable --now yaminet-testnet-host.service
sudo journalctl -u yaminet-testnet-host.service -n 50 --no-pager
```

Wait for a line reporting registration at the relay. Then read the public
address:

```sh
sudo -u yaminet cat /var/lib/yaminet/testnet/addresses.env
```

`HOST_URL` is the address to share.

### Option B: website behind a home router (recommended for NAT testing)

The home computer makes **outbound-only** connections. It opens no listener and
needs no router port forwarding.

On the home computer, build the binaries and install the wrapper. On Linux:

```sh
sudo useradd --system --create-home --home-dir /var/lib/yaminet \
  --shell /usr/sbin/nologin yaminet
sudo install -d -m 0755 /opt/yaminet
sudo install -m 0755 target/release/yami-node \
  target/release/yami-relay target/release/yami-testnet /opt/yaminet/
sudo install -m 0755 deploy/testnet/yaminet-testnet /opt/yaminet/yaminet-testnet
```

Create the outbound host profile with the **already running** bootstrap and
relay pins:

```sh
sudo -u yaminet /opt/yaminet/yaminet-testnet init \
  --profile /var/lib/yaminet/testnet-home \
  --home --site --client \
  --bootstrap-pin "BOOTSTRAP_ID@VPS_PUBLIC_IP:17444" \
  --relay-pin "RELAY_ID@VPS_PUBLIC_IP:17550" \
  --binary-dir /opt/yaminet
```

This creates only a host config, a demo site and a host unit; no bootstrap or
relay listener is created.

Review and start:

```sh
sudo -u yaminet cat /var/lib/yaminet/testnet-home/host/host.conf
sudo /opt/yaminet/yaminet-testnet install \
  --profile /var/lib/yaminet/testnet-home --user yaminet --yes --start
sudo journalctl -u yaminet-testnet-host.service -n 50 --no-pager
```

On Windows there is no systemd. From the repository root in PowerShell, create
the same outbound profile and start it manually:

```powershell
.\target\release\yami-testnet.exe init `
  --profile "$env:LOCALAPPDATA\YamiNetTestnetHome" `
  --home --site --client `
  --bootstrap-pin "BOOTSTRAP_ID@VPS_PUBLIC_IP:17444" `
  --relay-pin "RELAY_ID@VPS_PUBLIC_IP:17550" `
  --binary-dir (Resolve-Path .\target\release)
.\target\release\yami-node.exe --config "$env:LOCALAPPDATA\YamiNetTestnetHome\host\host.conf"
```

Do not create a Windows service or scheduled task automatically. If you choose
to add one, that is a separate explicit operator action. The outbound-only host
still needs no inbound firewall rule or router forwarding.

### Keeping the identity

The website's `yami://` address is derived from its persistent node identity.
Back up the identity directory privately. Losing or replacing it creates a new
website identity, and clients must learn the new address through a trusted
channel. Never place `node.key` inside the website directory or a Git
repository.

---

## 5. Share the website address

Public values are:

- the full bootstrap node ID and `IPv4:17444`;
- the full relay node ID and `IPv4:17550` (if used);
- the full website ID and its `yami://.../home` address.

Share them through a trusted channel. A signature or successful connection
proves possession of a key, not that the operator is trustworthy.

Example:

```text
Bootstrap: yn1-...@203.0.113.10:17444
Relay:     yn1-...@203.0.113.10:17550
Website:   yami://yn1-.../home
```

Do not publish `node.key`, state files or private logs. Do not imply that a
shared address is a verified public directory listing.

---

## 6. Connect from a separate client with Yami Browser

Use a different computer or connection from the website host. A laptop on a
mobile hotspot is a common choice.

### 6.1 Add the bootstrap pin

In Yami Browser:

1. Open **Settings & first launch**.
2. Enable network browsing.
3. Choose **Add bootstrap peer** in the sidebar.
4. Paste the full pin: `FULL_BOOTSTRAP_ID@PUBLIC_IPV4:17444`.
5. Enable **Relay fallback** only if you intend to use the relay.
6. Paste the website's `yami://.../home` address and press Enter.

Or, for testing from a terminal:

```sh
yami-browser --bootstrap "BOOTSTRAP_ID@PUBLIC_IPV4:17444" \
  --enable-relays --desktop "yami://HOST_ID/home"
```

### 6.2 Verify the identity indicators

For a relayed website, the native window should show:

```text
Verified node · TLS 1.3 · Relayed
```

For a direct website:

```text
Verified node · TLS 1.3
```

Compare the full destination ID with the website owner's value and the relay ID
with the relay operator's value. Do not accept "a connection succeeded" as
identity verification.

### 6.3 Headless preflight on the independent client

Build or copy the `yami-testnet` binary to the client. It needs no identity and
does not start a listener. Create `client.conf` on the client:

```text
yaminet-config 1
role browser
network public
state /home/CLIENT_USER/.local/share/yaminet/testnet-client
bootstrap BOOTSTRAP_ID@PUBLIC_IPV4:17444
relay-fallback yes
```

Then run:

```sh
yami-testnet preflight \
  --config client.conf \
  --url "yami://HOST_ID/home" \
  --asset cover \
  --route relay \
  --include-endpoints \
  --json > preflight-relay.json
```

To see the human-readable report, omit `--json`:

```text
YamiNet testnet preflight (yaminet-testnet-preflight-1 ...)
Target: yn1-.../home (asset cover)
Scope: automated local checks only; independent machines and physical topology NOT VERIFIED.

[PASS] Configuration validity - ...
[PASS] Bootstrap peer reachability - ...
[PASS] Signed DHT lookup - fresh signed record accepted ...
[PASS] Direct versus relay route selection - ...
[PASS] Exact destination TLS authentication - ...
[PASS] Relay identity - outer TLS authenticated the relay; destination authenticated separately
[PASS] YNP document retrieval - ...
[PASS] PNG asset retrieval - ...
[PASS] Relay fallback - ...
[NOT RUN] Validation scope - signed record uses remote numeric endpoints; independent machines and physical topology are still NOT VERIFIED

RESULT: PASS - ...
```

The `[NOT RUN] Validation scope` line is deliberate: a successful fetch from
one client does not prove provider routing or that the machines are physically
independent.

### 6.4 Test direct failure and relay fallback

For a relay-only home host:

```sh
yami-testnet preflight --config client.conf \
  --url "yami://HOST_ID/home" --route direct
```

Expected: `FAIL` because the signed record has no direct endpoint. This is the
correct negative result. The runner does not invent an endpoint and does not
fall back to plaintext.

Then run `--route relay` and confirm `PASS`. See
[the detailed relay procedure](../deploy/testnet/scenarios/relay-fallback.md)
for outage, identity-mismatch and recovery checks.

---

## 7. Troubleshoot failures

Start with the structured report, not with firewall changes.

```sh
yami-testnet check-config /path/to/config.conf
yami-testnet preflight --config client.conf --url "yami://HOST_ID/home" --asset cover
```

| Symptom | Check | Safe action |
| --- | --- | --- |
| `configuration failed strict validation` | Read the named line; compare with `deploy/testnet/templates/` | Fix the config; never disable strict validation |
| `identity directory is missing` | `identity` path; use the same identity directory | Restore from backup; do not generate a replacement under the same name |
| `node.key permissions are too open` | `ls -l` the identity directory | `chmod 700 DIR`, `chmod 600 DIR/node.key` |
| `bootstrap-reachability` FAIL | Is B running? Full ID correct? Provider/host firewall and the **advertised** port correct? | Fix routing or the pin; do not use plaintext |
| `signed-dht-lookup` FAIL | Clock synchronized? B has the host record? Host registered? | Check `journalctl`, `yami-node dht status`; wait for maintenance |
| `route-selection` FAIL | Does the record list a direct endpoint or relay route? Is `relay-fallback yes`? | Enable relay fallback explicitly, or fix the host publication |
| `destination-tls` identity mismatch | Compare the full destination ID with the owner's value | Stop; do not replace the pin to silence the error |
| `document-retrieval` FAIL | Is the page key correct? Does the site pass `site check`? | Fix the page; inspect host logs |
| `png-retrieval` FAIL | Is `assets/KEY.png` a bounded, non-animated PNG? | Replace the asset; keep it under the documented limits |
| `local-relay-registration` NOT RUN | Normally absent on a remote client | Run preflight on the host for a local snapshot; it is not external proof |
| Unit exits immediately | `journalctl -u UNIT -n 100`; config paths/permissions | Fix the config; do not run as root |
| Port already in use | `ss -ltn`; stop the conflicting process explicitly | Choose another approved port or stop the other service |
| Browser says relay routes need opt-in | Enable Relay fallback; verify relay pin | Enabling fallback is explicit; no automatic downgrade |

Useful commands:

```sh
sudo journalctl -u yaminet-testnet-bootstrap.service -n 100 --no-pager
sudo journalctl -u yaminet-testnet-relay.service -n 100 --no-pager
sudo journalctl -u yaminet-testnet-host.service -n 100 --no-pager
sudo /opt/yaminet/yaminet-testnet status --profile /var/lib/yaminet/testnet
sudo /opt/yaminet/yaminet-testnet logs --profile /var/lib/yaminet/testnet --lines 100
```

Do not:

- disable TLS or pin verification to "make it work";
- add a discovered key as a replacement pin;
- open `0.0.0.0` or every port;
- delete DHT state or keys as a routine fix;
- claim success based only on an open port or a signed advertisement.

---

## 8. Stop and completely remove test infrastructure

### 8.1 Stop services

```sh
sudo /opt/yaminet/yaminet-testnet stop --profile /var/lib/yaminet/testnet
```

### 8.2 Remove installed units

```sh
sudo /opt/yaminet/yaminet-testnet uninstall --yes
```

This stops/disables and removes only unit files whose Description identifies
this testnet. It does not delete profiles or identities.

### 8.3 Remove profiles and identities (irreversible)

Back up any identity you may need first. Then:

```sh
sudo /opt/yaminet/yaminet-testnet remove \
  --profile /var/lib/yaminet/testnet --confirm --purge-data
```

Safety behaviour:

- refuses without the profile marker;
- refuses without `--confirm` and `--purge-data`;
- refuses symlinked or overly broad paths;
- removes only known generated entries; unknown files cause the root directory
  to be retained so you can inspect them.

Repeat for a home-host profile (`/var/lib/yaminet/testnet-home`).

### 8.4 Close the firewall and provider rules

Use the plan output as a reminder, then remove the rules you added:

```sh
sudo ufw delete allow 17444/tcp
sudo ufw delete allow 17550/tcp
# also remove 17445/tcp if you used direct website mode
sudo ufw status
```

Remove the corresponding provider firewall/security-group rules. YamiNet does
not do this automatically.

### 8.5 Remove the account and binaries

```sh
sudo userdel yaminet
sudo rm -rf /opt/yaminet
```

Only do this after confirming no other service uses the account or directory.
Remove your backups and test reports according to your own data-retention rules.

### 8.6 Verify removal

```sh
systemctl list-units 'yaminet-testnet-*' --all
sudo ss -ltn | grep -E ':(17444|17550|17445)\b' || true
test ! -e /var/lib/yaminet/testnet && echo removed
```

The interface may still have unrelated services on those ports; check the
process before assuming YamiNet is still running.

---

## What the automated tests do and do not prove

The repository includes:

- configuration and strict-parser tests;
- missing/malformed identity tests and Unix owner-only permission tests;
- systemd rendering and idempotent-write tests;
- profile init/removal safety tests;
- loopback tests that start real TLS listeners, signed DHT services, a real
  relay, an outbound host, and fetch a real YLang document plus PNG;
- direct-failure/relay-fallback and wrong-destination-identity tests.

They prove software behaviour on one machine. They do **not** prove:

- VPS provider firewall, NAT or routing;
- home-router behaviour or absence of forwarding;
- an independent client on a different network;
- long-duration stability, abuse resistance or security review;
- anonymity or production readiness.

Record independent tests with
[the cross-network checklist](../deploy/testnet/scenarios/cross-network-checklist.md).
If a check was not performed, write `NOT RUN`. Never upgrade a loopback PASS to
an Internet PASS.

## Related documentation

- [Existing public-relay notes](public-relay.md)
- [Relay integration testing](relay-integration-testing.md)
- [Configuration reference](configuration.md)
- [Security notes](../SECURITY.md)
- [Deployment kit README](../deploy/testnet/README.md)
- [Local validation record and NOT RUN list](../deploy/testnet/VALIDATION.md)
- [Loopback rehearsal](../deploy/testnet/scenarios/loopback-rehearsal.sh)
