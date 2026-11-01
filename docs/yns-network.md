# YNS 0.2 — publication and discovery

YNS now publishes and retrieves the [existing signed v1 records](specs/v0.1/yns.md)
over authenticated YNP. No JSON import is required on the receiving browser.
The owner key remains separate from the serving node's TLS identity. Mirrors can
copy and serve signed chains, but cannot modify records or assign ownership.
Direct `yami://yn1-…` addresses, local aliases, destination-pinned TLS, games,
Discover, and the existing DHT node-location protocol retain their roles.

## Architecture and initial knowledge

Provider discovery is a bounded search through an **operator-configured peer
graph**. Each independently operated mirror may configure up to four neighboring
providers. A query returns a signed record chain (or a verified prefix such as a
revoked parent delegation), plus those neighboring provider hints. Browsers query
at most eight distinct provider identities per discovery, with a total eight-second
network budget. Servers do not forward queries recursively or contact referral
addresses while serving a request. There is no mandatory YamiNet server, global
provider index, DNS lookup, broadcast, or hardcoded bootstrap list.

A fresh client needs:

1. The full readable name, including its owner ID (`blog.FULL-OWNER-ID.yns`).
2. At least one independently obtained provider pin: full serving-node ID plus
   current numeric IPv4 endpoint. Any independently operated peer can be a seed.
   A seed need not hold the record if its configured graph leads to a mirror.
3. For subsequent page browsing, ordinary DHT bootstrap information to locate the
   resolved destination. YNS records contain full identities, never destination
   endpoints. A provider can also be a DHT bootstrap, but these are separate roles.
4. For `.yami` only, separately verified official registry keys configured locally.
   Ordinary namespaces do not depend on that authority.

```mermaid
sequenceDiagram
    participant O as Offline owner
    participant P as Publisher process
    participant M as Independent mirror
    participant S as Seed provider
    participant B as Fresh browser
    participant N as Destination node
    O->>P: Signed records, public data only
    P->>M: Publish signed chain over pinned TLS / YNP
    B->>S: Query exact scoped name over pinned TLS / YNP
    S-->>B: Provider hints, no ownership authority
    B->>M: Query over pinned TLS / YNP
    M-->>B: Owner-signed chain
    B->>B: Validate ownership, times, history and identity pin; persist
    B->>N: Existing DHT location lookup, then destination-pinned TLS
```

A provider pin authenticates the transport endpoint, not the human publisher or
freshness of the supplied records. Obtain pins independently; a hostile bootstrap
can hide records, return old still-valid records on first use, or eclipse a client.
Signatures still prevent it from inventing another owner's mappings. With no
provider pins, uncached remote naming fails explicitly with `No YNS providers
configured`. Valid cached names and local aliases remain usable; direct identity
browsing never opens naming state. There is no automatic trust-on-network-discovery
of a registry root or silent fallback to a central index.

The DHT still stores signed **node locations only**. No naming record, provider
advertisement, or naming ownership claim is inserted into its storage or codec.
This milestone uses a separately versioned, resource-bounded YNP extension;
[wire specification](specs/v0.2/yns-network.md).

## Run a mirror and publish

Use the existing node binary and TLS listener. Naming is disabled unless explicitly
enabled with `--yns-dir` or `yns yes` in a directly listening node configuration.
Configured startup uses `state/yns`, inside the generated systemd unit's existing
writable state directory. `yami-testnet init --yns` enables it on the bootstrap's
existing TLS listener; no extra port or registry root is created. Outbound relay
hosts cannot serve direct YNS. Server owner keys are not namespace owner keys.

