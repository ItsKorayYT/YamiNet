# YamiNet Games 0.4 — Milestone 1

**Status:** Milestone 1 implementation and automated validation are complete.
Milestone 2 (Yami Arcade) and Milestone 3 (showcase game) have not been started.
Work is on `feature/yami-games-04`, based on `origin/main` at `bfdaab7`.

This milestone extends the existing Wasmi worker, ABI 1/ABI 2 runtime, native tabs
and software renderer. It does not replace the engine and does not add a guest
filesystem, network, OS or device capability.

## SDK helpers

The `yami-game-api` guest SDK remains `no_std` and allocation-free. It is now a
workspace member so its host unit tests run in the normal workspace test suite.

| Module | Purpose |
| --- | --- |
| `anim` | `Clip`, `Animator`, `AnimationState` and `Facing`; loop/one-shot playback, state selection, overflow-safe elapsed time |
| `sheet` | checked sprite-sheet grids, margins/spacing, source rectangles and scaled drawing over the existing ABI 2 `sprite` import |
| `tilemap` | borrowed tile maps, empty-tile handling, camera culling, visible-tile iteration, command-budgeted drawing and solid/point/rect queries |
| `collision` | `Rect`, `Circle`, half-open overlap tests, penetration/separation, closest-point circle/AABB and movement smoothing |
| `motion` | kinematic `Body`, gravity, jump gating, world clamping and axis-separated tile collision with sub-tile steps |
| `ui` | measured ASCII/HUD text alignment using the bundled Open Sans advances, shadows, panels, bars, pips, badges and allocation-free counters/clocks |
| `pad` | digital axis/direction views and edge helpers over the shared input mask |

The helpers emit only the existing ABI 1/ABI 2 commands. No host import or ABI
version was added; ABI 1 and ABI 2 guests remain compatible. `WIDTH`, `HEIGHT`
and `MAX_COMMANDS` are available from the SDK root, alongside the existing input
bits.

Gamepad mapping is deliberately SDK-side and browser-side. A game still receives
the same held/pressed/released mask and does not learn whether a keyboard, mouse
or controller produced it.

## Native controller support

A new internal crate, `yami-gamepad`, maps up to four physical pads to the
existing digital bits:

- **Windows:** XInput slots 0–3. The only unsafe code in the crate is one
  documented `XInputGetState` call in `src/windows.rs`; it does not expose a
  pointer or device handle outside that module.
- **Linux:** non-blocking `/dev/input/js*` reads through safe `std` file APIs.
  No privileged ioctl, `libudev` build dependency or extra native library is
  required. Partial 8-byte events are carried between polls.
- **Other platforms:** silent no-op backend. Keyboard/mouse remains complete.

The left stick uses a 12,000 enter / 8,000 exit dead zone, so a direction does
not chatter near the threshold. D-pad buttons work independently. `A/X` and the
right trigger map to `ACTION`; `B/Y`, shoulders and the left trigger map to
`SECONDARY`; Start maps to `START`; Back maps to `RESET`.

Input ownership is the same as keyboard input:

- Pause, focus loss, occlusion, tab changes, browser modals and all browser
  actions clear held pad bits.
- A pad that disappears releases only its own bits; other pads and keyboard keys
  keep working.
- After pause/focus loss, a button held through the transition is latched until
  it is physically released. Resuming cannot replay a held direction or action.
- Controller polling happens only on game-frame wakeups; there is no input
  thread and no unbounded queue.

The browser footer shows `Controller ready` while a pad is connected. F3
diagnostics also include `pad`.

## Rendering work

Profiling was done with release builds on the software renderer, not by
assumption. The following changes were kept because they measure faster or
remove unnecessary work:

1. **Opaque surface blit.** A game frame whose first `clear` has alpha 255 is
   opaque: every validated command uses source-over, so the isolated game
   surface can be copied with `BlendMode::Source` instead of per-pixel
   source-over blending. The fallback for a translucent clear is unchanged.
2. **Static-prefix cache.** The renderer computes the longest command prefix
   shared with the previous frame and caches its rasterized pixels. Steady
   Breakout frames share 294 of 299 commands, so later frames copy the cached
   surface and paint only the moving suffix. Rebuilds happen after a changed
   early command (for example a score label), and cache memory is capped at
   16 MiB. The cache is dropped on tab change, resize, stop and failure.
3. **Full-window solid fill.** A radius-0 solid surface covering the whole
   pixmap now uses `Pixmap::fill` instead of building and scanning a rounded
   path. This affects the immersive game background and page fills.
4. **Presentation bookkeeping.** The window title is only set when the tab title
   changes; softbuffer `resize` is only called when the framebuffer size changes;
   RGBA→0RGB conversion iterates fixed chunks; and the runtime skips its stable
   layer sort when a frame is already ordered.

Correctness is covered by pixel-identity tests that compare cached-prefix output
with a full repaint, including resized surfaces, changed assets and short
prefixes. Existing DPI/clipping/alpha tests continue to pass.

## Validation

