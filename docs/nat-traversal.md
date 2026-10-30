# Direct NAT traversal assessment and prerequisite implementation

Status: **design and socket-free prerequisites only**. No NAT traversal transport,
UDP probing service or QUIC document path is enabled or claimed to work. Current
TCP/TLS direct connections and reverse relays remain the usable network paths.

## Feasibility and tradeoffs

| Mechanism | Fit for YamiNet | Limits |
| --- | --- | --- |
| TCP simultaneous open | Potentially retains the byte-stream TLS implementation | Needs socket reuse, coordinated source ports and OS-specific simultaneous-open handling; many NATs/firewalls reject it; current listener/connect APIs do not provide this |
| UDP endpoint discovery | Can observe a mapping from the exact UDP socket used later | A TCP relay's observed address is not a UDP mapping; mappings may differ by destination and expire quickly |
| UDP hole punching | Often works for address/port-independent mappings with coordinated outbound packets | Does not supply reliable streams, authentication, congestion control or reassembly; raw YNP/TLS-over-TCP cannot be placed in datagrams |
| QUIC using an established Rust stack such as Quinn/rustls | Provides TLS 1.3, reliable ordered streams and congestion control over UDP; a bidirectional stream could carry unchanged YNP frames | Requires a new transport adapter, exact node-key verifier integration, bounded async runtime and socket lifecycle; TLS records themselves are not tunneled over UDP |
| Existing reverse relay | Already implemented, independently destination-authenticated and works with outbound TCP | Uses relay bandwidth and reveals connection metadata; remains essential fallback |

A future QUIC transport is a plausible improvement for common home NATs. Its
benefit must be measured against implementation/maintenance complexity and the
extra rendezvous metadata exposed. It cannot promise direct reachability behind
endpoint-dependent (often called symmetric) NATs, nested CGNAT, UDP-blocking
networks, short mapping lifetimes or incompatible address families. CGNAT is not
uniform: some mappings punch successfully, others do not. Never infer support
from a router brand or one successful endpoint-discovery reply.

The present milestone cannot safely deliver that transport: the established
TCP verifier/byte-stream adapter is not a QUIC endpoint, and this environment has
no physical NAT topology to validate port mapping, coordinated socket reuse or
UDP reachability. Adding unauthenticated probes and calling them a working NAT
path would not meet the acceptance criteria. This is an explicit deferred
transport implementation, not a conclusion that NAT traversal is impossible.

## Proposed explicit opt-in architecture

1. Configure an independently operated coordinator by full Ed25519 ID and
   endpoint, optionally alongside existing relay pins. Never discover a provider
   implicitly or make one provider mandatory. A separate future `nat` setting
   must default disabled; current configuration has no such switch to enable a
   nonexistent transport.
2. Both peers first authenticate the coordinator over the existing pinned TLS
   channel. The coordinator creates a random 256-bit rendezvous session nonce.
   Both peers sign the same consent tuple: coordinator, initiator, responder,
   nonce and expiration (at most thirty seconds). Consent is specific to the
   other peer and coordinator, never authorization to probe arbitrary addresses.
3. Use an explicitly selected non-wildcard UDP interface and one bounded socket
   per participant. The opt-in listener is deliberate exposure, not an automatic
   public listener. No UPnP, NAT-PMP, PCP or router/firewall modifications occur.
   The same UDP socket must perform discovery, punching and QUIC handshakes.
4. A coordinator UDP endpoint validates a short-lived, single-use token bound to
   the authenticated participant, session and observed source tuple. It observes
   the source itself, never accepts a claimed third-party address as observation.
   Require both consents before disclosing endpoints to the other participant.
   Tokens must use an established authenticated construction, with rotation and
   replay accounting; no hand-designed encryption. Return no response larger
   than the validated request before source-address validation to avoid amplification.
5. Public candidates must pass the same conservative global-unicast IPv4 policy
   as public DHT endpoints. Never probe loopback, private, multicast, link-local,
   CGNAT or reserved destinations through a public advertisement. Private/LAN
   candidates require a separate isolated local policy and explicit selection;
   they cannot be smuggled through public rendezvous. An observed public source
   is only a hint, not proof of reachability to the other peer.
