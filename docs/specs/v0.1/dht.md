# Yami DHT v1: signed peer lookup MVP

**Experimental, not audited or production-safe. No anonymity.** This is a
small Kademlia-inspired signed-value lookup system, not a full Kademlia
implementation or compatibility protocol. The existing public IPv4 TLS
transport and manual bootstrap peer store were verified before implementation.
Neither is replaced. YNP document frames, YLang and YSL are unchanged. The
browser integrates this crate for native identity addressing; see
[native addressing](../../native-addressing.md).

## Identity, routing and lookup

The lookup key is the existing full Ed25519 node ID. Routing uses
`SHA-256(public-key-bytes)` and unsigned lexicographic XOR distance over 256
bits. Prefix-distance buckets hold at most four contacts each; a routing table
holds at most 128 contacts overall. Live bucket members are not displaced to
make space. Contacts are observed only after a successful RPC authenticated
against the expected TLS key. Failed contacts are removed; routing entries
expire after 15 minutes when the table is used for lookup or maintenance. Advertisement alone
does not add an authenticated route. This adapts the XOR/bucket approach of
[Maymounkov and Mazières](https://pdos.csail.mit.edu/~petar/papers/maymounkov-kademlia-lncs.pdf);
it does not claim their completeness or performance guarantees.

Lookup is iterative and sequential (`alpha = 1`), visiting the closest known
unqueried `(node ID, endpoint)` first. Each endpoint attempt, including failure,
consumes one of 16 queries. The pruned frontier contains at most 64 candidates;
its assembly is also bounded by the cache, route and bootstrap limits. A query
returns up to eight current signed records, including the responder's own
record when current (with up to seven other closest records). Every returned record is validated
before any part of that response is accepted; duplicate IDs, malformed data,
invalid signatures or rollback/equivocation reject the response atomically.
Freshly learned contacts become candidates for subsequent queries. There is
no recursive query forwarding, unbounded fanout or unsolicited lookup.

The complete lookup has a 15-second network budget. Each RPC gets at most
three seconds including TCP connect, the pinned TLS handshake, request and
response I/O; operations use the smaller remaining budget. OS scheduling and
bounded local signature/file work can add overhead. Lookups attempt to refresh
cached locations, including seeking a newer record through other peers after
an old endpoint fails. If no fresh response succeeds, a still-valid cached
record may be returned as an explicitly unconfirmed location. It is never a
liveness proof. The eventual YNP fetch must authenticate the expected ID again.

The signed record cache has 256 slots, including anti-rollback history.
Expired endpoints are excluded from lookup results, routing candidates and
RPC replies. Their signed high-water records remain only for sequence checks.
Known IDs can advance to higher sequences, but **new IDs are rejected at
capacity; history is not silently evicted**. This deliberately trades availability
for bounded storage and rollback protection. A Sybil attacker can fill this
small cache; this MVP does not solve that denial of service. An operator may
stop the node and deliberately reset its cache, but that loses remembered
sequence history. Back up persistent state; do not advertise this as a global,
unlimited or production DHT.

Resetting a server cache also loses its own publication counter. Other peers
retain their high-water marks and will reject a reset counter; restore the
latest state or use a deliberately new identity instead of silently reusing
old sequence numbers.

## Signed peer records

The existing sealed identity signing API adds purpose
`yaminet.peer-record.v1\0`. Ed25519 signs that tag followed by the canonical
body below. This domain is distinct from document metadata, site manifests,
X.509 certificate signing and TLS CertificateVerify. Verification uses the
existing strict Ed25519 verifier; no new cryptographic algorithm is introduced.

All integers are unsigned big endian. The canonical body is:

| Field | Bytes |
| --- | --- |
| Owner's Ed25519 public key (canonical node ID is reconstructed) | 32 |
| Expiration, Unix seconds | 8 |
| Monotonic positive record sequence | 8 |
| Endpoint count, 1..4 | 1 |
| Each endpoint: numeric IPv4 octets then nonzero TCP port | 4 + 2 |

Endpoints are strictly sorted by IPv4 and port; duplicates are invalid.
Append the 64-byte Ed25519 signature to form a wire record of 119..137 bytes.
Its owner is the signature verification key; substituting the owner or any
other field invalidates the signature. There is no scheme, hostname, private
key, document content, code or stylesheet field.

Network acceptance requires `expiration > now` and at most 24 hours into the
future. Sequence zero, invalid keys/signatures, invalid endpoints and expired
or overly future records are rejected. Once a cache has accepted sequence N,
it rejects lower sequences and any changed record at N, even after expiry and
restart. An exact duplicate is idempotent and does not extend validity or
rewrite the cache. A fresh cache cannot know the globally newest sequence;
valid older records can still be replayed to it until expiry. There is no
revocation service, trusted timestamp authority or global latest-record oracle.

On `serve-dir --dht` startup the node atomically saves a self-signed record with
the next sequence **before publishing it**. The previous signed self record
retains the counter across restarts and expiry. A missing state file starts a
new counter: do not delete/restore old state or run independent publishers
with a cloned identity and expect monotonicity. Sequence exhaustion fails closed.
Publication is one bounded initial pass to up to eight configured peers, at
most 15 seconds and 16 RPCs total. A successful STORE is followed by one FIND
near the joining node's ID to learn signed neighbors; this lets ordinary
joined peers help subsequent lookups. This remains the behavior without the
separate `--dht-maintain` opt-in.

## Optional automatic maintenance

`serve-dir ... --dht --dht-maintain` starts one node background worker. It uses
unchanged STORE/FIND RPCs and the existing signed-record and TLS validators.
Default renewal is six hours and health checks five minutes, both jittered by
±10%. Configurable renewal is 30..43200 seconds and health checks 5..3600
seconds. Each new publication attempt, including retries, reserves and persists
a strictly higher sequence before STORE. Self records keep the 24-hour lifetime.
Server state and identity survive restart; resetting either is not maintenance.

A pass rotates through at most four signed/previously authenticated/configured
contacts, sends at most eight RPCs, and shares a 15-second network budget.
Each RPC's three-second budget covers connect, TLS handshake and I/O. There is
one outbound connection at a time and at least five seconds between passes.
STORE is followed by FIND to learn neighbors; health-only passes issue FIND.
Failed routes are removed; stale routes expire. Newly advertised locations
always require exact-ID TLS before use. No unsigned exchange is introduced.

Complete failures and individual unreachable peers use exponential backoff
starting at `min(health interval, 30 seconds)` with up to 20% positive jitter,
capped at 900 seconds. At most 64 individual cooldowns are retained. Global
rate limits still apply when churn replaces cooldown entries. Signed/high-water
cache and route limits remain 256 and 128. Ordinary learned peers can carry
renewals without the original bootstrap. Configured compatible bootstrap pins
are re-read for recovery; no built-in central peer or quorum is required.

An optional bounded operator endpoint file can replace advertised endpoints
while running, causing a new signed sequence. It does not change the listener
bind or perform IP discovery/NAT traversal. Invalid updates preserve the last
valid set. Clock rollback causes a fresh sequence/time-bound record rather than
sequence reuse. A test-injected DHT clock is supported by the Rust service;
production uses the system clock, independently of real TLS/socket deadlines.

Stop signals interrupt the worker's wait; cancellation is checked between RPCs,
so one in-flight request can consume its existing budget. The serving pool drains
under its existing deadlines. Bounded local diagnostics report peers, recent
verification, last acknowledged publication, expiration and fixed error codes;
they are advisory snapshots, not presence or ownership proofs. See
[operations, shutdown and controlled-clock tests](../../dht-maintenance.md).

## Scope and trust

The default public DHT accepts only the existing transport's conservative
ordinary public-unicast IPv4 policy. It refuses RFC 1918, loopback, link-local,
CGNAT, documentation, multicast, unspecified and reserved ranges. There is no
DNS resolution. `--dht-local` / CLI `--local` explicitly selects an isolated
IPv4-loopback-only simulation policy. Public and loopback state files differ;
records cannot cross policies. This prevents a public peer from directing
lookups at localhost/private LAN services. It is not a guarantee against all
forms of scanning of public services: a malicious signer can claim permitted
public endpoints, but every connection must pin the claimed ID before RPC/data.

TLS is the existing `ynp-tls` transport: TLS 1.3 only, exact pinned Ed25519 key,
valid self-signed certificate and handshake signature, `ynp/1` ALPN, no DNS,
resumption, early data or plaintext fallback. DHT listeners require explicit
`--dht --tls` plus either existing `--public` opt-in or isolated `--dht-local`.
There is no automatic firewall, provider or router configuration. Public
listening retains its fixed workers, handshake/connection deadlines and global/
per-IP admission limits. The DHT server does not fetch advertised endpoints in
response to FIND/STORE; bounded iterative clients initiate those connections.

Bootstrap entries reuse the existing persistent manual peer configuration.
Their IDs are operator-supplied pins, not naming authorities. Adding a bootstrap
does not contact it, authenticate its advertisement, or replace existing pins.
Any compatible ordinary peer can be a bootstrap, and multiple are supported.
No vendor server or specific node is mandatory. Once peers have learned valid
records, cached contacts can be queried without the original bootstrap.

An owner signature proves that the key signed a location claim, not that its
owner occupies that endpoint or is a trustworthy person. Even a validly signed
lie must fail when the actual endpoint presents another key. `dht fetch`
performs that fresh TLS check before asking for a YNP document. It tries at
most four signed endpoints, each with a three-second total connection/I/O
budget, validates the YNP document key and rechecks record expiry before use.
`lookup` only prints `ynps://` locations (convenience key `/home`); it does not
open sites. Browser users may explicitly open that URL, or enter
`yami://<full-node-id>/home` to resolve and authenticate through this same crate.
Browser lookup requires no private key: its public routing/cache origin is not
a client authentication identity. Separate policy-tagged browser snapshots keep
public and explicit loopback state apart. See [configuration](../../native-addressing.md).

`RpcConnector` is a small transport extension point. The default `PinnedTls`
adapter delegates to `ynp-tls`. Future adapters must preserve exact-key TLS
authentication, encryption and budgets; policy validation precedes the hook.
No anonymous transport, alternate endpoint type, NAT traversal, Tor or I2P is
implemented, and a local modified adapter is outside the trust boundary.

## Versioned RPC over the existing TLS listener

With DHT enabled, the node reads four decrypted bytes after the normal TLS
handshake. `YNP\0` is replayed unchanged into the original YNP codec. `YDH\0`
selects the DHT codec. Unknown prefixes fail closed. Without the opt-in, the
original YNP server handles requests and rejects DHT magic. The browser and
document stack are not redesigned.

Each request and response is **exactly 2048 bytes**, including all-zero padding:

| Offset | Bytes | Field |
| --- | --- | --- |
| 0 | 4 | `YDH\0` |
| 4 | 1 | Version `1` |
| 5 | 1 | Message kind |
| 6 | 16 | Fresh OS-random request nonce, echoed in the response |
| 22 | 2 | Body length, at most 2024 |
| 24 | variable | Body, then zero padding to 2048 |

Kinds: 1 FIND (32-byte target public key); 2 STORE (u16 record length and one
signed record); 3 PEERS (u8 count 0..8, then u16-length-prefixed signed records);
4 STORED (empty body). The server accepts only FIND/STORE. Clients match the
response nonce and kind to the current query. Unknown versions/kinds, nonzero
padding, invalid lengths/counts, trailing fields, invalid keys, duplicate
record IDs and truncation are errors. Each connection carries one RPC. No
response is sent until the full request is received and validated. Fixed
equal-size frames cap application-level amplification at 1:1; TCP/TLS overhead
and TCP-level abuse remain outside that statement. There is no UDP DHT or
source-address reflection mechanism.

## Persistence and remaining limitations

`dht-{public|local}-{server|client}.cache` files share the selected peer/state
directory. Writers hold separate nonblocking OS locks and atomically replace
a synced temporary file; snapshots are at most 64 KiB and 256 signed records.
The binary header is `YDC\x01`, one network-policy byte, the local public key
(32 bytes), u16 record count, then u16-length-prefixed records. Bounds, owner,
scope, duplicates and all signatures are checked on load. Expired signatures
restore only high-water history, never active locations. Corruption does not
silently reset history. Startup removes an interrupted `.tmp` only after the
exclusive writer lock is held. Files are synced before rename; Unix also syncs
the containing directory before publication. A pre-rename write failure leaves
the prior committed state intact. An error after rename fails the service closed
until reopening, so disk/memory ambiguity cannot reuse a sequence. Durability
through power loss still depends on the platform/storage; Windows directory
sync and network-filesystem guarantees are not provided.

Server/client caches have separate locks so CLI lookups can run while serving.
The CLI imports current server records that do not roll back newer client
knowledge. A running server caches validated STORE publications; CLI-learned
records remain in the client cache rather than silently modifying the running
server. Routing contacts are transient and authenticated again after restart.
Unix new files/directories request 0600/0700; Windows inherits directory ACLs.
No private keys are serialized here. File permissions, backups and key/state
integrity remain the operator's responsibility.

Unsolved risks include **Sybil identities, eclipse attacks, cache filling,
malicious bootstraps, censorship, omission of newer records, replay to new
clients, clock errors, endpoint lies and traffic analysis**. XOR distance is
not an authorization policy. There is no address-ownership challenge, client
authentication, proof of work, subnet diversity, quorum, revocation or
production DHT availability guarantees. Bounded opt-in maintenance is implemented;
it does not solve adversarial topology or global replication. Lookup peers learn requested IDs and
can link them to source IPs; caches disclose ID-to-endpoint relationships and
sequence/expiry metadata. TLS does not provide anonymity or prevent routing
metadata collection. Public deployments need a separate security review.

See [reproducible testing](../../dht-testing.md) for the three-process simulation
and a clear distinction from unperformed real internet tests.

### Relay-aware record version 2

Version 1 records remain decodable and retain their original signing domain.
Version 2 uses `yaminet.peer-record.v2\0` and canonical bytes:
`YPR 02 || owner-key[32] || expiry-u64 || sequence-u64 || direct-count-u8 ||
IPv4:port[6]* || relay-count-u8 || (relay-key[32] || IPv4:port[6])*` followed by
an Ed25519 signature[64]. Integers use network byte order. The owner key is the
destination of every route. Direct endpoints (0..4) and relay routes (0..2) are
strictly sorted and unique; relay routes sort by raw relay key then endpoint.
An owner cannot advertise itself as relay. An empty v2 record withdraws all
locations. Only the destination signs; a relay advertisement is never liveness
or a replacement for either TLS identity check.

V1 lengths are odd, v2 lengths even, so legacy keys cannot collide with the
version marker. Maximum v2 size is 218 bytes: eight records still fit the 2048
byte frame and 256 records fit the existing 64 KiB snapshot. TTL, persistent
sequence high-water checks, atomic batch validation and public/isolated-loopback
endpoint policies apply to both versions. Relay-only records do not become
routing-table contacts: DHT RPC contacts still require direct endpoints.
Older software rejects v2 safely; use upgraded DHT peers for relay publication.
