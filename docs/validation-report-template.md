# Physical-network trial report

Status: NOT RUN (template; fill only with actual observations)

- Date and git revision:
- Binary provenance/build profiles:
- VPS OS/provider and local/public IPv4 relationship:
- Windows host version and router/NAT setup:
- Independent browser client OS/connection type:
- Public relay, DHT peer and website IDs (no secret keys):
- Configuration/state directories and permissions/ACL checks:
- OS/provider firewall ports and source restrictions:
- Clock synchronization:

| Scenario | PASS / FAIL / NOT RUN | Observed times, IDs and diagnostics |
| --- | --- | --- |
| Config validation and bounded pin diagnostic | NOT RUN | |
| Outbound registration with no home inbound forwarding | NOT RUN | |
| DHT publication/discovery from independent connection | NOT RUN | |
| End-to-end destination TLS identity and homepage rendering | NOT RUN | |
| Native link, Back, Reload, scrolling and tabs | NOT RUN | |
| Wrong relay/destination identity rejection | NOT RUN | |
| Selected relay outage and independent alternative | NOT RUN | |
| Relay recovery and signed DHT update | NOT RUN | |
| DHT peer outage/recovery | NOT RUN | |
| Persistent host identity/sequence restart | NOT RUN | |
| Signed direct endpoint change/rollback rejection | NOT RUN | |
| Slow/lossy physical network behavior | NOT RUN | |
| Graceful shutdown/resource cleanup | NOT RUN | |

Remaining issues and unsupported environments:

Automatic loopback/stream-impairment test results (record separately):

Direct NAT traversal/QUIC prototype: NOT IMPLEMENTED; only documented design and
socket-free consent/replay prerequisites exist. Independent security audit: NOT
PERFORMED. Do not attach private keys, credentials or raw environment dumps.

## 0.3 alpha supplement

| Scenario | PASS / FAIL / NOT RUN | Actual evidence |
| --- | --- | --- |
| Offline host/site/config preflight | NOT RUN | |
| Explicit configured pin checks | NOT RUN | |
| `--probe yami://ID/home --asset cover` from independent client | NOT RUN | |
| Direct document + PNG comparison | NOT RUN | |
| Wrong destination and relay pins rejected | NOT RUN | |
| First launch with no external requests | NOT RUN | |
| Windows install/upgrade/uninstall and key/library preservation | NOT RUN | |
| Linux launcher lifecycle | NOT RUN | |
| Keyboard/find/IME/focus and manual native rendering | NOT RUN | |

Attach the versioned probe output, exit status, timestamp, public identities and
observed topology. Record each stage independently; do not promote loopback or CI
results to physical-network PASS. Exclude secret keys and redact private paths.

## 0.3.1 stabilization record

Record exact source SHA, workflow run/job IDs, artifact hashes, OS build, actual
user privilege level, display session type and toolchain. Keep each status distinct:

| Check | PASS / FAIL / NOT RUN | Evidence and remaining blocker |
| --- | --- | --- |
| Native Windows MSVC source checks and release build | NOT RUN | |
| Windows portable runtime without Rust/Cargo | NOT RUN | |
| Actual prior-version NSIS install/upgrade/uninstall in a space-containing path | NOT RUN | |
| Exact identity/library/peer bytes after lifecycle | NOT RUN | |
| Ordinary non-admin Windows 10/11 permissions and ACLs | NOT RUN | |
| Native Windows window/clipboard test | NOT RUN | |
| Linux native package, launcher, UID and libraries | NOT RUN | |
| Virtual-X11 native-window test and restart | NOT RUN | |
| Physical keyboard, IME, scaling/high-contrast interaction | NOT RUN | |
| Fresh authenticated DHT response/publication | NOT RUN | |
| Direct or nested-relay document and PNG fetch | NOT RUN | |
| Operator-assisted relay cycle/recovery | NOT RUN | |
| Independent VPS/home NAT/browser topology and manual rendering | NOT RUN | |
| Opt-in log rotation/redaction/deletion | NOT RUN | |

Include failed attempts and resolutions, not only passing reruns. JSON CLI reports
never establish physical independence by themselves. Do not attach identities,
raw environment dumps, document contents or personal filesystem paths.
