# Repeatable VPS / home host / independent client trial

Physical topology validation is NOT RUN in the development container. The
following runner is implemented and its protocol checks are tested on loopback.
Use actual independent machines before recording a public-network success.
There is no separate test protocol or automatic firewall/provisioning command.

## Preflight

Record versions, OSes, time synchronization, complete public node IDs and the
actual connection types in [the validation report](validation-report-template.md).
Run `yami-node diagnose --config HOST-CONFIG` on the home host and
`yami-node diagnose --config VPS-CONFIG` on the relay/DHT machine. Check that
snapshots belong to the expected identity and are recent. Paths in those local
operator diagnostics should be redacted before sharing. Local snapshots do not
prove an externally authenticated route. All identity directories stay private.

Use existing explicit configurations: a public DHT listener and relay on the
VPS, an outbound relay host behind the home router, and a browser profile on the
independent client with one to eight trusted bootstrap pins and relay fallback
explicitly enabled. Compare pins through a trusted channel; never trust an ad
just because it was discovered. [Existing deployment instructions](cross-network-validation.md)
explain the same configurations. Public relay/DHT ports need manually configured
inbound TCP on OS/provider firewalls. Home hosts/clients need outbound TCP to the
configured peers/routes; outbound relay hosting needs no home inbound port or UDP.

## Run from the independent client

Linux shell (paths with spaces must be quoted):

```sh
./yami-browser --network-trial 'yami://FULL-ID/home' --config '/private/browser.conf' --asset cover > trial-report.json
status=$?
cat trial-report.json
printf 'Exit status: %s\n' "$status"
```

Windows PowerShell:

```powershell
.\yami-browser.exe --network-trial 'yami://FULL-ID/home' --config 'C:\private\browser.conf' --asset cover | Set-Content -Encoding utf8 trial-report.json
$trialExit = $LASTEXITCODE
Get-Content trial-report.json
Write-Host "Exit status: $trialExit"
```

The JSON report validates configuration, full identity pins, authenticated DHT
FIND responses, a signed publication returned by a current lookup, existing
expiration/rollback checks, canonical resolution, document parsing, same-owner
PNG retrieval/decoding and direct versus relayed delivery. A cached record alone
cannot pass fresh publication. A successful nested TLS fetch provides evidence
of a live authenticated relay/destination route at that instant; it does not
expose or certify the host's private control connection. A direct route marks
relay delivery NOT RUN. Failure returns a nonzero exit status and identifies the
failed stage. Reports omit raw error strings, filesystem paths, content and full
browsing URLs. Complete public destination/peer IDs remain because they identify
the explicit pins being checked. Endpoints are omitted unless
`--include-endpoints` is requested. Store reports locally and review before sharing.

If the host's filesystem is available locally, optionally add
`--host-config PRIVATE-HOST-CONFIG`. This checks the matching identity, site and
recent relay registration snapshot; it never replaces the key. An independent
client usually cannot read home-host files, so that check is NOT RUN there and
must be performed separately on the host. GUI rendering and physical topology
remain NOT VERIFIED by the CLI report; record those actual observations manually.

## Explicit operator disconnect/reconnect trial

Add `--operator-relay-cycle` only for a disposable test deployment under your
control. The baseline must actually use a relay. The runner prints the selected
relay's public ID on stderr, asks you to stop it using your ordinary service
controls, and continues only after you type `stopped`. It then expects failure
or a separately authenticated alternative route. After you restore the service
and type `restarted`, it checks document/PNG recovery within a bounded window.
No service, router or firewall is changed by the runner. Keep stderr visible so
prompts are not lost when stdout is redirected. With multiple relays, record
which selected route was stopped and which alternative was actually observed.
This operator-assisted phase was NOT RUN on physical networks here.

Finish by opening the same URL in the native browser and checking image layout,
links, Back/Forward/reload, find, tabs, verified destination/relay indicators and
outage recovery. Attach the CLI report and separate browser/topology observations.
A PASS for a loopback fixture or a VPS endpoint does not establish the required
VPS/home-NAT/independent-client topology.
