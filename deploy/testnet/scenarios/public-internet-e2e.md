# First public-internet trial: VPS, outbound home host, independent browser

This extends the existing testnet kit; it does not deploy a server, change a
firewall, introduce a new transport, or create a central registry. The local
rehearsal and its evidence are [recorded separately](../results/2026-11-01-public-e2e.md).
**The real public-internet run is NOT RUN in this checkout.** No authorized VPS,
home-machine or independent-client endpoints/access were supplied.

## Topology and approval gates

| Machine | Processes / persistent state | Inbound connectivity |
| --- | --- | --- |
| Linux VPS | Existing DHT bootstrap with YNS enabled; separate relay identity/state | Approved public IPv4 TCP 17444 (DHT + YNS on one TLS listener), 17550 (relay) |
| Home PC | Existing outbound relay-host; website, PNGs, hosted Wasm; separate namespace owner key | None; no listener, router mapping or firewall opening |
| Independent client | Yami Browser and testnet preflight; its own browser state | None; a different physical machine and internet connection |

Preparation, review and offline `init` do not authorize deployment. Have the VPS
and home operators explicitly approve process/service startup before performing
those steps. Firewall changes require their own explicit approval; use the
existing `firewall-plan` command for review. No firewall changes are performed by
the commands in the rehearsal. Keep the existing TLS pins, no-WASI sandbox, fuel,
memory and transfer bounds. Never clear keys/history or weaken verification to
obtain a PASS.

Build the same reviewed commit on all machines, or copy its validated native
packages. Record `git rev-parse HEAD`, platform and versions with the results.

```sh
cargo build --workspace --release --locked
```

For the publisher preparation tool, also build the existing example:

```sh
cargo build --release --locked -p yami-games --example publish
```

Use complete canonical IDs from the operators, never abbreviated IDs. Replace
all uppercase placeholders below; configuration files do not expand variables.
Documentation IP ranges such as `203.0.113.0/24` intentionally fail public-policy
validation. There is no built-in provider/registry root. Do not use `.yami` for
this trial: use an owner-scoped `.yns` name.

## 1. Linux VPS: prepare, review, then start only after approval

