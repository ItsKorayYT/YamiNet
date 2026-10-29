# Experimental security status

YamiNet 0.1 is **not secure for sensitive use**. It has no connection
authentication, encryption, anonymity or protection against a malicious local
user. Nodes can have persistent signing identities (below), but signatures only
attribute metadata to a key. Every localhost peer and every received document must be
considered untrusted. Do not put private documents, credentials or personal data
in this prototype. Do not expose it through port forwarding or a public proxy.

The concrete restrictions are numeric loopback-only TCP, strict binary field
and frame bounds, UTF-8 validation, a bounded document/style parser, rejection
of control characters in parsed content, no executable content, no automatic
network requests and no DNS resolution. These reduce accidental exposure but
are not a security audit. A modified caller can implement a different transport;
the loopback policy is enforced by `LoopbackTcp` and `LocalListener`, not by
the transport trait itself. CLI and desktop address parsing never select another transport.

Connections use three-second per-operation socket timeouts, and the node also
ends any connection after ten seconds in total. `serve-dir` handles at most its
configured number of workers (1..32) at once; other clients wait in the
operating system's accept queue, so a few slow local clients can still delay
others for up to that deadline. There are no quotas, rate limits, hardened multi-client scheduling,
authorization checks, sandboxing, or robust denial-of-service protections.
The frame codec bounds allocation before reading payloads but this is not a
claim of comprehensive resistance to resource exhaustion.

Directory sites (`serve-dir`) read only `<root>/<key>.ylang` and
`<root>/default.ysl`. Keys are restricted by the YNP decoder before any file
access; the canonical path of each file must be directly inside the canonical
site root under its exact name, which rejects traversal, symlink escapes, case
aliases, subdirectories and non-regular files. Windows device names are never
served. Hard links cannot be detected, and a file replaced between the check
and the read is a local race: anyone able to write the site directory already
controls what it serves. Keep site directories private to their owner.

Node identities are Ed25519 key pairs from `ed25519-dalek`, persisted per
node. Their format, domain-separated signatures and limitations are specified
in [docs/specs/v0.1/identity.md](docs/specs/v0.1/identity.md). A valid signature
shows only that the holder of a node's key signed that exact site or document
metadata; it gives no confidentiality, anonymity, freshness or revocation, and
YNP does not carry or check signatures. Which node ID to trust must be learned
out of band. The secret key file is unencrypted at rest. On Unix it is created
owner-only (0600) and lax permissions are refused; on Windows it inherits its
directory's ACL and is not checked. Secret keys are never printed, logged or
transmitted, and `node.key` is ignored by Git.

Authenticated transport, encryption, key rotation and revocation, and anonymous
routing require separate versioned designs, explicit peer-verification
semantics and independent review. Merely replacing TCP does not establish
anonymity.

The desktop UI is not a new trust boundary or a secure transport. Its glass
status indicators describe the UI and navigation state, not peer authentication
or encrypted connectivity. Requests run on bounded background workers; stale
responses are discarded using tab identity and navigation generation. This
prevents navigation races but does not authenticate the responding peer or
guarantee cancellation of a request already in progress.

The native text and graphics libraries, bundled font, window-system backend and
clipboard integration expand the local dependency surface. Documents still
use the original inert YLang/YSL parser and YNP bounds. No embedded web engine,
script runtime, remote font fetch, account or cloud service is introduced.
Bundled homepage bookmarks are explicit localhost targets, and unavailable nodes
produce an error rather than falling back to HTTP or a public service.
