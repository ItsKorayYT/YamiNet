# Experimental security status

YamiNet 0.1 is **not secure for sensitive use**. It has no anonymity and no
protection against a malicious local user. The default `ynp://` transport is
plaintext, unauthenticated and loopback-only. Nodes can have persistent signing
identities (below), and an experimental TLS 1.3 transport (below), used by the
browser for `ynps://` addresses, authenticates a node against a pinned node ID
on loopback or, by explicit opt-in, a private IPv4 LAN; neither has been
independently reviewed. Every peer on the machine or the LAN and every received
document must be considered untrusted. Do not put private documents,
credentials or personal data in this prototype. Do not expose it through port
forwarding or a public proxy.

The concrete restrictions are numeric loopback-only TCP for plaintext YNP,
numeric loopback or private IPv4 addresses for TLS, strict binary field
and frame bounds, UTF-8 validation, a bounded document/style parser, rejection
of control characters in parsed content, no executable content, no automatic
network requests and no DNS resolution. These reduce accidental exposure but
are not a security audit. A modified caller can implement a different transport;
the loopback policy is enforced by `LoopbackTcp` and `LocalListener`, not by
the transport trait itself. CLI and desktop address parsing never select another transport.

Connections use three-second per-operation socket timeouts, and the node also
ends any connection after ten seconds in total. `serve-dir` handles at most its
configured number of workers (1..32) at once; other clients wait in the
operating system's accept queue, so a few slow local clients can still delay
others for up to that deadline. Document serving has no quotas, rate limits, hardened multi-client scheduling,
authorization checks, sandboxing, or robust denial-of-service protections.
The frame codec bounds allocation before reading payloads but this is not a
claim of comprehensive resistance to resource exhaustion.

Directory sites (`serve-dir`) read only `<root>/<key>.ylang` and
`<root>/default.ysl`. Keys are restricted by the YNP decoder before any file
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
plaintext fallback are disabled. Listeners are loopback-only unless started
with `serve-dir --tls --lan`, which binds exactly one private IPv4 address
(never a wildcard or public address) and drops non-private peers; clients
reach loopback and private IPv4 addresses only. Any host on that LAN can
connect and read every page the node serves, so restrict the port with the
operating system firewall ([docs/lan-testing.md](docs/lan-testing.md)).
The node key signs TLS messages only through a format-checked API that cannot
produce a signature over YamiNet metadata. Clients are not authenticated; there
is no revocation or key rotation, so a stolen `node.key` impersonates its node
indefinitely; and which node ID to trust is still learned out of band. The
threat model and limitations are in
[docs/specs/v0.1/ynp-tls.md](docs/specs/v0.1/ynp-tls.md).

Key rotation and revocation, client authentication, internet exposure, NAT
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
