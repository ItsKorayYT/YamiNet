# Full-text page search

`yami-search` extracts visible text using the production YLang 0.1/0.2 parser.
It indexes the flat `<key>.ylang` layout served by `yami-node`: titles,
headings, paragraphs, inline text, lists, section/card names, link labels and
image alternative text. It never reads link targets, styles, keys or unrelated
files. Subdirectories are not websites' document keys and are not traversed.

```sh
cargo run -p yami-search -- index sites/alice --output ./alice-index
cargo run -p yami-search -- query ./alice-index "decentralized networks"
cargo run -p yami-search -- stats ./alice-index
cargo run -p yami-search -- rebuild ./alice-index
```

Supply `--identity YOUR-FULL-yn1-ID` after the index output to attach actual
canonical addresses. Local indexes without it have local page IDs, never
fabricated identities. An identity argument is an operator assertion; only
normal pinned TLS navigation authenticates the destination.

Queries use Unicode compatibility decomposition and Discover's case/accent
folding. Alphanumeric runs form tokens; partial substrings work inside tokens,
including scripts without whitespace. All query terms must match. Exact terms
receive twice the weight of partial matches; title/heading/body weights are
12/6/1, with per-token posting weights capped at 1024. Ties use stable page IDs.
Snippets preserve original characters and use mapped character boundaries.
This is deterministic search folding, not locale-specific linguistic analysis.

Limits: 256 pages, 4096 site directory entries, YLang's 256 KiB source and
4096-byte line limits, 64 KiB extracted text per page (including title and
headings), 4 MiB total text, 65,536 vocabulary terms, 262,144 postings, 256 bytes
per normalized token, 128 query characters, 8 unique query terms and 100 results.
Index snapshots are versioned JSON capped at 16 MiB. Loading validates all page
records and rebuilds postings. Invalid or over-budget builds fail as a unit.
Writes use Discover's synced temporary-file and atomic rename implementation.
No automatic rebuilding, network requests, telemetry or crawling occurs.

All selected path components and YLang page files must be non-symlinks; parent
traversal and unsafe/platform device keys are rejected. As with existing node
serving, the selected directory and local state must be trusted against
concurrent mutation by hostile local processes. Keep private material outside
public website directories. Atomicity is filesystem atomicity, not a multiwriter
transaction: use one writer per index directory.

## Publish and curate

```sh
yami-search index sites/alice --output alice-index --identity FULL-ALICE-ID
yami-search export alice-index --output published-alice
yami-search provider create community-search
yami-search provider add community-search alice-index
yami-search provider add community-search bob-index
yami-search provider build community-search
yami-search verify community-search/published
```

Import only content you are authorized to index. Imports copy the supplied,
validated content; they do not crawl or follow links. An index must have
operator-supplied canonical addresses to publish. An independent provider can
publish pages from several identities, with no assertion of ownership or trust.

Serve the generated directory using existing `yami-node serve-dir ... --tls`
or explicit relay hosting. The provider address is
`yami://PROVIDER-FULL-ID/search-manifest`. See the normative
[publication specification](specs/v0.4/fulltext-index.md) for strict bounds.
The JSON payload is hex-chunked inside inert YLang sections, using SHA-256 from
the existing identity crate to bind the manifest to exact shards. Original text
and normalized terms round-trip through the production parser and validator.

Export/build outputs must be new directories. For a subsequent generation use
`provider build community-search --output published-next`, verify it, stop the
node and switch its site directory. No unrelated site documents are overwritten.
Publishing is a separate explicit action: export never opens network listeners.

## Subscribe and search offline

```sh
yami-search subscribe search-state community yami://PROVIDER-FULL-ID/search-manifest
yami-search subscriptions search-state
yami-search refresh search-state community PATH-TO-EXISTING-PEER-STORE
yami-search search search-state "decentralized networks"
yami-search unsubscribe search-state community
```

