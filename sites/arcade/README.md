# Hosted arcade demo

Prepare this site as its publisher, then serve it with a TLS-enabled YamiNet node:

```sh
cargo run --locked -p yami-games --example publish -- games/breakout/game.yami-game.json sites/arcade prism
yami-node serve-dir 127.0.0.1:7443 sites/arcade --identity /absolute/test-identity --tls --dht --dht-local --dht-dir /absolute/host-state
```

Create the test identity with `yami-node identity new --dir /absolute/test-identity`.
Configure the browser's loopback bootstrap with that identity and endpoint, open
`yami://THE-PRINTED-NODE-ID/home`, and choose **Play game: Prism Breakout**.
The preparation command refuses to replace existing game files. For a deliberate
publisher update, remove the previous `games/prism.json` and `games/prism.wasm`
first. These publisher files are not an installation on player machines.

The automated two-process native-window reproduction is documented in
[Games 0.2](../../docs/games-0.2.md). Generated publications belong in a staging
copy; this example checks in only its inert page and stylesheet.
