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
