# Native private-payment devnet

**INSECURE SOFTWARE DEVNET. Disposable test YAMI only. No real funds, hardware
protection or production financial activation. Development validator keys are public.**

From the repository root:

```sh
./scripts/yamichain-devnet start
./scripts/yamichain-devnet wallet work/alice-native.wallet
BOB=$(./scripts/yamichain-devnet wallet work/bob-native.wallet)
./scripts/yamichain-devnet faucet work/alice-native.wallet 1000
./scripts/yamichain-devnet send work/alice-native.wallet "$BOB" 100
./scripts/yamichain-devnet balance work/alice-native.wallet # 900
./scripts/yamichain-devnet balance work/bob-native.wallet   # 100
./scripts/yamichain-devnet status
./scripts/yamichain-devnet history 2
./scripts/yamichain-devnet stop
./scripts/yamichain-devnet start
./scripts/yamichain-devnet balance work/bob-native.wallet   # still 100
```

Use fresh wallet filenames; wallet creation refuses to overwrite keys. Bob does
not participate in the transfer and can retrieve his balance later. The finite
1,000,000,000,000-unit genesis treasury funds faucet transfers; there is no mint
API. Real Ed25519 wallet signatures, X25519/ChaCha20-Poly1305 encrypted requests
and authenticated encrypted balance queries use the existing M20–M23 code.
Wallet files contain private keys, stay local, and are written with mode 0600.

First startup builds Rust release binaries. Prerequisites: the repository's Rust
toolchain, Python 3, C/C++ build tools and OpenSSL development dependencies.
`CARGO` can select Cargo. The detached supervisor starts four executors, four
SQLite-backed authenticated TCP rollback witnesses, four **continuous Malachite
validators**, four HTTP validator submission/history adapters, and the gateway.
No validator process is launched per transfer. Idle rounds continue without
creating empty blocks; timeout growth is disabled only for this insecure profile.
Run `start` again after a host reboot; this is not an installed boot service.

| Service | Local endpoint |
| --- | --- |
| Gateway/status | `http://127.0.0.1:28780/status` |
| Public pinned policy | `http://127.0.0.1:28780/policy` |
| Full payment history | `http://127.0.0.1:28780/history/HEIGHT` |
| Executor HTTP | `127.0.0.1:28790`–`28793` |
| Authenticated witness TCP | `127.0.0.1:28800`–`28803` |
| Validator HTTP | `http://127.0.0.1:28890`–`28893` |
| Complete finalized record | `http://127.0.0.1:28890/record/HEIGHT` |
| Malachite TCP | `127.0.0.1:28900`–`28903` |

Endpoints are inside the machine running the devnet; no public URL is created.

## Native finality and data availability

`Action::PrivatePaymentCertificateV1` is action tag 240, version 1, compiled only
with `insecure-devnet-input`. Its bounded payload contains the **complete** M23
witnessed execution certificate, including executor approvals and witness
confirmations. There is no naming transaction or hash-only finalization.

Each validator loads the pinned public witness/committee policy via
`YAMI_NATIVE_POLICY`. The policy identity and current private-state head enter
the authenticated application state root. Every proposal, synced block and
archive replay executes the same validation: three distinct approved executor
signatures over the identical committee, chain, sequence, previous state,
transaction commitment and resulting state; matching signed witness confirmations
under the pinned authorities; and exact continuity with the accepted private
head. The insecure native profile requires exactly one native action per block.
A feature-enabled binary without a pinned policy rejects the action.

Malachite finalizes the normal block using its existing validator commit
certificate. The gateway independently verifies that commit and the exact native
payload from at least three durable validator archives before committing private
executor snapshots or reporting success. Complete block records, not hashes,
are served through existing Malachite synchronization and HTTP record endpoints.
Gateway history also retains full certificates and finalized records. HTTP data
is untrusted; consumers must verify its certificates and execute its history.

The public block discloses commitments, executor identities, witness metadata,
sequence and timing, but no wallet transfer plaintext or private balances.
**Executors remain the financial authority**; validators do not prove the private
arithmetic. The M21 quorum safety assumption, independently controlled executor
trust domains and M23 external durable witness requirements still apply. Three
compromised executors can authorize false financial state. Witness compromise
can permit rollback/equivocation by its associated executor; sufficient combined
compromise can invalidate the quorum intersection assumption. Co-located services
are software protocol tests, not independent trust domains. There is no financial
safety or zero-trust custody claim.

