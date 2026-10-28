# Experimental security status

YamiNet 0.1 is **not secure for sensitive use**. It has no identity,
authentication, authenticated encryption, anonymity or protection against a
malicious local user. Every localhost peer and every received document must be
considered untrusted. Do not put private documents, credentials or personal data
in this prototype. Do not expose it through port forwarding or a public proxy.

The concrete restrictions are numeric loopback-only TCP, strict binary field
and frame bounds, UTF-8 validation, a bounded document/style parser, rejection
of control characters in parsed content, no executable content, no automatic
network requests and no DNS resolution. These reduce accidental exposure but
are not a security audit. A modified caller can implement a different transport;
the loopback policy is enforced by `LoopbackTcp` and `LocalListener`, not by
the transport trait itself. CLI address parsing never selects another transport.

Connections use three-second per-operation socket timeouts. These are not total
request deadlines: slow trickle traffic can occupy the sequential node for
longer. There are no quotas, rate limits, hardened multi-client scheduling,
authorization checks, sandboxing, or robust denial-of-service protections.
The frame codec bounds allocation before reading payloads but this is not a
claim of comprehensive resistance to resource exhaustion.

Future identity, authenticated encryption and anonymous routing require separate
versioned designs, explicit peer-verification semantics, key lifecycle handling
and independent review. Merely replacing TCP does not establish anonymity.
