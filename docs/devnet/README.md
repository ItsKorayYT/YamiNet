# Running YamiChain disposable devnet

**INSECURE SOFTWARE ONLY. Disposable test YAMI; no hardware protection, independent
trust domains, real funds or production financial activation. Loopback only.**

From the repository root, start all four executors, four authenticated rollback
witnesses and the gateway with one command:

```sh
./scripts/yamichain-devnet start
```

The first run builds the Rust binaries and can take several minutes. Subsequent
starts reuse the build and persistent stores. Requires Python 3, the repository's
Rust toolchain, a C/C++ build toolchain and OpenSSL development dependencies.
`CARGO` may select another Cargo executable.

Gateway: **http://127.0.0.1:28780/status** (inside the execution environment).
Executor HTTP adapters: `127.0.0.1:28790` through `28793`. Witness authenticated TCP
services: `127.0.0.1:28800` through `28803`. No public endpoint is exposed.

```sh
./scripts/yamichain-devnet wallet work/alice.wallet
BOB=$(./scripts/yamichain-devnet wallet work/bob.wallet)
./scripts/yamichain-devnet faucet work/alice.wallet 1000
./scripts/yamichain-devnet send work/alice.wallet "$BOB" 25
./scripts/yamichain-devnet balance work/alice.wallet  # 975
./scripts/yamichain-devnet balance work/bob.wallet    # 25
curl -fsS http://127.0.0.1:28780/status
```

Wallet creation returns a real Ed25519 public-key address and saves the private key
locally with mode 0600. Existing wallet files are never overwritten. The recipient
does not participate in the transfer; its signed encrypted query retrieves its
balance later. Wallet signing keys and query decryption secrets stay in the local
CLI/bridge, not in HTTP requests. Payment payloads are signed and encrypted using
the M20–M23 cryptography. Faucet transfers debit a finite disposable treasury;
they are not mint permissions or production currency.

## Blockchain finality

Each payment obtains the M23 witnessed 3-of-4 execution certificate. A SHA-256
commitment to its canonical certificate (232-bit prefix, encoded as a valid test
label) is then included in a **normal test naming transaction**, finalized by four
actual YamiChain/Malachite validator processes over TCP. The gateway authenticates
the commit certificate and exact commitment in all four durable block stores before
committing private balances and reporting success. Receipts contain the blockchain
height, block ID and commitment; full execution certificates are retained locally.

This is **blockchain finality for the private-payment certificate commitment**.
The production chain does not execute/validate these monetary transfers: financial
validity still comes from the isolated M23 executor/witness quorum. All production
financial proof rejections and hardware gates remain closed. Consensus proposal
input is an explicit `insecure-devnet-input` Cargo feature, disabled by default;
it does not change block validation, voting rules or accepted action types.

The four consensus validators run on demand for each payment (ports 28900–28903)
and stop after finalization; they retain their complete stores and signing WALs.
The four executor nodes, four witnesses and gateway remain running between sends.
Only certificate commitments are anchored, not plaintext
account balances or transactions. All consensus development identities are public;
this devnet has no production safety guarantees.

## Persistence and operations

```sh
./scripts/yamichain-devnet stop
./scripts/yamichain-devnet start
./scripts/yamichain-devnet status
./scripts/yamichain-devnet balance work/bob.wallet
```

Default data: `work/yamichain-devnet/`; override with `YAMI_DEVNET_DIR` before the
first start. Only one instance uses the fixed ports; stop before changing data
directories. Preserve the **entire** directory, including witness databases/WALs,
executor keys, policy, consensus signing stores, receipts and pending operations.
Wallet files are separate. Do not delete or selectively rewind signing databases.
The supervisor is a detached process, not a machine-boot service. Run `start` after
host/environment restarts. Logs and PID files are in the data directory.

A pending certified transfer is journaled before consensus submission and retried
on restart; commit delivery is idempotent. Lagging executor state can catch up
using certified history from a synchronized peer. Unexpected consensus failures
retain pending work and logs; this is not a hardened distributed operations stack.
The gateway serializes transfers. Network service exposure, automated failover,
authentication for gateway access and production deployment are out of scope.

The smoke record beside this README records a real Alice-to-Bob send, all-four
validator commitment verification, balances and finality surviving an actual
stop/start, and four synchronized executors. No exhaustive research/test suites
were run for this deployment change. The current instance is software-only and
co-located: separate processes/directories are not independent trust domains.
