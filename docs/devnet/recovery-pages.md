# Resumable private-executor recovery

Native development networks now recover executors across more than 256 missing
transitions. The previous bounded `ExportSince`/`Recover` protocol is retained
with its original 256-certificate refusal; the supervisor uses a paged protocol.
This is an extension of the existing software witness trust model. It does not
introduce production checkpoint authority or change who holds executor secrets.

1. A synchronized executor exports the current certified state encrypted to the
   pinned recipient transport key. The gateway durably retains that ciphertext
   and its target certificate before requesting any history pages.
2. History pages contain at most 32 consecutive witnessed certificates, starting
   at an exact head and ending no later than the pinned target. The source checks
   that the target exists in its committed history. A newer source tip cannot
   silently change the requested target.
3. The recipient and its own authenticated witness verify every executor quorum,
   witness confirmation, previous commitment and successor sequence. The witness
   appends new certificates and a small recovery cursor in one SQLite FULL/WAL
   transaction before acknowledging. Pages do not rewrite the installed encrypted
   snapshot. Duplicate pages must match the retained statements.
4. The installed ledger stays at its original head while pages arrive. New votes,
   resumed votes, ordinary commits and legacy catch-up cannot bypass an active
   recovery. Staged rows are excluded from committed history responses. Existing
   sign-once votes and all certified history remain retained.
5. Only after the complete prefix reaches the pinned target may the executor
   decrypt the recipient-bound snapshot, verify its secret-blinded state
   commitment and install it. The authority atomically publishes the new head,
   encrypted snapshot and certificate and clears the recovery cursor. A local
   write interrupted before authority acknowledgement is disposable; restart
   restores the old authoritative snapshot and durable verification progress.

There is no total missing-height limit in this paged path. Memory and verification
work per page are bounded. Total history bytes and total catch-up time still grow
with the gap. The supervisor processes at most two pages per lagging executor in
each recovery pass. Its retained encrypted forward file is a cache, never signing
or snapshot authority. A missing cache can be replaced from a synchronized source;
an in-progress target may only extend forward, preserving its verified prefix.

This implementation does **not** implement private-state deltas during ordinary
payments, historical pruning, a financial checkpoint bootstrap, bounded total
disk usage, independent hardware custody or revocation. Existing RPC body/response
and witness snapshot-size limits continue to refuse oversized data. A reviewed
checkpoint/custody policy is still required to replace full retained history.

Reproduce the process-level long-gap regression:

```sh
cargo test --release --manifest-path experiments/yamichain-tee-m20/Cargo.toml --test witness_e2e long_gap_recovery_pages_survive_restart_without_installing_partial_state
```

It generates 273 certified payments with one executor absent, stages nine pages,
restarts its authority and worker, rejects missing/forged/oversized pages and an
early or corrupted state install, advances the source while retaining the old
target, then recovers the next successor and verifies the receiver's balance.
The other witness regressions cover clone exclusion, sign-once durability, replay,
rollback, crash boundaries and refusal without a live authority.

Reproduce a live TLS PoS interruption across an epoch boundary:

```sh
scripts/yamichain-rpc-config --out work/new-paged-network --port-offset 3000
python3 tools/yamichain-recovery-smoke.py work/new-paged-network/network.json
```

The directory and recipient wallet must be new. This pauses one witness while
three executors and the actual Malachite network finalize native fee-bearing
payments, resumes it, waits for paged recovery, and restarts the entire network.
It uses configured private peers and disposable YAMI. Genuine TDX deployment and
independent multi-machine validation remain external work.

[Executed native interruption](recovery-smoke.json) recovered seven missing
fee-bearing transitions across an epoch boundary and retained balance 7 after a
full restart. [Validation results](recovery-validation.json) record the separate
273-transition process regression, direct authority bypass refusals, strict
Clippy, production proof rejection and unchanged compact-checkpoint regressions.
