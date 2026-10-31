# YNS network protocol v1 (YNS milestone 0.2)

Status: experimental. The signed [YNS record version remains v1](../v0.1/yns.md).
This extension carries existing records and introduces no new root authority,
record ownership rule, global index, or DHT storage type.

## Transport and framing

Use the existing YNP TLS 1.3 transport, ALPN, explicit numeric endpoint policy and
full serving-node identity pin. A request MUST NOT be written before the pinned
handshake succeeds. Plaintext naming requests MUST be refused, including on
loopback. There is one request and response per connection, with no forwarding,
subscriptions, arbitrary URLs, redirects or streaming uploads.

Use the existing 16-byte YNP header:

| Offset | Bytes | Meaning |
| --- | --- | --- |
| 0 | 4 | `YNP\0` |
| 4 | 1 | Wire version `4` (naming); document v1, asset v2 and game v3 unchanged |
| 5 | 1 | `1` naming request, `2` naming response |
| 6 | 2 | Reserved, MUST be zero |
| 8 | 4 | Unsigned big-endian payload length, 1–36864 bytes |
| 12 | 4 | Unsigned big-endian request ID, echoed unchanged by server |

The client implementation uses request ID `4` on each new connection. It MUST
check magic, wire version, response kind, reserved bits and correlation before
body allocation, and MUST reject unrelated document/asset/game responses without
allocating their larger payloads. Unsupported versions/kinds and oversized lengths
are rejected before reading a body. Old nodes may reject v4; clients can try other
configured providers, without plaintext fallback.

## Payload

UTF-8 JSON envelope `{"version":1,"message":OPERATION}`. All fields are required
except the explicit nullable bundle. Unknown/duplicate object fields, unsupported
versions, malformed names, oversized vectors or records MUST be rejected. JSON
whitespace and field order do not affect the existing signed canonical payload.

| Operation | `message` object |
| --- | --- |
| Query | `{"op":"query","name":"blog.FULL-OWNER-ID.yns"}` |
| Publish | `{"op":"publish","bundle":BUNDLE}` |
| Query response | `{"op":"answer","answer":{"bundle":BUNDLE_OR_NULL,"peers":[PROVIDER,...]}}` |
| Publication accepted | `{"op":"stored"}` |
| Publication rejected | `{"op":"rejected"}` |

A provider object is `{"id":"FULL-SERVING-NODE-ID","endpoint":"NUMERIC-IPv4:PORT"}`.
It is an untrusted location hint. Check its valid full NodeId, numeric nonzero
endpoint and selected public/loopback address policy before making a connection;
then pin TLS to that ID. Referrals are not signed ownership/delegation claims and
cannot alter configured registry keys, alias bindings or user trust policy.
Only locally configured operator neighbors appear in mirror responses; publishing
a record does not mutate that neighbor set.

A bundle is `{"name":"CANONICAL-REQUESTED-NAME","records":[V1_RECORD,...]}`.
It contains at most eight records and at least one. `name` is at most 330 bytes,
canonical, and never a local alias. Each encoded record is at most 4096 bytes.
All records MUST share the requested namespace and match the exact root-to-leaf
path sequence or a nonempty prefix of it, with no duplicates or unrelated records.
For `www.team.OWNER.yns`, paths are `team`, then `www.team`. For a scoped root
mapping, the only path is empty. Official names retain namespace `yami` and require
configured official root keys.

Verify signatures, full keys, namespace ownership, delegation, sequence history,
and `issued <= now < expires` for every supplied record. All ancestors before the
last supplied record MUST delegate to its next signer. The last record can be a
mapping, delegation, or tombstone. A prefix conveys only its authenticated facts;
it does not assert an absent leaf or grant a destination mapping. Parent revocation
prefixes MUST be usable to invalidate cached descendants. A response MUST echo the
requested full name; a valid signed record for another name is insufficient.

## Discovery and state processing

The browser starts with at most eight operator-configured provider pins, walks
referrals breadth-first, and admits at most eight distinct serving identities total,
including seeds and pending peers. It never follows a second endpoint for the same
identity in that lookup. Responses supply at most four referrals. Requests run
sequentially, each with at most two seconds and within a total eight-second network
budget and the browser navigation deadline. No server recursively searches peers
for the client. No hardcoded seed is specified. No seeds plus no valid cache means
an explicit bootstrap failure; absence of a record is not proof it does not exist.

Compare all individually verified replies in the searched subset with retained
records. Select highest sequences under the currently selected delegation chain;
never let the fastest mirror determine authority. Conflicting payload digests at
an equal highest sequence produce a persistent conflict barrier at that sequence.
A later lower-sequence conflict MUST NOT reduce the barrier. Reject all records at
or below that barrier until a strictly higher, authorized record is available.
Existing high-water marks and name-to-identity approval rules remain in force,
including across delegation/registry key changes. Fresh clients cannot detect
unseen equivocation or still-valid stale records suppressed by an eclipse.

Persist accepted records/history before a resolved identity can be used for a page
request. Preserve authenticated parent updates even if a descendant is unavailable.
Preserve identity pins and history on failures. Do not let missing/forged/expired
replies erase existing records or authorize older sequences. Ordinary navigation
can reuse a still-valid previously verified cache when refresh yields no usable
record; explicit refresh reports that retrieval failed. A successful refresh is
reused for at most 60 seconds, with earlier refresh on any chain expiry or clock
rollback. Local aliases are never transmitted.

Publication accepts existing signed chains independently of the transport client's
identity: server-pinned TLS authenticates the mirror, while record signatures
authorize updates. The mirror verifies the complete supplied chain and all retained
sequence/conflict history. It persists accepted ancestors in order. A rejected
publication may therefore have retained an authenticated parent; acknowledgments
are not multi-record transactions. Identical publications do not rewrite unchanged
state. Each service permits at most 32 publication attempts per monotonic minute,
with queries unaffected by that quota. Storage is capped by the existing 4096
records / 4 MiB profile limit and never evicts rollback history to admit a new owner.
The existing maximum 32 server workers and TLS/YNP I/O deadlines apply.

No wire operation configures peers, roots, aliases or binding approvals. Mirrors
have no naming authority beyond whatever ordinary namespace keys their operator
separately possesses. No operation creates global uniqueness, assigns an unscoped
short name, or puts arbitrary naming data into the node-location DHT.
