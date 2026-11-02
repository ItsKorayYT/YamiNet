# Rust game SDK — ABI 1 and 2 helpers

The dependency-free `no_std` SDK keeps the original drawing helpers and adds an
opt-in `v2` module plus allocation-free helpers for animation, sprite sheets,
tilemaps, collision, movement, HUD text and controller input. The host accepts
ABI 1 and ABI 2; existing ABI 1 Wasm does not need rebuilding. `export_game!`
supplies all five exports for a Rust `std` guest. Never target WASI: use
`wasm32-unknown-unknown`.

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

## Animation and sheets

`anim::Clip` describes one horizontal strip; `Animator` advances by simulation
microseconds and never allocates. `AnimationState` selects between a fixed table
of clips without panicking on an out-of-range state byte.

```rust,ignore
use yami_game_api::anim::{AnimationState, Clip};
use yami_game_api::sheet::Sheet;

let sheet = Sheet::new(0, 16, 24, 6, 4);       // asset, cell, columns, rows
let clips = [
    Clip::new(0, 0, 4, 120_000, true),         // idle, 4 frames, 120 ms
    Clip::new(1, 6, 6, 80_000, true),          // run
    Clip::new(2, 12, 5, 90_000, false),        // hurt, one-shot
];
let mut animation = AnimationState::new(clips);
animation.set(1);
animation.advance(dt_us.max(0) as u32);
if let Some(frame) = animation.frame_index() {
    sheet.draw(frame, x, y, 2);
}
```

`Sheet::source` is checked arithmetic: invalid cells, zero dimensions and sheets
outside the host's 2048-pixel texture bound return `None` instead of emitting an
invalid command. `draw`/`draw_scaled`/`draw_frame` are thin adapters over ABI 2
`Sprite`; call them only from `update`.

## Tilemaps

```rust,ignore
use yami_game_api::sheet::Sheet;
use yami_game_api::tilemap::TileMap;

static TILES: [u16; 40 * 15] = [/* 0 = empty, n = sheet frame n-1 */];
let map = TileMap::new(&TILES, 40, 15, 16, 16);
map.draw(Sheet::new(0, 16, 16, 16, 16), camera_x, camera_y,
         yami_game_api::WIDTH, yami_game_api::HEIGHT, 0, 400);
```

`TileMap` is a borrowed slice, so levels can live in static memory or on the
stack. `visible` culls to a camera rectangle and skips empty tiles;
`draw` stops at the caller's command budget (pass the host's remaining budget,
not the full 512). `tile`, `point_solid` and `rect_solid` are bounds-checked and
treat the world edge as solid. Tile `0` is empty; a non-zero value selects
`frame = tile - 1` on the supplied sheet.

## Collision and movement

```rust,ignore
use yami_game_api::collision::{approach, Circle, Rect};
use yami_game_api::motion::{move_and_slide, Body, SolidMap};

let mut player = Body::new(80.0, 40.0, 12.0, 16.0);
player.apply_gravity(dt, 900.0, 500.0);
let movement = move_and_slide(&mut player, vx * dt, player.vy * dt,
                              &SolidMap::new(map, 1));
if movement.on_ground && jump_pressed {
    player.jump(320.0);
}
let hit = Rect::new(player.x, player.y, player.w, player.h)
    .intersects(Rect::new(coin_x, coin_y, 8.0, 8.0));
```

`Rect`/`Circle` expose half-open overlap tests, penetration depth, a shallow-axis
separation vector and closest-point circle/AABB tests. `move_x`, `move_y` and
`move_and_slide` resolve one axis at a time with substeps no larger than half a
tile, so a fast actor cannot tunnel through a wall. `on_ground` is set only by a
downward collision; `jump` only succeeds from the ground. No square roots or
allocations are used, so the helpers build for `no_std` Wasm.

## HUD and text

The host draws one line per `text` command. `ui` measures the bundled Open Sans
advances (including the semibold face used at size 18 and above) so centering and
right alignment stay stable without another host import. Kerning can shift a long
line by a pixel or two.

```rust,ignore
use yami_game_api::ui;
use yami_game_api::rgb;

let mut clock = [0u8; 5];
let mut score = [0u8; 10];
ui::centered("LANTERN DRIFT", 320, 18, 24, rgb(255, 216, 129));
ui::text("SCORE", 24, 52, ui::SMALL, rgb(169, 183, 207));
ui::text(ui::format_u32(score_value, &mut score), 72, 52, ui::SMALL, rgb(234, 244, 255));
ui::text(ui::format_clock(elapsed_seconds, &mut clock), 24, 70, ui::SMALL, rgb(169, 183, 207));
ui::bar(24, 92, 180, 8, hearts, 3, rgb(231, 128, 199), rgb(30, 44, 62), 0);
ui::panel(20, 400, 600, 56, rgb(19, 42, 50), rgb(255, 216, 129));
```

`panel`, `bar`, `pips` and `badge` clamp their geometry and skip commands that
would fall outside the canvas. `format_u32` and `format_clock` write into
caller-owned buffers, so the SDK stays `no_std` and allocation-free.
`ui::shadow`, `ui::right`, `ui::alpha` and `ui::blend` cover the common HUD
treatments. Text colors must keep alpha 255; use rects for translucent panels.

## Controller and digital input

Controllers are not a guest capability. The native browser reads up to four pads
and maps their buttons and left stick to the same `LEFT`/`RIGHT`/`UP`/`DOWN`/
`START`/`ACTION`/`SECONDARY`/`RESET` bits as the keyboard. A bound button must be
released before it can press again after focus loss, pause or resume. Disconnect
releases that pad's bits; other pads and the keyboard keep working.

`pad` provides small views over the shared mask:

```rust,ignore
use yami_game_api::pad;
if pad::confirm(&input) { start_or_restart(); }
let (dx, dy) = pad::direction(&input);
```

Windows uses XInput; Linux reads legacy `/dev/input/js*` devices in non-blocking
mode, so there is no privileged ioctl or extra native library. Other platforms
compile to a silent backend. Controller output is optional: every game must also
be playable with the keyboard and, where offered, the mouse.

## Building

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release --target wasm32-unknown-unknown \
    --manifest-path games/my-game/Cargo.toml
```

The guest package needs `crate-type = ["cdylib"]`, a path dependency on this
crate and release `panic = "abort"`. See [Lantern Drift](../lantern-drift/src/lib.rs)
for a complete ABI 2 game, its [manifest](../lantern-drift/game.yami-game.json)
for resource declarations and [Games 0.3](../../docs/games-0.3.md) for hosted
publishing. Milestone 1 of Games 0.4 is described in
[the SDK and controller report](../../docs/games-0.4-milestone-1.md).
