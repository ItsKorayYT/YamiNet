# Native devnet supervision and observed resources

The existing startup command now supervises witnesses and executor/validator
wrappers with bounded exponential restart backoff (1..60 seconds, resetting
after a minute of stability). Wrappers also restart failed private workers and
consensus processes. Restart always reuses the same configs, witness databases,
encrypted snapshots, validator journals and signing reservations. Failures never
trigger fresh keys, new funding, journal deletion or a relaxed restore check.

Linux parent-death signals terminate children if a supervisor/wrapper is killed;
the child checks for parent loss before exec as well. This prevents an orphaned
consensus process from retaining a journal lock while a replacement tries to
start. The detached top-level supervisor intentionally survives its launching
CLI. This runner requires Linux for managed service lifetimes.

Private-worker pipe reads and writes have a 15-second deadline and a 16 MiB
response bound. A stuck worker is terminated and reopened through the existing
authenticated witness recovery path. Financial operations remain serialized.
A background recovery pass retries the original pending certificate and catches
up reachable lagging executors using existing authenticated recovery bundles.
Process-health metadata grants no signing or recovery authority.
Executor catch-up uses [resumable authenticated pages](recovery-pages.md), allowing
gaps beyond 256 transitions while retaining the complete certified prefix and
the original snapshot until verification reaches its target.

Authenticated gateway GET `/services` reports wrapper PIDs, liveness and restart
counts. GET `/recovery` reports the last recovery-pass time and whether at least
three executors match the finalized head. Process liveness alone does not imply
committee membership, finality or complete synchronization; consult `/status`,
`/committee` and verify certificates. These health files contain no wallet keys,
credentials or private financial portfolios. Recovery and lifecycle retries are
bounded; permanently invalid credentials or missing safety state require repair.

```sh
python3 tools/yamichain-service-smoke.py work/my-tls-network/network.json
python3 tools/yamichain-runtime-sample.py --pid "$(cat work/my-tls-network/data/supervisor.pid)" --seconds 10
```

The [executed restart test](service-smoke.json) killed a witness, an executor
wrapper, a validator wrapper and a private worker. All recovered with the same
finalized head and original stores. [Resource observations](runtime-sample.json)
cover five seconds of an idle four-validator TLS development network with 21
processes: approximately 188 MiB peak summed PSS and 0.67 observed CPU seconds.
Summed RSS overcounts shared pages. This is neither a throughput benchmark nor
a claim that a production resource target has been met.

For the unified disposable demo, including live staking/join/exit/slashing,
encrypted Alice/Bob wallets, offline receipt, fees, independent history
verification and interruption/restart recovery over authenticated TLS:

```sh
python3 tools/yamichain-unified-smoke.py --out work/new-unified-network --port-offset 2000
```

The output directory must be new. Peers use the configured private bootstrap
inventory; public peer discovery and independent multi-machine operator trials
remain outstanding. No public deployment, hardware-attestation fallback or
production financial activation is performed.

[Unified execution evidence](unified-smoke.json) records the complete TLS run: live PoS through 35 blocks, followed by encrypted payment and independent finality verification at height 37.

The subsequent [witness TLS run](witness-tls-unified.json) verifies the same flow
with encrypted witness transport through height 38. Its
[service crash test](witness-tls-service.json) retained that head and all safety
stores. A separate [idle resource sample](witness-tls-resources.json) observed 23
processes (including five validator processes), approximately 220 MiB peak summed
PSS and 0.84 CPU seconds over 5.16 seconds. This small co-located network does not
establish a production or worst-case resource bound. The
[validation record](witness-tls-validation.json) includes the latest linked TDX
binary's refusal to start without genuine hardware; no guest image or hardware
boot was validated by this run.
