# YamiChain 0.1 — isolated non-financial prototype

**NOT production-ready; no real money.** This independent workspace experiment
uses Malachite 0.8.2 and four public development identities. No YAMI is issued,
no wallet exists, and every payment/private-value action proof is rejected.
Ordinary YAMI payments, when separately authorized and implemented, must be
shielded. There is no transparent transfer alternative.

The browser, hosted node, YNS, identity, DHT, relay and games do not depend on
these crates, do not connect to validators, and keep working without a chain.
`test-*` registrations have no authority in browser resolution. `.yami` remains
reserved and existing YNS trust semantics are unchanged.

## Run locally

Use the repository's Rust 1.90.0 toolchain. Existing browser-native dependencies
are needed only for the full workspace build, as in the existing CI workflow.

```sh
cargo build -p yami-validator --locked
cargo run -p yami-validator --locked -- --help
python tools/yamichain-local.py --report work/yamichain-local.json
python tools/check-chain-independence.py
```

The Python rehearsal starts **separate processes** and checks normal agreement,
one offline validator, a two-of-four quorum halt, delayed-start catch-up,
and abrupt restart after proposal signing, after precommit signing, and before
durable finalized commit. It independently replays each history and compares
both block IDs and state roots. Temporary stores are deleted after the run;
on failure bounded diagnostic logs are preserved in the JSON report.

For manual use, run these in four separate terminals (Windows binary ends `.exe`):

```sh
target/debug/yami-validator run --dir work/chain/node-0 --index 0 --height 20
target/debug/yami-validator run --dir work/chain/node-1 --index 1 --height 20
target/debug/yami-validator run --dir work/chain/node-2 --index 2 --height 20
target/debug/yami-validator run --dir work/chain/node-3 --index 3 --height 20
```

All peers use `127.0.0.1:28100..28103`; change `--base-port` identically in all
four terminals if those ports are occupied. Each process has a default 60-second
deadline; `--timeout 120` changes it. It exits at the requested finalized height.
A deadline failure is a failed liveness run, not proof of finality.

After stopping the relevant process:

```sh
target/debug/yami-validator status --dir work/chain/node-0
target/debug/yami-validator inspect --dir work/chain/node-0
```

Both commands authenticate and replay the full certified history. They acquire
an exclusive store lock and refuse uninitialized directories. Output includes
`payments_enabled:false` and `production_ready:false`. First height registers
`test-local` using an independent public registry fixture key; later blocks are
empty non-financial transitions. No transaction submission endpoint is exposed.

## Development state

| Area | State | Practical limit |
| --- | --- | --- |
| Canonical signed envelopes, headers, state roots, version/feature gates | Implemented | Prototype format; only signed test naming actions |
| Checked amount arithmetic | Implemented utility | No monetary state, decimals or issuance schedule selected |
| Mandatory shielded payment policy | Enforced by rejection | No proof backend, no spendable value, no transparent mode |
| Four-validator Malachite finalization | Implemented, genuine engine/channel integration | Public dev keys, equal fixed power, loopback only, unaudited |
| Certified history sync | Implemented | Bounded sequential replay from pinned genesis, no arbitrary snapshots |
| WAL, durable signing guard and certified recovery | Implemented | Full store must survive; rollback/deletion/Byzantine disk unsupported |
| Equivocation, delayed/reordered votes | Simulated delivery around actual Malachite core | Four seeds, modeled timers and certified catch-up; not a hostile socket-network test |
| Crash injection | Implemented process tests | Exit 86 at selected boundaries; not physical power-loss/disk-full testing |
| Snapshots and pruning | Interfaces only, blocked | No accepting snapshot implementation; pruning refuses |
| YNS/browser integration | Out of scope | Test TLDs are non-authoritative; browsing stays chain-independent |
| Real payments, PoS election, delegation, slashing, auctions, governance, treasury | Out of scope / gated | Separate specifications, authorization and security review required |
| Public network | Blocked | Dependency notices, alpha consensus, no external audit or adversarial soak |

## Storage and trust

Keep the **whole validator directory**, including `signed-*`, candidates, WAL,
genesis and finalized records. Never copy one development key into concurrent
independent stores: these keys are public and offer no operator independence or
security against a hostile user on the same host. The four processes exercise
independent identities and state machines, not independent real-world operators.
A missing WAL with signing history causes restart refusal. Restoring older
backups, deleting safety files, or malicious filesystem rollback defeats local
anti-equivocation assumptions and is unsupported.

Finalized records are immutable, bounded, fsynced and renamed before acknowledgment.
Recovery rejects invalid certificates, unknown members, duplicate signatures,
wrong chain/committee, gaps, malformed records and invalid state transitions.
A leftover unrenamed temporary file is uncommitted and removed under the lock.
On Unix the containing directory is fsynced. Windows directory durability after
physical power loss has not been established by this std-only implementation.
No filesystem operation is claimed safe against a malicious local administrator.

The application validates values in `ProposalAndParts` mode. It deliberately
avoids upstream `ProposalOnly`, whose current path marks received values valid
without an application callback. Cached previously received/built values are independently executed before they
are returned to consensus; fresh peers must receive actual payloads. Every decided certificate is rechecked
before committing the associated block.

No hard resource claims: caps are defensive prototype defaults (64 KiB block,
64 actions, 1 KiB envelope, 63-byte label, 64 pending candidate payloads per
height, 128 KiB pubsub and 1 MiB sync frame, eight values per sync batch).
History/signing files are not pruned and grow over time. The executable limits
requested height to 100,000; the wire decoder limits heights/rounds to 1,000,000.
There is no permissionless admission, production key management, IP anonymity,
fee market, staking or claim of privacy-grade currency.

See the [architecture ADRs](specs/yamichain/v0.1/architecture.md),
[privacy gates and next milestone](specs/yamichain/v0.1/privacy-and-next-milestone.md),
[dependency review](specs/yamichain/v0.1/dependencies.md),
[canonical vectors](specs/yamichain/v0.1/vectors.md), and
[validation evidence](validation-yamichain-0.1.md).
