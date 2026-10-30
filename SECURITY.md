# Experimental security status

YamiNet 0.2 is **not secure for sensitive use**. It has no anonymity and no
protection against a malicious local user. The default `ynp://` transport is
plaintext, unauthenticated and loopback-only. Nodes can have persistent signing
identities (below), and an experimental TLS 1.3 transport (below), used by the
browser for `ynps://` and `yami://` addresses, authenticates a node against a
pinned node ID on loopback, LAN or known public IPv4 endpoints. Public listening needs a
separate `--public` opt-in. This integration has not been independently
reviewed or established as production-safe. Every peer and every received
document must be considered untrusted. Do not put private documents,
credentials or personal data in this prototype. Public experiments should use
disposable content and narrowly restricted firewall rules, as described in
[internet testing](docs/internet-testing.md).

The concrete restrictions are numeric loopback-only TCP for plaintext YNP,
numeric loopback/private/public IPv4 endpoints for pinned TLS, strict binary field
and frame bounds, UTF-8 validation, a bounded document/style parser, rejection
of control characters in parsed content, no executable content, no automatic
link navigation or third-party asset requests and no DNS resolution. These reduce accidental exposure but
are not a security audit. A modified caller can implement a different transport;
the loopback policy is enforced by `LoopbackTcp` and `LocalListener`, not by
the transport trait itself. CLI and desktop address parsing never select another transport.

Connections use three-second per-operation socket timeouts, and the node also
ends any connection after ten seconds in total. `serve-dir` handles at most its
configured number of workers (1..32) at once; other clients wait in the
operating system's accept queue, so a few slow local clients can still delay
others for up to that deadline. Explicit public listeners additionally cap
accepted connections and starts per source IP/globally, and cap the TLS
handshake at five seconds. There is no client authorization, sandboxing or
robust denial-of-service protection.
The frame codec bounds allocation before reading payloads but this is not a
claim of comprehensive resistance to resource exhaustion.

Directory sites (`serve-dir`) read only `<root>/<key>.ylang`,
`<root>/default.ysl` and safe `<root>/assets/<key>.png` image keys. Keys are restricted by the YNP decoder before any file
access; the canonical path of each file must be directly inside the canonical
site root under its exact name, which rejects traversal, symlink escapes, case
aliases, subdirectories and non-regular files. Windows device names are never
served. Hard links cannot be detected, and a file replaced between the check
and the read is a local race: anyone able to write the site directory already
controls what it serves. Keep site directories private to their owner.

Node identities are Ed25519 key pairs from `ed25519-dalek`, persisted per
node. Their format, domain-separated signatures and limitations are specified
in [docs/specs/v0.1/identity.md](docs/specs/v0.1/identity.md). A valid signature
shows only that the holder of a node's key signed that exact site or document
metadata; it gives no confidentiality, anonymity, freshness or revocation, and
YNP does not carry or check signatures. Which node ID to trust must be learned
out of band. The secret key file is unencrypted at rest. On Unix it is created
owner-only (0600) and lax permissions are refused; on Windows it inherits its
directory's ACL and is not checked. Secret keys are never printed, logged or
transmitted, and `node.key` is ignored by Git.

The experimental TLS transport (`ynp-tls`, `serve-dir --tls`, `ynps://` URLs)
carries unchanged YNP frames inside TLS 1.3 from rustls, with Ed25519
certificates self-signed by the node identity key and valid for 24 hours. A
client accepts only the node ID it was given before connecting and refuses
other identities, invalid, altered or expired certificates, copied certificates
without the key, and tampered traffic. TLS 1.2, resumption, early data and
plaintext fallback are disabled. Listeners are loopback-only unless LAN or
public mode is explicit. `serve-dir --tls --lan` binds exactly one private IPv4 address
(never a wildcard or public address) and drops non-private peers; clients
reach loopback, private and public IPv4 addresses. Any host on that LAN can
connect and read every page the node serves, so restrict the port with the
operating system firewall ([docs/lan-testing.md](docs/lan-testing.md)).
The node key signs TLS messages only through a format-checked API that cannot
produce a signature over YamiNet metadata. Clients are not authenticated; there
is no revocation or key rotation, so a stolen `node.key` impersonates its node
indefinitely; and which node ID to trust is still learned out of band. The
threat model and limitations are in
[docs/specs/v0.1/ynp-tls.md](docs/specs/v0.1/ynp-tls.md).

`serve-dir --tls --public` is a separate listening mode, mutually exclusive
with `--lan` and `--discoverable`. It binds one specific local IPv4 address
(including a private bind behind a manually configured forward), refuses
wildcards, and accepts public clients under bounded admission limits. It
never alters firewall, router or provider settings. The browser/CLI use the
same exact pinned-key verifier for public endpoints; authentication failure
never triggers plaintext or key substitution. All served pages are public
to reachable clients. Basic limits do not prevent internet-scale abuse.
Actual separate-network/VPS and external firewall/capture testing have not
been performed during development; see [the guide](docs/internet-testing.md).