```text
# Public service: select one IPv4 interface, configure inbound TCP yourself.
yami-node serve-dir SPECIFIC-IP:7401 /absolute/site --identity /private/serving-node --tls --public --yns-dir /private/mirror-records --yns-peer FULL-OTHER-PROVIDER-ID@PUBLIC-IP:7401

# Isolated local simulation; no --public or --lan.
yami-node serve-dir 127.0.0.1:7401 /absolute/site --identity /private/serving-node --tls --yns-dir /private/mirror-records

# On the signing machine (YNS 0.1 commands are retained):
yami-browser yns namespace-create /private/offline-owner
yami-browser yns record-sign /private/offline-owner FULL-OWNER-ID.yns blog 1 ISSUED EXPIRES map FULL-DESTINATION-ID > signed-blog.json

# On any publisher machine, only the signed public file is required:
yami-browser yns publish blog.FULL-OWNER-ID.yns FULL-PROVIDER-ID@PUBLIC-IP:7401 signed-blog.json

# A fresh receiving profile receives no record files:
yami-browser yns providers-set FULL-SEED-ID@PUBLIC-IP:7401 [SECOND-SEED-ID@PUBLIC-IP:7401 ...]
yami-browser yns resolve blog.FULL-OWNER-ID.yns
yami-browser --desktop yami://blog.FULL-OWNER-ID.yns/home
```

An optional `--config FILE` chooses an existing browser profile. Without one,
`YAMINET_PEER_DIR` selects the profile directory; `YAMINET_BROWSER_DHT_NETWORK=local`
explicitly selects loopback simulation. The default is public IPv4. Provider hints
cannot cross these policies: public profiles reject private, loopback, reserved,
unspecified and multicast endpoints, and never use hostnames or DNS. YNS providers
currently require direct IPv4 TLS; LAN and reverse-relay-only providers are not
implemented. Existing destination-page relay support is preserved.

For delegated names, publish the root-to-leaf chain in order:

```text
yami-browser yns publish www.team.FULL-OWNER-ID.yns FULL-PROVIDER-ID@IP:PORT signed-team-delegation.json signed-www.team.json
```

Every record keeps its original signed namespace, relative path, signer, action,
sequence, issued time and expiry. A mirror acknowledgment means those records were
validated and stored, not that the mirror grants ownership or promises availability.
Publishing identical records is idempotent. Queries for a missing name can still
return peers, so seeds can be independent of the record holders. To update records,
advance their sequences and publish coherent chains to independently operated
mirrors. Automatic gossip replication, advertisements, endpoint migration, provider
availability guarantees, and global reachability are not claimed.

```text
yami-browser yns providers
yami-browser yns providers-set  # remove configured providers locally
# Copy through authenticated YNP without any owner key or receiver JSON imports:
yami-browser yns mirror blog.FULL-OWNER-ID.yns SOURCE-ID@IP:PORT DESTINATION-ID@IP:PORT
# Force a network refresh rather than waiting for the cache interval:
yami-browser yns refresh blog.FULL-OWNER-ID.yns
```

## Caching, conflicts and official trust

Browser profiles retain validated records, local aliases, configured provider pins,
official registry keys, rollback history, conflict barriers, destination bindings,
and successful refresh timestamps. Existing YNS 0.1 profiles migrate by adding empty
optional network/cache fields; their trust history is retained.

A live cached chain is reused for up to 60 seconds after a successful refresh.
Expiration of any ancestor or leaf forces an earlier refresh. Refresh compares all
valid replies in the bounded search with retained records and chooses the highest
sequence under the currently authorized delegation chain. It does not let response
order determine ownership. Forged, malformed, wrong-name, wrong-identity, expired,
or unauthorized replies cannot poison the validated cache. Lower sequences cannot
replace remembered higher records. A signed newer parent revocation is useful even
when the leaf is missing: it is cached and blocks the former delegated mapping.

Equal-sequence conflicting signed payloads are persisted as a conflict barrier.
Neither that sequence nor a lower sequence is usable after restart; recovery needs
a strictly higher signed sequence. Older conflicts cannot reduce a newer barrier.
These rules do not decide which of two equivocations was globally first. Histories
are never silently reset or evicted to make space. Restoring an older profile or
using a new device still loses the newer local observations.

