# Validated networking configuration and unattended operation

Configuration is optional. Existing CLI commands remain supported. No listener,
identity, router mapping or service is created merely by reading/checking a file.
The configured startup paths require an **existing persistent identity**; a typo
cannot silently create a replacement website/relay key. Configuration and status
are implemented in Rust with no new external parser dependency.

## File format and validation

Use UTF-8, LF or CRLF, with `yaminet-config 1` as the first non-comment line.
Each subsequent line is `field value`. Full lines starting with `#` are comments.
Empty lines are ignored. Paths must be absolute; spaces in paths require **no
quotes**. Backslashes in Windows paths are literal, not escapes. Do not use TOML
syntax, `=`, inline comments, shell substitution or environment placeholders.
The file contains public pins, paths and settings, never private key material.

Files are limited to 16 KiB and 96 lines. Unknown fields, duplicate singleton
fields, invalid booleans/integers, conflicting role settings, wildcard listeners,
special-use endpoints, duplicate pins and out-of-range budgets fail before
network startup. Error messages identify the field/line without dumping values.
`network` defaults `loopback`; public discovery needs explicit `network public`.
Public listening **also** needs `public-listen yes` and a specific `listen` IPv4.
Public relays require a separately selected public `advertise` endpoint. Merely
selecting public network policy does not enable a listener.

| Field | Meaning/limits |
| --- | --- |
| `role` | Required: `node`, `relay` or `browser`; executable must match |
| `state` | Required absolute state directory, unique per running instance |
| `network` | `loopback` (default) or `public`; never mix public and isolated records |
| `identity` | Existing absolute identity directory for node/relay; forbidden for browser |
| `site` | Absolute directory of YLang pages and optional default.ysl; node only |
| `listen` | Explicit IPv4 and nonzero TCP port for direct node/relay |
| `public-listen` | `yes`/`no` (default no); explicit outside-loopback listener opt-in |
| `advertise` | Repeat 1..4 direct endpoints for DHT node; exactly one for public relay |
| `bootstrap` | Repeat up to fifteen unique full `ID@IPv4:port` pins; node/browser |
| `relay` | One or two explicit `ID@IPv4:port` routes select outbound-only node hosting |
| `relay-fallback` | Browser only, `yes`/`no` (default no) |
| `dht` | Direct node only, `yes`/`no` (default no); yes enables existing maintenance |
| `yns` | Direct node only, `yes`/`no` (default no); serves existing YNS on the same pinned TLS listener, storing records in `state/yns`; forbidden on outbound relay hosts |
| `workers` | Direct node workers, 1..32 (default 4) |
| `renew-secs` | Node maintenance renewal, 30..43200 (default 21600) |
| `health-secs` | Node maintenance health, 5..3600 (default 300) |

A node selects either direct TLS serving or outbound relay-host mode. Relay-host
mode has no listen/advertise/dht fields: registration and DHT maintenance are
automatic, and no inbound socket is constructed. Hybrid direct-plus-relay node
CLI configuration is not provided in this milestone. Configured bootstrap pins
are imported idempotently into the existing peer store; conflicting persisted
endpoints fail until explicitly removed with `yami-node peer remove ID --dir
STATE`. Existing additional manual peers remain available. A config file does
not silently replace trust pins or become a remotely supplied trust graph.

## Validate, start and inspect

Use [templates](../deploy) after replacing all uppercase placeholders. They use
separate persistent state directories for each process. Public templates are
intentionally invalid until edited, preventing accidental public exposure.

```sh
yami-node check-config /etc/yaminet/host.conf
yami-node --config /etc/yaminet/host.conf

yami-relay check-config /etc/yaminet/relay.conf
yami-relay --config /etc/yaminet/relay.conf
yami-relay status --config /etc/yaminet/relay.conf

yami-browser check-config /home/USER/.config/yaminet/browser.conf
yami-browser --config /home/USER/.config/yaminet/browser.conf --desktop yami://FULL_HOST_ID/home
yami-browser --config /home/USER/.config/yaminet/browser.conf --network-status
```

`check-config` checks syntax/policy and has no directory/key/network side effects.
Startup additionally loads the existing key, opens state and checks site paths.
Node and relay configurations are whole startup profiles, not mixed with other
positional commands. Browser `--enable-relays`/`--disable-relays` override its
file setting for subsequent requests; its Add bootstrap peer action uses the
configured directory. No file setting can turn on public browser listening.

Node status uses existing maintenance snapshots:

```sh
yami-node dht status --dir /var/lib/yaminet/host/state
yami-node dht peers --dir /var/lib/yaminet/host/state
```