Run on Windows 11 (build 26300), AMD Ryzen 7 6800H, 16 logical CPUs, 15 GiB RAM,
Rust 1.90.0, release profile, software rendering. Temporary directories were
placed under the repository's ignored `work/` directory.

```sh
cargo fmt --all -- --check
cargo fmt --manifest-path games/yami-game-api/Cargo.toml -- --check
cargo clippy --release -p yami-gamepad -p yami-games -p yami-browser \
    -p yami-game-api --all-targets --locked -- -D warnings
cargo test --release --workspace --locked
cargo build --release --workspace --locked
cargo build --manifest-path games/yami-game-api/Cargo.toml \
    --target wasm32-unknown-unknown --release
cargo build --manifest-path games/lantern-drift/Cargo.toml \
    --release --target wasm32-unknown-unknown --locked
cargo check -p yami-gamepad --target x86_64-unknown-linux-gnu --locked
cargo clippy -p yami-gamepad --target x86_64-unknown-linux-gnu --locked \
    -- -D warnings
```

The workspace suite passed **470 tests, 0 failures, 10 ignored**. The ignored
entries are the pre-existing manual visual/network checks plus the two new manual
rendering benchmarks. The final local run used `--test-threads=1` because this
Windows host intermittently denies writes to temporary fixtures when many
temp-file tests run in parallel; the same suite also completed in parallel before
that transient host condition, and CI runs the default parallel harness. The SDK's 20 host unit tests cover loop/one-shot animation,
checked sheet geometry, tile culling and edge solidity, non-finite/huge collision
inputs, jump/landing/substep behaviour, text measurement, narrow formatting
buffers and controlled gamepad edges. The gamepad crate's seven tests use an
injected backend for mapping, multi-pad disconnects, suspend/release latching,
empty input and the four-slot bound. Browser tests cover controller release
through pause/browser actions and pixel-identical prefix-cache rendering. The
runtime's existing malformed-Wasm, fuel, memory, command, text and trap tests are
unchanged and still pass.

The Linux `yami-gamepad` backend is compile- and Clippy-checked with the Linux
target from this Windows host; that is not a substitute for a physical Linux
gamepad test.

A debug-profile workspace build could not be completed in this Windows session
because the existing `ring` dependency's MSVC C build failed to start `cl.exe`
(`D8050`, unrelated to this change); release builds and tests succeeded. CI
continues to run the normal debug `cargo clippy`/`cargo test` workspace checks.

## Measurements

Composition benchmark: 180 mixed Breakout frames at 1440×900, browser chrome and
game surface, excluding softbuffer presentation. Three release runs:

| Run | Full repaint avg / p50 / p95 | Prefix-cache avg / p50 / p95 |
| --- | --- | --- |
| 1 | 4.110 / 4.099 / 5.186 ms | 3.925 / 3.804 / 4.740 ms |
| 2 | 4.153 / 3.915 / 5.215 ms | 3.798 / 3.654 / 4.901 ms |
| 3 | 4.042 / 3.939 / 4.972 ms | 3.900 / 3.765 / 4.828 ms |
| **Average** | **4.10 / 3.98 / 5.12 ms** | **3.87 / 3.74 / 4.82 ms** |

The earlier unmodified `main` baseline on the same machine was **4.895 ms
average / 4.828 ms p50 / 5.960 ms p95** for the same 180-frame scene. The final
cached composition is about **21% lower average** than that baseline; the full
repaint path is about **16% lower** from the opaque blit and fill changes. The
prefix cache contributes roughly another 5% across these runs.

Isolated guest-surface rasterization at 1051×788 (the actual focused surface at
1440×900):

| Pass | Average | p50 | p95 |
| --- | --- | --- | --- |
| Full 299-command frame | ~0.36 ms | ~0.35 ms | ~0.43 ms |
| Cached moving suffix | ~0.35 ms | ~0.20 ms | ~1.2 ms |

The cached average is dominated by occasional rebuild frames after early HUD
changes; its median is about 43% lower. The prefix cache is therefore a
latency/median improvement, not a guaranteed win on every frame.

Headless VM benchmark, 10,000 frames (unchanged sandbox and fuel limits):

| Game | Load | Mean frame | p95 | Max commands |
| --- | --- | --- | --- | --- |
| Prism Breakout (ABI 1) | 2.235 ms | 0.0371 ms | 0.0478 ms | 299 |
| Lantern Drift (ABI 2) | 5.254 ms | 0.0140 ms | 0.0166 ms | 51 |

These are local diagnostics, not hardware-independent guarantees. Software
rasterization, window size, text shaping and CPU contention remain limits.

## Manual follow-up

No physical controller was attached in this environment. The XInput path was
compiled and its mapping is covered by injected-state tests, but real Xbox/
PlayStation pads, hot-plug timing, Linux `js` permissions and controller
vibration/rumble (not exposed) need physical validation on Windows and Linux.
Physical high-DPI displays, audible audio output and non-Windows compositors also
remain manual checks. No telemetry or automatic reporting was added.
