# YamiChain parser fuzzing

This separate workspace compiles no wallet, accepting proof verifier or financial
feature. `fuzzing` exposes parser-only checks. Every accepted encoding must round
trip exactly; all 11 application network/WAL types plus blocks, transactions and
finalized records are exercised. Network authentication/execution remains separate.

```sh
cargo run --release -p yami-chain-consensus --features fuzzing --example fuzz-seeds --locked -- fuzz/corpus/wire
# Install cargo-fuzz 0.13.2 with a compiler meeting its tool MSRV (not the node MSRV).
cargo +nightly-2025-10-01 fuzz run wire -- -max_total_time=60 -max_len=1048577 -rss_limit_mb=1536 -seed=17
```

The recorded run uses nightly 1.92.0 with default AddressSanitizer, debug assertions
and overflow checks. The ordinary node stays on Rust 1.90.0. This workspace has its
own lockfile. Default locked node tests separately perform all seed truncations,
20,000 deterministic mutations and upper-length boundary cases. Keep generated
corpora/artifacts locally; replay and minimize any crash before a regression fix.
A bounded campaign with no crash is not an audit or evidence of complete coverage.
