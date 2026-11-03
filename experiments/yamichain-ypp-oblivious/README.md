# YPP Milestone 4 — distributed private-write rejection

**Decision: reject this dense two-server private-write / full-download
composition. It is not a functioning oblivious private-vault mechanism.**

This is a new, isolated storage experiment after `21ab5d1`. It uses additive
secret-shared point writes, hiding Pedersen vector commitments, a standard
Schnorr AND/OR shape proof, symmetric authenticated encryption, and authenticated
full-snapshot retrieval. It uses neither historical-root replacement nor KZG
refresh hints. No payment, note, inbox, chain verifier, or financial module is
implemented or enabled. All earlier experiments are preserved.

| Requested property | Experimental finding |
| --- | --- |
| Independent concurrent updates survive | Pass: separately prepared deltas commute, in either order, without replacing a root |
| Conflicting updates cannot both succeed | **Fail:** both real shape proofs verify; their combined ciphertext is unusable |
| No stable identifier / write-position leak | **Fail:** joining write shares exposes support; ordinary recovery downloads also expose stable changed slots |
| Wallet witness refresh after unrelated writes | Conditional pass: fetch full snapshots and authenticate against an independently pinned head; this is a storage witness, not a private Merkle membership proof |
| Authenticated offline recovery | Conditional pass with both complete latest snapshots, original wallet key/index and independently pinned current head; tampering, stale snapshots and truncation fail |
| Offline receiving / encrypted availability | **Fail:** no recipient-authorized credit relation; one missing server share prevents recovery |
| No trusted coordinator / secret-bearing validator | **Not established:** private channels need separated storage views; public reconstruction leaks, and head generation / distributed atomic publication are only honest reference execution |

The proof establishes **at most one nonzero aggregate cell delta**. It does not
prove owner authority, that an old ciphertext is current, a conflict marker, or
the correctness/availability of a new head. Accepting it as a complete state
transition proof would be an implementation error. These missing properties and
the concrete transcript attack trigger the requested stop conditions. No payment
system is built on top of the failed storage composition.

## Reproduce

Rust 1.90, Python 3, Linux for per-process RSS measurements. From repository root:

```sh
manifest=experiments/yamichain-ypp-oblivious/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo clippy --manifest-path "$manifest" --all-targets --locked -- -D warnings
cargo test --manifest-path "$manifest" --release --locked
cargo build --manifest-path "$manifest" --release --locked
binary=experiments/yamichain-ypp-oblivious/target/release/yamichain-ypp-oblivious
"$binary" demo /tmp/ypp4-transcripts
"$binary" verify /tmp/ypp4-transcripts
python3 experiments/yamichain-ypp-oblivious/observer.py /tmp/ypp4-transcripts/downloadable-snapshots.json
python3 experiments/yamichain-ypp-oblivious/measure.py > /tmp/ypp4-measurements.json
```

The checked-in [fixtures](fixtures) can be independently checked using `verify`
and the Python observer instead of generating fresh transcripts. The observer
receives only ciphertext-share snapshots; no target labels, wallet keys,
plaintexts, or commitment openings. It identifies every updated slot and every
same-slot pair in this trace. `verify` also validates shape proofs and exposed
share openings. Fixture heads are hash consistency checks, not independent
finality certificates.

`packets.json` is the limited public transcript of the private-channel mode.
`exposed-channels.json` models publishing both channels to validators or colluding
storage servers; it includes disposable commitment blinds for independent opening
checks. `downloadable-snapshots.json` is exactly what the full-download recovery
protocol returns to any wallet. These threat models are distinct: the recovery
attack needs no server collusion. No keys are saved. The library is an unaudited
offline harness and does not harden/zeroize wallet secrets.

[DECISION.md](DECISION.md) specifies the algebra, threat model, rejected trust
boundary, other established building blocks, and limitations.
[validation.md](validation.md) records checks and performance;
[measurements-x86_64.json](measurements-x86_64.json) retains raw samples and
machine context. Sizes cover 2–256 vaults, with exact byte formulas and measured
native timings; larger sizes are extrapolations only.
