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

## Remaining phases

3. Generalize existing authenticated document fetcher without weakening identity
   checks; implement bounded subscriptions, atomic valid snapshots, offline
   multi-provider merging and authenticated loopback/relay tests.
4. Add Pages mode and provider management to the existing native Discover UI;
   worker I/O/search, result provenance/status, normal authenticated navigation,
   UI tests and native smoke where supported.

Final required checks: fmt, workspace Clippy, tests and build with `--locked`.
Physical internet and cross-platform/manual accessibility checks have NOT RUN.
