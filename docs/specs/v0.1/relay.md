# Reverse relay transport v1 (loopback MVP)

**Experimental; no anonymity or production-security claim.** `yami-relay` pairs
an outbound registered host connection with a client requesting that host's
existing full Ed25519 identity. The relay has no API to dial an arbitrary IP,
hostname or port. It forwards bytes over connections the participants opened.
The CLI accepts only numeric loopback relay endpoints. Public deployment, DHT
routes, browser selection, discovery and NAT hole punching are out of scope.

## Two distinct TLS sessions

Both host and client authenticate the relay with the existing pinned TLS 1.3
transport. Relay control messages are inside those outer sessions. After the
relay pairs them, a second TLS 1.3 session runs between client and host inside
the opaque byte tunnel. Only the host terminates this inner TLS session carrying
YNP. The client pins the exact destination ID independently of the relay ID.
No CA, TOFU, new cipher/signature algorithm, TLS resumption, plaintext fallback
or relay-issued destination identity is introduced. ALPN remains `ynp/1`.

```mermaid
sequenceDiagram
    participant H as Host (outbound only)
    participant R as Relay
    participant C as Client
    H->>R: Pinned outer TLS; REGISTER(host ID)
    R->>H: Fresh challenge and expiry
    H->>R: Host signature bound to this TLS channel
    R->>H: REGISTERED (30-second idle lease)
    C->>R: Pinned outer TLS; CONNECT(host ID)
    R->>H: GO(host ID)
    R->>C: PAIRED(host ID)
    C->>H: Inner TLS handshake through opaque forwarding
    H->>C: Prove exact destination identity
    C->>H: Encrypted YNP GET
    H->>C: Encrypted YLang document and style
```

`ynp-tls::ByteStream` carries bounded blocking I/O and propagates read/write
timeouts and shortened absolute deadlines to its underlying transport. Complete
TLS read/write calls also have per-operation budgets, so TLS control records or
trickled fragments cannot reset the blocking budget. `TlsTransport::connect_over` uses the
same pinned verifier as direct TCP. `TlsAcceptor` uses the existing renewing
node credentials without opening a host listener. The raw `Route::open_tunnel`
result is a `ByteStream`, **not** an authenticated YNP `Connection`.
`Route::connect` applies the destination's pinned TLS; `Route::fetch` then
requests exactly one document through the existing YNP codec. Only that inner
connection's protection describes destination authentication.

Direct `ynps://` behavior keeps its existing ten-second total deadline,
five-second handshake deadline, scope policy and verification. Explicit outer
sessions are separately capped at 60 seconds; they do not extend direct YNP
fetch deadlines. A session exporter binds registration to the exact TLS channel.
Unix listeners use address reuse to permit a stopped relay to restart on its
configured port; no wildcard bind or automatic exposure is added.

## Strict control framing

All integers are unsigned big endian. A control frame is:

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | `YRL\0` |
| 4 | 1 | Version, exactly `1` |
| 5 | 1 | Kind |
| 6 | 2 | Body length, exactly as below and at most 64 |
| 8 | variable | Body |

| Kind | Name | Body |
| --- | --- | --- |
| 1 | REGISTER | Host public key, 32 bytes |
| 2 | CHALLENGE | Fresh OS-random nonce (32), expiration Unix seconds (8) |
| 3 | PROOF | Ed25519 signature, 64 bytes |
| 4 | REGISTERED | Idle lease expiration Unix seconds, 8 bytes |
| 5 | CONNECT | Destination public key, 32 bytes |
| 6 | GO | Destination public key, 32 bytes |
| 7 | PAIRED | Destination public key, 32 bytes |
| 255 | ERROR | One fixed error code, 1 byte |

Keys are checked through `NodeId::from_public_key`, including validity and
non-weak Ed25519 point checks. CLI identities use the complete canonical `yn1-`
encoding and checksum. Unknown versions, kinds, incorrect lengths and invalid
keys are rejected. Reads allocate only a fixed 64-byte body buffer; fragmented
frames must complete within their phase budget. Unsolicited host bytes after
registration and before GO invalidate the connection. ERROR codes are 1
busy/conflicting registration, 2 no live host, 3 authentication failed, 4 invalid
request, 5 capacity, 6 expired registration. Unknown codes are invalid.

The host control sequence is REGISTER → CHALLENGE → PROOF → REGISTERED → GO.
The client sends CONNECT and must receive PAIRED for exactly the requested ID.
After GO/PAIRED, the respective byte stream carries raw **inner TLS records**;
the relay no longer interprets those bytes as control frames or YNP messages.
This is not a general-purpose public proxy and supports no endpoint request.

