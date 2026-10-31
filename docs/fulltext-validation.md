# Full-text search validation

Validated on 2026-10-31 in the managed Linux environment, Rust 1.90.0,
starting from main `d1e1c4959c4ee2056bc7954bbf80f8a2148252ba`.
Implementation branch: `feature/fulltext-search`.

## Required workspace checks

All commands completed successfully against the final implementation:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

Cargo test harness summaries: **322 passed, 0 failed, 8 ignored**. The ignored
tests require owned public endpoints, a selected LAN/multicast interface, or
explicit manual screenshot capture. They were not counted as passing.
The new search crate contributes 12 tests. Browser additions exercise worker
cancellation/latest-query behavior, persistent providers/offline search,
corrupt-cache/invalid-provider reporting, Pages controls and complete independent
destination authentication. Existing directory Discover, browsing, DHT, TLS,
relay, identity, assets, configuration and node tests passed in the workspace run.

Phase checks also passed separately before committing each phase. Local CLI
smokes exercised index/query/stats/rebuild and provider/create/add/build/verify.
The format validator tested real YLang limits, multi-shard UTF-8 byte splits,
unsupported versions, structure/hash/payload inconsistencies and duplicates.

## Real network integration scope

Tests used **real loopback sockets**, existing signed DHT records, pinned TLS
and YNP node dispatchers. Search tests cover authenticated direct downloads,
wrong-node rejection before document requests, signed but expired records,
explicit signed-relay fallback, malformed manifests/shards, incomplete transfer,
outages, valid-cache retention, cancellation, multi-provider deduplication,
offline search and request-count/key assertions for query privacy.

The browser integration starts independently identified destination, provider
and bootstrap nodes. It refreshes through the production fetcher, searches
offline, opens a provider-supplied canonical address using the existing loader,
renders the destination's actual title instead of the provider's claimed title,
rejects a forged destination identity and preserves tab/history/reload behavior.

These tests are not physical internet or two-computer validation.

## Actual native UI smoke

The native Rust browser ran in an actual X11 window under Xvfb 21.1.16 at
1440×900. Both first-launch and reopened-profile smokes passed. Checks include
native window/surface creation, resize events, Unicode clipboard, tabs, find,
long-document/image rendering, native Pages mode, cached full-text results and
keyboard selection. The reopened run also checked bookmark persistence.

The full-text UI cache was an explicitly prepared local fixture. This verifies
the native window/rendering path; authenticated transport is verified by the
separate real-socket integration tests. Input was in-process logical actions
and the native clipboard, not physical keyboard/mouse/IME input. Additional
control tests paint Pages/results/provider views at 800×600, 1000×700 and
1440×900 with 1×/2× scales, check hit geometry and exercise keyboard/mouse actions.

## Limits and remaining validation

NOT RUN: physical internet/two-computer tests, Windows/macOS native runtime,
physical keyboard/IME and manual accessibility review. The minimal Linux
environment lacks CJK fonts; original Unicode and search boundaries are tested,
but glyph coverage needs appropriate system fonts.

Provider authentication proves who published the index, not the truth of its
website claims. Fetching an index reveals interest in that provider; no anonymity
is claimed. Queries never go to providers. The documented byte/page/provider/
time budgets, locally trusted non-mutating directories and single-writer state
assumptions apply. Publication updates use new output directories.

Git author and committer are both `ItsKorayYT
<123951134+ItsKorayYT@users.noreply.github.com>` for all implementation commits.
Remote push preparation was tested with a non-mutating Git push dry run; it
failed because this environment has no HTTPS Git username/credential available.
The four phase commits remain on the local review branch. No main rewrite,
force push, release publication or merge was attempted. After configuring Git
push access, push `feature/fulltext-search` and open a review against `main`.

Next milestone: the Windows and two-independent-network validation procedure
recorded in [the roadmap](fulltext-roadmap.md), followed by release review.
