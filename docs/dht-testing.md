# Reproducing decentralized lookup

The prerequisite public IPv4 TLS transport and persistent manual peers already
exist; this milestone reuses them. The tests below exercise `yami-dht`, not a
new transport or a centralized directory. Read the
[protocol/security limitations](specs/v0.1/dht.md) before enabling the feature.

## Automated validation and its limits

```powershell
cargo test -p yami-dht
cargo test -p yami-browser --test native_dht --locked -- --nocapture
cargo test -p yami-node --test dht -- --nocapture
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

The three-node integration test creates three persistent identities, starts
three real node processes with encrypted loopback listeners, and configures
only B as A/C's manual bootstrap. A publishes its signed record; B stores and
verifies it; C uses A's **ID**, queries B, resolves the endpoint, authenticates A
over TLS 1.3 and fetches Alice's `home` document. Other lookups demonstrate all
three peers can locate one another. The initial bounded STORE/FIND pass teaches
C signed neighbors from B so C can itself bootstrap a later lookup after B stops.
A validly signed false endpoint fails the
actual TLS identity check. Restarting A advances its persisted sequence; B
rejects the older signed record. C can still fetch A after B stops, using the
validated location learned earlier. All processes and temporary test state
are cleaned up after the test.

On 2026-10-29, formatting, all-target Clippy with warnings denied, the locked
workspace tests and workspace build passed on Windows, including this actual
three-process loopback simulation. This does not extend the validation scope
to public internet connections or a security review.

Crate tests additionally cover real multi-hop iteration through an intermediate
peer, refreshing a dead cached endpoint with a higher-sequence record, invalid
signatures, endpoint policies, expiration, rollback after expiry/restart,
equivocation, full cache/bucket limits, route expiry, malformed/oversized/truncated
frames, versions, padding, duplicate records, nonce/type mismatches, partial
responses, atomic batch rejection, writer locks and monotonic self sequences.
Injected connection failures verify the exact 16-query cap; real unavailable
loopback sockets verify failure handling and deadlines. A timeout may end a
lookup before it spends all queries. Existing YNP/TLS/browser/LAN tests remain.

**These are one-computer simulations, not internet validation.** No three-machine
public DHT deployment, VPS/provider-firewall traversal, public packet capture,
Linux execution, sustained adversarial load or independent security review
was performed for this milestone. The test endpoint policy is explicitly
loopback-only. No success here proves internet reachability or production safety.

If an existing executable is in use, append `--target-dir target/dht-validation`
to Cargo commands. The implementation was validated in that separate directory
on Windows. Loopback simulation needs no public-network exposure or port forward.

## Manual three-node Windows session

From the repository root, first build and prepare isolated ignored test state:

```powershell
cargo build -p yami-node --target-dir target/dht-validation
$node = Join-Path $PWD "target\dht-validation\debug\yami-node.exe"
$demo = Join-Path $PWD "target\dht-demo"
New-Item -ItemType Directory -Force -Path $demo | Out-Null
& $node identity new --dir "$demo\a-identity"
& $node identity new --dir "$demo\b-identity"
& $node identity new --dir "$demo\c-identity"
$aId = (Get-Content "$demo\a-identity\node.id").Trim()
$bId = (Get-Content "$demo\b-identity\node.id").Trim()
$cId = (Get-Content "$demo\c-identity\node.id").Trim()
& $node dht bootstrap $bId 127.0.0.1:17442 --local --dir "$demo\a"
& $node dht bootstrap $bId 127.0.0.1:17442 --local --dir "$demo\c"
```

Run this preparation once. Identity creation refuses overwriting existing keys;
bootstrap add refuses replacing an existing pin. To repeat with existing state,
reuse the public `node.id` values and configured peers instead of recreating
them. Do not share or commit `node.key`; the ignored `target` directory contains
disposable private test identities. Inspect their Windows ACLs if needed.

Open three terminals in the repository root. Start B first:

```powershell
.\target\dht-validation\debug\yami-node.exe serve-dir 127.0.0.1:17442 sites/alice --identity .\target\dht-demo\b-identity --tls --dht --dht-local --dht-dir .\target\dht-demo\b
```

Then A:

```powershell
.\target\dht-validation\debug\yami-node.exe serve-dir 127.0.0.1:17441 sites/alice --identity .\target\dht-demo\a-identity --tls --dht --dht-local --dht-dir .\target\dht-demo\a
```

Then C:

```powershell
.\target\dht-validation\debug\yami-node.exe serve-dir 127.0.0.1:17443 sites/alice --identity .\target\dht-demo\c-identity --tls --dht --dht-local --dht-dir .\target\dht-demo\c
```

Wait for A and C to report `DHT startup publication complete: 1 authenticated
bootstrap(s)`. B has no mandatory upstream and normally reports zero. Each
node also prints its reserved signed sequence and expiration. If a port is
already occupied, choose three free loopback ports and update B's bootstrap
configuration deliberately. No wildcard/public binding is needed.

Back in the original terminal:

```powershell
& $node dht peers --local --dir "$demo\b"
& $node dht lookup $aId --local --dir "$demo\c" --identity "$demo\c-identity"
& $node dht fetch $aId home --local --dir "$demo\c" --identity "$demo\c-identity"
& $node dht lookup $cId --local --dir "$demo\a" --identity "$demo\a-identity"
```

C's lookup/fetch commands contain A's full node ID and **no A endpoint**. B's
signed-record cache supplies the location. The fetch prints Alice's YLang
source and `Verified node ... over TLS 1.3 after distributed lookup`. Its
output escapes control characters and executes no remote content. A successful
lookup alone prints a signed but unconfirmed `ynps://.../home` location.

