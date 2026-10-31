# Yami Games 0.2

Prism Breakout is an original Rust game running entirely through the reusable
WebAssembly runtime. No browser-side Breakout logic is used.

From the repository root, with Rust 1.90:

```sh
cargo run --release -p yami-browser -- --game games/breakout/game.yami-game.json
```

Or click **Open local game** in the browser sidebar and enter the absolute path
to `games/breakout/game.yami-game.json`. This opens a separate game tab and keeps
your document tabs. A checked-in binary module is provided; players do not need
the Wasm compilation target. Hosted games use an explicit **Play game** action
on authenticated YamiNet pages; ordinary navigation never executes code. See
[hosted-game publishing and demonstration](../docs/games-0.2.md).

Use **Space** to start, then **Space** again to launch each ball. Move with
**A/D** or **Left/Right**. **P** or **Escape** pauses/resumes. **R** resets to the
start screen. Games launch in a focused arcade layout with Pause/Resume, Restart,
Stop and **Browser** in a compact toolbar. Browser restores the normal sidebar
and pauses the game; **Focus game** returns to the dedicated surface. Resume
explicitly after changing layouts. **Tab** pauses and focuses toolbar controls;
**Enter** activates the focused control. **F3** toggles local performance counters.
Switching tabs, opening a browser panel or losing focus pauses the game; resume
explicitly. Close a game tab normally, including with Ctrl+W. Stop drops the VM;
Restart after stopping explicitly reloads the local package or re-fetches the
hosted game from its pinned publisher.

## Rebuilding the included game

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path games/breakout/Cargo.toml
cp games/breakout/target/wasm32-unknown-unknown/release/yami_breakout.wasm games/breakout/breakout.wasm
```

The guest is an independent workspace. `yami-game-api` contains the Rust drawing
and input helpers; `breakout/src/lib.rs` demonstrates all five required exports.
It uses the Rust standard library compiled for `wasm32-unknown-unknown`, without
WASI. Its small FFI wrappers call only the four versioned host drawing functions.
The host workspace continues to forbid unsafe Rust in its own code.

See [the ABI, limits, architecture and validation report](../docs/games-0.1.md).

See [the 0.1.1 visual overhaul and native validation](../docs/games-0.1.1.md)
for display-resolution text rendering, screenshots and current checks.
