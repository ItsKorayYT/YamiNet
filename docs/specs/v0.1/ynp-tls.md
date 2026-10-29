# YNP over TLS 1.3 0.1 draft

**EXPERIMENTAL. Not independently reviewed. Localhost only. No anonymity.**
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
cargo run -p yami-node -- fetch ynps://<node-id>@127.0.0.1:7443/home
```

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
EXPERIMENTAL: TLS 1.3 to a pinned node ID, localhost only. Not independently reviewed; no anonymity.
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
ynps://<node-id>@<numeric-loopback-IP>:<port>/<key>
```

The node ID before `@` is the identity the client requires. It MUST parse as a
`yn1-` node ID (exact canonical spelling, checksum, valid non-weak key). The
remainder follows the `ynp://` rules: numeric loopback IP, nonzero port, no
DNS, and a valid YNP key. A `ynps://` URL without a node ID is rejected rather
than connected to without verification, and a `ynp://` URL never selects TLS.
The node ID in the URL is the trust decision; the address only says where to
connect, so a wrong or swapped address fails rather than reaching another node.

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

## Limits

- Loopback only: `TlsListener::bind` refuses non-loopback and wildcard
  addresses before creating a socket, `accept` drops non-loopback peers and
  `TlsTransport::connect` refuses non-loopback destinations.
- Three-second socket timeouts per operation, as for plaintext YNP, and a
  ten-second total deadline per connection, including the handshake, on both
  the client and the node.
- YNP frame limits are unchanged (at most 278682 bytes). TLS records are at
  most 16 KiB; rustls bounds its buffers.
- The node's worker pool (1..32 workers) and accept queue behave as for
  plaintext `serve-dir`. A handshake runs inside the connection's worker, so a
  failed handshake affects only that connection.

## Threat model

**Assets.** The content and integrity of YNP requests and replies, and the
client's assurance that it is talking to the node whose ID it chose.

**Protected against**, assuming the node's secret key is secret and the
libraries are correct:

- A local process that reads loopback traffic: it sees TLS records, not YNP
  frames or document keys.
- A local process that modifies, truncates, reorders or injects traffic: the
  record protection or handshake transcript check fails and the client gets an
  error, never a modified document.
- A local process that listens on the expected port first, a node at a wrong
  or swapped address, or a stale process holding another identity: the
  identity check fails.
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
  use (the ALPN value is sent in clear) are visible to a local observer. This
  is not anonymity.
- **Malicious clients.** Clients are not authenticated; any local process can
  fetch any page a node serves, as with plaintext YNP.
- **Malicious content.** A verified node can still serve hostile documents.
  Verification says who sent a document, not that it is safe; the browser's
  bounded, inert parsers remain the defence.
- **Denial of service.** Handshakes cost CPU; a local process can occupy all
  workers until their deadlines.
- **Past content.** A recorded session cannot be decrypted later without the
  ephemeral key exchange secrets (TLS 1.3 forward secrecy), but nothing here
  protects content the node itself stores.

## Limitations

- Localhost only. LAN and internet listeners are deliberately not enabled;
  doing so needs a separate review of exposure, quotas and abuse handling.
- The desktop and terminal browsers still fetch only `ynp://` plaintext URLs.
  Retrieval over TLS is available from `yami-node fetch` and the `ynp-tls`
  library. Browser status indicators do not describe TLS.
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
certificate renewal.