Refresh uses existing manually pinned bootstrap peers, signed fresh DHT lookup,
exact-node TLS 1.3 and YNP document requests. Public-network policy is the CLI
default. `--relays` explicitly enables the existing signed relay fallback after
direct endpoints fail. Brief relay registration gaps permit at most three
connection-refusal retries within the same document budget. Identity failures
are never downgraded. The browser uses its existing selected network/bootstrap
policy instead. There is no plaintext fallback or server-side query endpoint.

Search reads only local state. Subscribe, list, remove, startup and query never
fetch anything. Refresh requests only the manifest key and its validated shard
keys at the **same authenticated provider identity**. It cannot follow shard
URLs to another host. No search terms, query telemetry or analytics are sent.
Fetching an index reveals interest in that provider to network observers;
this is not anonymous browsing. Independent providers may lie about titles,
snippets or claimed website identities. Opening a result must separately
authenticate its actual destination through normal browser navigation.

The versioned `search-state.json` contains subscriptions, last-success times,
bounded error messages and complete validated snapshots in one atomic file.
Bounds: 8 providers, 512 cached pages and 8 MiB extracted text overall; 32 MiB
serialized state. Invalid/corrupt state is reported, never silently overwritten.
Use one local writer per state directory. Each refresh has a 120-second absolute
network deadline propagated through existing DHT and TLS/relay connectors;
document parsing and disk commits additionally have fixed byte/node budgets.
Cancellation is cooperative between bounded requests. A failed, cancelled,
corrupted or incomplete refresh retains the previous valid snapshot. Failed
refreshes show cached/stale or unavailable status. A successful refresh is
labelled cached with its timestamp, never as a current reachability guarantee.

Cross-provider search deduplicates canonical destination addresses, picks the
highest scoring claim (provider URL breaks ties), and retains every matching
publisher's provenance. A provider's authenticated identity proves publication,
not ownership of every site it references. It does not change bookmarks or trust.

## Native browser

Open **Discover** in Yami Browser and select **Pages**. **Websites** retains
directory metadata search; its **Directories** view retains existing directory
subscriptions. Pages has a separate **Search providers** view and state under
`<Discover state>/pages`. There is no mandatory or preinstalled provider.

In **Search providers**, enter a name and canonical manifest address. Add saves
only the subscription. Select it and choose **Refresh selected provider** to
download an authenticated index. Local-only mode blocks refresh; change that
setting explicitly if you want network access. Relay fallback follows the
existing explicit browser setting. Cancel stops between bounded requests and
retains the old cache. Failure, corrupt-cache and authentication errors appear
in the panel; failed refreshes retain searchable cached/stale data.

Return to **Pages**, enter a query and press Enter or Search. Search runs on a
worker against all saved providers. Results show original-text snippets,
relevance, claimed destination address, source provider, source count and cache
age/status. Arrow keys select results; Enter opens the selection. Tab/Shift-Tab
move through native controls and Enter activates them. Mouse selection, wheel,
Previous/Next and Page Up/Down work with bounded pagination. Opening uses the
existing navigation, DHT, TLS/relay, tab, history and bookmark machinery. Indexed
text never substitutes for the authenticated destination document.

Index loading, validation, disk I/O, local full-text search and network refresh
run outside the UI thread. One bounded worker runs at a time; query changes
coalesce to the latest text and stale completions cannot replace newer results.
No network operations occur on startup, mode switching or query submission.

Validation includes real loopback provider-to-browser-to-destination tests,
native control tests at multiple sizes/scales and an actual Linux X11 window
smoke under Xvfb. The native smoke uses an explicitly prepared local cache
fixture for rendering; transport authentication is tested separately using real
TLS/DHT/relay sockets. Physical input/IME, physical internet, multiple computers
and Windows/macOS native runtime validation are NOT RUN for this milestone.
Non-Latin text is preserved and searched safely; displayed glyphs depend on
available system fonts (the minimal Linux smoke environment lacks CJK fonts).