An offline/missing provider can be bypassed through other seeds/referrals within the
request budget. If refresh yields no authenticated records, ordinary navigation
may use an existing still-valid chain, subject to all recorded history and pins.
It never uses an expired chain. `refresh` reports failure when no authenticated
response chain is available. A signed change, conflict, revocation, state-write
failure or rollback check is not treated as permission to use an older mapping.
A different mapped identity still produces the YNS 0.1 blocking warning with both
full IDs. Explicit approval must match the stored old identity and current validated
new identity; mirrors cannot approve on the user's behalf.

Aliases are never queried, published or disclosed to providers. Provider callbacks
cannot edit aliases, provider configuration, official keys or binding approvals.
Queries disclose the exact scoped name and client network metadata to contacted
providers; this feature does not provide query anonymity.

`.yami` remains reserved for official YamiNet services and selected introductory
sites. Clients use their existing `yns registry-set` configuration. Mirrors that
serve official records must separately configure trusted keys with repeated
`--yns-registry-key FULL-REGISTRY-ID` arguments; omitting them disables official
admission on that mirror. No production root is generated or shipped. Changing
mirror peers or their TLS identities has no effect on ordinary namespace ownership,
and an ordinary key cannot claim official authority by publishing a `yami` record.

## Resource limits and deployment limitations

| Resource | Limit |
| --- | --- |
| Naming payload | 36 KiB before body allocation; unrelated larger wire versions rejected by naming clients before allocation |
| Record | Existing maximum 4096 encoded bytes |
| Signed chain | 1–8 records; exact canonical selector and root-to-leaf paths |
| Referrals per response / configured mirror neighbors | 4 |
| Configured initial browser providers / total distinct contacts per lookup | 8 / 8 |
| Discovery network time / individual RPC | 8 seconds / at most 2 seconds, within navigation deadline |
| Mirror publication attempts | 32 per minute per process, including rejected attempts; queries remain available |
| Persistent record entries / state size | Existing 4096 / 4 MiB; full history fails closed rather than being evicted |
| Concurrent server connections | Existing fixed node worker pool, maximum 32; existing TLS/YNP deadlines |

The storage and publication budgets bound work but do not solve Sybil admission,
spam economics, disk failure, or availability under attack. An open mirror can
reach capacity through valid registrations and then reject new records. Operators
choose which mirrors to run and peer with, monitor capacity, and preserve history
when managing state. Quotas are local service policy, not global registration
consensus. No new mandatory infrastructure, NAT traversal, firewall opening,
central registration, DNS dependency or JavaScript is introduced.

## Reproducible independent-process demonstration

```text
cargo build --workspace --locked
python tools/yns-network-demo.py --report dist/yns-network-demo.json
```

The same command runs in Linux and Windows CI. It starts native node processes
with separate serving identities, signs with a third offline owner identity,
publishes from another browser process to a mirror, and gives a fresh browser only
a seed pin. The seed refers it to the record holder. The browser retrieves the
signed name, persists it, resolves the page destination through the existing DHT,
and renders the page through pinned TLS. A separate publisher then copies the chain
to the seed; after the first mirror stops, another fresh browser resolves through
the independent copy. The demo also checks offline validated caching, aliases, and
explicit failure without bootstrap information. No JSON record is imported into
either receiving browser profile.

This is an IPv4 **loopback simulation with independent processes**. It does not
prove operation between physical networks, NATs, public peers or hostile internet
operators. Real-internet validation remains unverified: operators must provision
reachable public IPv4 TLS nodes, independently exchange provider and DHT bootstrap
pins, and repeat publication, retrieval, expiration/refresh and mirror outage tests
across separate networks. No such remote topology is supplied or inferred here.

Globally unique short permissionless names remain deferred. Neither this provider
graph nor DHT storage establishes first-come-first-served global ownership; see
[the separate consensus design considerations](yns.md#global-uniqueness-is-deferred).
