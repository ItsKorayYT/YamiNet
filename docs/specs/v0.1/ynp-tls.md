# YNP over TLS 1.3 0.1 draft

**EXPERIMENTAL. Not independently reviewed or production-safe. Local and public
IPv4 clients; LAN/public listening requires separate opt-in. No anonymity.**
This draft gives YNP an encrypted, integrity-protected connection in which the
client verifies that the server holds the key of one node ID that the client
trusted before connecting. It builds on the persistent node identities in
[identity.md](identity.md). YNP frames ([ynp.md](ynp.md)) are carried
unchanged inside the TLS stream.

Implemented by the `ynp-tls` crate using [`rustls`](https://crates.io/crates/rustls)
0.23 (TLS 1.3 only, `ring` cryptography provider) and
[`rcgen`](https://crates.io/crates/rcgen) 0.14 to encode certificates. No
cryptographic algorithm or handshake is invented here: the handshake is
standard TLS 1.3 (RFC 8446) with Ed25519 certificates (RFC 8410) and the
connection keys are TLS's own. What this draft defines is which certificate a
node presents and which certificate a client accepts. The same pattern, a
self-signed certificate checked against a key fixed in advance, is used by
Syncthing device IDs and libp2p peer IDs.

## Command line

```sh
cargo run -p yami-node -- tls-demo <identity-directory-1> <identity-directory-2>
cargo run -p yami-node -- serve-dir 127.0.0.1:7443 sites/alice --identity <identity-directory> --tls
cargo run -p yami-node -- serve-dir 192.168.1.50:7443 sites/alice --identity <identity-directory> --tls --lan
cargo run -p yami-node -- fetch ynps://<node-id>@127.0.0.1:7443/home
cargo run -p yami-browser -- --desktop ynps://<node-id>@192.168.1.50:7443/home
```

### Private LAN

`--lan` makes `serve-dir` listen on one private IPv4 address of the computer
instead of loopback; see [Network scope](#network-scope). It requires `--tls`
and therefore `--identity`: YNP is never served in plaintext beyond loopback.
Without `--lan` or `--public`, any non-loopback address is refused, and with `--lan`, `0.0.0.0`,
loopback, link-local and public addresses are refused before an identity is
loaded. The node prints its node ID, the `ynps://` URL of every page and an
`Open in Yami Browser:` line with the full address of its start page (`home`
if present). Clients need no option: a `ynps://` URL may name a loopback,
private LAN or ordinary public IPv4 address. Step-by-step instructions for two Windows computers,
including the firewall, are in [LAN testing](../../lan-testing.md).

### Explicit public mode and bootstrap pins

`serve-dir <specific-local-IPv4:port> <site> --identity <dir> --tls --public`
opts into public clients, including a private bind behind a manually configured
forward. It is mutually exclusive with `--lan` and `--discoverable`. It never
sets up forwarding/firewalls or advertises an external endpoint. Loopback binds
are accepted for isolated tests; wildcards and remote IPv6 are refused.
Clients still require the exact operator-supplied ID in every `ynps://` URL.
No verifier, certificate rule, ALPN, TLS version or plaintext policy is weakened.
Manual pins and historical connection status use the separate
[bootstrap peer store](bootstrap-peers.md). See [internet testing](../../internet-testing.md)
for address exclusions, budgets, VPS/forwarding instructions and tests not run.

### Two-node demonstration

`tls-demo` loads, or creates on the first run, a persistent identity in each of
the two directories, refusing two directories that hold the same identity. It
starts two TLS listeners on OS-assigned loopback ports. Node one fetches
`about` from node two while trusting node two's ID, and node two fetches
`welcome` from node one while trusting node one's ID. Both documents are
compared byte for byte with the originals. Finally a client that trusts node
one is sent to node two's address, and the demo succeeds only if that
connection is refused:

```console
$ cargo run -p yami-node -- tls-demo ./demo-ids/one ./demo-ids/two
EXPERIMENTAL: TLS 1.3 to an exact pinned node ID. Public listening requires --public. Not security-reviewed or production-safe; no anonymity.
Node one  yn1-sp2hcdqey6gfx62bqb3vzymxh4hjsjv2ocfqk4hecwplc77pkh2yatmgrq (created in ./demo-ids/one)
Node two  yn1-imc2rmri54qbb6f3msck3fm5x7k3my4eik6phfbbi7hiv5npbs4usbyivy (created in ./demo-ids/two)
Node one received about from ynps://yn1-imc2…@127.0.0.1:30862/about over TLS 1.3 (328 bytes).
Node two received welcome from ynps://yn1-sp2h…@127.0.0.1:30861/welcome over TLS 1.3 (429 bytes).
Refused node two at 127.0.0.1:30862 when expecting node one: server is not the trusted node yn1-sp2h…: it presented node identity yn1-imc2…
```

Run it again with the same directories and both node IDs are unchanged
(`loaded` instead of `created`). The directories contain secret keys: keep them
outside the checkout, or delete them after the demo.

### Two persistent node processes

In two terminals, give each node its own identity directory:

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7443 sites/alice --identity ./demo-ids/alice --tls
```

```sh
cargo run -p yami-node -- serve-dir 127.0.0.1:7444 sites/bob --identity ./demo-ids/bob --tls
```

Each prints its node ID and the `ynps://` URL of every page. In a third
terminal, fetch a page from Alice's node while trusting Alice's node ID:

```sh
cargo run -p yami-node -- fetch ynps://<alice-node-id>@127.0.0.1:7443/home
```

The page is printed after `Verified node <alice-node-id> over TLS 1.3`. Now use
Bob's node ID with Alice's address, or Alice's node ID with Bob's address:

```sh
cargo run -p yami-node -- fetch ynps://<bob-node-id>@127.0.0.1:7443/home
```

The fetch fails with `server is not the trusted node <bob-node-id>: it
presented node identity <alice-node-id>`, prints no document and exits with
status 1. A plaintext `fetch 127.0.0.1:7443 home` also fails: a TLS node never
answers plaintext YNP. Restart a node with the same `--identity` directory and
the same `ynps://` URLs keep working.

`--tls` requires `--identity`. Without `--tls`, `serve-dir` still serves the
legacy plaintext transport, even when `--identity` is given.

## Addresses

```text
ynps://<node-id>@<numeric-IP>:<port>/<key>
```

The node ID before `@` is the identity the client requires. It MUST parse as a
`yn1-` node ID (exact canonical spelling, checksum, valid non-weak key). The
remainder follows the `ynp://` grammar (numeric IP, nonzero port, no DNS, and a
valid YNP key), except that the IP may be a loopback or private IPv4 LAN
address ([Network scope](#network-scope)). A `ynps://` URL without a node ID is rejected rather
than connected to without verification, and a `ynp://` URL never selects TLS.
The node ID in the URL is the trust decision; the address only says where to
connect, so a wrong or swapped address fails rather than reaching another node.

### Links between pages

YLang links are always `ynp://<address>/<key>` ([ylang.md](ylang.md)); this
draft does not change YLang. When the browser shows a page fetched from
`ynps://<node-id>@<address>/...`, a link whose numeric address and port equal
`<address>` exactly is opened as `ynps://<node-id>@<address>/<key>`: the same
node under the same pinned identity. Every other link is used as written, so a
link to another LAN address is refused (plaintext YNP is loopback-only) rather
than sent unprotected, and no link ever selects a node ID the user did not
open. A LAN site therefore links its own pages with the node's LAN address and
port.

## Network scope

| Scope | Addresses | Used by |
| --- | --- | --- |
| Loopback | `127.0.0.0/8`, `::1` | Every listener by default; `TlsListener::bind` |
| LAN | Loopback plus private IPv4 `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16` (RFC 1918) | Listeners from `TlsListener::bind_lan` |
| Internet | Existing local endpoints plus ordinary public unicast IPv4; conservative special-use exclusions | Clients (`TlsTransport::connect`, `ynps://` parsing); explicitly opted-in `TlsListener::bind_public` |

- `TlsListener::bind_lan` binds exactly one private IPv4 address. It refuses
  `0.0.0.0` and every other wildcard, loopback, link-local (`169.254.0.0/16`),
  shared (`100.64.0.0/10`), multicast, broadcast and public addresses before
  creating a socket. There is no option to listen on all interfaces.
- A LAN listener accepts TCP only from loopback or private IPv4 peers and
  closes other connections before the handshake, so a public source forwarded
  to it by a misconfigured router is dropped.
- IPv6 is limited to `::1`. Unique local (`fc00::/7`) and link-local IPv6
  addresses are refused.
- Plaintext YNP is unchanged: `LoopbackTcp`, `LocalListener` and `ynp://`
  parsing accept loopback only, so LAN traffic is always TLS 1.3 to a pinned
  node ID.
- Public listeners bind one specific IPv4 address, request backlog 128, and
  bound active connections to 32 globally/two per source IP, starts to 32 per
  sliding second globally/eight per source IP. Rejected addresses do not grow
  an unbounded table. The CLI worker pool remains 1..32 (default four).
  TLS handshake budget is five seconds, total TLS budget ten seconds; socket
  operation timeouts are capped by the remaining budget and three seconds.
  Public CLI outcome logging is sampled to 20 lines/second. These basic controls
  are not a security review or DDoS protection. Full limits and scope are in
  [internet testing](../../internet-testing.md#limits-and-remaining-risks).
- Which machines on the LAN may connect is not configurable: any host that can
  reach the address and port can fetch every page. Restrict it with the
  operating system firewall (see [LAN testing](../../lan-testing.md)).

## Node credential

A node presents exactly one X.509 v3 certificate:

| Field | Value |
| --- | --- |
| Subject public key | The node's Ed25519 identity public key |
| Issuer, subject | `CN=<node-id>` (self-issued) |
| Signature | Ed25519 by the same key over the DER `TBSCertificate` |
| Validity | From 5 minutes before issue, for 24 hours (whole seconds) |
| Subject alternative name | DNS name `<node-id>` (62 characters, a valid DNS label) |
| Key usage | `digitalSignature` |
| Extended key usage | `serverAuth` |
| Serial number | 16 bytes, positive, derived from the key and issue time |
| Basic constraints | Absent (not a CA) |

A listener issues a certificate at startup and issues a new one on the first
handshake after half the lifetime has passed, so a running node never presents
an expired certificate. Certificates are not stored on disk.

### Signing domains

The node key now signs three kinds of message. `yami-identity` exposes TLS
signing only through `NodeIdentity::sign_tls`, which refuses any message not
in the stated format. The first byte keeps the domains apart, so no message
can be valid in two of them:

| Message | First byte | Accepted format |
| --- | --- | --- |
| YamiNet metadata | `79` (`y` of `yaminet.`) | Purpose tag and canonical bytes ([identity.md](identity.md)) |
| X.509 `TBSCertificate` | `30` (DER SEQUENCE) | Any DER SEQUENCE up to 4096 bytes |
| TLS 1.3 server CertificateVerify | `20` (space) | 64 spaces, `TLS 1.3, server CertificateVerify`, `00`, a 32- or 48-byte transcript hash |

The secret key never leaves `yami-identity`; rustls and rcgen receive only
signatures.

## Handshake

Both sides use TLS 1.3 only (TLS 1.2 and earlier are not compiled in), the
rustls `ring` cipher suites (AES-256-GCM, AES-128-GCM, ChaCha20-Poly1305) and
key exchange groups (X25519, P-256, P-384), and ALPN protocol `ynp/1`.

- Session resumption, session tickets and 0-RTT early data are disabled, so
  every connection is a full handshake with fresh keys and a fresh identity
  check, and no ticket links one connection to the next.
- SNI is not sent. The client uses the trusted node ID as the server name for
  its own checks only.
- The node does not request client certificates. Clients are anonymous to the
  TLS layer.
- The node requires the client to offer the Ed25519 signature scheme and the
  `ynp/1` protocol, and fails the handshake otherwise.

### Client verification

The client accepts the server only if every step succeeds, in this order:

1. The server sent exactly one certificate and it parses.
2. Its subject public key is the Ed25519 key contained in the trusted node ID.
   Otherwise the connection fails with an identity mismatch that names the
   node ID the server presented, when it presented one.
3. The certificate verifies with itself as the only trust anchor (webpki):
   valid Ed25519 self-signature, current time within the validity period, not
   a CA, `serverAuth` usage permitted, and the trusted node ID as DNS name.
   Any altered byte breaks the self-signature, the encoding, the name or the
   key comparison.
4. The server's CertificateVerify is a valid Ed25519 signature by that key
   over the handshake transcript. This proves possession of the secret key;
   a copied certificate alone does not pass.
5. The handshake completes and `ynp/1` was negotiated.

`TlsTransport::connect` completes these steps before it returns, so no YNP
request is written to an unverified peer. Every connection repeats them.

## Exchange

After the handshake the client sends one YNP Get frame and reads one Document
or NotFound frame, exactly as in [ynp.md](ynp.md), inside TLS application
data. Frame bounds, request ID and key correlation are unchanged. Neither side
sends `close_notify`; YNP frames are length-delimited, so truncation is
already detected.

The connection protection is reported as `Protection::Tls13PinnedServer`.
There is no negotiation between plaintext and TLS: a TLS client meeting a
plaintext node, or a plaintext client meeting a TLS node, fails. Nothing falls
back to plaintext.

## Browser

Yami Browser opens `ynps://` addresses in the desktop window, the address
field, links and the terminal viewer. It fetches through `TlsTransport` and
sends the Get request only after `connect` returned a connection whose
protection is `Tls13PinnedServer`. The verification state belongs to the
loaded page, taken from that connection, not to the address:

- A page loaded over TLS shows a padlock, **Verified node · TLS 1.3** and the
  full node ID under the address field.
- A page loaded over plaintext `ynp://` shows **Not encrypted · not
  verified**. Built-in pages, the start page, loading and error states show
  neither.
- An identity mismatch is shown as `Authentication failed at <address>: ...`,
  naming the expected and the presented node ID. Other handshake failures say
  that nothing was requested and that nothing fell back to plaintext. The
  failed page is an ordinary error page: tabs, history, Reload (which verifies
  again) and other pages keep working.

## Limits

- Addresses follow [Network scope](#network-scope): `TlsListener::bind`
  refuses non-loopback addresses, `TlsListener::bind_lan` refuses everything
  but one private IPv4 address, both check before creating a socket, `accept`
  drops peers outside the listener's scope. `bind_public` is a separate opt-in
  for one specific IPv4 address. `TlsTransport::connect` accepts local and
  ordinary public IPv4 endpoints while refusing the documented special-use
  ranges, names and remote IPv6.
- At most three seconds per socket operation, capped by the remaining
  five-second handshake deadline or ten-second total TLS connection budget,
  on both client and node. TCP connect has a separate three-second timeout.
- YNP frame limits are unchanged (at most 278682 bytes). TLS records are at
  most 16 KiB; rustls bounds its buffers.
- The node's worker pool (1..32 workers) and accept queue behave as for
  plaintext `serve-dir`. A handshake runs inside the connection's worker, so a
  failed handshake affects only that connection. Public listeners additionally
  apply the bounded active/start admission limits documented above.

## Threat model

**Assets.** The content and integrity of YNP requests and replies, and the
client's assurance that it is talking to the node whose ID it chose.

**Protected against**, assuming the node's secret key is secret and the
libraries are correct:

- A local process, or another host on the LAN, that reads the traffic: it
  sees TLS records, not YNP frames or document keys.
- A local process or LAN host that modifies, truncates, reorders or injects
  traffic, including an on-path attacker on the LAN (ARP spoofing, a rogue
  access point): the record protection or handshake transcript check fails
  and the client gets an error, never a modified document.
- A process or host that takes over the expected address or port first, a
  node at a wrong or swapped address (for example after a DHCP change), or a
  stale process holding another identity: the identity check fails.
- Replaying a node's certificate, which every client receives: without the
  secret key the CertificateVerify signature fails.
- An expired or not yet valid certificate, a certificate edited after signing,
  a certificate chain, a CA certificate or a non-Ed25519 key.
- Downgrade: no TLS 1.2, no plaintext fallback, no resumption.

**Not protected against:**

- **Choosing the wrong node ID.** The client trusts exactly the ID it is
  given. How a user learns a node's ID is out of band; there is no directory,
  web of trust or certificate authority, and nothing stops a user from being
  handed an attacker's ID.
- **A stolen node key.** Anyone with `node.key` can impersonate the node until
  users stop trusting that ID. There is no revocation and no key rotation; the
  24-hour certificate lifetime does not help because the thief can issue new
  certificates. See the key-at-rest limitations in [identity.md](identity.md).
- **A compromised machine or user account.** Malware running as the user, an
  administrator or a debugger can read keys and plaintext from memory.
- **Clock manipulation.** Certificate validity is checked against the
  client's clock; a client with a wrong clock may accept an expired
  certificate or refuse a valid one.
- **Traffic analysis.** Connection timing, sizes and the fact that YNP is in
  use (the ALPN value is sent in clear) are visible to a network
  observer, as are both IP addresses. This is not anonymity.
- **Malicious clients.** Clients are not authenticated; any local process, and
  for a LAN/public listener any host that can reach its address and port, can fetch
  any page the node serves. Only the operating system firewall narrows who
  can connect.
- **Malicious content.** A verified node can still serve hostile documents.
  Verification says who sent a document, not that it is safe; the browser's
  bounded, inert parsers remain the defence.
- **Denial of service.** Handshakes cost CPU; hostile clients can
  occupy workers until their deadlines. Public admission limits are basic
  controls, not comprehensive protection of bandwidth, CPU or OS resources.
- **Network exposure beyond the LAN.** LAN listeners drop public peers,
  but nothing stops a router from forwarding a
  port with source NAT, which makes internet clients appear as a private
  address. Do not forward a LAN-mode service; deliberate remote tests use the
  separate public TLS mode and narrow firewall rules.
- **Past content.** A recorded session cannot be decrypted later without the
  ephemeral key exchange secrets (TLS 1.3 forward secrecy), but nothing here
  protects content the node itself stores.

## Limitations

- Public IPv4 is experimental and not validated across providers during
  development. IPv6 beyond loopback, names and NAT traversal are not
  implemented. No claim of production-safe internet operation is made.
- The separate opt-in [DHT MVP](dht.md) reuses this exact verifier and listener;
  it adds signed endpoint lookup without making bootstrap peers authorities.
- Optional LAN discovery is a separate, unauthenticated query-response
  protocol. It supplies an untrusted candidate ID and endpoint; the existing
  TLS verifier still requires proof of that exact key. See
  [LAN discovery](lan-discovery.md).
- LAN listening has been tested with client and node on one computer bound to
  its private address, not yet across two physical machines (see
  [LAN testing](../../lan-testing.md)).
- Documents fetched over TLS are attributed to the connection's node, but the
  site signatures (`site.sig`) are still not transmitted or checked by clients.
- No client authentication, revocation, key rotation, certificate
  transparency, or post-quantum key exchange.
- The integration has not been independently reviewed. rustls, ring and webpki
  are widely used and audited; how this project uses them has not been.

## Tests

`apps/yami-node/tests/tls.rs` covers: fetching and parsing a YLang document
from a pinned node; refusal of a node with another identity; expired and not
yet valid certificates; a certificate altered at many byte positions; a copied
certificate presented without its key; bit flips in transit in both the
handshake and the encrypted reply; byte-exact maximum-size and Unicode
documents; no plaintext fallback in either direction and loopback-only
addresses; clients without `ynp/1` ALPN or Ed25519; many sequential and
concurrent reconnections, a node restarted with its persistent identity, and a
replaced identity; `ynps://` URL parsing; and the CLI demo, `serve-dir --tls`
and `fetch ynps://` end to end. Unit tests cover the signing-domain checks and
certificate renewal, and the network scope: allowed and refused addresses,
`bind_lan` refusals and `ynps://` LAN URL parsing.

`apps/yami-node/tests/lan.rs` binds a real private IPv4 address of the test
machine (`YAMINET_TEST_LAN_ADDRESS` overrides the choice; without one these
tests report that they skipped) and covers: an encrypted fetch from a pinned
LAN node; refusal of a LAN node with another identity before any request;
refusal of plaintext listeners, clients, URLs and raw plaintext requests on
the LAN; the CLI refusing LAN listening without `--tls --lan` or on wildcard,
loopback, public and link-local addresses; and `serve-dir --tls --lan`
printing a `ynps://` address that `fetch` opens, that the wrong ID cannot.

The browser tests cover `ynps://` address validation, links on a verified page
staying on the same pinned node, navigation, links, history, reload, tabs and
NotFound over TLS (on the LAN address when one exists), authentication
failures followed by normal use, a plaintext node behind a `ynps://` address,
the sidebar indicator and scrolling of a verified page, and the terminal viewer
refusing the wrong node ID.

## Bounded byte streams and reverse relay sessions

The same pinned TLS 1.3 configuration can now run over `ByteStream` through
`TlsTransport::connect_over` and `TlsAcceptor::accept_over`. Adapters must
propagate deadlines to underlying blocking reads/writes. These entry points
retain the existing ten-second TLS/YNP budget and exact-key verifier; the
acceptor opens no network listener. Direct TCP APIs, scope checks, ALPN,
certificate renewal and identity verification remain unchanged.

`SecureStream` exposes bounded timeout controls and a TLS-exporter channel
binding for the separate relay protocol. Explicit outer control sessions may
last at most 60 seconds; this does not extend ordinary direct fetch lifetimes.
The relay's raw tunnel is a byte stream, not authenticated destination proof.
Only the subsequent inner pinned handshake authenticates the destination.
See [reverse relay v1](relay.md) for channel-bound registration, session limits
and the distinction between outer relay TLS and inner YNP TLS.