Manual bootstrap peers contain only operator-entered IDs/endpoints and
historical status/success times. Add does not authenticate or connect. Every
connect rechecks TLS, even after a previous success, and a failed connection
preserves the pinned ID/address. Discovery and documents cannot add or replace
peers. Local configuration and its timestamps/status are not cryptographic
proof; protect the directory from modification. See
[peer storage](docs/specs/v0.1/bootstrap-peers.md).

The optional `yami-dht` lookup protocol reuses those manual bootstrap pins and
the same encrypted TLS listener. It accepts only bounded, owner-signed records
with valid endpoints, expiry and monotonic sequence; an older or conflicting
sequence cannot replace a cache's high-water mark, including after expiry or
restart. Records alone never prove endpoint presence. `dht fetch` pins the
actual connection to the requested ID before any YNP document request. Public
lookup policy refuses private/loopback/link-local/special-use probes; simulation
requires a separate explicitly loopback-only policy. Serving DHT is off without
`--dht`. Browser `yami://` navigation runs identity-free lookup against configured
ordinary peers and its own persistent, policy-tagged signed cache; it stores no
browser private key. The signed location never replaces the requested TLS pin.
Native pages cannot silently follow plaintext links. See
[native browser addressing](docs/native-addressing.md) for configuration and limits.

Routing tables, persistent signed caches, per-message record counts, RPC frames
and lookup attempts/time are bounded. RPC requests/responses are the same fixed
size, with one request per TLS connection and no recursive server forwarding.
The cache retains signed sequence history after expiry; at 256 IDs it rejects
new IDs instead of evicting rollback protection. This is a deliberate finite
MVP tradeoff and a cache-filling denial-of-service risk. Fresh/reset caches do
not know the newest global sequence, and stolen keys cannot be revoked.

Sybil/eclipsing, malicious bootstrap omission/censorship, routing metadata
collection, endpoint lies, clock errors and production DHT availability
guarantees are unsolved. Participating peers can link queried IDs to source IPs and collect ID-to-endpoint records. There is no anonymity. The transport
hook permits future adapters but implements no Tor/I2P or NAT traversal. The
three-node test is loopback simulation, not internet validation or a security
review. See [DHT protocol](docs/specs/v0.1/dht.md) and
[test procedure](docs/dht-testing.md).

Opt-in `--dht-maintain` reuses the same validators and exact-key TLS for
bounded automatic renewal and peer checks. Every new publication reserves a
higher persisted sequence first. Peer cooldowns and global retry limits bound
recurring attempts, but malicious signed public endpoints can still induce
bounded public connection attempts. Maintenance increases recurring metadata
visibility and does not guarantee replication, reachability or availability.
There is no new trust in bootstrap peers. See [maintenance operations](docs/dht-maintenance.md)
for timing, storage failure behavior, explicit endpoint updates and shutdown.

Key rotation and revocation, client authentication, production internet operation, NAT
traversal and anonymous routing require separate versioned designs and
independent review. Encrypting a connection does not establish anonymity.

The desktop UI is not a new trust boundary. It opens `ynps://` addresses only
through the TLS transport and requests nothing until the node proved the node
ID in the address. The padlock and **Verified node** marking under the address
field come from the connection that delivered the page on screen; plaintext
pages are marked **Not encrypted · not verified**, and the glass styling of the
UI says nothing about the connection. A link on a verified page to the same
node's address stays on that node and identity; no link can choose a new node
ID. Requests run on bounded background workers; stale
responses are discarded using tab identity and navigation generation. This
prevents navigation races but does not authenticate the responding peer or
guarantee cancellation of a request already in progress.

Opt-in LAN discovery (`--tls --lan --discoverable`) answers only multicast
queries on the explicitly selected private interface and subnet. It is off
by default and never periodically announces nodes. Browser searches send one
bounded query only after the user selects an interface and starts a search.
Packets and result counts are bounded, matched to a random current request ID,
deduplicated and rate limited. OS packet-info enforces the receiving interface
and destination even though multicast reception uses a wildcard UDP bind on
Windows. TLS listeners continue to bind one specific address.

Discovery exposes public IDs, addresses, ports and homepage keys to LAN
observers. Advertisements are untrusted claims; a malicious peer can advertise
its own ID or forge another one. The user must explicitly select and open a
result before the unchanged TLS verifier checks the advertised key. Proving
that key does not establish the owner's identity. Discoveries are never saved
as trusted identities or substituted for existing pins. Private keys and
document contents are never in discovery packets. No anonymity, LAN observer
protection, general denial-of-service resistance or firewall configuration is
provided. See [the protocol](docs/specs/v0.1/lan-discovery.md) and
[Windows discovery testing](docs/discovery-testing.md).

