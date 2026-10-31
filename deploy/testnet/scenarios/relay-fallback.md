# Reproducing direct failure and explicitly enabled relay fallback

This procedure tests the existing YamiNet behaviour; it does not add a new
protocol or weaken authentication. Two levels are provided:

1. **Loopback rehearsal** (automated, single machine): exercises a dead direct
   endpoint followed by a real relay, with real TLS and signed DHT records.
2. **Independent machines** (manual, NOT RUN until recorded): VPS relay, a
   website host behind a home router, and a client on another connection.

## Level 1: loopback rehearsal

```sh
cargo build --locked -p yami-node -p yami-relay-cli -p yami-testnet
./deploy/testnet/scenarios/loopback-rehearsal.sh
```

What it does:

- creates a throwaway profile with a bootstrap node, relay, outbound host and
  client config;
- starts all three services on `127.0.0.1`;
- runs `preflight --route relay` against the client profile;
- runs `preflight --route direct` and expects failure because the host is
  relay-only;
- stops everything and removes the temporary profile.

A successful run ends with:

```text
RESULT: PASS - ... failed, ... not run.
Rehearsal PASS (loopback simulation only; independent machines NOT VERIFIED).
```

The focused Rust test additionally publishes a record with **both** an
unreachable direct endpoint and a relay route, then proves the runner tries the
direct candidate first and only then authenticates through the relay:

```sh
cargo test -p yami-testnet --locked --test loopback \
  direct_failure_then_enabled_relay_fallback_passes -- --nocapture
```

Its report data contains direct attempts with a non-`authenticated` result and
a relay attempt with `authenticated`. This is still one machine.

## Level 2: VPS relay, home host, independent client

### Roles and trust

- **VPS:** bootstrap node B and relay R, normally different identities and TCP
  ports (17444 and 17550 in the examples).
- **Home host A:** the website identity. It opens outbound connections only and
  has no inbound listener.
- **Client C:** a different computer/connection (for example a laptop on a
  mobile hotspot). It knows B's pin, R's pin and A's full `yami://` address.

The relay is an explicitly trusted transport provider. It authenticates with
its own identity; the destination authenticates separately over the inner TLS
session. A relay cannot impersonate A without failing the destination pin.

### Prepare and verify on the VPS

Follow [the deployment guide](../../docs/public-testnet.md) steps 1–3. Record:

- VPS public IPv4;
- bootstrap node ID and endpoint;
- relay node ID and endpoint;
- confirmation that the provider firewall and host firewall allow only the
  approved test ports.

Do not change firewall rules just to make a test pass.

### Prepare the home host

On the home computer (Windows or Linux), build the binaries and run:

```sh
sudo -u yaminet /opt/yaminet/yami-testnet init \
  --profile /var/lib/yaminet/testnet-home \
  --home \
  --site \
  --client \
  --bootstrap-pin "BOOTSTRAP_ID@VPS_PUBLIC_IP:17444" \
  --relay-pin "RELAY_ID@VPS_PUBLIC_IP:17550" \
  --binary-dir /opt/yaminet
```

On Windows, run the same initializer from PowerShell and then start the host
process directly (no systemd unit is used):

```powershell
.\target\release\yami-testnet.exe init `
  --profile "$env:LOCALAPPDATA\YamiNetTestnetHome" `
  --home --site --client `
  --bootstrap-pin "BOOTSTRAP_ID@VPS_PUBLIC_IP:17444" `
  --relay-pin "RELAY_ID@VPS_PUBLIC_IP:17550" `
  --binary-dir (Resolve-Path .\target\release)
.\target\release\yami-node.exe --config "$env:LOCALAPPDATA\YamiNetTestnetHome\host\host.conf"
```

This creates only an outbound host unit/profile; no bootstrap or relay listener
is created. Review `host/host.conf` and `units/yaminet-testnet-host.service`.
Start it through the documented explicit install/start commands (Linux) or the
manual process above (Windows). Wait for the host log to report registration.

Share only:

```text
yami://HOST_ID/home
```

### Verify from the independent client

Create a browser-role config on C (or use the generated `client.conf` with the
state path adjusted):

```text
yaminet-config 1
role browser
network public
state /home/CLIENT_USER/.local/share/yaminet/testnet-client
bootstrap BOOTSTRAP_ID@VPS_PUBLIC_IP:17444
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
  --json > relay-pass.json
```

Expected important checks:

```text
[PASS] Bootstrap peer reachability
[PASS] Signed DHT lookup
[PASS] Direct versus relay route selection
[PASS] Exact destination TLS authentication
[PASS] Relay identity - outer TLS authenticated the relay; destination authenticated separately
[PASS] YNP document retrieval
[PASS] PNG asset retrieval
[PASS] Relay fallback
```

The `destination-tls` data must show the exact host ID. The relay route data
must show the relay ID. Do not accept an endpoint that proves a different ID.

### Prove direct failure

The home host is relay-only, so:

```sh
yami-testnet preflight --config client.conf \
  --url "yami://HOST_ID/home" --route direct
```

should fail with `the signed record has no direct endpoint`. That is a valid
negative result: the host is not reachable directly, and the runner does not
invent an endpoint or fall back to plaintext.

To test a *stale/unreachable direct candidate* plus relay fallback on real
machines, use a host that previously advertised a direct public endpoint and
then moved behind the relay. Do not fabricate signed records for a test you
intend to report as real-world evidence. The loopback test covers the code path
deterministically.

### Prove relay failure is detected

With the client preflight passing through R:

1. Stop R through the normal operator controls.
2. Re-run the same preflight. It must `FAIL` (no authenticated route), not
   silently use plaintext or a different identity.
3. Restart R with the **same identity and endpoint**.
4. Wait for host re-registration and re-run preflight until it passes.

Record the timestamps and the exact report files. A restart that changes R's
identity is a trust change, not a routine recovery.

### Prove destination identity is checked

Stop the real host but leave its signed record present. Run preflight. It must
fail with a timeout/unreachable result, never print document contents from a
different node. For a deliberate wrong-identity fixture, see the automated test
`wrong_destination_identity_is_refused_without_a_document`.

## What remains NOT RUN

Unless a human performed and recorded them:

- VPS public reachability through the provider firewall/NAT;
- home router outbound-only behaviour and absence of port forwarding;
- independent client on a separate network;
- relay outage/recovery on real machines;
- Windows service lifecycle on the actual host;
- long-duration stability, abuse resistance and security review.

Mark these `NOT RUN` in the validation record. Do not upgrade a loopback PASS to
a real-world PASS.
