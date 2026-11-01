# Rust game SDK 0.2 / game ABI 2

The dependency-free `no_std` SDK keeps the original drawing helpers and adds an
opt-in `v2` module. The host accepts ABI 1 and ABI 2; existing ABI 1 Wasm does not
need rebuilding. `export_game!` supplies all five exports for a Rust `std` guest.
Never target WASI: use `wasm32-unknown-unknown`.

```rust,ignore
use yami_game_api::{clear, rgb, START};
use yami_game_api::v2::{Input, Sprite, Channel, MOUSE_LEFT};
#[derive(Default)]
struct Game { elapsed: u64 }
impl Game {
    fn frame(&mut self, input: Input, dt_us: i32) {
        self.elapsed += dt_us as u64;
        clear(rgb(9, 23, 31));
        let (x, y) = input.pointer().unwrap_or((320, 240));
        Sprite { asset: 0, source: [0, 0, 32, 32], size: [64, 64], layer: 2 }
            .animated(self.elapsed, 140_000, 4)
            .draw(x - 32, y - 32);
        if input.pressed(START | MOUSE_LEFT) {
            Channel(1).play(1, 500, false);
        }
        if input.released(MOUSE_LEFT) { Channel(1).stop(); }
    }
}
yami_game_api::export_game!(Game);
```

Declare PNG ID 0 and WAV ID 1 in the version 2 local manifest. IDs are shared
across resource types and range from 0 through 15. Textures contain all animation
frames: `animated` selects horizontal frames, leaving the sheet decoded once.
Destination sizes are independent of source rectangles. Sprites can extend beyond
the canvas and are clipped; source rectangles must remain inside the PNG.
Layers -128..127 are stable, back to front; primitives occupy layer 0. Call clear
before drawing anything else. Rendering and audio commands are allowed only in
`frame`; initialization/reset should only reset guest state.

`Input` has held, pressed and released bits, including quick taps between ticks.
Pointer coordinates use the fixed 640×480 canvas; `pointer()` is `None` outside
it. Aliases are A/Left, D/Right, W/Up, S/Down, Space/Enter, R, Z/J and X/K.
P/Escape, Tab, browser shortcuts and F3 remain browser-owned. Resetting, pausing,
changing tabs or losing focus clears held inputs. Resume requires new presses.

Eight audio channels replace their previous sound on `play`. Channel 0 is useful
for a looping background; others for effects. Use `.volume(0..=1000)`, `.stop()`
and `v2::mute(bool)`. WAV files must be signed PCM16, mono/stereo, 8–48 kHz,
up to 10 seconds; loop short background tracks. Audio output may be silent on
machines without an available device. Guest logic must not depend on playback.
The worker pauses channels with the game and drops them on stop, failure or close.

Use [Lantern Drift](../lantern-drift/src/lib.rs) for a complete game, its
[manifest](../lantern-drift/game.yami-game.json) for resource declarations and
[Games 0.3](../../docs/games-0.3.md) for publishing. The guest Cargo.toml needs a
`cdylib` library, path dependency on this crate, and release `panic = "abort"`.
Build with `cargo build --release --target wasm32-unknown-unknown`. Run the
publisher helper on the resulting package; players only choose Play in YamiNet.