Add `--local` for isolated loopback state. Counts include known peers, recently
verified peer samples, last successful publication, next attempt, errors,
registration availability, registration failures and cache history occupancy.
A relay health sample over ten seconds old contributes no currently registered
routes to diagnostics. Browser diagnostics are signed cache/location counts,
not a verified indicator. A configured relay records bounded connection,
registration, tunnel and forwarded-byte counters once per second in its existing
accept loop. No additional maintenance thread or status network endpoint is
created. `status` is read-only and reports stale/stopped snapshots after a crash
or clean exit; it does not prove external reachability or destination identity.

## Windows hosting

Use the [existing Windows build and host walkthrough](public-relay.md), including
one-time MSVC/Rust setup, two sample pages, bootstrap/relay ID verification and
outbound firewall rules. Afterwards use a config file in Notepad:

```text
yaminet-config 1
role node
network public
identity C:\Users\YOUR_USER\AppData\Local\YamiNetRelayTrial\identity
state C:\Users\YOUR_USER\AppData\Local\YamiNetRelayTrial\peers
site C:\Users\YOUR_USER\AppData\Local\YamiNetRelayTrial\site
relay FULL_RELAY_ID@REAL_PUBLIC_IPV4:17550
bootstrap FULL_DHT_PEER_ID@REAL_PUBLIC_IPV4:17444
```

Replace the username/IDs/IPs; do not quote paths even if they contain spaces.
Save as UTF-8 without BOM. Then, from the build directory:

```powershell
.\target\release\yami-node.exe check-config C:\Users\YOUR_USER\yaminet-host.conf
.\target\release\yami-node.exe --config C:\Users\YOUR_USER\yaminet-host.conf
```

No inbound Windows firewall rule or home-router port forwarding is required.
Keep identity/state/site directories private using Windows ACLs; Unix mode checks
do not inspect Windows ACLs. Ctrl-C stops hosting; wait for exit before restart.
Windows runtime behavior remains unverified in this Linux development environment.

## Optional Linux VPS service

Follow [public VPS setup](public-relay.md) for the unprivileged account, explicit
key creation, independently verified IDs, local/public addresses and OS/provider
firewall configuration. Install binaries under `/opt/yaminet`, keys under
`/var/lib/yaminet/relay/identity`, state under `/var/lib/yaminet/relay/state`, and
the edited profile at `/etc/yaminet/relay.conf`. The service account must own its
key/state; use 0700 directories and 0600 key/config files. Do not run as root.

[The optional systemd example](../deploy/yaminet-relay.service) is not installed or
enabled automatically. Review its paths and limits for your host, then explicitly:

```sh
/opt/yaminet/yami-relay check-config /etc/yaminet/relay.conf
sudo cp deploy/yaminet-relay.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now yaminet-relay
systemctl status yaminet-relay
journalctl -u yaminet-relay -n 50 --no-pager
/opt/yaminet/yami-relay status --config /etc/yaminet/relay.conf
```

Use an appropriately authorized account for private state inspection. This is
an example service, not a tested production deployment. Bound application
resources do not stop distributed/volumetric DoS. The separate DHT peer needs
its own identity/config/state and public TCP port. No mandatory central server
or provider is introduced.

## Safe upgrade and rollback

1. Record the public node/relay IDs and deployed revision. Keep trusted binaries
   and configuration under operator control. Public identity pins must not
   change as a workaround for failed authentication.
2. Stop the service (`systemctl stop yaminet-relay`) or Ctrl-C maintained nodes;
   wait for process exit and state locks to release. Do not kill during a normal
   update merely to bypass a lock. After a crash preserve snapshots before repair.
3. Back up the identity directory and the **matching full state directory** with
   filesystem permissions/ACLs preserved, using a secure backup mechanism. Do
   not print, email or commit `node.key`. Keep browser DHT history too.
4. Install the new binaries, retain absolute paths, run `check-config`, restart
   with the same key/state, and compare the public IDs and publication sequence.
   DHT renewal reserves increasing sequence values before publication. Do not
   replace identity/state with fresh empty directories.
5. A binary rollback must keep current sequence history and use a version that
   can read the current record format. Never restore an old cache backup after
   newer signed records have escaped: peers should reject that rollback. Contact
   operators/repair state using an explicit recovery design if a sequence backup
   was lost; clearing caches or weakening pinning is not a safe upgrade procedure.

The current snapshot stays compatible with earlier v1/v2 records. Reserved
cache admission does not erase legacy full snapshots; see [remaining storage
and Sybil limits](dht-maintenance.md). No deployed infrastructure or live firewall
was modified while preparing these instructions.
