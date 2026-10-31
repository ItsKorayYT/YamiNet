# Full-text search milestone

Base: latest main `d1e1c4959c4ee2056bc7954bbf80f8a2148252ba`.
Branch: `feature/fulltext-search`. Preserve existing native Discover and transport.

## Phase 1 — local engine

Implemented library and CLI, YLang AST extraction, shared Discover folding and
atomic writer, Unicode decomposition, inverted postings, deterministic relevance
and IDs, character-safe original snippets, bounded validated snapshots and
explicit rebuild. Flat page layout matches the existing node server. Local
files have no invented identity. See `fulltext-search.md` for budgets and use.

Validation PASS: `cargo test -p yami-search -p yami-discover --locked`
(4 full-text integration tests plus existing Discover suite); targeted Clippy
with warnings denied; formatting; local Alice index/query/stats/rebuild CLI smoke.

## Phase 2 — publication

Implemented format-1 manifest and content-hashed shards as inert YLang 0.2
documents, canonical addresses and validated normalized terms, deterministic
builds, explicit multi-site provider imports, conflict rejection, new-directory
export and verification CLI. Normative specification: `specs/v0.4/fulltext-index.md`.
Validation PASS: 8 search tests including multi-site combination, multi-shard
UTF-8 round-trip, malformed payload/envelope/hash rejection and serving a
publication through the real existing TLS/YNP node. Targeted Clippy with warnings
denied; index/provider/create/add/build/verify CLI smoke. All networking used
loopback; physical internet validation NOT RUN.

## Phase 3 — authenticated cached search

Implemented shared authenticated document retrieval through existing Discover
transport, fresh signed DHT with propagated deadlines, bounded subscriptions,
atomic complete cache/status transactions, explicit refresh/cancellation,
offline cross-provider search, canonical deduplication and publisher provenance.
The relay adapter handles its existing single-slot host registration gaps with
bounded connection-refusal retries. No new TLS/resolver/relay protocol exists.
CLI supports subscribe/unsubscribe/list/refresh/offline search.

Validation PASS: `cargo test -p yami-search -p yami-discover --locked` and
targeted Clippy with warnings denied; existing DHT suite also passed after the
shared fresh-lookup adapter was added. Search has 12 passing tests. Direct,
identity rejection, expired records, relay opt-in, invalid/incomplete refresh,
outage/cache preservation, cancellation, multi-provider deduplication, offline
query and privacy checks use real loopback sockets or explicit local-state
assertions. Query privacy inspects document keys and request counts.
Physical internet validation NOT RUN.

## Phase 4 — native Discover Pages

Implemented Websites/Pages modes in the existing native panel, independent
search-provider management, explicit refresh/cancel with local-only and relay
settings, loading/error/cache-age states, snippets/relevance/provider provenance,
keyboard/mouse selection and existing authenticated destination navigation.
Full-text store I/O, validation, local query and networking run on one bounded
worker; query generations discard stale completions. Directory subscriptions,
tabs, bookmarks and navigation retain their existing behavior. Fixed title-only
matches to show a matching original title snippet rather than unrelated body text.

Validation PASS: browser library/control/integration suites, including three
full-text worker-state tests, native Pages controls at three sizes/scales,
existing Discover compatibility, and real provider-to-browser-to-destination
loopback tests with forged destination identity rejection. An actual X11 window
smoke under Xvfb passed with cached results, keyboard selection, clipboard,
resize and rendering. Its cache is a deliberate local UI fixture; transport
validation separately uses real authenticated sockets.

## Final verification and next milestone

Final workspace fmt, Clippy (warnings denied), locked tests and locked build
all PASS. Cargo summaries: 322 passed, 0 failed, 8 ignored. Native X11 first-run
and reopened-profile full-text smokes both PASS. See `fulltext-validation.md`
for precise scope and unrun tests. Git push dry run was blocked by missing HTTPS
Git credentials; the committed branch is prepared locally for review.
Implementation commits: phase 1 `f594d10`, phase 2 `1cee9b5`, phase 3 `f8d692e`;
phase 4 is the commit introducing native Discover Pages and this status.

No requested implementation phase remains intentionally deferred. This is a
bounded experimental foundation, with flat website layouts, explicit imports,
new-directory publication generations and one local writer per state directory.
Providers can lie about sites; index retrieval reveals interest in that provider.
No anonymity, remote query submission, crawling or automatic refresh is claimed.

NOT RUN: physical internet/two-computer validation, physical keyboard/IME,
Windows/macOS native runtime, and manual accessibility review. Existing ignored
platform/network/capture tests remain opt-in. The minimal smoke environment
lacks CJK fonts; Unicode storage/search is tested but glyph display needs fonts.

Exact next milestone: validate this branch on Windows and two independent
physical networks using independent website/provider nodes, exercise direct and
signed-relay refresh plus outages/offline search, capture destination identity
rejection and privacy observations, and record results before release review.
Review and merge remain explicit; no main rewrite or release publication occurs.
