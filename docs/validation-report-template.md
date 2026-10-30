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
