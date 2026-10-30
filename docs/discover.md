# Website discovery (YamiNet 0.4 preview)

YamiNet 0.4 starts **decentralized website discovery**. The `yami-discover`
crate provides a standalone local directory engine, authoring tools, persistent
catalogs and explicitly requested authenticated remote fetching, including
opt-in fallback through existing signed relay routes. It is ready for browser
integration but does not change the browser UI, networking implementations,
TLS, DHT, relays, installers or GitHub Actions.

The normative format is
[website directory format 1](specs/v0.4/website-directory.md). This page covers
the user workflow, the sample data, the trust model and the API a later browser
milestone can call.

## Why a local directory engine

Today a user must already know a site's full `yami://` address. A **directory**
is an independent list of websites maintained by an operator or community. It
contains a title, a short description, a canonical `yami://` address and
optionally a category.

Directories are ordinary YLang documents. They can be copied, emailed, put on a
USB stick, versioned in Git or served by an existing YamiNet site. There is no
central search server, no account, no telemetry, no crawler and no new network
protocol.

```ylang
@page 0.2
  title YamiNet Community Directory
  language en

:: directory 1
  :: card Alice's Notebook
    > yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home | Open Alice's Notebook
    Field notes on local-first software, identity and small YamiNet networks.
    :: category Personal
```

The format reuses YLang 0.2 metadata, sections, cards, links, inline text,
spans and resource limits. See the
[format specification](specs/v0.4/website-directory.md) for the exact grammar,
validation rules and bounds.

## Try it without a network

The three sample documents in `sites/directories/` are compiled into the demo
command. Their identities are synthetic; the sites are illustrations, not live
services. Two entries are deliberately listed by more than one source to
demonstrate deduplication.

```sh
cargo run -p yami-discover -- demo
cargo run -p yami-discover -- demo cafe
cargo run -p yami-discover -- demo 日本
```

The default demo query is `yami`. You should see 10 unique entries from 3
sources and 2 duplicate listings removed. No socket, DNS lookup or HTTP request
is performed.

Search local directory files:

```sh
cargo run -p yami-discover -- search \
  --directory sites/directories/community.ylang \
  --directory sites/directories/garden.ylang \
  cafe

cargo run -p yami-discover -- list \
  --directory sites/directories/community.ylang \
  --directory sites/directories/lab.ylang

cargo run -p yami-discover -- validate \
  --directory sites/directories/community.ylang \
  --directory sites/directories/garden.ylang \
  --directory sites/directories/lab.ylang
```

`--directory` is repeatable (`--dir` is an alias). `search` joins unquoted
positional words with spaces, so `search --directory FILE local first` works.
The CLI chooses a unique source label from each file name and reports how many
entries were added or skipped as duplicates.

## Authoring directories

Build once with `cargo build -p yami-discover`; the examples below use the
resulting `target/debug/yami-discover` binary. The CLI can create and maintain
a directory without any network access:

```sh
# Create a new empty directory. Existing files are never overwritten.
yami-discover create community.ylang --title "YamiNet Community Directory"

# Append a listing. The address is canonicalized and must not be duplicated.
yami-discover add community.ylang \
  --title "Alice's Notebook" \
  --description "Field notes on local-first software and small networks." \
  --address "yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home" \
  --category Personal

# Change metadata; the entry keeps its position and canonical address.
yami-discover edit community.ylang \
  --address "yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home" \
  --description "Updated field notes." \
  --category Journal

# Remove a category, or remove the listing entirely.
yami-discover edit community.ylang --address "yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home" --clear-category
yami-discover remove community.ylang --address "yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home"

# Inspect and re-validate with the existing parser.
yami-discover show community.ylang
yami-discover validate --directory community.ylang

# Import and search the generated directory.
yami-discover search --directory community.ylang notes
```

`create` requires a title and refuses to replace anything. `add`, `edit` and
`remove` require an existing directory document, so a mistyped path is an
error rather than a new file. An omitted `yami://` key is canonicalized to
`home`, the same rule as the native address parser.

Authoring rules:

1. **Valid output.** `Directory::render` emits a canonical YLang 0.2 document.
   It is parsed again before writing; a round-trip mismatch aborts the write.