The production/default action decoder rejects tag 240. Production financial
verification still rejects all 65,536 proof versions. Production wire encodings,
consensus thresholds and M16 are preserved. The genuine TDX/DCAP wallet path and
fail-closed activation gate remain separate; this software profile supplies no
hardware evidence. Software identities or HTTP assertions never activate TDX.

## Persistence and recovery

Default data is `work/yamichain-native-devnet/`; set `YAMI_DEVNET_DIR` before the
first start to choose another location. Preserve the entire directory, including
witness databases/WALs, executor encrypted snapshots and keys, public policy,
validator journals/signing WALs, complete finalized records, receipts, state and
pending requests. Wallets are separate. Never selectively rewind signing stores.
Logs and PID files live alongside the state. A pending full certificate is fsynced
before submission; retry/restart delivers the same action and idempotent executor
commit. Lagging executors recover with authenticated certificates from a peer.
Validators recover by replaying the full native history; compact/pruned snapshots
are deliberately unsupported in this profile so certificate data cannot vanish.

The previous naming-anchor devnet remains untouched in `work/yamichain-devnet/`.
This is a separate, policy-bound native genesis; old test balances are not silently
imported or treated as native finality. Stop the old deployment before starting
this one on the same ports. Default restarts never reset or re-fund native state.

## Private multi-machine lab

Endpoint lists are configurable with `YAMI_DEVNET_CONFIG=/absolute/network.json`.
Copy and edit [network.example.json](network.example.json). The same four pinned
positions must agree everywhere. Validator P2P addresses must be numeric private
or loopback IPv4; public fixture-key networking is refused. The default one-command
`start` supervises services locally; for remote machines run individual roles:

```sh
export YAMI_DEVNET_CONFIG=/srv/yami/network.json
export YAMI_DEVNET_DIR=/srv/yami/data
# On the provisioning machine, after building binaries:
./scripts/yamichain-devnet bootstrap
# On each assigned host (replace 0 with that host's pinned index):
./scripts/yamichain-devnet witness 0
./scripts/yamichain-devnet executor 0
./scripts/yamichain-devnet validator 0
# On the gateway host, after witnesses/executors/validators are reachable:
./scripts/yamichain-devnet gateway
```

Build on each host with:

```sh
cargo build --release -p yami-validator --features insecure-devnet-input
cargo build --release --manifest-path experiments/yamichain-tee-m20/Cargo.toml \
  --bin devnet-bridge --bin witness-node --bin witnessed-worker
```

Provision `policy.json` to every host, `allocations.json` to executor hosts,
`executor-I.json` only to its executor, and `witness-I.json` only to its witness.
Provision `state.json` and the finite treasury wallet to the gateway/operator.
Use the same absolute data path on all hosts, or update the snapshot/database
paths in their individual configs. Never distribute every node's secret config
as a shared bundle. Start witnesses before executors; the executor role creates
its encrypted genesis snapshot on first start. Keep existing stores on restart.
Roles run in the foreground and can be managed by your lab's service manager.
The example co-locates roles for convenience and does not establish independence.

A remote CLI can use `YAMI_DEVNET_URL=http://10.77.0.10:28780`; wallet operations
pin the public policy locally on first use and reject later changes. Provision the
public policy out of band before first use if identity authenticity matters.
HTTP is unauthenticated, with no TLS, rate limiting or access controls. Run only
on a trusted disposable private lab network. Requests remain end-to-end encrypted
to executors, but malicious infrastructure can deny service and observe metadata.
A separate-machine deployment has not been exercised in this environment.

## Reproduce the smoke check

```sh
./scripts/yamichain-devnet-smoke
cargo test -p yamichain-devnet-cert -p yami-chain-privacy
cargo test -p yami-chain --test native_devnet
cargo test -p yami-chain --features insecure-devnet-input --test native_devnet
cargo clippy -p yami-validator -p yamichain-devnet-cert -p yami-chain \
  --all-targets --features insecure-devnet-input -- -D warnings
```

The smoke creates fresh wallets, funds 1000, sends 100 without recipient
participation, checks 900/100 and unchanged validator PIDs across payments,
replays complete authenticated records, rejects altered approvals/witnesses,
committee/sequence/state changes and replay through native `State` execution,
then actually stops/restarts and checks balances and identical finalized records
at all four validators. It leaves the network running. The checked-in
[smoke.json](smoke.json) records the executed result without private keys.

See [the separate live PoS devnet](pos.md) for stake-selected Malachite committees,
candidate synchronization, deterministic rewards and evidence-based slashing.

Remote HTTP service endpoints now require [authenticated TLS RPC](rpc.md);
legacy plaintext compatibility is limited to loopback.