6. Exchange the two observed candidates only within the authenticated consented
   session. Bound concurrent sessions to 64 at the coordinator, starts per IP,
   total packet/byte counts and per-peer attempts. Proposed punch schedule:
   four small authenticated/token-bound probes over at most two seconds, then
   one QUIC handshake within three seconds. Overall candidate discovery and
   traversal share the browser's remaining navigation budget, never extend it.
7. Use Quinn with rustls TLS 1.3 and YamiNet's exact Ed25519 certificate/key
   verification, `ynp-quic/1` ALPN, no 0-RTT or plaintext fallback. Disable features
   that bypass fresh identity proof. Initial QUIC reachability or rendezvous
   success earns no verified UI state. Enforce QUIC address validation, stream
   limits, flow control, connection quotas, idle/handshake/overall deadlines and
   orderly cancellation before exposing a YNP adapter.
8. Only after exact destination authentication, open one bidirectional reliable
   stream and send original bounded YNP frames. QUIC handles ordering,
   retransmission, congestion and packet protection. Do not invent a UDP YNP
   codec or retransmission/encryption protocol. Keep YLang/YSL/native rendering
   unchanged. The two nodes may each run the server role, but initiator/responder
   roles must be fixed by consent to avoid duplicate unbounded connections.
9. Candidate routes need a new transport-tagged DHT record version and signature
   domain before publication. V2 endpoints mean TCP and **must not** advertise a
   UDP mapping as a direct TCP endpoint. Ephemeral rendezvous candidates should
   initially remain session-only. Keep `RpcConnector`/transport boundaries so
   stronger privacy routing can later be added independently.
10. Failed or unsupported traversal closes its sockets/session and returns to
    configured relay fallback within the original deadline. It never weakens
    identity verification or silently changes trust pins. Display `Verified
    node · TLS 1.3` only after an authenticated direct fetch; keep separate relay
    metadata for relayed fetches. No anonymity or metadata-hiding claim follows.

## Implemented prerequisites

`yami_identity::RendezvousConsent` is sealed, domain separated with
`yaminet.rendezvous-consent.v1\0`, and canonically encodes three raw 32-byte
identity keys, a 32-byte nonce and a big-endian u64 expiration. Both participants
must sign the same tuple. It transmits no private key and cannot overlap document,
record, registration or TLS signing domains.

`yami_relay::rendezvous::ConsentGate` is an isolated in-memory validator. It
requires mutual signatures, distinct participants/coordinator, a nonzero nonce,
a live expiration within thirty seconds, and monotonically observed time. It
rejects replays and caps retained sessions at 64; only expired entries are
removed. `validate_candidate` enforces public endpoint policy without sending
anything. Neither API proves endpoint observation or permits document delivery.
These APIs are not wired into a production listener, CLI or browser setting.
The consent gate is intentionally in-memory: a future wire coordinator must bind
issued sessions to a fresh process instance or persist replay state so a restart
cannot admit an old still-live consent again. The present helper is not a
restart-safe rendezvous service.

```sh
cargo test -p yami-relay rendezvous --locked
cargo test -p yami-identity --locked
```

Tests cover forged consent, substituted parties/coordinator/nonce/expiry,
one-sided consent, replay, capacity, clock rollback, expiration, signing-domain
separation and private/special endpoint rejection. They do not test NAT traversal.

## Required follow-up verification before a transport prototype ships

Implement an isolated optional QUIC adapter and coordinator protocol with a
reviewed threat model and interoperability test vectors first. Then test exact
identity mismatch, wrong ALPN/certificate, lost/reordered/duplicated packets,
latency, expired/replayed/source-substituted tokens, third-party reflection,
unconsented disclosure, shared-source flooding, resource cleanup and fallback.
Use network namespaces/NAT appliances or controlled real routers for independent
mapping, endpoint-dependent mapping, nested NAT/CGNAT, UDP blocking, rebinding
and mapping expiry. Repeat on supported Windows/Linux versions and separate
Internet connections. Report measured success rates and every unsupported case;
loopback consent tests cannot establish any of those results.
