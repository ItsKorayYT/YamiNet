# Validation, 2026-11-05 (Europe/Berlin)

Base: `22a57cf`. Rust 1.90.0, Linux x86_64. Tests and commands are in README.md;
raw final test summaries are in evidence/. Cargo.lock pins this standalone crate.

- Eight integration tests passed; strict all-target Clippy passed.
- Existing production-boundary release test passed for all 65,536 u16 versions,
  both payments and protocol actions. The test and production code were unchanged.
- Debug tests cover honest execution, offline credits and authenticated encrypted
  queries, concurrent independent senders, competing spends, nonce/replay protection,
  invalid signatures, wrong domains/chains/keys, zero/oversized spends, u64 overflow,
  ciphertext tampering, old/corrupted database snapshots and wrong storage keys.
- Receipt forgery by a malicious validator without the signing key fails. An enclave
  with that key can authenticate fabricated commitments; this negative safety result
  is an explicit passing attack demonstration, not a mitigated vulnerability.
- Tests cover synthetic measurement/identity/key/challenge/time policy mismatch and
  rejection of every input to the unimplemented TDX verifier. No genuine TDX security
  property was tested. Software evidence is unauthenticated.
- Crash coverage uses deterministic failure injection at write durability boundaries
  plus a torn pending-file fixture. No physical crash/power-loss claim.
- Replay of both the database and its external head is accepted: demonstrated
  dependency on an independent rollback-resistant finalized checkpoint.
- Disclosed full-state checking rejects balanced theft and inflation; no private
  public proof mechanism was demonstrated. This remains a financial safety blocker.

One benchmark run per size (1,000 payments each; see measurements.json):

| Accounts | Combined payments/s | Peak RSS KiB | Snapshot bytes | Reload ms |
| ---: | ---: | ---: | ---: | ---: |
| 2 | 1,895.03 | 9,984 | 2,015 | 0.028 |
| 1,000 | 196.48 | 10,240 | 508,546 | 3.973 |
| 10,000 | 25.80 | 16,740 | 5,080,179 | 39.236 |

These are software/container measurements, including wallet crypto, durable writes
and receipt verification, not Intel TDX execution or consensus throughput. Recovery
assumes retained keys and a matching independently trusted head. There is no hardware
measurement or replication/reconciliation benchmark.

Change boundary: only experiments/yamichain-tee-m20 is added. Root workspace and
lockfile, prior experiments/rejections, Milestone 16 and all consensus/production
sources are preserved. Main was neither changed nor merged. No real funds, public
service, deployment, activation, pull request or dependency on this experiment from
production was created. Overall verdict: functional isolated software baseline;
NO-GO for production and financial safety. See README.md for each remaining gate.
