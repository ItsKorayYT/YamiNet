# YNS naming record v1 (YNS 0.1)

Status: experimental foundation. This specification concerns readable names, not
node identity format, routing, TLS certificates, or globally ordered registration.
[Usage, trust model and open consensus work](../../yns.md).

## Names and authority

A namespace is `FULL-OWNER-NODE-ID.yns` or the reserved literal `yami`. The former
is self-certifying: the full ID encodes the root Ed25519 verifying key and checksum
according to the existing node identity specification. Creation needs only a key
pair. `yami` is authorized exclusively by independently configured registry keys;
the default trusted set MUST be empty.

A path is empty for a scoped root mapping, or 1–8 dot-separated labels. Each label
is 1–32 ASCII bytes: first `[a-z]`, middle `[a-z0-9-]*`, last `[a-z0-9]` (a one-byte
letter is valid). Prefix `xn--` is forbidden. Names MUST be accepted only in this
canonical spelling; do not lowercase, IDNA-convert, trim, percent-decode or perform
Unicode normalization. The reserved official namespace requires a nonempty path.
Node IDs retain their own canonical encoding rules. `@label` is an explicit local
alias and is never a signed namespace or registry lookup.

Given path `a.b.c`, fetch and verify `c`, then `b.c`, then `a.b.c`. Each nonfinal
record MUST be a live delegation; the next record MUST be signed by its delegated
key. The first record MUST be signed by the scoped root or a configured official
registry key. Require exact namespace/path equality at every step. No recursive
name-to-name mapping or wildcard exists, so chains have at most eight records.
Root (`path=""`) delegation is forbidden in v1. Root mappings are allowed in scoped
namespaces and do not control the other top-level labels.

## JSON envelope

UTF-8 JSON object, maximum 4096 bytes on input. All fields are required. Unknown
fields, duplicate fields, unsupported versions, invalid integers, malformed IDs,
invalid signatures and noncanonical strings MUST be rejected. JSON whitespace and
object field order are not signed; the canonical payload below is signed.

| Field | Type and constraint |
| --- | --- |
| `version` | Integer, exactly `1` |
| `namespace` | Canonical namespace above |
| `path` | Canonical relative path above |
| `signer` | Full canonical NodeId of the actual signing key |
| `sequence` | `u64`, strictly positive; persistent high-water per `(namespace,path)` |
| `issued` | `u64`, positive Unix seconds |
| `expires` | `u64`, strictly greater than `issued` |
| `action` | `{"kind":"map","value":"FULL-ID"}`, `{"kind":"delegate","value":"FULL-ID"}`, or `{"kind":"revoke"}` |
| `signature` | Exactly 128 lowercase hexadecimal digits encoding 64 Ed25519 signature bytes |

`map` names a destination identity, not an endpoint or another name. `delegate`
assigns authority only to descendants. `revoke` is a signed exact-path tombstone.
No extra `value` is permitted for `revoke`. Owner and target IDs MUST encode valid,
non-weak Ed25519 public keys. No production registry root is specified here.

## Canonical signing bytes

`str(s) = u16be(UTF-8 byte length(s)) || UTF-8(s)`. Validated strings fit `u16`.
All integers below are unsigned big-endian. Concatenate in this exact order:

```text
payload = u8(1)
       || str(namespace) || str(path) || str(signer)
       || u64be(sequence) || u64be(issued) || u64be(expires)
       || action

action(map)      = u8(1) || str(full target NodeId)
action(delegate) = u8(2) || str(full delegate NodeId)
action(revoke)   = u8(3)

message = ASCII("yaminet.yns-record.v1") || 0x00 || SHA256(payload)
signature = Ed25519.Sign(owner secret key, message)
```

Verify with the exact `signer` public key using strict Ed25519 verification, then
verify its authority from the namespace/delegation chain. The dedicated signing
domain cannot authenticate TLS handshakes, DHT records or document metadata.
Future incompatible encodings require a new version and signing domain.

## Freshness and binding protection

Accept only when `issued <= now < expires`. No clock skew grace is applied.
There is no maximum TTL in v1; publishers should use short validity intervals.
All ancestors must also be valid at resolution time. Authenticated records advance
persistent `(namespace,path)` high-water marks containing sequence and payload
digest. A lower sequence MUST fail; an equal sequence is accepted only with the
same digest. Expiration, tombstones, delegation rotation and registry key changes
MUST NOT automatically erase these marks. A new delegate must continue the path's
sequence beyond its remembered high-water mark.

A first successful mapping pins the complete displayed name to its destination ID.
A later different destination MUST fail with an identity-change warning containing
the old and proposed full IDs. Explicit local approval MUST match both the stored
old binding and the current validated new mapping. Do not auto-approve changes
because their signatures or delegation transitions are valid. This rule applies
to `.yami` too. Local aliases require explicit old-ID matching for edits/removal.

Persist authenticated parent high-water marks even if a subsequent step fails,
and persist an authenticated changed mapping's high-water before returning its
identity-change error. Persist trust before permitting a network request. Return
state-write errors instead of proceeding with unrecorded trust. On unavailable
records, corrupt history, bad signatures, missing delegation, untrusted registry,
expired records or write failures, fail closed without substituting another name.
The browser MUST still permit direct full-identity navigation independently.

These protections depend on retained local history and an accurate clock; they do
not prove global freshness, globally first registration, or owner reputation.
Root key migration/recovery and global consensus remain explicitly deferred as
described in the usage and architecture document.