2. **Escaping.** Descriptions are emitted as YLang literal lines (`\` prefix),
   so text beginning with `::`, `=`, `>`, `-`, `+`, `!`, `@`, `:`, `---`, `\`
   or containing `[` / `]` survives parsing exactly. Titles, directory names
   and categories are single-line raw text; control characters are rejected.
3. **Deterministic order.** `add` appends, `edit` replaces in place and
   `remove` deletes in place. A rewrite never reorders remaining entries.
4. **Duplicates rejected.** Add and edit operations reject an address already
   in the directory. Import remains tolerant and reports duplicates, but
   authoring never creates them.
5. **Limits.** Entry count, field lengths and the 256 KiB rendered source limit
   are checked before the file is changed. A failed change leaves the original
   file untouched.
6. **Atomic writes.** The new source is written to a unique temporary file in
   the destination directory, flushed, and then exposed by an atomic rename
   (or a no-clobber hard link for `create`). Existing unrelated files are not
   touched, and temporary files are removed on failure.
7. **Language preserved.** If an existing document has YLang `language`
   metadata, `show`, `edit`, `add` and `remove` preserve it on rewrite. New
   files omit language metadata.

The full format and escaping rules are in the
[format specification](specs/v0.4/website-directory.md#9-authoring-and-canonical-rendering).

## Persistent sources and cached catalogs

Register local directory files once, then search the merged catalog without
repeating `--directory`:

```sh
yami-discover source add community sites/directories/community.ylang
yami-discover source add garden sites/directories/garden.ylang
yami-discover source list

# Explicitly reload local sources and write validated snapshots.
yami-discover catalog rebuild

# Search the last built catalog. Source files are not read here.
yami-discover catalog search café
yami-discover catalog list

# Refresh one source only, then remove it and its cached state.
yami-discover source refresh community
yami-discover source remove garden
```

The default state directory is `.yami-discover`; use `--state DIR` on any
persistent command to choose another directory. Relative source paths are made
absolute at registration so a catalog remains usable from another working
directory. The state directory contains:

```text
sources.conf                       versioned source registry
snapshots/<sha256>.ylang           validated version-1 directory snapshot
snapshots/<sha256>.status          versioned provenance/cache status
dht-public-discover.cache          bounded DHT lookup cache (remote refresh)
dht-local-discover.cache           bounded loopback DHT lookup cache
*.lock                             existing DHT/registry locking files
```

`source add` validates a local directory with the existing parser before
writing the registry. Registration does **not** build the catalog and does not
write a snapshot. `catalog rebuild` is the explicit full refresh;
`source refresh NAME` is the explicit single-source refresh. `catalog search`,
`catalog list` and `source list` read only the registry and snapshots. They
never load a source file and never refresh automatically.

### Fresh, cached and inactive states

Each source has one bounded status:

| State | Meaning |
| --- | --- |
| `fresh` | The source was loaded successfully at the last explicit refresh for that source. Search results loaded from its snapshot display `[fresh]`; this is build-time provenance, not current network availability. |
| `cached` | The source was unavailable at the last explicit refresh, so its last validated snapshot was used. The bounded reason is shown with the results. |
| `unavailable` | Neither the source nor a usable snapshot was available at the last refresh. No entries are contributed. |
| `inactive remote` | A remote subscription with no successfully fetched snapshot yet. Nothing is scheduled; only an explicit `source refresh` performs network access. |
| `not built` | The source is registered but no catalog snapshot has been built yet. |

`catalog rebuild` refreshes all local sources in registration order. It does not
fetch remote subscriptions; their existing status and snapshot are loaded
unchanged. `source refresh NAME` refreshes exactly one source. For a remote
source it performs one explicitly requested authenticated fetch and then
reloads the combined view from snapshots; other source files are never read.
Both paths use the existing `Catalog`, so source order is deterministic and the
first source wins when two sources list the same canonical address.

### Authenticated remote refresh

Remote `yami://` subscriptions become usable through the existing networking
stack:

```sh
yami-discover source add future --remote yami://yn1-.../home

# Use one or more existing trusted manual bootstrap pins.
yami-discover source refresh future \
  --bootstrap yn1-BOOTSTRAP@198.51.100.10:7443 \
  --network public

# Or read the existing YAMINET_PEER_DIR / peering peers.tsv read-only.
yami-discover source refresh future --peer-dir /path/to/peers --network public

# Loopback/private test networks select the local DHT policy explicitly.
yami-discover source refresh future --network local --peer-dir ./test-peers

# Explicitly allow signed relay routes after all direct endpoints fail.
yami-discover source refresh future \
  --bootstrap yn1-BOOTSTRAP@198.51.100.10:7443 \
  --network public --enable-relays
```

The fetcher reuses, rather than reimplements:

1. the existing bounded DHT lookup cache (`Service::open_lookup`) and signed
   `lookup_fresh` record validation;
2. existing manually trusted bootstrap `Contact`s supplied by `--bootstrap`
   and/or the existing `peers.tsv` manual peer store, read without writing;
3. existing `ynp-tls` `TlsTransport`, which proves the exact node ID from the
   subscription before sending a request, with TLS 1.3 pinning and no plaintext
   fallback;
4. existing signed `RelayRoute`s through `yami_relay::Route::connect_with_timeout`
   when relay fallback is explicitly enabled. The outer TLS session pins the
   relay identity; the inner TLS session independently pins the destination
   identity. The relay is never treated as proof of the destination;
5. existing YNP document framing and the existing `Directory::parse`
   validator, including size and version limits.

A successful fetch writes the validated directory into the existing snapshot
store and marks the source `fresh`. The document is accepted only after the
source node's TLS authentication (directly or inside the relay tunnel) and full
version-1 directory validation. A failure retains the last valid snapshot as
`cached` with a bounded reason, or marks the source `unavailable` when no
snapshot exists. Failure categories distinguish configuration, signed lookup,
direct endpoint failure, relay failure, identity mismatch, transport/document
framing, and invalid directory errors.

`--network public` is the default and requires public IPv4 bootstrap endpoints.
`--network local` uses the existing loopback DHT policy. Direct signed endpoints
are always attempted first. Signed relay routes are attempted only when
`--enable-relays` is present and every direct endpoint has failed; relay support
is disabled by default. A source advertising only relay routes without the flag
fails with a clear relay error rather than silently using another path. Each
relay attempt uses the existing bounded relay budget and limits, never a
plaintext fallback. There is no automatic refresh, startup fetch, search-time
fetch, crawling, telemetry or query transmission. The DHT cache and snapshot
files remain bounded by the existing count and byte limits.

TLS authentication proves which node delivered the directory, and relay
authentication proves the relay's own identity. Neither makes the directory
content, listed websites or their owners trustworthy. The relay cannot decrypt
the inner destination TLS session.

Snapshot and status files are bounded and validated. A corrupt snapshot makes
its source unavailable with a bounded reason; a corrupt status falls back to
using a valid snapshot with a `cached` reason; a corrupt `sources.conf` is a
hard error because source identity cannot be guessed. None of these files
confer trust on a listed website.

## Importing and merging

Import is explicit and local:

1. Read at most 256 KiB from a regular, non-symlink UTF-8 file.
2. Parse it as YLang 0.2 using the existing parser.
3. Require the `:: directory 1` wrapper and validate every entry.
4. Canonicalize each `yami://` address.
5. Merge entries into the catalog in import order.

A source label must be non-empty, at most 80 characters and unique in the
catalog. At most 32 sources and 2048 unique entries are retained. A failed
import leaves the catalog unchanged.

Deduplication is by canonical address. `yami://<id>` and
`yami://<id>/home` are the same listing. The **first imported source wins** for
title, description, category and source attribution; later copies are counted
as duplicates. This is a deterministic editorial rule, not a trust decision.
Two different document keys on one identity are different entries.

## Local search

Search never leaves the process. The catalog precomputes a bounded folded index
at import time.

- Partial substrings match: `noteb` finds "Alice's Notebook".
- Case and common Latin accents are folded: `CAFÉ`, `café` and `cafe` all find
  "Café & Code"; `Straße` matches `strasse`.
- Non-Latin text is matched exactly: `日本` finds a Japanese title or
  description, and `日記` finds a Japanese category.
- Whitespace-separated terms are ANDed: `local first` must match both terms
  somewhere in an entry.
- Addresses are searchable without the scheme, so a pasted
  `yami://<id>/home` works, but the word `yami` does not match every entry
  merely because of the scheme.
- Results are ranked by title, category, description and address matches, then
  by folded title and canonical address. An empty query lists all entries in
  deterministic order. Results are capped at 100; an explicit limit is clamped
  to that cap.

Queries are limited to 128 characters and 8 terms. Control characters are
rejected. A query is never logged by this crate and is never sent to a
directory operator.

## Trust model

**A directory entry is an untrusted hint.** It is not a signature, a
certificate, a bookmark trust decision or a claim that the site is safe. A
directory operator can list any identity, describe it inaccurately, omit sites
or disagree with another operator. Version 1 defines no directory signatures or
reputation.

The canonical address still names a YamiNet identity. When a future browser
opens an entry, the existing networking stack must authenticate that exact node
ID over TLS 1.3, directly or through the existing relay path. Discovery must not
weaken TLS, DHT, relay or browser security, and it must not bypass the normal
address parser or identity pin. The browser should show results as untrusted
and never open a site automatically.

This milestone also does not add crawling, automatic fetching, JavaScript,
HTML, CSS, an external web engine, accounts, telemetry or query reporting.

## API for the next browser milestone

The crate is independent of the browser. It contains only the explicit remote
refresh path, which reuses the existing `yami-dht`, `ynp-tls` and `ynp` crates
rather than duplicating them. A later milestone can add it as a path dependency:

```toml
yami-discover = { path = "../../crates/yami-discover" }
```

Typical integration:

```rust
use yami_discover::{Catalog, Error};

fn search_local(query: &str, directory_text: String) -> Result<(), Error> {
    let mut catalog = Catalog::new();
    catalog.import_file("community", "/path/to/community.ylang")?;
    catalog.import_text("garden", &directory_text)?;

    for hit in catalog.search(query)? {
        let entry = hit.entry;
        // Render as UNTRUSTED. Do not mark it verified.
        println!("{} — {}", entry.title(), entry.address());
        println!("{}", entry.description());
        if let Some(category) = entry.category() {
            println!("category: {category}");
        }
        println!("source: {}", entry.source());

        // On explicit activation, hand the canonical address to the existing
        // native address/navigation path. That path performs the TLS 1.3
        // identity authentication. Discovery supplies a destination, never
        // trust.
        let _url = entry.address().to_string();
    }
    Ok(())
}
```

Public API surface:

| Item | Purpose |
| --- | --- |
| `WebsiteAddress::parse(&str)` | Strict canonical `yami://` parsing and display |
| `Directory::parse(&str)` | Parse and validate one version-1 document |
| `Directory::load(path)` | Bounded local file loading |
| `Directory::render()` | Canonical, escaped, bounded YLang 0.2 output |
| `Directory::save(path)` / `save_new(path)` | Atomic replace / no-clobber create |
| `Directory::add_entry` / `remove_entry` / `replace_entry` | Ordered mutation with duplicate and limit checks |
| `Directory::entry(address)` | Look up one entry by canonical address |
| `Directory::language()` | Optional YLang language metadata preserved on rewrite |
| `DirectoryEntry` | Validated title, description, address, optional category |
| `Catalog::new()` | Empty merged local catalog |
| `Catalog::import_text(label, text)` | Parse and merge a document |
| `Catalog::import_file(label, path)` | Load and merge a local document |
| `Catalog::add_directory(label, directory)` | Merge an already parsed `Directory` |
| `Catalog::search(query)` | Bounded local search returning `SearchHit`s |
| `Catalog::search_with_limit(query, limit)` | Same, with a caller limit capped at 100 |
| `Catalog::entries()`, `Catalog::sources()` | Merged entries and import reports |
| `Catalog::len()`, `Catalog::is_empty()` | Unique entry count |
| `ImportReport`, `SourceInfo` | Entries read, added and deduplicated per source |
| `SearchHit` | Borrowed entry and local relevance score |
| `SourceRegistry` | Versioned, persistent local source settings |
| `DiscoveryStore::open(state_dir)` | Open the registry and snapshot state |
| `DiscoveryStore::register_local` / `register_remote` / `remove_source` | Manage registered sources |
| `DiscoveryStore::rebuild` / `refresh_source` | Explicit local rebuild or single-source refresh |
| `DiscoveryStore::refresh_source_with(name, fetcher)` | Explicit authenticated remote refresh returning `RefreshReport` |
| `DiscoveryStore::load_last` | Load the merged catalog from snapshots only |
| `AuthenticatedFetcher` | Signed DHT lookup + pinned TLS 1.3 + optional signed relay routes + YNP directory fetch |
| `RemoteDirectoryFetcher` | Transport adapter trait; must authenticate the exact identity |
| `RemoteFetchError` / `RemoteFetchErrorKind` | Bounded diagnostic categories and messages, including direct vs relay failure |
| `load_bootstrap_peers` / `merge_bootstrap_peers` | Read existing `peers.tsv` manual pins and merge conflicts |
| `RefreshReport` | Post-refresh view plus optional remote fetch failure |
| `CatalogView` | Merged `Catalog` plus per-source `SourceStatus` |
| `CatalogView::search` / `status_for` | Existing Unicode search plus provenance |
| `SourceState` | `Fresh`, `Cached`, `Unavailable`, `InactiveRemote`, `NotBuilt` |

For browser integration, open a `DiscoveryStore` at a per-user state directory
and use `load_last` + `CatalogView::search` for normal searches. Call `rebuild`,
`refresh_source`, or `refresh_source_with` only in response to an explicit user
refresh action, since those are the only operations that read source files or
perform network access. For a remote refresh, construct `AuthenticatedFetcher`
with existing manually trusted bootstrap `Contact`s (or read them through
`load_bootstrap_peers`), then pass it to `refresh_source_with`. Relay fallback
is disabled by `AuthenticatedFetcher::open`; use `open_with_relays(..., true)`
only when the user explicitly enables it. Show `RefreshReport::remote_error`
when present. The browser must still mark every result untrusted and
authenticate the canonical destination over TLS 1.3 when it is opened.

```rust
use yami_discover::{DiscoveryStore, Error};

fn cached_search(state_dir: &str, query: &str) -> Result<(), Error> {
    let store = DiscoveryStore::open(state_dir)?;
    let view = store.load_last()?;          // no source-file reads
    for hit in view.search(query)? {
        let entry = hit.entry;
        let provenance = view.status_for(entry.source());
        // Display entry and provenance as untrusted metadata.
        let _ = provenance;
    }
    Ok(())
}
```

All entry text is operator-supplied and must be treated as untrusted display
content. The browser should not fetch assets, run markup or follow links from a
directory document; it should only show the listing and act on an explicit
activation of the canonical address.

## Limits

| Resource | Limit |
| --- | --- |
| One directory document | 256 KiB |
| Entries per document | 512 |
| Sources per catalog | 32 |
| Unique entries per catalog | 2048 |
| Directory name | 80 characters |
| Entry title | 120 characters |
| Entry description | 320 characters |
| Entry category | 48 characters |
| Source label | 80 characters |
| Registered sources | 32 |
| Registry file | 128 KiB |
| Local source path | 1024 UTF-8 bytes |
| Snapshot per source | 256 KiB (version-1 directory limit) |
| Status file | 4 KiB |
| Status reason | 240 characters |
| Manual bootstrap peers read from `peers.tsv` | 256 (existing peer-store bound) |
| Signed DHT lookup | Existing fixed query, record and timeout budgets |
| Signed relay attempts | One pass over signed routes; existing relay lifetime/byte limits and bounded per-attempt budget |
| Query | 128 characters, 8 terms |
| Results | 100 |

A file is read with a hard byte cap, parsing reuses YLang's bounded parser, and
catalog counts are checked before mutation. There is no unbounded document,
query or result buffer.

## Validate this milestone

```sh
cargo fmt --all -- --check
cargo clippy -p yami-discover --all-targets -- -D warnings
cargo test -p yami-discover
cargo run -p yami-discover -- demo
```

The crate tests cover sample parsing, unexpected YLang versions, missing or
duplicate fields, invalid schemes and checksums, oversized fields and
documents, entry/source/catalog caps, duplicate handling, Unicode and partial
search, query limits, the result cap, authoring round-trips, reserved-text
escaping, in-place edits/removals, atomic writes, registry restarts, stable
source ordering, cached fallback, corrupt state, source removal, individual
refresh, inactive remote subscriptions, authenticated loopback TLS/YNP fetches,
wrong identities, invalid delivered documents, mock transport failures,
loopback relayed fetches, direct-first behavior, relay fallback, wrong relay and
destination identities, unavailable/disabled relays, tampered relay routes and
remote cache recovery.

## Scope

In scope for this milestone:

- the `yami-discover` library and CLI;
- the version-1 YLang directory format;
- sample directories and synthetic identities;
- local validation, merging and search;
- local directory authoring, escaped rendering and atomic writes;
- a persistent source registry with bounded cached snapshots and explicit
  refresh;
- explicitly requested authenticated remote `yami://` directory fetching over
  the existing signed DHT lookup, pinned TLS 1.3 and YNP stack;
- explicit, opt-in relay fallback over existing signed relay routes with
  independent outer relay and inner destination authentication;
- documentation and unit tests.

Out of scope, deliberately:

- browser UI integration;
- automatic or background downloading/crawling of directory documents;
- directory signatures or reputation;
- a central search server;
- changes to TLS, DHT, relays, installers, CI or the existing browser model.

## Related

- [Website directory format 1](specs/v0.4/website-directory.md)
- [Native addressing](native-addressing.md)
- [YamiNet security boundary](../SECURITY.md)
- [Sample directories](../sites/directories)
