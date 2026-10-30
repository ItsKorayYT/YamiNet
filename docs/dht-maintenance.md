# Operating automatic DHT maintenance

Maintenance is an explicit addition to a DHT-serving node:

```sh
cargo build --workspace --locked
target/debug/yami-node serve-dir <specific-local-IPv4>:7443 <site-directory> \
  --identity <persistent-identity-directory> --tls --public --dht \
  --dht-dir <persistent-peer-directory> --dht-maintain \
  --dht-endpoint <advertised-public-IPv4>:7443
```

Replace the placeholders. Configure ordinary reachable bootstrap peers using
`yami-node dht bootstrap <full-ID> <IPv4:port> --dir <persistent-peer-directory>`.
Any compatible ordinary DHT node can fill that role. No vendor peers are
bundled and no central service is required. Keep identity and peer/state
directories across restarts, outside the source checkout with appropriate
permissions and backups. Do not run independent publishers with a cloned key.

`--dht` alone retains its existing bounded initial publication behavior.
`--dht-maintain` requires `--dht --tls` and either public serving or the explicit
loopback simulation policy. It starts one background worker alongside the
existing serving pool. It does not change the browser, YLang, YSL, TLS pinning,
DHT wire version, TCP listener policy or firewall settings.

## Renewal, health checks and bounds

| Setting | Default | Allowed range |
| --- | --- | --- |
| `--dht-renew-secs` | 21600 (6 hours) | 30–43200 seconds |
| `--dht-health-secs` | 300 (5 minutes) | 5–3600 seconds |

Successful publication schedules the next renewal with uniform integer jitter
within ±10% of the configured renewal interval. Every record still has the
existing 24-hour maximum lifetime; even the largest normal renewal interval
leaves substantial expiry headroom. Health intervals also have ±10% jitter
(integer rounding applies). Randomness comes from the existing OS random
source. If randomness is unavailable the worker waits and reports an error;
it does not replace it with a synchronized fixed production schedule.

Each new publication attempt reserves a fresh strictly increasing sequence
through `Service::prepare_self`, including retries after failed attempts.
The signed self record is synced and atomically installed before any STORE.
Unix additionally syncs the containing directory. The previous signed self
record retains the high-water mark across restart and expiry. A crash before
installation may discard an unadvertised reservation; an already advertised
sequence is not reused. Sequence exhaustion, corrupt state or ambiguous
persistence fail closed. A staging file left by a crash is removed at startup
only after acquiring the server cache's exclusive writer lock. A failure before
rename leaves the last committed state usable; a failure after rename makes
that service unavailable until storage is repaired and it is reopened, avoiding
a conflicting sequence when disk may be ahead of memory. Backups restored to
older counters are not a recovery mechanism: other peers retain newer marks.
Power-loss behavior still depends on the filesystem/platform; use reliable
local storage. Windows directory durability is not strengthened by Unix fsync.

One pass rotates over at most four contacts drawn from signed current records,
recent authenticated routes and compatible configured bootstrap pins. Each
publication contact gets STORE, then FIND; a health-only contact gets FIND.
FIND targets alternate between local and selected peer IDs to learn neighbors.
Every connection pins the peer's exact full ID over TLS 1.3. Received records
use the existing signature, policy, expiration and sequence validators, with
atomic batch acceptance. An advertisement alone never creates a verified route.
Failed RPCs remove that routing contact; stale routes expire after 15 minutes.
Expired signed records remain only as bounded anti-rollback history.

The limits per worker are:

- One outgoing connection at a time; no extra incoming listener or recursive RPC.
- Four contacts, eight RPC attempts and a 15-second network budget per pass.
- A three-second total connect/handshake/I/O budget per RPC.
- At least five seconds between passes, measured after a completed pass.
- Global failure backoff and individual failed-peer cooldowns: exponential growth
  from `min(health interval, 30 seconds)`, plus up to 20% positive jitter, capped
  at 900 seconds. Healthy peers can continue while another backs off.
- At most 64 individual cooldown entries. Old entries can be discarded under
  churn, with the global rate limit still applying.
- The existing 256 signed/high-water records, 128 routes and four endpoints per
  record. Candidate assembly is bounded by those limits and 256 manual peers;
  no new unbounded peer store is introduced.

No-peer and complete-outage passes back off as well. Learning and publication
use ordinary learned peers, so the original bootstrap is not permanently
required. Compatible bootstrap pins are re-read during maintenance, allowing
recovery when learned routes have expired or peers return. Recovery is eventual
within scheduling/network budgets, not immediate or guaranteed. A lone node
without any reachable known peer can serve directly but cannot distribute its
record. At the 256-ID cache limit, new identities still fail closed rather than
silently evicting sequence protection; renewal of known IDs remains possible.

## Operator-supplied endpoint changes

Use the existing repeated `--dht-endpoint <IPv4:port>` to publish known external
endpoints, for example behind a manually configured forward. Restarting with
new endpoints and the **same identity and state** advances the sequence safely.

For changes to advertised endpoints while the listener remains usable, choose
`--dht-endpoints-file <path>` instead. The file is at most 256 bytes and holds
one to four numeric IPv4:port entries separated by whitespace. For example:

```text
<operator-supplied-public-IPv4>:7443
```

