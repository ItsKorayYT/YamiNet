# Running the reverse relay MVP locally

This milestone adds the `yami-relay` library and the `yami-relay-cli` package,
which builds an executable named **`yami-relay`**. It is a working byte tunnel
with independent end-to-end node authentication. This page preserves the
isolated transport demonstration. For the integrated host/DHT/browser path use
[the four-process tests](relay-integration-testing.md) and
[public VPS/Windows instructions](public-relay.md).
Read [the protocol, bounds and limitations](specs/v0.1/relay.md) first.

## Three terminals (Linux/macOS)

From the repository root:

```sh
cargo build --workspace --locked
target/debug/yami-node identity new --dir target/relay-demo/relay-identity
target/debug/yami-node identity new --dir target/relay-demo/host-identity
```

Run identity creation once. To repeat, retain those directories and read the
public `node.id` values. Never share or commit `node.key`. Example identities
under ignored `target/` are disposable; keep persistent production identities
outside source checkouts. Public operation requires its separate explicit mode.

In terminal 1, start R:

```sh
target/debug/yami-relay serve 127.0.0.1:17550 --identity target/relay-demo/relay-identity
```

In terminal 2, set R's public ID and start the outbound-only site host A:

```sh
relay_id=$(cat target/relay-demo/relay-identity/node.id)
target/debug/yami-relay host "$relay_id" 127.0.0.1:17550 sites/alice \
  --identity target/relay-demo/host-identity
```

This host process uses the existing `yami-node::Site` and YNP server; it opens
**no incoming listener**. Wait for `Registered outbound host ... at relay ...`.
The site and stylesheet are read through the normal node implementation.

In terminal 3, client B fetches A's document:

```sh
relay_id=$(cat target/relay-demo/relay-identity/node.id)
host_id=$(cat target/relay-demo/host-identity/node.id)
target/debug/yami-relay fetch "$relay_id" 127.0.0.1:17550 "$host_id" home
```

The client knows R's endpoint and both full node IDs. It needs no A IP or inbound
host port. It authenticates R, requests A's registered stream, authenticates A
over a separate inner TLS 1.3 handshake, and only then sends YNP GET. The CLI
prints `Verified destination ... over end-to-end TLS 1.3 through relay ...`
to stderr and the fetched YLang source to stdout (terminal control characters
are escaped). Styles are fetched in the same existing YNP reply.

For Cargo invocation, use `cargo run -p yami-relay-cli -- serve ...` (or `host`
or `fetch`). The library package itself has no executable. For PowerShell, use
`$relayId = (Get-Content target/relay-demo/relay-identity/node.id).Trim()` and
`$hostId = (Get-Content target/relay-demo/host-identity/node.id).Trim()`, replace
`target/debug/...` with `./target/debug/...exe`, and put each command on one line.
Those are equivalent local instructions, not a claim of Windows validation.

After one request the host opens another registration with a fresh challenge.
A second concurrent client may receive a busy/unknown error during the single
slot's use or re-registration gap. Retry after it is ready. Stop R and restart
on the same port with the same identity: A retries automatically with bounded
backoff. Replacing R's identity requires an explicit host/client trust-pin
change. Stop A: R removes its registration and closes any active tunnel. Ctrl-C
stops normal processes; Unix SIGTERM/SIGHUP also request shutdown. No router or
firewall changes are made. Every relay endpoint here must be numeric loopback;
private LAN, wildcard and public relay endpoints are refused.

## Rust interfaces

`Relay::bind` / `Relay::run` start the bounded server. `Registration::connect`
performs one signed outbound registration; `Registration::accept` uses a
`ynp_tls::TlsAcceptor` to authenticate the host over the tunnel. `run_host`
maintains the one-slot registration/reconnection loop. `Route::connect` returns
an end-to-end authenticated YNP connection; `Route::fetch` handles a single
document. `Route::open_tunnel` is a lower-level byte stream with **no destination
authentication claim**. Apply the destination's `TlsTransport::connect_over`
before using it for YNP. Relay identity and destination identity are separate.

## Automated validation

```sh
cargo test -p yami-relay --locked
cargo test -p yami-relay-cli --test processes --locked -- --nocapture
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

The process test starts a real R process, an outbound-only A host process and a
B fetch process. It retrieves and checks a roughly 180 KiB YLang document over
multiple inner and outer TLS records, verifies the destination success marker,
rejects a wrong relay pin and unknown destination, restarts R and observes A
reconnect without a host restart, then kills A during a paired tunnel and checks
prompt client closure. A test-only dishonest host subprocess proves A's
registration key but presents B's inner TLS credential; the real client CLI
rejects the substituted identity before requesting any document. No production
CLI option lets registration and serving silently use different identities.

Relay tests use real pinned TLS sockets to exercise proof replay on a second
channel, hijacking with another key, signature mutation, controlled expiry
checks, conflicting registration rejection, slot release on disconnect, expired
registration rejection, host/tunnel/per-IP capacity, live opaque-forwarding
bandwidth/byte quota enforcement, malformed messages and refusal of implicit
public exposure. Signature freshness is tested with supplied time;
production control and tunnel deadlines use real monotonic time. Existing
direct TLS, DHT, maintenance and native-browser tests remain in the workspace.

These tests simulate the **connection direction** needed for NAT: the host
only dials out, and the client does not dial the host. They do not use a real
NAT, port forwarding, public VPS, separate networks or public relay deployment.
Success on one machine is not internet reachability validation, anonymity,
abuse resistance or a security review. The newer [integration milestone](relay-integration-testing.md) also tests
DHT relay advertisements, native yami:// browsing/rendering and outage recovery;
public physical-network validation remains separate.

Historical validation of the isolated relay baseline on 2026-10-30 with Rust 1.90.0 on Linux passed formatting, all-target
Clippy with warnings denied, the locked workspace build and workspace tests:
**165 passed, 8 existing opt-in tests ignored**. This includes the three-process
relay test, hostile registration/identity cases, byte quota, and existing direct
TLS, DHT, maintenance and browser tests. Public endpoints, selected-interface
multicast and manual visual capture tests remain opt-in and were not exercised.
