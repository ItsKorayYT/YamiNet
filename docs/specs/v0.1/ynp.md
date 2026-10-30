# YNP 0.1 draft (wire version 1)

This remains the unchanged 0.1 specification. See the explicit [0.2 extension](../v0.2/ynp-assets.md) for rich documents/styles or PNG asset frames; old documents and wire-v1 document messages remain supported.


**EXPERIMENTAL: plaintext, unauthenticated, not secure for sensitive use.**
The same frames can be carried over authenticated TLS 1.3; see
[ynp-tls.md](ynp-tls.md).
YNP is a custom binary request/response protocol. It does not use HTTP.
This draft defines one request and one reply per loopback TCP connection.
There is no negotiation, discovery, compression, routing, encryption or identity.

## Frame

Integers use unsigned big-endian encoding. A frame consists of the following
16-byte header followed by exactly `payload_length` bytes:

| Byte offset | Size | Meaning |
| --- | --- | --- |
| 0 | 4 | Magic `59 4e 50 00` (`YNP` followed by NUL) |
| 4 | 1 | Wire version, MUST equal 1 |
| 5 | 1 | Kind: 1 Get, 2 Document, 3 NotFound |
| 6 | 2 | Reserved, MUST equal zero |
| 8 | 4 | Payload byte length, excluding header |
| 12 | 4 | Request ID echoed unchanged in reply |

All unknown versions, kinds and nonzero reserved bytes MUST be rejected before
reading a payload. Stream reads/writes MUST handle fragmentation; one socket
read is not a frame. Early EOF is an error. The decoder returns one frame and
does not consume a following frame. Extra bytes *inside* a declared payload are
an error; additional frames on a connection have no defined v0.1 semantics and
are not processed by the node.

## Payloads and bounds

`key` is a u16 byte length followed by 1..128 ASCII bytes from
`A-Z a-z 0-9 _ -`. It is an opaque identifier, never a filesystem path. A directory-backed
node MAY map a key to exactly one file named `<key>.ylang` directly inside its
site root, and MUST NOT serve any file that resolves elsewhere.
`text(max)` is a u32 byte length followed by valid UTF-8 of at most `max` bytes.
No NUL terminators or implicit padding exist. Text may be empty at the wire
layer; the language parser determines whether a document is valid.

| Kind | Payload |
| --- | --- |
| Get (1) | key |
| Document (2) | key, text(262144) document, text(16384) YSL |
| NotFound (3) | key |

Get/NotFound payload length MUST be 3..130 bytes. Document payload length MUST
be 11..278666 bytes. The header bound MUST be checked before payload allocation.
Every internal length MUST fit both its individual bound and the remaining
payload. Invalid UTF-8, empty/invalid keys, truncated fields, integer overflow
and unconsumed payload bytes MUST be rejected. Encoding enforces the same
bounds before writing a header. No partially valid document is returned.
A maximum frame occupies 278682 bytes including the header.

Golden Get for key `a`, request ID `01020304`:

```text
59 4e 50 00 01 01 00 00 00 00 00 03 01 02 03 04 00 01 61
```

## Exchange and addressing

The client sends Get, then requires Document or NotFound with the identical
request ID and key. Any other kind/key/ID is an error. NotFound is a typed
client I/O error, not document content. The MVP client uses ID 1 because each
connection carries a single exchange. Server errors, malformed requests and
unexpected message kinds close the connection; no free-form error packet is
specified. Subsequent connections can still be served.

URLs take the form `ynp://127.0.0.1:7301/welcome` or
`ynp://[::1]:7301/welcome`. Address parsing MUST use a numeric `SocketAddr`, a
loopback IP and a nonzero destination port. No DNS, user information, query,
fragment, path traversal, redirects or alternative schemes are supported.
Listeners MAY use port 0 for ephemeral local tests. Wildcard and non-loopback
bind/connect addresses MUST fail before socket creation.

The built-in transport uses three-second connect/read/write operation timeouts.
The client has no total request deadline. The reference node additionally
limits each connection to ten seconds in total. `serve` handles connections
sequentially; `serve-dir` uses a fixed pool of 1..32 workers (default 4), each
handling one connection at a time.

## Extension boundary

`Transport::connect` and `Incoming::accept` return a `Connection` implementing
`Read + Write + Send`, with an explicit protection report. The loopback TCP
transport described above reports `ExperimentalPlaintext`; it is kept as the
legacy, insecure transport for local tests and the original demos.
`Tls13PinnedServer` is reported by the TLS 1.3 transport specified in
[ynp-tls.md](ynp-tls.md), which carries these frames unchanged after
authenticating the server against a pinned node ID. Frame encoding, document
parsing and layout are independent of sockets; an in-memory test transport
exercises this boundary. Other handshakes or anonymous routing require their own
specified protection states, validation and versioning. Their availability
MUST NOT be inferred from these traits. No anonymity is implemented in this
version.