The native text and graphics libraries, bundled font, window-system backend and
clipboard integration expand the local dependency surface. Documents still
use the original inert YLang/YSL parser and YNP bounds. No embedded web engine,
script runtime, remote font fetch, account or cloud service is introduced.
Bundled homepage bookmarks are explicit localhost targets, and unavailable nodes
produce an error rather than falling back to HTTP or a public service.

## Public relay transport and signed routes

Public relay operation requires an explicit listening address, existing
persistent identity, `--public` and a separately chosen public advertised
endpoint. Loopback remains the default. Wildcard and special-use endpoints are
refused. Nothing configures routers, provider NAT, firewalls or a central server.

The relay authenticates through its pinned Ed25519 TLS identity. Hosts prove a
fresh signed challenge bound to both IDs, expiry and a TLS exporter. Proofs
cannot be replayed on another session, and conflicting registrations cannot
replace a live one. A registration is not destination authentication: the
browser establishes an independent pinned TLS 1.3 session to the exact node ID
inside the tunnel before requesting YNP. There is no plaintext downgrade or
relay-issued substitute identity. The relay cannot decrypt document traffic.

V2 DHT signatures bind the destination key, relay key and endpoint, plus expiry
and sequence. Direct-only v1 records still validate; older software safely
rejects v2. Both endpoint policies, TTL, atomic validation, persisted rollback
high-water marks and existing routing/cache bounds apply. Relay-only records
need no public direct endpoint. An advertised route is not proof of registration,
reachability or a verified connection. Disconnects remove routes from new
publications; cached earlier records can remain live for their signed TTL.
A host restart reserves a newer persisted sequence before publishing. Restoring
old identity/cache backups, losing sequence history or hostile local storage
can prevent publication or permit stale views; do not routinely delete state.

The existing DHT maintainer receives live registration state and renews/routes
with bounded jittered backoff and publication frequency. There are at most two
explicit host relay slots. Each serves one client at a time and re-registers
after its lease or request. Browser workers try direct routes first, then at
most two relay routes and two passes over eligible locations, with three seconds
per connection attempt and a thirty-second overall queued navigation budget. Verified UI status
comes only from a completed authenticated fetch; destination and relay IDs are
shown separately. Bootstrap configuration and discovery never open content.

A relay can deny service, correlate identities/endpoints, and observe timing,
volume and the inner handshake. TLS identity attribution does not make content
safe, authorize clients or provide anonymity. All reachable pages are public.
There is no key revocation, fair sharing, distributed DoS defense or Sybil
resistance. Neither relay availability nor DHT publication guarantees global
replication or discovery. There is no UDP hole punching, ICE/STUN, automatic
router configuration, Tor or I2P.

Connections, per-IP starts, registrations, tasks, simultaneous tunnels,
forwarding bytes/buffers, bandwidth, idle periods and lifetimes are bounded.
The public TLS listener admits at most 32 active sessions, two per IP, 32
starts/second globally and eight starts/second per IP before handshake work.
The relay further limits 32 tracked connections, eight registrations and four
tunnels. Each tunnel has an 8 KiB forwarding buffer, 2 MiB byte quota, 256 KiB
per-second-window bandwidth cutoff, five-second idle cutoff and fifteen-second
lifetime. Over-quota traffic closes; these limits can affect large legal YNP
pages and multiple users behind one NAT. Registered hosts and unauthenticated
clients can still occupy bounded capacity, consume cryptographic CPU and
exhaust OS/network resources. Application limits do not prevent volumetric,
backlog or distributed attacks. Use OS/provider firewall and DoS protections.

Public mode is **experimental and requires independent security review before
any production-security claim**. Automated tests exercise loopback processes,
including a public-mode listener locally; no real public VPS/home-router or
Windows runtime test has been completed. Read [protocol limits](docs/specs/v0.1/relay.md),
[test coverage](docs/relay-integration-testing.md) and the
[actionable deployment procedure](docs/public-relay.md).

## Reliability and defensive review milestone

This developer review is **not an independent security audit**. New regression
coverage addresses three concrete issues: oversized `node.id` files could hide
trailing bytes after a bounded read; identity loading could follow linked key
paths; and a trusted but malicious relay could drip-feed a GO frame using the
long outer-session budget. Identity loading now rejects oversized IDs, symlink
and special-file paths, and Unix multiply hard-linked key files. GO control
frames have a separate five-second total deadline, including partial reads.
Directory ownership/ACLs remain required: path prechecks are not a race-proof
sandbox against a hostile local filesystem.

