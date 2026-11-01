# Independent-machine validation checklist

Copy this file into your test record and replace every `NOT RUN` with evidence
only after the check actually ran. A loopback test must stay labelled as a
simulation. Do not claim that a signed DHT record, an open port or a successful
registration proves reachability or authenticity by itself.

## Environment

| Item | Value |
| --- | --- |
| Date/time and timezone | |
| YamiNet commit / version | |
| VPS provider and region | |
| VPS public IPv4 and bind IPv4 | |
| Home host OS and connection type | |
| Client OS and connection type (must be independent) | |
| Bootstrap node ID | |
| Relay node ID | |
| Website ID and full `yami://` address | |

## Preconditions

- [ ] Operator authorized all changes to the VPS, DNS/firewall and home router.
- [ ] No port forwarding was added automatically by YamiNet.
- [ ] Only the approved TCP ports are open.
- [ ] Identities were created once, backed up privately, and never pasted into a report.
- [ ] The client learned the full node IDs through a trusted channel.

## Checks

| Check | Result | Evidence |
| --- | --- | --- |
| Configuration validation on VPS | NOT RUN | |
| Configuration validation on home host | NOT RUN | |
| Bootstrap listener reachable from a separate network | NOT RUN | external probe or provider evidence |
| Signed DHT lookup from the independent client | NOT RUN | preflight JSON |
| Exact destination TLS authentication | NOT RUN | preflight `destination-tls` |
| YNP document retrieval | NOT RUN | preflight `document-retrieval` |
| PNG asset retrieval | NOT RUN | preflight `png-retrieval` and dimensions |
| Direct route fails as expected | NOT RUN | preflight `--route direct` report |
| Explicit relay fallback succeeds | NOT RUN | preflight `relay-fallback` PASS |
| Relay identity distinct from destination identity | NOT RUN | report IDs, operator comparison |
| Wrong destination or relay identity is refused | NOT RUN | negative test log; no document returned |
| Relay stop is detected | NOT RUN | preflight FAIL while stopped |
| Relay restart re-registers and recovers | NOT RUN | preflight PASS after restart |
| Home host has no inbound listener | NOT RUN | `ss -ltn` on host; router config review |
| Windows/Linux service restart behaviour | NOT RUN | journal/system log excerpt |
| Browser shows Verified node / Relayed identity | NOT RUN | screenshot or exact UI text |
| Logs contain no keys or document contents | NOT RUN | reviewed log excerpt |

## Report files

Attach or reference:

- `preflight-relay.json`
- `preflight-direct.json` (expected failure)
- `preflight-relay-stop.json` (expected failure)
- `preflight-relay-recovery.json`
- bootstrap/relay/host journal excerpts with timestamps
- `yaminet-testnet firewall-plan` output (plan only, not proof of provider state)

Redact public IPs only if your test policy requires it; never include private
keys, document bodies or unrelated personal data.

## Final classification

Choose one and keep the wording:

- **Automated local validation:** `cargo test`/loopback rehearsal passed. Real
  machines `NOT RUN`.
- **Partial independent validation:** list exactly which checks ran on which
  machines. Everything else is `NOT RUN`.
- **Independent end-to-end validation:** only if bootstrap, relay, home host and
  a separate client were all exercised on distinct machines/connections and the
  negative identity/outage checks were recorded.

This kit does not certify anonymity, production security, abuse resistance or
permanent availability.

## YNS 0.2 and hosted game extension

Use [the current three-machine runbook](public-internet-e2e.md). Record these
separately with PASS / FAIL / NOT RUN and actual UTC evidence:

- [ ] Direct VPS provider pin checked independently; no home inbound YNS listener.
- [ ] Signed scoped name published; owner key kept separate from serving identities.
- [ ] Fresh client retrieves the name without record JSON imports and compares it to the independently supplied full host ID.
- [ ] `--network-trial ... --name NAME --navigate about --game prism` completed; endpoint disclosure was an explicit choice.
- [ ] Actual native-window navigation, Play, keyboard, pause/restart/stop observed on the independent connection (headless frames do not satisfy this row).
- [ ] Wrong provider/bootstrap/destination identities reached the intended verifier and failed; no replacement pins were silently accepted.
- [ ] Relay outage and recovery retained the original keys and history, and restored page/game delivery.
- [ ] Short-lived YNS record expired in real time, expired cache was refused, and a higher signed sequence recovered.
- [ ] Natural DHT record expiry observed or marked NOT RUN (fake-clock tests are separate software evidence).
- [ ] No remote deployment, firewall or router changes occurred without the appropriate operator's explicit approval.

The [current readiness ledger](../results/2026-11-01-public-e2e.md) starts all
real-public-internet rows at NOT RUN. It is not an operator attestation.
