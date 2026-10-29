# Node identity and signatures 0.1 draft (format version 1)

**EXPERIMENTAL. Not reviewed. Not secure for sensitive use.** A node identity
lets a node sign metadata so that anyone holding its node ID can check that the
holder of the matching secret key signed exactly that metadata. Signatures do
**not** encrypt anything, hide who is talking, authenticate a YNP connection,
or make communication anonymous. YNP frames are unchanged by this draft. The
same key authenticates a node in the separate TLS transport, specified in
[ynp-tls.md](ynp-tls.md).

Implemented by the `yami-identity` crate using
[`ed25519-dalek`](https://crates.io/crates/ed25519-dalek) 3 (Ed25519, RFC 8032)
and [`sha2`](https://crates.io/crates/sha2) (SHA-256). Random keys come from
the operating system through [`getrandom`](https://crates.io/crates/getrandom).
No algorithm is invented here; this document defines only encodings.

## Command line

```sh
cargo run -p yami-node -- identity new                       # create in the default directory
cargo run -p yami-node -- identity show                      # print node ID and public key
cargo run -p yami-node -- identity inspect <node-id>         # validate and decode an ID
cargo run -p yami-node -- identity sign-site sites/alice     # write sites/alice/site.sig
cargo run -p yami-node -- identity verify-site sites/alice <node-id>
cargo run -p yami-node -- serve-dir 127.0.0.1:7301 sites/alice --identity <identity-directory>
```

`new`, `show` and `sign-site` accept `--dir <identity-directory>`. Without it
the default directory is used:

| Platform | Default identity directory |
| --- | --- |
| Windows | `%LOCALAPPDATA%\YamiNet\identity` (does not roam to other machines) |
| Other | `$XDG_DATA_HOME/yaminet/identity`, else `~/.local/share/yaminet/identity` |

`new` refuses to overwrite an existing identity. `serve-dir --identity <dir>`
loads the identity in `<dir>` (creating one if the directory has none), prints
its node ID and reports at startup whether `site.sig` matches the site. The
node ID is the same on every restart. No command prints secret key material.

Each node needs its own directory: two nodes sharing one directory share one
identity. Back the directory up somewhere private; a lost key cannot be
recovered, and a new key is a different node ID.

## Node ID

```text
yn1-<58 characters>
```

- `yn1-` is the literal prefix; `1` is the node ID format version. A future
  format uses a different prefix. Parsers MUST reject any other prefix.
- The body is RFC 4648 base32 with the lowercase alphabet `a-z 2-7` and no
  padding, encoding 36 bytes: the 32-byte Ed25519 public key followed by a
  4-byte checksum.
- `checksum = SHA-256("yaminet.node-id.v1" || 0x00 || public_key)[0..4]`.
- Decoding MUST reject uppercase, padding, wrong length, nonzero trailing
  bits, a wrong checksum, a public key that is not a valid Ed25519 point, and
  small-order (weak) public keys. Each identity therefore has exactly one
  spelling, and node IDs can be compared as strings.

The checksum catches typing and copying mistakes. It is not a security
mechanism: the ID is secure only because it contains the whole public key.
Because the key is in the ID, a signature can be checked against an ID without
looking anything up.

## Identity directory

| File | Contents | Secret |
| --- | --- | --- |
| `node.key` | 72-byte binary key file, below | **Yes** |
| `node.id` | Node ID followed by a newline | No |

`node.key` layout:

| Offset | Size | Meaning |
| --- | --- | --- |
| 0 | 4 | Magic `59 4e 53 4b` (`YNSK`) |
| 4 | 1 | Key file format version, MUST be 1 |
| 5 | 1 | Algorithm, MUST be 1 (Ed25519) |
| 6 | 2 | Reserved, MUST be zero |
| 8 | 32 | Ed25519 secret seed (RFC 8032 private key) |
| 40 | 32 | Ed25519 public key derived from the seed |

Loading MUST reject a file of any other length, wrong magic, unknown version
or algorithm, nonzero reserved bytes, or a stored public key that differs from
the one derived from the seed (which catches most corruption of either half).
If `node.id` exists it MUST parse and match the key, otherwise loading fails
with an identity mismatch. A missing `node.id` is allowed because it is derived
from the key. If `node.key` is missing but `node.id` exists, the identity is
lost: no new key is generated in its place. Damaged files are reported and
never replaced automatically.

Files are created with exclusive-create semantics, so an existing identity is
never overwritten. The secret seed is held in zeroizing buffers in memory and
`NodeIdentity` never formats it in `Debug` output, errors or logs.

## Signatures

Every signature is a pure Ed25519 signature (RFC 8032, not prehashed) over

```text
message = purpose_tag || canonical_bytes
```

Purpose tags provide domain separation. Each is distinct NUL-terminated ASCII,
so no tag is a prefix of another and a signature made for one purpose never
verifies for another purpose, even over identical bytes:

| Purpose | Tag (shown without the final `00` byte) |
| --- | --- |
| Document metadata | `yaminet.document-metadata.v1` |
| Site manifest | `yaminet.site-manifest.v1` |
| Signed DHT peer record | `yaminet.peer-record.v1` |

The tag names the version of the canonical encoding; a changed encoding gets a
new tag. Every tag starts with `yaminet.`, so no metadata message begins like
the X.509 certificates (byte `30`) or TLS 1.3 CertificateVerify content (byte
`20`) that the node key also signs for the TLS transport; that signing API
refuses any other format (see [ynp-tls.md](ynp-tls.md#signing-domains)). Verification uses `verify_strict`, which rejects non-canonical
signatures and small-order keys. Integers are unsigned big-endian, and `key` is
a YNP key (1..128 bytes of `A-Z a-z 0-9 _ -`) prefixed by its u16 length.

**Document metadata v1** binds a key to the exact bytes served under it:

```text
key || SHA-256(source UTF-8 bytes) || SHA-256(style UTF-8 bytes)
```

**Site manifest v1** covers every page and the shared style of a directory
site, so additions and removals invalidate it as well as edits:

```text
SHA-256(style) || u32 page_count || (key || SHA-256(page source))*
```

Pages are in strictly ascending byte order of key; duplicates are invalid. The
hashes cover the bytes the node would serve: after removing a leading UTF-8
byte-order mark, with a missing `default.ysl` hashed as empty text. Sites with
256 or more pages cannot be signed yet.

### Detached signature file

`identity sign-site` writes `<site>/site.sig`, which the node never serves:

```text
yaminet-signature 1
purpose yaminet.site-manifest.v1
signer yn1-...
signature <128 lowercase hex digits: the 64-byte Ed25519 signature>
```

Exactly four lines, each ending in LF or CRLF, at most 512 bytes, with single
spaces between field name and value. The metadata itself is not included; the
verifier recomputes it from the files. A verifier MUST first check that the
purpose is the expected one and that `signer` equals the node ID it expected,
obtained independently. A signature by any other node is reported as an
identity mismatch, never as success. `verify-site` therefore requires the
expected node ID as an argument rather than trusting the `signer` line.

### DHT peer records

`PeerRecordMetadata` is another sealed canonical signing type: owner public
key, expiration, monotonic sequence and 1..4 sorted numeric IPv4 endpoints.
Its `yaminet.peer-record.v1` domain cannot verify as document/site/TLS data.
The DHT transmits these public signed claims and enforces expiry, endpoint
scope and remembered sequence high-water marks. This does not establish
endpoint presence or replace pinned TLS verification. See [dht.md](dht.md).

## Security limitations

- **No confidentiality or anonymity.** Signing adds no encryption. Plaintext
  YNP remains unauthenticated; anyone who can read the traffic or the files can
  read the content and see the node ID. The TLS transport encrypts traffic but
  does not hide that a connection exists.
- **Signatures are not transmitted.** Site signatures are not carried in YNP
  frames, so the browser does not verify documents it fetches. Over plaintext
  YNP a peer is not authenticated by its node ID at all; over the TLS
  transport the connection is, but `site.sig` is still not checked.
- **Trust in node IDs is out of band.** A valid signature proves only that the
  holder of the key for that ID signed the content. Deciding which node ID
  belongs to whom is the reader's job; there is no directory, web of trust or
  certificate authority.
- **No freshness, rollback or revocation for site/document metadata.** An old signed version of a site
  stays valid forever. There is no timestamp, sequence number, expiry, key
  rotation or revocation. A stolen key can sign anything as that node.
- **Exact bytes.** Signatures cover exact file bytes. Tools that change line
  endings (for example Git with `core.autocrlf`) or re-encode files invalidate
  the signature; sign on the machine that serves the files.
- **Key at rest is not encrypted.** `node.key` is not password-protected. Anyone
  who can read it, including malware running as the same user, administrators,
  backups and disk images, can impersonate the node.
- **Memory hygiene is best effort.** Buffers are zeroized on drop, but the
  operating system may still swap, hibernate or crash-dump process memory.
- **Not audited.** The libraries are widely used but this integration has not
  been independently reviewed.

### File permissions

On Unix-like systems the identity directory is created with mode `0700` and
`node.key` with mode `0600` from the moment it exists. Loading refuses a key
file that group or other users can access (`chmod 600 node.key` fixes it).

**Windows limitation:** the Rust standard library cannot set Windows access
control lists, and this project forbids `unsafe` code, so on Windows
`node.key` simply inherits the ACL of its directory and permissions are not
checked when it is loaded. The default `%LOCALAPPDATA%` directory is normally
accessible only to the user, `SYSTEM` and Administrators, but this depends on
the machine's configuration. Do not use `--dir` to place identities in shared,
synchronized (for example OneDrive) or world-readable folders. You can inspect
a directory with `icacls <identity-directory>`. Removing inheritance and
granting only your own account access is possible with `icacls` but is not
done automatically.

### Never commit or transmit secret keys

Keys are created outside source checkouts by default, and `node.key` is listed
in `.gitignore`. Nothing in YamiNet sends `node.key` over the network: YNP has
no message that carries it, and only node IDs, public keys and signatures are
ever printed. If a key is exposed, stop using that identity and create a new
one; the old node ID cannot be revoked.