Follow [the existing VPS preparation guide](../../../docs/public-testnet.md#1-prepare-the-vps)
for a dedicated unprivileged account and `/opt/yaminet` binaries. Retain existing
identities if a profile already exists; do not silently replace it. For a fresh
profile, set the real local interface IPv4 and externally reachable public IPv4:

```sh
VPS_PROFILE=/var/lib/yaminet/public-trial
VPS_BIND_IPV4=REPLACE_WITH_SPECIFIC_INTERFACE_IPV4
VPS_PUBLIC_IPV4=REPLACE_WITH_ROUTABLE_PUBLIC_IPV4
sudo -u yaminet /opt/yaminet/yami-testnet init \
  --profile "$VPS_PROFILE" --mode public \
  --bind-ip "$VPS_BIND_IPV4" --public-ip "$VPS_PUBLIC_IPV4" \
  --relay --yns --binary-dir /opt/yaminet
sudo -u yaminet /opt/yaminet/yami-testnet check-config "$VPS_PROFILE/bootstrap/bootstrap.conf"
sudo -u yaminet /opt/yaminet/yami-testnet check-config "$VPS_PROFILE/relay/relay.conf"
sudo -u yaminet cat "$VPS_PROFILE/addresses.env"
sudo /opt/yaminet/yaminet-testnet firewall-plan --profile "$VPS_PROFILE"
```

There is deliberately no `--site`: the user's website is on the home PC. The
bootstrap still has the kit's small informational site, which is not the home
website. `--yns` writes `yns yes` to the bootstrap node profile. Configured startup
reuses `serve-dir --yns-dir STATE/yns`, under the unit's existing writable state
path and TLS listener. It opens no additional port and generates no namespace or
official root key. An outbound `--home` profile cannot enable this provider.

Review both configs and generated units. Check the specific bind, public
advertisement, separate identities, unprivileged account, `ProtectSystem=strict`,
`NoNewPrivileges=true` and restricted `ReadWritePaths`. `init` refuses different
existing configs; enable `yns yes` by an explicit reviewed edit on an existing
direct node rather than expecting `init` to upgrade it silently.

Only after the operator authorizes service installation/start and independently
reviews any needed host/provider firewall plan:

```sh
sudo /opt/yaminet/yaminet-testnet install --profile "$VPS_PROFILE" --user yaminet --yes --start
sudo /opt/yaminet/yaminet-testnet status --profile "$VPS_PROFILE"
sudo /opt/yaminet/yaminet-testnet logs --profile "$VPS_PROFILE" --lines 80
```

Share only `BOOTSTRAP_ID`, `BOOTSTRAP_ENDPOINT`, `RELAY_ID`, `RELAY_ENDPOINT` and
`YNS_PROVIDER_PIN` from the locally generated `addresses.env` through an
independently trusted channel. Do not source shell files received from strangers.
Keep `node.key`, state/history files and raw private logs on their own machines.
If the VPS uses provider NAT, verify that the advertised address/ports actually
forward to the specific bind; neither the kit nor this test opens mappings.

## 2. Home PC: outbound-only website and hosted game

Use the pins verified from the VPS. On Linux, set the variables below to actual
values (the bootstrap and provider pins share an ID/port; the relay ID is separate):

```sh
HOST_PROFILE=/absolute/private/yaminet-home-trial
BINARY_DIR=/absolute/YamiNet/target/release
BOOTSTRAP_PIN=FULL_BOOTSTRAP_ID@PUBLIC_IPV4:17444
RELAY_PIN=FULL_RELAY_ID@PUBLIC_IPV4:17550
"$BINARY_DIR/yami-testnet" init --profile "$HOST_PROFILE" --home --site --client \
  --bootstrap-pin "$BOOTSTRAP_PIN" --relay-pin "$RELAY_PIN" --binary-dir "$BINARY_DIR"
"$BINARY_DIR/yami-testnet" check-config "$HOST_PROFILE/host/host.conf"
```

The example executable is `target/release/examples/publish`, not a normal packaged
binary. Invoke the example built above from the source checkout:

```sh
target/release/examples/publish games/breakout/game.yami-game.json "$HOST_PROFILE/host/site" prism
printf '\n> game:prism | Play Prism Breakout\n' >> "$HOST_PROFILE/host/site/home.ylang"
"$BINARY_DIR/yami-node" site check "$HOST_PROFILE/host/site"
cat "$HOST_PROFILE/host/host.conf"
cat "$HOST_PROFILE/addresses.env"
```

The publication tool refuses existing game files; prepare this once in a fresh
staging site. It validates the local Wasm, emits the hosted integrity manifest,
and copies the module into the publisher's site. Players never run this tool or
install the module. The generated personal site already has `/about` navigation
and `assets/cover.png`.

After the home operator approves starting the host, run in a terminal:

```sh
"$BINARY_DIR/yami-node" --config "$HOST_PROFILE/host/host.conf"
```

There must be no `listen`, `advertise`, `public-listen yes`, `dht yes` or `yns yes`
in this outbound host config. Wait for authenticated relay registration and DHT
publication. Read the home profile's `HOST_ID` / `HOST_URL`; do not accidentally
use the bootstrap's informational page address. No home inbound port is needed.

### Windows home PC equivalents

Build with the same commands in PowerShell; use these absolute paths and verified
pins. Keep the node in its own terminal. No Windows service, scheduled task,
firewall or router changes are implied.

```powershell
$YamiBin = (Resolve-Path target/release).Path
$TrialRoot = Join-Path $env:LOCALAPPDATA 'YamiNet\home-public-trial'
$BootstrapPin = 'FULL_BOOTSTRAP_ID@PUBLIC_IPV4:17444'
$RelayPin = 'FULL_RELAY_ID@PUBLIC_IPV4:17550'
& "$YamiBin/yami-testnet.exe" init --profile $TrialRoot --home --site --client --bootstrap-pin $BootstrapPin --relay-pin $RelayPin --binary-dir $YamiBin
& "$YamiBin/yami-testnet.exe" check-config "$TrialRoot/host/host.conf"
& "$YamiBin/examples/publish.exe" games/breakout/game.yami-game.json "$TrialRoot/host/site" prism
[IO.File]::AppendAllText("$TrialRoot/host/site/home.ylang", "`n> game:prism | Play Prism Breakout`n", [Text.UTF8Encoding]::new($false))
& "$YamiBin/yami-node.exe" site check "$TrialRoot/host/site"
Get-Content "$TrialRoot/addresses.env"
& "$YamiBin/yami-node.exe" --config "$TrialRoot/host/host.conf"
```

## 3. Sign and publish the readable name, without making the owner a server

Use a separate private owner directory outside the website. Signing can happen
offline, then only the signed public file moves to the publisher. The VPS never
needs the owner key. On the publisher, `PUBLIC_HOST_ID` is the independently checked
home website identity, not the relay or bootstrap ID.

```sh
CLIENT_CONFIG="$HOST_PROFILE/client/client.conf"
YNS_OWNER_DIR=/absolute/private/offline-trial-owner
YNS_NAME=trial.FULL_OWNER_ID.yns
PUBLIC_HOST_ID=FULL_HOME_WEBSITE_ID
YNS_PROVIDER_PIN=FULL_BOOTSTRAP_ID@PUBLIC_IPV4:17444
"$BINARY_DIR/yami-browser" --config "$CLIENT_CONFIG" yns namespace-create "$YNS_OWNER_DIR"
# Set YNS_NAME using the printed full OWNER-ID.yns before signing.
ISSUED=$(date +%s)
EXPIRES=$((ISSUED + 3600))
"$BINARY_DIR/yami-browser" --config "$CLIENT_CONFIG" yns record-sign "$YNS_OWNER_DIR" FULL_OWNER_ID.yns trial 1 "$ISSUED" "$EXPIRES" map "$PUBLIC_HOST_ID" > signed-trial.json
"$BINARY_DIR/yami-browser" --config "$CLIENT_CONFIG" yns publish "$YNS_NAME" "$YNS_PROVIDER_PIN" signed-trial.json
```

For the Windows home PC, in a separate PowerShell terminal using the variables
from step 2:

```powershell
$HomeClientConfig = "$TrialRoot/client/client.conf"
$OwnerDirectory = "$TrialRoot/naming-owner"
$Namespace = @(& "$YamiBin/yami-browser.exe" --config $HomeClientConfig yns namespace-create $OwnerDirectory)[0]
$YnsName = "trial.$Namespace"
$PublicHostId = ([IO.File]::ReadAllText("$TrialRoot/host/identity/node.id")).Trim()
$Issued = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$SignedRecord = & "$YamiBin/yami-browser.exe" --config $HomeClientConfig yns record-sign $OwnerDirectory $Namespace trial 1 $Issued ($Issued + 3600) map $PublicHostId
[IO.File]::WriteAllText("$TrialRoot/signed-trial.json", $SignedRecord + "`n", [Text.UTF8Encoding]::new($false))
& "$YamiBin/yami-browser.exe" --config $HomeClientConfig yns publish $YnsName $BootstrapPin "$TrialRoot/signed-trial.json"
Write-Output "Name: $YnsName; destination: $PublicHostId"
```

These writers avoid Windows PowerShell 5's default UTF-8 BOM. Store owner keys
privately and back them up. Do not use an arbitrary test key as a production
`.yami` registry root. Mirrors have no authority to change signed records.

## 4. Independent internet client: preflight, named delivery, navigation and play

Use a genuinely different internet connection (e.g. a laptop on mobile data, not
another process on the home network). Copy only verified public pins/name/host ID.
Create an absolute-path `client.conf` from [the client template](../templates/client.conf.example):

```text
yaminet-config 1
role browser
network public
state /absolute/private/public-trial-client-state
bootstrap FULL_BOOTSTRAP_ID@PUBLIC_IPV4:17444
relay-fallback yes
```

On Windows, use an absolute Windows state path; write the file without a BOM.
No identity or listener is configured on the client. For the commands below,
`CLIENT_CONFIG`, `YNS_NAME`, `PUBLIC_HOST_ID` and `YNS_PROVIDER_PIN` refer to the
verified values; `yami-browser` and `yami-testnet` refer to that machine's binaries.

```sh
yami-testnet check-config "$CLIENT_CONFIG"
yami-browser --config "$CLIENT_CONFIG" yns providers-set "$YNS_PROVIDER_PIN"
yami-testnet preflight --config "$CLIENT_CONFIG" --url "yami://$PUBLIC_HOST_ID/home" --asset cover --route relay --json > client-preflight.json
yami-browser --network-trial "yami://$PUBLIC_HOST_ID/home" --config "$CLIENT_CONFIG" --asset cover --name "$YNS_NAME" --navigate about --game prism > client-trial.json
```

The trial forces a live YNS query, checks its result against the separately supplied
full destination ID, requires a fresh signed DHT response, authenticates relay and
destination, parses the page/PNG, follows the advertised same-node link, and loads
the advertised game using the original origin pins. `--game prism` is explicit
consent to download and execute the bounded no-WASI sandbox. It waits for frames
and draw commands after synthetic START input. No `--game` means game execution
is NOT RUN; arbitrary unadvertised keys are refused. Reports redact endpoints by
default; add `--include-endpoints` only when comfortable storing those public
addresses in the evidence. Nonzero exit and any FAIL require investigation.

A Windows independent client can create the same profile without a BOM and run
the checks as follows, after assigning the independently verified `$BootstrapPin`,
`$YnsName` and `$PublicHostId` values (do not reuse the home PC's private state):

```powershell
$ClientBin = (Resolve-Path target/release).Path
$ClientRoot = Join-Path $env:LOCALAPPDATA 'YamiNet\independent-public-trial'
New-Item -ItemType Directory -Force $ClientRoot | Out-Null
$ClientConfig = Join-Path $ClientRoot 'client.conf'
$ClientText = "yaminet-config 1`nrole browser`nnetwork public`nstate $ClientRoot/state`nbootstrap $BootstrapPin`nrelay-fallback yes`n"
[IO.File]::WriteAllText($ClientConfig, $ClientText, [Text.UTF8Encoding]::new($false))
& "$ClientBin/yami-testnet.exe" check-config $ClientConfig
& "$ClientBin/yami-browser.exe" --config $ClientConfig yns providers-set $BootstrapPin
& "$ClientBin/yami-browser.exe" --network-trial "yami://$PublicHostId/home" --config $ClientConfig --asset cover --name $YnsName --navigate about --game prism | Out-File -Encoding utf8 "$ClientRoot/client-trial.json"
# Check $LASTEXITCODE and inspect every status; Out-File's report BOM does not affect configs.
& "$ClientBin/yami-browser.exe" --config $ClientConfig --desktop "yami://$YnsName/home"
```

Then do the separate native-window check:

```sh
yami-browser --config "$CLIENT_CONFIG" --desktop "yami://$YNS_NAME/home"
```

Explicitly enable network browsing and relay fallback in first-launch settings if
the browser is in local-only mode. Compare the actual full destination and relay
IDs in the security view. Click About, Back/Forward, and **Play game: Prism Breakout**;
exercise movement/start, pause/resume, restart and stop. Capture evidence from the
independent client. A headless VM producing draw commands does not mark GUI
rendering, keyboard input or physical independence PASS.

## 5. Failure and recovery checks (operator-controlled, preserve history)

Use a dedicated trial, not another operator's live services. Keep the same keys
and state; never disable TLS, relax sandbox bounds or erase rollback history.
Record the exact failing check; a config typo is not a successful negative test.

| Check | Procedure / required observation |
| --- | --- |
| Wrong destination ID | `yami-browser --probe "ynps://$PUBLIC_HOST_ID@PUBLIC_IPV4:17444/home"`; actual bootstrap must present its different ID; require the trusted-node authentication error and no document request |
| Wrong bootstrap ID | A separate client config pins the home host ID at the VPS bootstrap endpoint; trial must report bootstrap-pin FAIL; restore the correct config |
| Wrong YNS provider ID | A fresh negative browser profile pins the home host ID at the VPS provider endpoint; resolving the known name must fail with no imported record; do not replace the working profile's pins |
| Direct route to outbound-only host | Preflight `--route direct` must FAIL at route-selection because the signed record has no direct endpoint; `--route relay` must PASS |
| Unavailable bootstrap/provider | After explicit VPS operator approval, stop only the bootstrap service; fresh trial must fail despite old cached records; restore the same service/key/state and require recovery |
| Relay outage | After explicit VPS operator approval, stop only `yaminet-testnet-relay.service`; authenticated delivery must fail or use an explicitly approved alternate relay; no plaintext fallback |
| Relay recovery | Start that same relay service/key/state; allow host reconnect/backoff; rerun the full trial including the game and verify the recovered destination/relay IDs |
| Expired YNS record | Sign another label with short TTL (e.g. 20 seconds), publish and resolve it; after UTC >= expires, forced refresh must fail rather than use the expired cache; publish a higher sequence with a live expiry and require recovery |
| Expired DHT record | Record the returned sequence/expiry; with renewal stopped, wait for actual expiry and verify rejection. DHT TTL is currently 24 hours, independent of renewal interval. Local fake-clock tests are not this public test; leave NOT RUN unless actually completed |
| Game integrity | On an approved staging home site only, alter the served Wasm without changing its manifest; explicit game trial must FAIL before execution; restore the original publisher bytes and require PASS |
| Alias / direct-ID compatibility | Resolve an existing local alias and open the full host ID; naming/provider failures must not block direct-ID parsing or weaken TLS; an unavailable route may still fail normally |

Do not use firewall changes to simulate outages. Do not kill unrelated processes.
The browser's `--operator-relay-cycle` option prompts for operator confirmations
and performs no remote service control; with `--game` it also retests game execution
after recovery. GUI/game tests on real machines still need the operator checklist.

## Evidence and first-public-test decision

Attach the three machines' commit/version, UTC times, their roles and distinct
connection types, public pins, stdout JSON and authorized service-control evidence.
Keep private keys and unreviewed raw logs out of the report. Fill
[the existing cross-network checklist](cross-network-checklist.md). For each check
record PASS, FAIL or NOT RUN, the command, observed identities, route and scope.
An expected negative rejection is PASS for that test only when the intended
verification was reached; record the observed FAIL separately.

Do not promote loopback or one-client success to a public-internet PASS. A public
trial remains incomplete until all three operators attest to the physical topology
and the required positive, negative and recovery checks ran. This work makes the
existing tooling ready for that authorized trial; it does not claim it happened.
