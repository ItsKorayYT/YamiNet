# Authenticated native-chain RPC

Remote gateway, executor and validator HTTP endpoints now require HTTPS. The
existing loopback-only HTTP profiles remain supported without changing their
ledgers. TLS authenticates the configured host through an explicitly supplied
CA; service-specific bearer credentials authenticate callers. RPC credentials
never authorize spending, consensus membership or private balance queries: the
existing signed/encrypted operations, witness checks and Malachite certificates
remain mandatory. No production financial activation is enabled.

Create a separate disposable localhost TLS profile (OpenSSL required):

```sh
./scripts/yamichain-rpc-config --out work/my-tls-network
export YAMI_DEVNET_CONFIG="$PWD/work/my-tls-network/network.json"
export YAMI_DEVNET_DIR="$PWD/work/my-tls-network/data"
./scripts/yamichain-pos-devnet start
./scripts/yami-wallet --network pos create work/alice.secure
./scripts/yami-wallet --network pos status
```

The helper refuses existing output directories, uses distinct ports and data,
and creates short-lived **development-only** certificates. It does not replace
certificates or alter existing networks. Preserve the complete data directory,
including witness databases, executor snapshots and consensus signing WALs.

For multiple machines, provision a CA and distinct server certificate/private
key per host, with SANs matching its configured HTTPS addresses. Configure the
same service inventory and pinned chain policy on each host. Set `rpc.ca_file`,
`rpc.certificate_file`, `rpc.key_file`, and the required `rpc.tokens` paths:
`gateway_file`, `executor_file`, `validator_file`. Token files contain at least
32 printable ASCII characters and must be private regular files (mode 0600);
symlinks are refused. The TLS private key must also be mode 0600. Distribute
credentials out of band, not through URLs, shell arguments or repository files.
Clients reject redirects and unconfigured origins before sending credentials.

Wallet machines require only the CA, gateway token and service inventory; remove
backend token paths and server key/certificate paths from wallet configuration.
Gateway hosts need backend RPC access. Service authentication does not make the
current software executors or their recovery custodians hardware-secure.
Fresh TLS profiles also encrypt witness connections using certificate-verified
TLS while retaining the signed-request/encrypted-snapshot protocol. Legacy
loopback witness profiles remain supported. Non-loopback witness clients and
listeners require TLS; there is no remote plaintext fallback.

`witness_tls.server` contains `certificate_file` and `key_file` paths.
`witness_tls.client` contains the explicitly provisioned public CA certificate
as `certificate_pem` bytes and the expected `server_name`. Trust excludes ambient
system roots and verifies both the certificate chain and DNS/IP identity. The
provisioned name may differ from the numeric dial address. Private key files
must be regular, non-symlinked and private. Existing service configurations must
match the network configuration; changing only the top-level file is refused
instead of silently leaving an old plaintext service running.

Witnesses have eight fixed connection workers, eight queued sockets, a listen
backlog of sixteen and a five-second absolute deadline covering handshake and
framed I/O, including time in the queue. Byte trickling cannot renew that limit.
Frames retain their existing 32 MiB refusal bound; this is not a small worst-case
memory claim. Authority mutations remain serialized and durable before reply;
no handshake or socket I/O holds the authority lock. TLS alone cannot authorize
a witness operation: signed actor, policy, nonce and revision checks still run.

TDX provisioning can supply `witness_tls` in the operator-signed configuration,
with the CA bytes and server name covered by that signature. `tdx-admin` still
requires genuine QVL enrollment verification before configuring an executor.
This transport implementation does not solve recovery-custodian impersonation,
executor replacement/revocation or prove hardware security.

Admission is bounded to eight concurrent connections, a queue of sixteen, five
seconds for TLS handshakes/body reads, 1 MiB request bodies and 16 MiB client
responses. Each source IP has a burst of 120 requests and refills at 60/second;
IP accounting is capped at 1024 entries. Oversized headers, duplicate lengths,
transfer-encoding ambiguity and missing/wrong credentials are rejected before
backend dispatch. Backend stdin/stdout and financial mutations stay serialized.
Authenticated slow or unavailable consensus operations can still delay requests;
these are development service limits, not measured production capacity claims.

Reproduce the executed encrypted payment/offline receive/restart test on a fresh
TLS profile:

```sh
python3 tools/yamichain-rpc-smoke.py work/my-tls-network/network.json work/tls-wallet-check
python3 -m unittest discover -s tools -p 'test_yamichain_rpc.py' -v
cargo test --release --manifest-path experiments/yamichain-tee-m20/Cargo.toml --test witness_tls
```

[rpc-smoke.json](rpc-smoke.json) records native finality, 898/100 private balances,
unauthenticated-call rejection and restart preservation. Public peer discovery,
large financial-history checkpoint import/pruning and hardware-bound executor
replacement remain separate outstanding requirements; this does not claim that
all Phase 1 or production networking is complete.
