# YamiNet public-testnet kit

This directory contains operator tooling for a **self-hosted, experimental**
YamiNet test network. It does not create a central service, account, telemetry
feed or mandatory hosting relationship.

> Public-network testing is incomplete. YamiNet is not an anonymity network and
> this kit makes no production-security claim. Do not publish sensitive content.
> Do not run the deployment steps on a real server without the operator's
> explicit authorization.

## What is here

| Path | Purpose |
| --- | --- |
| `yaminet-testnet` | Operator wrapper: init, preflight, check-config, install/uninstall, status, logs, stop, firewall plan, remove |
| `templates/` | Manual configuration and systemd templates; the generated profile is safer than editing these |
| `scenarios/loopback-rehearsal.sh` | Starts bootstrap + relay + outbound host on loopback and runs the same preflight path |
| `scenarios/relay-fallback.md` | Reproducible direct-failure and relay-fallback procedure |
| `scenarios/cross-network-checklist.md` | Record which checks actually ran on independent machines |
| `scenarios/public-internet-e2e.md` | VPS/provider + relay, outbound home website/game, independent client commands and approval gates |
| `scenarios/public-e2e-rehearsal.py` | Combined existing-kit rehearsal: YNS, DHT, pinned relay delivery, navigation, assets, sandboxed games, failures and recovery; LOOPBACK ONLY |
| `results/2026-11-01-public-e2e.md` | Current observed local results and explicitly NOT RUN real-internet checks |
| `VALIDATION.md` | What this checkout validated locally, and what remains NOT RUN |
| `../../docs/public-testnet.md` | Full beginner deployment guide |

## Safety model

- `init` is offline. It writes a marked profile, keys, a demo site, configs and
  unit files. It never starts a listener, publishes a record, installs a unit or
  changes a firewall.
- `preflight` sends bounded requests only to explicitly configured bootstrap,
  relay and destination endpoints. It never falls back to plaintext.
- `install` requires root, prints the reviewed units and asks for confirmation.
  Units are not enabled or started unless `--start` is supplied.
- `uninstall` removes only unit files whose Description names this testnet and
  only after confirmation. Profiles and identities are retained.
- `remove` refuses to delete anything without a marker file, `--confirm` and
  `--purge-data`. It never follows a symlinked profile and never deletes
  unrecognized files silently.
- No command performs router port forwarding, changes firewall rules, integrates
  Tor/I2P, creates accounts or sends telemetry.

## Build

```sh
cargo build --release --locked -p yami-node -p yami-relay-cli -p yami-testnet
```

## Quick start (loopback rehearsal, one machine)

```sh
cargo build --locked -p yami-node -p yami-relay-cli -p yami-testnet
./deploy/testnet/scenarios/loopback-rehearsal.sh
```

The script prints `Rehearsal PASS (loopback simulation only; independent
machines NOT VERIFIED)`. That is local software validation, not a VPS or
cross-network test.

## Quick start (real VPS)

Follow [the full guide](../../docs/public-testnet.md). The abbreviated sequence
is:

```sh
# Prepare the VPS, dedicated user and binaries first (guide step 1).
sudo -u yaminet /opt/yaminet/yami-testnet init \
  --profile /var/lib/yaminet/testnet \
  --mode public \
  --bind-ip YOUR_LOCAL_IPV4 \
  --public-ip YOUR_PUBLIC_IPV4 \
  --relay --site --client \
  --binary-dir /opt/yaminet

# Review every config and unit, then print (not apply) firewall commands.
sudo /opt/yaminet/yaminet-testnet firewall-plan --profile /var/lib/yaminet/testnet

# Install only reviewed units; --start is a separate explicit action.
sudo /opt/yaminet/yaminet-testnet install \
  --profile /var/lib/yaminet/testnet --user yaminet --yes --start
```

The generated `addresses.env` contains the public bootstrap/relay/website IDs
and the website's `yami://` address. Share only those public values.

## Preflight status meanings

| Status | Meaning |
| --- | --- |
| `PASS` | The automated check ran on this machine and succeeded |
| `FAIL` | The check ran and failed, or a required prerequisite is invalid |
| `NOT RUN` | The check was not possible or not requested; never treat this as success |

`PASS` never means "the Internet can reach this service". The report's scope
always states that independent machines and physical topology are `NOT VERIFIED`
unless a human separately records those tests.

## Tests

```sh
cargo test -p yami-testnet --locked
cargo clippy -p yami-testnet --all-targets --locked -- -D warnings
cargo fmt --all -- --check
```

The loopback integration tests start real TLS listeners, signed DHT services,
a real relay and an outbound host. Their PASS is still a single-machine
simulation.

## YNS 0.2 and hosted-game public trial

Use [the three-machine runbook](scenarios/public-internet-e2e.md) and the
[bootstrap/provider template](templates/bootstrap-yns.conf.example). `init --yns`
is an explicit, offline opt-in for a directly listening bootstrap provider on its
existing TLS port. It never enables YNS on an outbound home host. Existing configs
are not silently rewritten when a new flag is supplied.

The combined local rehearsal reuses this kit and the existing native binaries:

```sh
cargo build --workspace --locked
python deploy/testnet/scenarios/public-e2e-rehearsal.py --report dist/public-e2e-rehearsal.json
```

It performs local child-process starts/stops only, never remote deployment,
service installation, firewall edits or router changes. Its PASS cannot satisfy
the public-internet gate: the report separately records that gate as NOT RUN.
Both Windows and Linux CI run this same rehearsal.
