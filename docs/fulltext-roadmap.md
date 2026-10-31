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

## Remaining phases

2. Specify and implement bounded inert YLang manifest/shards, portable validation,
   explicit independent provider imports/builds and integration tests.
3. Generalize existing authenticated document fetcher without weakening identity
   checks; implement bounded subscriptions, atomic valid snapshots, offline
   multi-provider merging and authenticated loopback/relay tests.
4. Add Pages mode and provider management to the existing native Discover UI;
   worker I/O/search, result provenance/status, normal authenticated navigation,
   UI tests and native smoke where supported.

Final required checks: fmt, workspace Clippy, tests and build with `--locked`.
Physical internet and cross-platform/manual accessibility checks have NOT RUN.
