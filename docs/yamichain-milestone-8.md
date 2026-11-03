# Milestone 8: authenticated isolated P2P

This milestone adds functioning private-network infrastructure. It does **not**
establish public-network security. The default runner remains loopback-only;
public listeners, permissionless validator admission, production YPP verification,
real staking and payments remain unavailable. Existing experiments are retained.

## Identity and authority

The explicit `--isolated-network CONFIG` profile accepts independently generated
Ed25519 consensus and transport identities. `roster.bin` is a locally pinned,
fixed, equal-power committee (4–100 members), not a discovery result. The existing
height-specific committee verifier, strict greater-than-two-thirds certificates,
durable anti-double-signing reservations, WAL and authenticated journal replay
remain in use. This profile does not add an election or committee-update API.

`init-isolated` uses OS randomness, refuses an existing output directory, writes
owner-only private identities, and does not print keys. Give each node only its
own `identity-N.bin`; distribute public roster/bootstrap files separately. Known
public development consensus/transport keys are explicitly rejected by this
profile. Do not copy the provisioning directory into any node/container/image.
The original public-key development runner still binds only loopback.

TCP sessions use the existing pinned Malachite 0.8.2 libp2p Noise/Yamux transport.
Dial addresses include the expected transport PeerId. Validator proofs additionally
bind a pinned consensus key to its scheduled transport identity. A transport
handshake or a valid discovery signature cannot add voting power. Certificate
verification and deterministic block execution still authenticate catch-up data.

## Discovery and reconnect

Two bootstrap files supply untrusted signed hints. Each record signs a domain,
genesis ID, pinned consensus key, numeric private IPv4 endpoint and expiry. The
transport key is taken exclusively from the pinned roster. Unknown identities,
public/wildcard/loopback endpoints, low/reserved ports, malformed signatures,
trailing data, oversized tables, expired hints and conflicting same-version
announcements fail closed. Newer expiry values order updates; exact signed
retransmission is idempotent. The maximum hint lifetime is one hour.

Nodes exchange pages of at most 16 signed records over UDP at the advertised TCP
port **plus one**. A node needs self plus two bootstrap hints initially, not the
address of every validator. Pages propagate missing endpoints and signed address
changes. Neither the UDP sender's IP nor the page's contents confer authority.
Only known bootstrap/learned peer IPs reach signature processing; all records
still need a pinned key's valid signature. Numeric addresses prevent DNS rebinding.

The application handles at most four datagrams per two-second tick, using a fixed
1,762-byte receive buffer. Larger datagrams are rejected after truncation. Pages
rotate through the bounded roster. At most 100 signed hints and their signatures
are retained. Unrestricted upstream Kademlia/peer exchange remains disabled.

Verified updates use the engine's runtime persistent-peer API. Old addresses are
removed before replacements are added because upstream removal clears
identity-level persistence. The expected Noise identity stays pinned. Upstream
persistent-peer reconnect/backoff remains responsible for retrying connections.
A validator can publish a new endpoint without changing its keys or committee;
changing its own active listen address requires restarting that validator.

A bounded signed `network-hints.bin` checkpoint is atomically replaced and fsynced
under the journal's exclusive lock. It retains freshness tombstones and signed
records across restarts, including endpoints learned only through gossip. Older
hints cannot roll back a newer endpoint after restart. Invalid incoming updates
leave the last verified set intact until expiry; expiry stops the isolated
listener. Keep this checkpoint with the WAL, signing records and finalized store.
Local storage rollback/deletion is outside the network attacker model.

## Resource limits

The isolated profile sets discovery-derived limits even though unrestricted
discovery is disabled. In upstream 0.8.2 the connection multiplier is four:

| Resource | Limit |
| --- | ---: |
| Pending / established incoming TCP connections | 32 each |
| Pending / established outgoing TCP connections | 100 each |
| Established connections per PeerId | 4 |
| Pending + established incoming connections per IP | 4 |
| Reconnect throttle for non-persistent IPs | 2 seconds |
| Ephemeral connection retention | 5 seconds |
| Discovery records per page / retained identities | 16 / 100 |
| Accepted discovery datagrams per tick | 4 per 2 seconds |
| Bootstrap/cache file size | 32 KiB |
| Gossip message / RPC response size | 128 KiB / 1 MiB |
| Sync batch / parallel requests | 8 / 1 |
| Concurrent / pending inbound sync requests | 4 / 8 |
| Inbound sync requests per 10-second window | 64 per peer |

Kernel listen backlog and TIME_WAIT sockets are not established libp2p sessions;
TCP `connect()` success alone does not mean handshake admission. Persistent IPs
are exempt from reconnect throttling, not from the per-IP connection count.

## Reproduction

Build with the repository Rust toolchain, then run:

```sh
cargo build --release -p yami-validator
python3 tools/yamichain-p2p-lab.py --output work/p2p-lab-run --height 96
```

The harness uses a digest-pinned Python image and creates an **internal-only**
Docker bridge. Four validators run in distinct network namespaces at private
non-loopback addresses. Each has a two-CPU, 2-GiB cgroup, 512-FD and 256-process
limits, no Linux capabilities, no-new-privileges, a read-only root and config,
and a separate writable store. Only the own private identity is mounted. No
container ports are published, and no public deployment is performed.