To browse, configure B in a separate browser peer directory and select
`YAMINET_BROWSER_DHT_NETWORK=local`, then enter `yami://<A-full-ID>/home`
directly in the browser. The browser uses `yami-dht` itself, authenticates A
over TLS 1.3 and renders normally without A's IP in its address. The older
returned `ynps://` URL still works. See [native browser setup and testing](native-addressing.md)
for exact commands and identity-preserving links. `/home` is a convenience
default, not a homepage field in the DHT record.

Stop B and repeat C's fetch after C learned A: it should query/authenticate A
directly without B. Stop A: lookup may still return its unexpired signed
location, while fetch must fail. Restart A with the same identity/state and
observe the larger sequence. Never interpret a signed record as presence or
availability. Tests also inject a validly signed endpoint lie and confirm that
the wrong TLS identity is refused, with no YLang document returned.

Stop the three servers with Ctrl-C. No firewall rules were installed. Retain
state for sequence continuity, or deliberately discard only your disposable
demo after all processes stop; resetting state removes rollback history.

## Configuring ordinary bootstrap peers

`dht bootstrap` uses the existing manual `peers.tsv` store and its locking,
bounds and duplicate-ID policy. `peer list` and `dht peers` inspect the same
configured pins. Use `peer remove <id> --dir <directory>` before changing one.
Compatible configured manual peers are used as bootstraps; entries outside
the chosen public/loopback address policy are not queried. A normal TLS YNP
node without DHT opt-in simply fails an RPC; it does not become a naming server.

Default configuration uses the existing per-user peer directory or
`YAMINET_PEER_DIR`; `--dir` selects CLI configuration and `--dht-dir` selects
serving configuration/state. CLI lookup uses the existing default identity or
`--identity`. Keep one state/identity pair per logical node. Bootstrap settings
are sampled at startup, and publication is a single bounded initial pass;
changing the file does not launch background crawling. Retry explicitly or
restart if a bootstrap was unavailable. There are no mandatory built-in peers.

Server and CLI client snapshots have separate locks. Incoming STORE updates
the running server; CLI lookup updates the client cache and can read newer
server publications. CLI-learned records are not automatically copied into an
already-running server. Both maintain bounded signed sequence history. At 256
known IDs, new IDs fail closed rather than evicting rollback protection; inspect
the state and security tradeoff before any deliberate reset.

## Owned public-node experiment (not performed)

Use three reachable **owned** public IPv4 nodes or manually forwarded test
endpoints. Follow [internet connectivity testing](internet-testing.md) first
to establish direct pinned TLS and appropriate narrow firewall rules. Then
enable `--tls --public --dht` on each, with separate persistent identities and
state directories. Do not use `--dht-local` or `--local` for this experiment.

```text
yami-node dht bootstrap <B-ID> <B-PUBLIC-IPv4>:7443 --dir <A-state>
yami-node serve-dir <A-local-IPv4>:7443 <test-site> --identity <A-identity> --tls --public --dht --dht-dir <A-state> --dht-endpoint <A-PUBLIC-IPv4>:7443
yami-node dht lookup <A-ID> --dir <C-state> --identity <C-identity>
yami-node dht fetch <A-ID> home --dir <C-state> --identity <C-identity>
```

Replace all placeholders, start B before publishing A/C, and configure B as a
manual bootstrap for C as well. `--dht-endpoint` may appear up to four times;
it is necessary when the specific local bind address/port differs from the
known external endpoint, for example behind a manual forward. Otherwise the
specific public bind address is used. The same endpoint must serve both DHT
and YNP over pinned TLS. There is no automatic NAT traversal or public-IP
discovery. Only the existing TCP service needs an explicitly approved ingress
rule; there is no DHT UDP listener. Protect both OS and provider firewalls.

Record actual endpoint reachability, node IDs, sequence/expiry, wrong-key
failures, refresh/expiry behavior, and captures from independent networks.
Do not substitute these loopback results for that validation. Sybil/eclipse,
metadata collection, omission/censorship, cache filling and production DHT
maintenance remain unsolved. No anonymity or production-security claim is made.
