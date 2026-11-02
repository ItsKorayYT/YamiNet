# YamiChain network and storage hardening

Continues draft #17 without merging its stack. Payments, staking, elections,
randomness and historical pruning remain disabled. No public deployment.

Implemented:

* Validate startup coordination before opening stores or spawning networking.
  Convert wall time once to a bounded monotonic deadline; later clock corrections
  cannot extend the wait. Reject invalid library timeout/linger settings before
  allocating those resources. Regression tests check invalid configurations leave
  no store and test past/future deadline boundaries.
* Bound directory recovery work, including orphan-temp preflight and the history
  gap/name scan. The default is four million root entries; library callers can set
  explicit open/replay limits. Budget refusal preserves signing records, evidence,
  certified history and pending files. It never returns a partially verified tip.
  Existing record/byte budgets and exclusive store locks remain enforced.
* Extend the owned local lab with seeded 10–50-ms delays per stream read and forced
  encrypted-stream closures, plus 60-second partition measurements of WAL/signing
  bytes. Forced closures terminate the connection; ciphertext bytes are never
  removed from an otherwise continuing stream. Require nonzero closure coverage.
  The test PRNG is not a chain randomness mechanism.

## Infrastructure and networking review

There is one managed executor, no configured remote identities/host inventory,
no VPN and no permitted external TCP destinations. The validator runner also
continues to accept only public development keys and loopback endpoints. Real
25/50/100-node networks across independent hosts could not be tested.

An owned isolated Docker namespace with NET_ADMIN was probed for netem delay/loss.
It returned `Error: Specified qdisc kind is unknown.` No host interface was changed.
[Probe evidence](evidence/yamichain-network-storage/infrastructure.json).
Consequently these are local latency/jitter/connection-failure simulations, not
packet-loss, WAN, physical-host separation or NAT traversal measurements.

Authentication is not peer diversity. In the optional 100-member static ring/chord
fixture, withholding traffic at node 0's four scheduled neighbors (1, 10, 90, 99)
can isolate it while controlling less than one third of total voting power. That
is an availability/eclipse concern, not a certificate forgery. Full mesh remains
the default; unchecked peer discovery is not enabled. Real NAT mapping expiry,
CGNAT, port forwarding and authenticated adaptive peer selection need independent
hosts and a reviewed discovery policy. Per-IP protections and all BFT/signature
rules remain unchanged.

## Storage findings and remaining gate

A 60-second split kept the four-process network at height 5. Samples from 5.232 to
60.839 seconds remained at 39,021 logical bytes in aggregate, including 2,318 WAL
bytes and 7,830 signing bytes. Thus this halt did not demonstrate
continuing growth; it is not evidence of a global storage bound.

The separate cache-only fixture keeps height zero while admitting 80 distinct,
authenticated proposal rounds. It measures 8,822 bytes/33 entries at round 16,
35,318/129 at round 64, and 44,150/161 at round 80, then reopens and recovers every
acknowledged payload. These are small fixture payloads, without a live WAL or
signing-reservation workload. Required distinct-round data still grows. There is
no reviewed basis to discard locked/reproposable values, guards, WAL or evidence.

The change bounds recovery work; it does **not** impose a total live-storage quota
or solve indefinite multi-round disk growth. Raising an inspection/recovery budget
is not permission to delete records. A crash-safe retention mechanism, slashing
window and exact future spent-tag preservation remain prerequisites for pruning.
No historical deletion or snapshot-import capability is introduced.

## Dependencies

The refreshed official cargo-audit scan still reports three vulnerabilities:
Hickory 0.25.2 (RUSTSEC-2026-0118/0119) and crypto-bigint 0.5.5
(RUSTSEC-2026-0387). No compatible patched release appeared in the pinned series;
fixed 0.26.3/0.7.10 solver dry runs still conflict with upstream requirements.
No warnings are suppressed, lockfile versions forced or cryptography replaced.
[Audit](evidence/yamichain-network-storage/audit.json),
[registry](evidence/yamichain-network-storage/registry.json),
[solver attempts](evidence/yamichain-network-storage/upgrade-attempts.json).

## Reproduce the local tests

```sh
cargo build --release -p yami-validator --locked
python3 tools/yamichain-container-lab.py --case wan-rehearsal100 --output work/new-hardening
# Also wan-rehearsal25, wan-rehearsal50, stall4; each needs a fresh case directory.
cargo test -p yami-chain-consensus --locked more_than_sixty_four_rounds -- --nocapture
```

The case names do not denote real WANs. All measured clusters use two CPU
equivalents total, 2 GiB and no swap, with public development keys and four signed
naming actions per block. The 25/50-node barriers are 15 seconds; 100 uses 45 seconds.
Proposal windows are 5 seconds. All-target time includes startup and recovery.
CPU sums validator processes; proxy/monitor CPU and replay time are separate JSON
fields. Forwarded transport includes gossip/handshakes; kernel loopback counters
also include proxy hops and TCP overhead. RSS and cgroup memory differ because
shared mappings are counted repeatedly in process RSS.

## Measured local results

[Measurements](evidence/yamichain-network-storage/measurements.json) preserve binary/driver
digests, all samples, kernel limits and replay verification. Each tabled network case includes
three membership transitions, a partition, healing and a crash/restart.

| Validators | All-target seconds | Validator CPU seconds | Cgroup peak MiB | Forwarded MiB | Forced closures |
| --- | ---: | ---: | ---: | ---: | ---: |
| 25 | 40.367 | 14.592 | 179.5 | 11.9 | 26 |
| 50 | 53.607 | 46.838 | 349.1 | 42.9 | 76 |
| 100 | 150.429 | 201.032 | 855.3 | 354.7 | 253 |

All reached height 8 with 32 naming actions and independently authenticated every
store. These short rehearsals establish neither sustained WAN capacity nor shielded
proof performance, large-state storage or production readiness.

Final validation: **485 passed / 0 failed / 10 ignored**, formatting, Clippy, build,
exhaustive release payment rejection and independence passed. All 12 signed
4/25/50/100-core schedules and 11 original process scenarios passed. Seven original
manual/ignored tests passed again (screenshots/rendering and two owned internal-LAN
TLS/multicast tests); two still require an owned public endpoint unavailable here.
[Checks](evidence/yamichain-network-storage/checks.json),
[retention fixture](evidence/yamichain-network-storage/retention.json),
[original scenarios](evidence/yamichain-network-storage/original-processes.json).

Remaining gates: independent private hosts and kernel packet-loss support; reviewed
permissionless discovery/NAT/eclipsing defenses; upstream dependency remediation;
and verified retention before any historical pruning. No safety data was removed
to make recovery or benchmarks pass.
