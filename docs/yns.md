# YNS 0.1 — decentralized naming foundation

YamiNet identity addresses (`yami://yn1-…/home`) remain fundamental and always
supported. A readable name resolves to a **full NodeId**, then the existing DHT
locates that identity and TLS authenticates it before a document is requested.
Naming never supplies an endpoint, changes network policy, enables relay fallback,
or bypasses destination-pinned TLS. Games, Discover and DHT record semantics are
unchanged. This implementation is native Rust (`yami-yns`), with no DNS, browser
engine, JavaScript, blockchain, mandatory server or project registration.

## Address forms

| Form | Trust and scope |
| --- | --- |
| `yami://FULL-NODE-ID/home` | Existing direct identity; no YNS state is opened |
| `yami://@blog/home` | User-controlled local alias to a full node ID |
| `yami://blog.FULL-OWNER-ID.yns/home` | Self-certifying namespace; root authority is the full owner public key |
| `yami://www.team.FULL-OWNER-ID.yns/home` | Owner delegates `team`; delegate signs `www.team` |
| `yami://FULL-OWNER-ID.yns/home` | Optional signed mapping at the namespace root |
| `yami://welcome.yami/home` | Separately trusted official registry, disabled until keys are configured |

The capitalized placeholders must be replaced with complete, canonical lowercase
`yn1-` identities, including checksums. The owner identity need not be a running
node. Generate a separate offline owner key rather than reusing a website's TLS
identity. Anyone can create an owner key and namespace without project approval.
Two owners can both have `blog`: their full names differ by owner key. A bare
`blog` is deliberately unsupported; it has no globally agreed owner.

YNS 0.1's browser source is an explicitly imported local cache of signed records.
There is no automatic name discovery/publication in this milestone. The reusable
`RecordSource` interface permits future peer/file/DHT distribution; every source
remains untrusted and must pass the same verification. The existing DHT continues
to distribute signed **node location** records only. Export/import the public
record JSON to share a name; never share the owner's secret key directory.

## Browser management

Run these commands with `yami-browser`, or `cargo run -p yami-browser --` in a
source checkout. An optional `--config FILE` selects the existing browser profile.
State is in `<selected peer/config state directory>/yns`; without a config this
uses the existing peer directory (`YAMINET_PEER_DIR` can override it). The same
profile is used by the desktop, terminal and management commands. These commands
are explicit local operations and do not contact a registry.

```text
yami-browser yns namespace-create /absolute/private/owner
# Prints FULL-OWNER-ID.yns; back up this private directory securely.

yami-browser yns alias-set blog FULL-NODE-ID
yami-browser yns aliases
yami-browser --desktop yami://@blog/home
yami-browser --terminal yami://@blog/home
yami-browser yns alias-replace blog OLD-FULL-ID NEW-FULL-ID
yami-browser yns alias-remove blog OLD-FULL-ID

# Times are positive Unix seconds; ISSUED <= current time < EXPIRES.
yami-browser yns record-sign /absolute/private/owner FULL-OWNER-ID.yns blog 1 ISSUED EXPIRES map FULL-NODE-ID > blog.json
yami-browser yns import blog.json
yami-browser yns resolve blog.FULL-OWNER-ID.yns
yami-browser --desktop yami://blog.FULL-OWNER-ID.yns/home

# Give a separate key authority over descendants of team.
yami-browser yns record-sign /absolute/private/owner FULL-OWNER-ID.yns team 1 ISSUED EXPIRES delegate FULL-DELEGATE-ID > team.json
yami-browser yns import team.json
yami-browser yns record-sign /absolute/private/delegate FULL-OWNER-ID.yns www.team 1 ISSUED EXPIRES map FULL-NODE-ID > www.json
yami-browser yns import www.json

# For an update, advance the sequence and import the new signed record.
# A changed destination produces an error showing both full identities.
# Verify the change independently, then approve the exact pair explicitly:
yami-browser yns approve-change blog.FULL-OWNER-ID.yns OLD-FULL-ID NEW-FULL-ID

# Tombstone an exact mapping or delegation. '-' is the absent target.
yami-browser yns record-sign /absolute/private/owner FULL-OWNER-ID.yns blog 2 ISSUED EXPIRES revoke - > revoked.json
```

Use path `-` for a namespace-root mapping. A delegated child's sequence must
advance beyond the child's remembered sequence even after its signing key changes.
Delegation records must be imported before descendants. Every ancestor must be a
live delegation: a mapping at `team` does not also authorize descendants.

The desktop displays readable addresses and the existing security view displays
the actual authenticated destination ID. Relative links from a loaded named page
use that authenticated ID; alias/record edits cannot redirect its same-node
resources, images or games. Plaintext links remain blocked from named native
pages. Bookmarks/history may retain the readable address and re-resolve it, subject
to the same persistent binding checks.

## The reserved `.yami` exception

`.yami` is exclusively for official YamiNet services and selected introductory
websites. It is not a permissionless namespace. Registry operation, allocation
policy and production key selection are separate governance work.

```text
yami-browser yns registry-keys
yami-browser yns registry-set FULL-TRUSTED-REGISTRY-KEY [ANOTHER-TRUSTED-KEY ...]
yami-browser yns registry-set
# No keys means .yami resolution is disabled.
```

