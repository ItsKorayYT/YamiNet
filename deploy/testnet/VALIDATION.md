# Validation record for the public-testnet kit

The historical record below is retained. See the [2026-11-01 public end-to-end
readiness record](results/2026-11-01-public-e2e.md) for YNS 0.2, hosted games and
current public-internet NOT RUN statuses.

Revision: rebased onto `origin/main` at `fcace92`; feature branch commit
`feat(testnet): add public test-network deployment and preflight kit`.

Environment: Windows 11, Rust 1.90.0, a single machine. All network traffic below
was loopback. This is **automated local validation**, not a VPS, home-router or
independent-client test.

## Commands that ran and passed

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy -p yami-testnet --all-targets --locked -- -D warnings` | PASS |
| `cargo clippy -p yami-node -p yami-relay-cli --tests --locked -- -D warnings` | PASS |
| `cargo check --workspace --locked` | PASS |
| `cargo test --workspace --locked` | PASS (final clean run) |
| `bash -n deploy/testnet/yaminet-testnet` | PASS |
| `bash -n deploy/testnet/scenarios/loopback-rehearsal.sh` | PASS |
| `deploy/testnet/scenarios/loopback-rehearsal.sh` | PASS, loopback only |

The rehearsal started a real TLS bootstrap node, a real relay, an outbound-only
website host and a client, all on `127.0.0.1`. Its report ended:

```text
[PASS] Bootstrap peer reachability
[PASS] Signed DHT lookup
[PASS] Direct versus relay route selection
[PASS] Exact destination TLS authentication
[PASS] Relay identity
[PASS] YNP document retrieval
[PASS] PNG asset retrieval
[NOT RUN] Validation scope - all observed endpoints are loopback

RESULT: PASS - 10 passed, 0 failed, 9 not run.
```

The same rehearsal then ran `--route direct` against the relay-only host. That
run failed at `Direct versus relay route selection` with `the signed record has
no direct endpoint`, as expected, and did not fall back to plaintext.

## Automated test coverage

- strict configuration parsing and cross-field checks;
- missing, malformed and permission-unsafe identities;
- idempotent profile initialization and retained identities;
- safe removal with a marker, confirmation, purge acknowledgement and
  unrecognized-file retention;
- systemd unit rendering, quoting, restart policy, journal logging and
  idempotent writes;
- structured PASS / FAIL / NOT RUN reports and redaction;
- real loopback signed DHT lookup, direct pinned TLS, document and PNG fetch;
- direct failure followed by explicitly enabled relay fallback;
- relay-only records refused when fallback is disabled;
- wrong destination identity refused before a document is returned;
- unreachable bootstrap reported as FAIL, not success;
- existing `yami-node` and `yami-relay` CLI compatibility;
- operator-script safety text, LF line endings and bash syntax.

## NOT RUN

| Item | Status |
| --- | --- |
| Real VPS deployment and provider firewall/NAT | NOT RUN |
| systemd install/start/stop/uninstall on a real Linux host | NOT RUN |
| Relay or bootstrap reachable from another network | NOT RUN |
| Website host behind a real home router | NOT RUN |
| Independent client on a different connection | NOT RUN |
| Relay outage/recovery on real machines | NOT RUN |
| Windows service/scheduled-task lifecycle | NOT RUN (manual process only) |
| Long-duration stability, abuse resistance, security review | NOT RUN |
| Anonymity or production-security claims | NOT CLAIMED |

A one-machine PASS must not be presented as real-world reachability. See
[`scenarios/cross-network-checklist.md`](scenarios/cross-network-checklist.md)
when recording independent-machine evidence.
