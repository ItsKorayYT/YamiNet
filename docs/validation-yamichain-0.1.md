# YamiChain 0.1 validation — 2026-11-01

Scope: non-financial foundation and isolated four-validator local prototype.
Base: latest inspected `main`, `bfdaab70e41b0f1a0b1ced5a5e34ba6a685dd02a`.
Preparation archive SHA-256:
`17cf69ed663c8af0e145abd66349d2d5f9039f8c0ec4ec332e3b7a61dcdb0a31`.
No public deployment, real funds, wallet or spend-enabled proof backend.

## Host and reproducibility

Debian 13, x86_64, kernel 6.18.57, three logical/affinity CPUs, 8 GiB cgroup
memory limit. Rust 1.90.0 (`1159e78c4`), LLVM 20.1.8. These are host limits,
not peak memory measurements or validation on the proposed 2-core/2-GB target.

Builds used `CARGO_BUILD_JOBS=2`, `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0` to keep the shared environment responsive. Missing
host ALSA development metadata was supplied by extracting the matching Debian
`libasound2-dev` 1.2.14-1+deb13u1 package under the scratch directory, with
`PKG_CONFIG_PATH`/`LIBRARY_PATH` pointing there and the actual installed ALSA
runtime linked. The initial workspace Clippy attempt failed for missing `alsa.pc`;
checks below reflect the completed run with that host prerequisite supplied.
No repository code or lint rules were changed to bypass that failure.

## Checks

| Check | Actual result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo test --workspace --locked` | PASS; 464 passed, 0 failed, 9 ignored |
| `cargo build --workspace --locked` | PASS |
| `cargo test -p yami-chain-privacy --release --locked` | PASS; model plus non-test integration boundary |
| `python tools/check-chain-independence.py` | PASS; eight existing component dependency closures exclude chain crates |
| `python tools/yamichain-local.py --report docs/evidence/yamichain-0.1-local.json` | PASS; all seven scenarios, including 30 synced payloads in the delayed-start run |
| `cargo bench -p yami-chain --bench execution --locked` | PASS; 1,000 non-financial blocks in 3 ms, single optimized fixture run |
| Native Windows execution, ARM64, 2-core/2-GB hardware | NOT RUN locally; existing Linux/Windows CI matrix extended |
| Physical power loss, disk-full injection, hostile socket-network equivocation/partitions, long soak, 25/100 validators | NOT RUN |
| Whole-workspace release build, GUI package lifecycle | NOT RUN locally for this milestone; existing CI retains those steps |
| Independent cryptography/consensus audit; `cargo audit` | NOT RUN; dependency/source advisory review documented separately |

The execution benchmark registers one test label then executes 999 empty blocks.
It measures no proofs, wallets, storage fsync, networking, throughput under load,
consensus finalization latency or validator peak RSS. Its state root was
`c1969ac2c45502bc5b9082a16ea47964ebbdf56de565deb8d7613cd82f800880`.
Do not interpret this tiny fixture as a production performance claim.

## Security and negative coverage

- Stable vectors independently encoded in Python; canonical round-trips/replay,
  all truncated block prefixes, wrong chain/version/features, huge length fields,
  trailing bytes, unknown action tags, block/action count caps and overflow.
- Registry signatures, role-key reuse refusal, nonce/expiry replay, duplicate
  actions, invalid/reserved `.yami` labels and atomic failure without state mutation.
- The privacy integration library is compiled **without `cfg(test)`**. It refuses
  every u16 proof version, fake proof bytes and private-value protocol actions.
  Release mode repeats this check. No test model can become a network verifier.
- Three distinct precommits required from the fixed four-member committee.
  Wrong height/value, duplicate signatures, invalid signatures and short quorum
  certificates fail. Every durable record and sync response is bound to pinned
  genesis/committee and state execution; malformed/trailing/oversized network data
  is rejected before unbounded allocation.
- Controlled delivery schedules use the real Malachite core, production signing
  and signature verification, four seeds, delayed/reordered authenticated proposals
  and votes, one offline/equivocating identity, modeled timeouts/rebroadcast and
  certificate-verified catch-up. Honest decisions agree; two of four cannot finalize.
  This is a single-height deterministic event harness, not a formal proof or
  adversarial multi-process socket-network campaign.
- Independent process rehearsal checks four-validator agreement, one offline,
  quorum halt with two offline, live certified payload sync for a late validator,
  and crash exit 86 after durable proposal signing, after precommit signing and
  before finalized-record commit. Restart reproduces roots; the persistent signer
  refuses a conflicting vote after reopening its journal.
- Exclusive OS store lock, torn uncommitted temporary cleanup, immutable records,
  history gaps/corruption and failed certificate-verifier callbacks are tested.
  No snapshot importing or pruning trust shortcut exists.
- Existing workspace tests cover browser/hosted-node/YNS/DHT/games behavior without
  a running chain. The added CI check enforces dependency independence explicitly.

## Remaining release limits

The [development-state table](yamichain-0.1.md#development-state) distinguishes
implemented engine behavior, simulated delivery and blocked functionality.
Malachite is alpha/unaudited; known transitive Hickory and crypto-bigint security
notices plus unmaintained paste remain documented in the
[dependency review](specs/yamichain/v0.1/dependencies.md). No clean audit is claimed.
Full store preservation and trustworthy local storage remain safety assumptions;
Windows power-loss directory durability and rollback-resistant signing storage
are not established. Test identities are public, not independent operators.
All monetary, permissionless PoS, registry validity/auction, governance, treasury
and wallet functionality remains behind the [open gates](specs/yamichain/v0.1/privacy-and-next-milestone.md).