No production registry key is generated or embedded. Keys are configured locally
only after independent verification. An official record uses namespace `yami`;
its top-level signer must be one of these keys. Below a signed delegation, only
the delegated key is accepted. Adding a registry key does not authorize silent
replacement of an existing destination binding. Removing all registry keys has no
effect on scoped namespaces or aliases. Ordinary owners cannot publish official
names simply by signing a `yami` record. Tests use disposable test identities only.

## Security and persistence

Records require strict Ed25519 verification, full valid non-weak public-key IDs,
exact namespace/path matching, a positive monotonic sequence, and a live validity
interval. The signed payload includes every authority, target and timing field.
See the [normative v1 record format](specs/v0.1/yns.md).

Successful first resolution pins a name to its full destination ID (trust on first
verified resolution, not a human reputation guarantee). Higher-sequence records
can update metadata or renew a name without changing that binding. A changed
identity is blocked with old/new IDs; explicit approval must match the current
trusted ID and currently authenticated proposed record. Importing an update does
not approve a changed destination. Aliases similarly require the exact old ID
for replacement/removal. There is no automatic retry through another name system.

The locked profile stores aliases, configured official keys, public records,
per-record sequence/digest high-water marks, and name-to-identity pins together.
Writes use an owner-only temporary file, flush, rename, and directory sync on
Unix; failed writes are surfaced. A leftover `yns.tmp` after a crash blocks writes:
with all browser/management processes stopped, retain `yns.json` and remove only
the temporary file before retrying. History is never automatically evicted when
records expire or are revoked. Equal sequences with different payloads are rejected.
History from authenticated parent records is retained even if a child lookup fails.
Record imports validate the complete authority chain before changing stored state.

Missing state starts empty. Corrupt, oversized, symlinked, locked or unwritable
state fails closed; it is never silently replaced with an empty profile. File
locking serializes browser workers and excludes competing processes. Keep the
profile in a directory controlled by the user. A malicious local administrator,
manual deletion, restoring an older backup, a fresh device, or clock rollback can
remove freshness assurances. There is no global latest-record oracle: an unseen,
still-valid older record may be accepted on first use, and an eclipse can prevent
updates/revocations from being learned. Use a reliable clock and back up history.

Names have one accepted spelling: lowercase ASCII labels of 1–32 characters,
starting with `a-z`, then `a-z`, `0-9`, or `-`, ending in a letter/digit. `xn--`
IDNA labels, Unicode (including mixed-script lookalikes), control/format/zero-width
characters, escapes, case folding, whitespace, empty labels and trailing dots are
rejected. There are at most eight readable labels. The full key suffix uses the
existing canonical NodeId rules. ASCII lookalikes (`l`/`1`, `rn`/`m`) and deceptive
human labels remain possible; never treat readable spelling as reputation, hide
the owner scope, or abbreviate IDs when reviewing trust changes. Explicit `@`
aliases cannot shadow `.yami` or key-scoped names.

## Rotation and recovery

A self-certifying namespace is permanently anchored to its owner key. YNS v1
intentionally has no root replacement/recovery override. To rotate the root,
create a new namespace and distribute it over independently authenticated
channels; users explicitly update their aliases/bookmarks. Losing the root key
means losing update authority. Compromise is not recoverable by asking the project
or the `.yami` operator to take over an ordinary namespace.

Keep roots offline and use short-lived delegated signing keys. A parent can rotate
or revoke a delegate with a higher-sequence parent record; old delegated records
then fail against the new authority. Reissue descendants with higher sequences.
Existing leaf destination pins still require explicit approval if the mapped node
changes. A compromised parent can sign newer delegation records, so clients still
need out-of-band verification of destination changes. Tombstones revoke the exact
path; a root mapping tombstone does not revoke the namespace's owner key.

Threshold signatures, precommitted recovery keys, cross-signed root transitions,
revocation transparency and multi-device history synchronization require a new
reviewed specification. None is implied by this foundation. Back up private owner
keys separately from the public browser trust history.

## Global uniqueness is deferred

A DHT is a distribution/lookup mechanism. Signatures prove who signed a claim;
they do not establish which conflicting claim was first globally. Network
partitions, Sybil peers, eclipses, clock disagreement and replay make DHT-only
first-come-first-served ownership claims invalid. YNS 0.1 promises uniqueness only
within a cryptographically scoped owner namespace or a user's local aliases.

Possible separate designs for short globally unique permissionless names include:

- An open, Sybil-resistant PoW/PoS consensus ledger with explicit finality,
  registration ordering, fees/anti-squatting, expiration and reorganization rules.
  This adds substantial security/economic assumptions; no blockchain is implemented.
- A publicly specified BFT quorum with independently operated validators and
  verifiable checkpoints/conflict evidence. It can give deterministic ordering
  under a bounded-fault assumption, but validator membership and censorship
  resistance must be addressed before calling registration permissionless.
- Federated registries with transparency logs and witness quorums, explicitly
  scoped to chosen trust sets. These are useful opt-in authorities, but do not
  create one universally unique permissionless namespace by themselves.

A future proposal must define adversary/Sybil assumptions, ordering/finality,
partition behavior, recovery, governance, economic abuse controls and migration
from scoped names. Globally unique permissionless names remain deferred until
that consensus design is approved. `.yami` is not a substitute consensus mechanism
for ordinary names.