A shared per-Service DHT STORE budget caps signature/persistence work at sixteen
admitted STORE requests per second, alongside existing fixed frame, TLS admission
and record bounds. Rejected/slow requests do not allocate an unbounded queue.
Reserved owner slots protect local/configured bootstrap history from new-owner
floods. Exact rollback history remains finite and is never evicted on expiry;
Sybil owners can still exhaust untrusted admissions. Neither these limits nor
bootstrap diversity prevents eclipse, distributed exhaustion, withholding or
traffic correlation. See [maintenance policy](docs/dht-maintenance.md).

The [NAT assessment](docs/nat-traversal.md) adds only socket-free, mutually signed
consent/replay prerequisites. It does not enable UDP, QUIC, endpoint discovery,
punching or any new document transport. Raw UDP cannot carry the existing TCP
TLS stream safely; a reviewed transport adapter and physical-NAT validation are
still required. Existing exact identity pinning and relay operation are unchanged.


Validated local configuration never accepts secret fields or silently selects a
public listener. Unknown/duplicate fields, bounded lists and inappropriate role
settings fail before startup; configured node/relay identities must already
exist. Bootstrap pin conflicts require explicit operator replacement. Status
files and diagnostic commands expose public IDs/counters only and are not
cryptographic reachability proofs. `diagnose --connect` is an explicit bounded
TLS pin check, not registration, document retrieval or a DHT-advertisement probe.
See [deployment profiles](docs/configuration.md) and [the unexecuted physical
network plan](docs/cross-network-validation.md). No live firewall or external
infrastructure changes were made as part of this development work.


## Rich documents, PNGs and local browser data (0.2)

Explicit language headers retain 0.1 semantics; the 0.2 AST stays inert and
bounded. Native layout limits nodes, shaped lines, total text and flow height;
YSL has only documented bounded integers/colors/alignments, no arbitrary
expressions, selectors, positioning, scripts or downloaded fonts. Text shaping,
bundled font parsing, PNG decoding and native drawing remain attack surfaces;
this work is developer regression testing, **not an independent security audit**.

PNG requests use wire version 2 while v1 document frames remain unchanged.
Only the authenticated parent destination and its selected direct/relay policy
may serve an asset. Each new connection proves that exact node, plus the relay
pin when relayed. A wrong identity yields a missing-image fallback; it cannot
replace the verified parent or authorize another host. Plaintext assets are
refused before requests, even on loopback. Explicit image keys cannot contain
URLs, paths, extensions or Windows device names. Asset directories/files must
be regular and non-symlink, canonical paths must match exactly, and bytes are
bounded before decoding. Hard links and hostile local filesystem races are not
sandboxed: the site directory must be controlled by its owner.

Per PNG: 256 KiB compressed, at most 2048 in either dimension and 1048576 pixels,
at most 4 MiB retained premultiplied RGBA. Decoding has an 8 MiB internal limit;
output dimensions/allocation are checked independently. Text/ICC metadata is
ignored; APNG and non-PNG formats are rejected. Per page: eight distinct image
keys, 1 MiB total incoming asset bytes including invalid frames, 16 MiB retained
decoded pixels. The two existing loader workers fetch sequentially per page;
there are no unlimited resource threads. Image connection attempts are capped
at two and share three seconds per key and the original navigation deadline.
Navigation/tab-close cancels queued work; an in-flight image socket can remain
until its bounded timeout. No persistent image/document cache is introduced.
PNG requests reveal which keys the browser uses to the destination; the
existing relay/IP/timing and DHT privacy limits continue to apply.

Bookmarks, Home and optional history are **local plaintext data**, not trust
certificates or encrypted storage. History defaults off; disabling stops future
recording, while Clear history removes retained entries from the current file.
Deletion does not erase backups, filesystem journals or SSD remnants. Library
files are bounded to 256 KiB, 256 bookmarks and 512 visits, protected against
simultaneous writers by an OS lock and updated with a flushed temporary file and
atomic rename. New Unix directories/files are 0700/0600; Windows inherits ACLs.
Existing directory ownership/ACLs and local backups remain the user's
responsibility. Malformed/unsupported files fail closed without automatic
replacement; browsing continues with a visible storage error. Saved titles,
folders and addresses never create a verified indicator: only the live TLS
connection does. No library data is sent to a server.

The native authoring commands validate parser/link/image budgets and export
only site content. They never export keys/state, overwrite an existing template
or build destination, publish content to a central service, or enable public
listeners. Validation is a snapshot; protect the source/output against local
concurrent mutation. Keep persistent identities and DHT sequence state outside
site directories. Preview reuses existing identity loading and loopback TLS.
See [the exact 0.2 limits and tests](docs/websites-0.2.md). Linux local TLS/relay
simulations do not establish Windows-runtime, real NAT or Internet behavior.