The default campaign uses eight non-financial naming transactions per block
(`--transactions 0..64` selects the workload). Each validator starts with only
two bootstrap peers. The campaign floods stalled
and malformed TCP connections, partitions one validator, kills it, publishes its
new signed IP endpoint to one bootstrap, and restarts it with the original WAL
and signing records. Other validators must learn the replacement through gossip.
Add `--quorum-loss` to disconnect a second validator and verify that finality
stalls without quorum before healing. The harness also injects 512 forged
discovery pages from a known peer IP and 2,048 oversized UDP datagrams from an
unknown source. The harness checks final block/state agreement and independently replays each
stopped validator's certified disk history. It retains raw process/cgroup CPU,
RSS/high-water memory, FD/socket, bandwidth samples, logs and event timings.
It removes only its own containers and bridge; it retains stores and identities
under the git-ignored output directory. Do not publish that directory.

Manual provisioning for an already isolated lab:

```sh
yami-validator init-isolated --dir work/private-provision \
  --endpoints 10.81.0.2:28100,10.81.0.3:28100,10.81.0.4:28100,10.81.0.5:28100
# Distribute only public files and each node's own identity to its config directory.
yami-validator run --dir work/node-0 --index 0 --isolated-network work/config-0
# Publish an own endpoint change, then replicate the public bootstrap-0.bin
# to one or more live bootstrap nodes. Other peers learn it through signed gossip.
yami-validator announce-isolated --dir work/config-0 --index 0 --endpoint 10.81.0.6:28100
```

An announcement must advance the clock/version; a second endpoint in the same
second is refused. All hints and bootstrap files must be refreshed before expiry.
`status`/`inspect` infer the initialized store's index and load its persisted signed
discovery cache. As before, stop the validator before taking the exclusive store
lock for inspection.

## Deployment blockers

This is a development profile, not a public-network release. Private addresses
are not proof of network isolation: operators must supply firewall/namespace
isolation and must not publish/NAT the listeners. No NAT traversal, public DNS
seeds, public discovery admission, cross-AS/subnet diversity policy, authenticated
bootstrap distribution service, automatic signed lease renewal, or clock
synchronization protocol is supplied. UDP gossip is signed but neither encrypted
nor session-authenticated; it discloses only public hints. A hostile source
can consume the bounded discovery processing budget and delay learning.

Pinned identities bound Sybil state and retain trusted reconnect targets; they
do not prove resistance to routing-level eclipse, colluding authorized validators,
bandwidth saturation, distributed source-IP flooding, or a hostile network
operator. An authorized identity can announce an unreachable endpoint or refuse
to serve history. More than one-third faulty committee power can stop progress;
no networking change bypasses the BFT safety rules to restore liveness.

The four-process lab measures a small committee on a shared two-core host.
Per-node two-core/2-GiB limits are enforced, but these results do not certify
100-validator capacity, geographically independent routes, prolonged churn,
production workloads, or Internet-scale adversarial security. Production YPP
verification remains disabled for every one of the 65,536 proof versions.

## Validation results (2026-11-03)

Formatting, strict workspace/all-target Clippy, workspace regressions and the
complete release workspace build passed with Rust 1.90.0. The workspace ran **503
passing tests, zero failures, 10 existing ignored manual/environment-dependent
tests**. The non-test privacy integration test rejected all 65,536 proof versions
for both payments and public value-bearing actions. Matching Debian ALSA/libudev
development files were supplied outside the repository for desktop crate builds.

Both final four-process campaigns converged at height 96 and passed independent
certified disk replay. Each node initially knew only two bootstrap endpoints.
The stronger campaign used eight test naming transactions per block (768 records),
2,048 oversized UDP datagrams from an unknown IP, 512 forged signed-page payloads
from a known peer IP, and 256 stalled/malformed TCP connection attempts. Only one
attacker socket remained open after four seconds, below the per-IP cap of four.
Every stopped store retained all four authenticated endpoints, including the
new IP learned through gossip.

During the quorum-loss campaign all four stores remained at height 16 while two
validators were disconnected. Healing and restarting the migrated validator with
its original WAL restored equal durable heights within the next observed sample,
**3.10 seconds** after the heal/restart event. All four subsequently finalized the
same height-96 block and state root. Total campaign time was 116.13 seconds,
including a 60-second terminal history-serving period.

| Validator | Peak sampled CPU (one core = 100%) | Peak RSS/HWM | Max observed FDs | Last sampled RX / TX |
| --- | ---: | ---: | ---: | --- |
| 0 | 4.94% | 20.46 MiB | 17 | 6.36 MB / 2.65 MB |
| 1 | 4.89% | 19.55 MiB | 17 | 2.59 MB / 2.36 MB |
| 2 | 5.14% | 19.88 MiB | 16 | 2.08 MB / 2.09 MB |
| 3 | 4.93% | 18.98 MiB | 16 | 2.05 MB / 2.06 MB |

Docker inspection confirmed the two-CPU, 2-GiB memory/swap, 256-process and 512-FD
limits for every validator and the internal-only bridge. CPU samples include
short telemetry processes; separate PID-1 CPU tick counters are retained.
Bandwidth counters are rounded observations, and validator 3 counters reset on
restart/IP migration. Validator 0 received the extra malformed-traffic load.
These observations are below the resource target but do not establish worst-case
or production capacity. The initial candidate IP-migration experiment failed to
catch up within its harness window; its stores/logs remain under
`work/p2p-lab-first`. The completed signed-gossip/cache implementation passed the
subsequent stronger campaigns.

Public, key-free evidence: [validation summary](evidence/yamichain-m8-checks.json),
[gossip campaign](evidence/yamichain-m8-gossip.json), and
[quorum-loss/adversarial campaign](evidence/yamichain-m8-quorum.json).
Provisioning identities and raw stores remain only in the git-ignored lab output.
Public-network deployment remains blocked for the reasons above.
