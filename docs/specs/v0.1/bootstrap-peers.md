# Manual bootstrap peers v1

This is a local list of manually configured identity pins and endpoints, not
a peer exchange, routing table, DHT or mandatory bootstrap service. Nothing
is added by discovery, documents, remote advertisements or successful browsing.
There are no automatic connections, announcements, retries or identity updates.

## Commands and storage

```text
yami-node peer add <node-id> <numeric-IP:port> [--dir <peer-directory>]
yami-node peer list [--dir <peer-directory>]
yami-node peer remove <node-id> [--dir <peer-directory>]
yami-node peer connect <node-id> [--dir <peer-directory>]
```

By default the directory is alongside the per-user identity directory:
`%LOCALAPPDATA%\YamiNet\peers` on Windows, or
`$XDG_DATA_HOME/yaminet/peers` / `$HOME/.local/share/yaminet/peers` on Unix.
`--dir` overrides `YAMINET_PEER_DIR`, which overrides the default. The override
may be a relative directory. Do not commit personal peer configurations.

`add` validates the canonical existing Ed25519 node ID, numeric endpoint,
permitted address scope and nonzero port, then writes an **unverified** entry
without connecting. DNS names, wildcard, link-local, CGNAT, multicast,
documentation/reserved IPv4 and nonloopback IPv6 are refused. Duplicate node
IDs are refused, including attempts to silently change the endpoint. Remove
and re-add deliberately to change an endpoint; this resets its history.
Different IDs may deliberately name the same endpoint, but only the actual
TLS key there can authenticate. Local pins are configuration, not public proof.

`connect` performs a fresh TLS 1.3 handshake to the stored endpoint, requiring
the exact stored ID through the unchanged verifier. It closes after handshake
without requesting a document. A serving node may log the subsequent EOF as a
rejected connection because no YNP Get was sent. This command is a connectivity
and authentication check, not an ongoing peer session. Fetching and browsing
continue to use the ordinary YNP request/response protocol.

Every entry contains the full node ID, numeric IP/port, optional last successful
handshake time (Unix seconds), and last-attempt status:

| Status | Meaning |
| --- | --- |
| `unverified` | Added manually; no recorded authenticated handshake |
| `authenticated` | The last explicit attempt proved this key over TLS 1.3 |
| `failed` | The last attempt failed; any prior success time remains historical |

An authentication failure never replaces the pin with the received key. The
status is never used to bypass the verifier or claim a live connection. Every
reconnection authenticates anew. Client browsing and discovery do not update
these entries. A locally modified database cannot disable TLS checks.

## Bounded, versioned file

`peers.tsv` is UTF-8 with header `YAMIPEERS<TAB>1<LF>`, followed by zero to 256
records. Each record is four tab-separated fields followed by LF:

```text
<canonical-node-id>\t<canonical-IP:port>\t<Unix-seconds-or-minus>\t<status>\n
```

The escape sequences above denote actual tabs/newlines in the file. `-`
means no successful connection. The file is limited to 64 KiB, each record to
160 bytes, and unknown versions/statuses, invalid addresses/keys, malformed
fields, inconsistent status/timestamp combinations and duplicate IDs fail
closed. Damaged configuration is not discarded, migrated or silently reset.

Commands hold an exclusive nonblocking OS lock through `peers.lock`, including
while connecting. A concurrent command fails with a lock error instead of
losing an update. The handle releases the lock on exit/crash; the existence
of the lock file alone is harmless. No unbounded lock wait is introduced.

Updates use a new `peers-<process-id>.tmp`, written and synced before rename
over `peers.tsv`. Failed writes/replacements preserve the previous file and
roll back in-memory mutations. A preexisting staging file is never overwritten.
If a crash leaves one, inspect it and remove that specific stale staging file
before reusing its process ID. This is atomic replacement on the local file
system, not a guarantee of directory durability through power loss; network
file system locking/rename behavior is not validated. Keep backups if needed.

New Unix directories/files request modes 0700/0600; existing directory modes
are not audited. Windows files inherit directory ACLs. The list has no private
keys or document content, but reveals the user's chosen peer IDs and IPs.
It is not encrypted or signed, and does not protect against a local user who
can modify it. Protect that directory accordingly.

See [internet testing](../../internet-testing.md) for explicit public listening,
firewalls, the absence of an automatic exposure mechanism and test limitations.