## Signed registration and replay prevention

The existing sealed identity API adds purpose
`yaminet.relay-registration.v1\0`. Its canonical 136-byte body is:

```text
relay public key (32) || host public key (32) || nonce (32) ||
TLS exporter/channel binding (32) || challenge expiration (8)
```

The exporter uses the TLS 1.3 implementation's `export_keying_material` with
label `EXPORTER-YamiRelay-v1`, no context and a 32-byte output. Both endpoints
compute it locally; the relay cannot choose the host's channel-binding bytes.
The host first pins the relay and checks a challenge expiry within the next
five seconds, then signs through `NodeIdentity::sign`. The relay verifies
freshness and the exact host ID's signature before inserting any registration.
Challenges are generated afresh per connection and accepted only once in that
connection's state machine. Replaying a valid proof on another connection fails
because both nonce and exporter differ. Signatures bind both identities, expiry
and the channel, preventing credential forwarding to another relay or session.
Private keys never leave the existing identity crate and are never serialized.

A registration has no transferable bearer token. Its authenticated outbound
connection is the only host-side stream used for pairing. At most one
registration, idle or active, can claim a host ID. A conflicting registration
cannot replace a live one, even with the same key; disconnect/expiry removes it.
Generation-checked cleanup cannot delete a subsequent registration. A successful
proof establishes this host key's authorization of registration, **not** proof
of a destination TLS connection or of future availability.

## Resource bounds and lifecycle

Default hard maxima (library configuration can only lower them):

- 32 accepted outer connections and at most 32 tracked session tasks.
- 8 concurrent connections per source IP and 16 admissions per IP per second.
  Rate-history storage has at most 64 IP entries and expires after one second.
- 8 registered host IDs; one queued/active client per host; 4 active tunnels.
- Five seconds for an outer TLS handshake, followed by a five-second total
  control phase, with at most three seconds in an individual control I/O call.
- 30-second idle registration leases and 60-second maximum outer sessions.
- 15-second active relay tunnels and 2 MiB total forwarded bytes per tunnel,
  counting both directions. Two directions share one 8 KiB forwarding buffer.
- Relay forwarding polls reads at 10 ms per direction; writes have a one-second
  timeout. A stalled destination closes the tunnel rather than growing a queue.
- Client outer lifetime is 20 seconds including connection setup. Inner TLS/YNP
  gets the smaller of the remaining client budget and the existing ten-second
  YNP budget. A slow/malicious relay cannot extend it indefinitely.

The relay removes disconnected hosts, expires idle leases and releases all
connection/tunnel permits on errors. Either tunnel endpoint's EOF/error closes
both connections; truncated YNP responses fail the existing frame decoder.
Each tunneled connection handles one YNP request. The host reacquires an idle
outbound slot after serving or lease expiry. Failed connections use exponential
backoff with jitter: 250 ms base, increasing to at most five seconds including
jitter. Only one outbound registration/tunnel is in flight for that host worker.
An idle host polls GO at 200 ms to observe shutdown without a long blocking wait.

Relay shutdown stops new admissions, closes idle registrations and active pumps,
and joins its bounded task list. Existing control/handshake phases and writes
finish within their budgets; no detached workers survive a normal return.
Host shutdown ends its registration loop and closes the outbound connection;
active YNP I/O remains bounded by the TLS deadline. Forced host disconnection is
detected by the relay's read polling and closes the paired client promptly.

## Security and availability limits

This MVP deliberately supports one client at a time per host. There is a short
re-registration gap after a request. Clients can see busy/unknown errors and
must retry explicitly; no availability or capacity guarantee is made. The host
reconnects automatically to its explicitly configured relay. It neither finds
another provider nor changes trust pins after failure.

The relay terminates the **outer** TLS sessions and sees IDs, source addresses,
timing, byte counts and the inner TLS handshake/ciphertext. It does not receive
the host's private key or terminate the inner session, and cannot decrypt YNP
document contents from that traffic. A malicious relay can refuse service,
drop/delay traffic, correlate users, or attempt substitution. The client's
independent destination pin prevents substituting another node; TLS does not
hide metadata or provide anonymity. Authenticated content can still be malicious.

Unauthenticated clients and malicious registered hosts can occupy bounded slots
or force cryptographic/connection work; this is not public-internet abuse
protection. There is no client authentication, fairness guarantee, revocation,
multi-relay failover or production security review. Protect identities and local
configuration. Public relay deployment, DHT relay records/maintenance, browser
relay selection, NAT hole punching, Tor and I2P are separate future work.

See [local operation and three-process tests](../../relay-testing.md).