The worker checks the file at least every 60 seconds while idle. A valid changed
set schedules a fresh signed publication, respecting the five-second pass rate
limit. Invalid/unavailable replacements leave the last valid endpoint set in
place and report a configuration error. The file and direct endpoint flags are
mutually exclusive. Replace the file atomically where practical.

This changes the signed advertisement; it does **not** rebind TCP, discover an
external IP, update a router, traverse NAT or prove ownership of the advertised
address. The operator must arrange actual reachability. If the listener's bind
address itself changes, restart with the new specific bind address while keeping
the identity/state pair. Old endpoints may remain in remote caches until a
newer record is learned; a failed native-browser location triggers bounded
newer-record lookup. Signed endpoints always require fresh TLS authentication.

## Shutdown and diagnostics

Ctrl-C, and SIGTERM/SIGHUP on Unix, request graceful shutdown for maintained
nodes. Idle server workers poll nonblocking accept with interruptible 50 ms
waits; accepted streams retain the existing bounded blocking TLS I/O. Existing
server worker/admission limits remain unchanged. The maintenance wait wakes
immediately, stops between individual RPCs and joins on shutdown. An in-flight
outgoing RPC may take up to three seconds; active incoming requests drain under
the existing connection deadlines (up to ten seconds), plus bounded local work.
No sequence is decremented and no final network publication is required.
Forced termination still releases OS locks; normal restart restores counters.
The cross-platform `ctrlc` dependency supplies safe signal handling; no custom
unsafe signal handler or asynchronous networking stack was added.

```sh
target/debug/yami-node dht status --dir <persistent-peer-directory>
# For the isolated simulation:
target/debug/yami-node dht status --local --dir <local-test-peer-directory>
```

The command shows known peers, recently TLS-verified peers, last successful
publication (Unix seconds), seconds until the current local record expires,
and the last maintenance error. A successful publication means at least one
authenticated STORE acknowledgement, not proof of advertised endpoint ownership
or network-wide replication. Errors use fixed descriptions, never keys or raw
configuration. Diagnostics are written as a bounded policy-tagged sidecar
`dht-{public|local}-maintenance.status`; no private material is serialized.
Server/client/browser cache files and locks remain separate.

Status is advisory local data: the command prints the sample age and last
reported running/stopped state. A forced kill can leave a stale “running” sample;
it is not process liveness proof. Recent verification means a successful pinned
RPC within the routing lifetime; stale diagnostic samples do not establish it.
With maintenance disabled no live maintenance statistics are claimed. Local
record expiry can still be inspected from the signed server cache.

## Reproducing the tests

```sh
cargo test -p yami-node --test maintenance --locked -- --nocapture
cargo test -p yami-node --test dht --locked -- --nocapture
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

The maintenance test starts real A/B/C loopback TLS nodes with the existing
DHT dispatcher. A publishes, B stores/serves its record, and a cold C lookup
through B authenticates A and fetches its YLang website. A controlled DHT clock
advances across six renewal periods and beyond the original 24-hour lifetime.
B is disconnected for two periods and restored without restarting A. A learns
C and continues distributing records during that outage, then B catches up.
An explicit IP change is published and fetched, and reopening A's persisted
state demonstrates sequence continuity.

Additional tests cover first-bootstrap failure/recovery, individual peer
cooldowns alongside healthy peers, global retry/query/memory bounds, no busy
retry at unchanged clock time, clock rollback, expiry rejection, current signed
replacements, interrupted snapshot recovery, endpoint-file bounds/policy,
cancellation between RPCs, cancellation of a sleeping worker, diagnostics and
writer-lock release. A Unix subprocess test exercises CLI opt-in validation,
status, SIGTERM shutdown and restart. TLS certificate time and real socket
deadlines are never replaced by the test clock. The production CLI has no
clock-override flag.

For a manual local session, use the existing [three-node setup](dht-testing.md)
and append `--dht-maintain` to each serving command. Optionally choose
`--dht-renew-secs 30 --dht-health-secs 5`, wait across renewals, inspect `dht
status --local`, stop B, then restart B with its existing identity/state. Keep
`--dht-local` and all local peer directories separate from public-network state.

These are **one-computer simulations, not real internet validation**. No
production-security or anonymity claim follows. Sybil/eclipse attacks, malicious
omission, replay to cold caches, cache filling, cloned/stolen keys and lack of
revocation remain unresolved. Maintenance increases recurring contact and
metadata visibility: peers can correlate queried IDs, source IPs, sequences and
endpoints. Exact-key TLS does not provide anonymity. There is no Tor/I2P, NAT
traversal, naming service, automatic exposure, quorum or guarantee of global
availability. See the [DHT specification](specs/v0.1/dht.md) and
[security notes](../SECURITY.md) before public experiments.

Validation on 2026-10-30 with Rust 1.90.0 on Linux passed formatting, all-target
Clippy with warnings denied, the locked workspace build and the locked workspace
suite: **157 passed, 8 existing opt-in tests ignored**. This includes the new
maintenance simulation and existing DHT/TLS/native-browser compatibility tests.
Owned public endpoints, selected-interface multicast and manual visual capture
tests remain opt-in; those environments were not validated by this run.
