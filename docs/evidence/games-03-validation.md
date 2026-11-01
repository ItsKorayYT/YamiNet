# Games 0.3 validation — 2026-11-01

Base: latest fetched `origin/main`, `0ac0926`; branch `feature/yami-games-03`.
Debian 13 x86_64 managed container; Rust/Cargo 1.90.0. Linux release binaries were
built and run. Windows GNU cross-target compilation was checked from Linux.
Xvfb was 1600×1000×24, with real native 1440×900 browser windows, XTEST input
through xdotool, and ImageMagick window captures. Temporary player/publisher
profiles and generated node identities were isolated from user state.

## Checks

All required source/build checks passed. The final full workspace run completed
with **450 passed, 0 failed, 9 pre-existing ignored tests**. Native-audio package
tests also passed separately with the audio feature explicitly enabled:

```sh
cargo fmt --all -- --check
cargo fmt --manifest-path games/yami-game-api/Cargo.toml -- --check
cargo fmt --manifest-path games/lantern-drift/Cargo.toml -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
cargo build --locked -p yami-games --release --examples
cargo clippy --manifest-path games/yami-game-api/Cargo.toml --target wasm32-unknown-unknown -- -D warnings
cargo clippy --manifest-path games/lantern-drift/Cargo.toml --target wasm32-unknown-unknown --locked -- -D warnings
cargo build --manifest-path games/breakout/Cargo.toml --release --target wasm32-unknown-unknown --locked
cargo build --manifest-path games/lantern-drift/Cargo.toml --release --target wasm32-unknown-unknown --locked
cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked
```

Both guest builds passed. The committed Lantern Drift Wasm matches its release
build. Prism Breakout's source, manifest and committed Wasm remain unchanged;
only its lockfile follows the SDK package version update. Existing ABI 1 runtime
and native-window gameplay checks passed against the original committed binary.

Coverage includes exact manifest/module ABI agreement, forbidden imports and
fuel/memory/table/stack limits, invalid sprite IDs/source rectangles/scales/layers,
PNG corruption/truncation/dimension and decoded-cache bounds, wrong resource
types, WAV format/rate limits, asset SHA-256 failure, duplicate IDs, foreign asset
keys/URLs, untrusted manifests, local module and sprite symlink confinement,
input press/release and pause clearing, and native channel pause/resume, volume,
mute, stop/reset and unavailable-output behavior, plus stalled-output source
admission and recovery (including reset attempts while stalled). DPI tests check source cropping,
alpha compositing, negative destination clipping and canvas isolation at
1×, 1.25×, 1.5× and 2×.

The ABI 2 hosted integration test uses actual node TLS, publisher pins, YNP,
browser tabs and Play. It validates site check/build resource copying, verifies
six initial transfers (manifest + Wasm + four assets), observes no resource
requests during gameplay, and rejects a corrupted PNG before any frame executes.
Existing tests continue checking plaintext rejection, changed destination pins,
relay + publisher pins, interrupted transfers and cancellation. Actual Wasm
Lantern Drift tests reach both win and lose screens and verify scoring and mute.

## Native gameplay reproduction

With a running X11 display (or under `xvfb-run`) and the tools above:

```sh
python3 scripts/test-games-x11.py --browser target/release/yami-browser --manifest games/lantern-drift/game.yami-game.json --output /tmp/lantern-local
python3 scripts/test-games-x11.py --browser target/release/yami-browser --manifest games/breakout/game.yami-game.json --output /tmp/prism-local
python3 scripts/test-hosted-games-x11.py --browser target/release/yami-browser --node target/release/yami-node --publisher target/release/examples/publish --game lantern --output /tmp/lantern-hosted
```

All three runs passed. The local ABI 2 run exercised mouse steering, pulse,
expanded keyboard input, mute/unmute, stable pause/resume, tab shortcuts,
background pausing, stop/restart, UI local-file launching, 900×600 resizing and
clean process shutdown. The hosted run opened an actual `yami://` page, verified
that it remained inert before Play, clicked Play, fetched the declared resources
through the authenticated publisher, played, paused, browsed another tab, returned
and closed cleanly. It checked that no executable files were persisted in the
player profile. This is same-machine authenticated loopback evidence.

Machine-readable results: [X11 reports](games-03-x11.json).

Actual window captures (no mockups):

- [Start screen](../images/game-lantern-03-start.png)
- [Gameplay and score](../images/game-lantern-03-playing.png)
- [Mouse/pulse and mute](../images/game-lantern-03-mouse-muted.png)
- [Paused](../images/game-lantern-03-paused.png)
- [Native performance overlay](../images/game-lantern-03-performance.png)
- [Hosted page before Play](../images/game-lantern-03-hosted-before-play.png)
- [Authenticated hosted gameplay](../images/game-lantern-03-hosted-playing.png)

## Measurements

Release headless VM, 10,000 frames each, using the `measure` example:

| Game | Load including local assets | Mean frame | p95 | p99 | Maximum | Maximum commands |
| --- | --- | --- | --- | --- | --- | --- |
| Lantern Drift | 7.097 ms | 0.0306 ms | 0.0202 ms | 0.0730 ms | 35.8644 ms | 51 |
| Prism Breakout | 1.567 ms | 0.0922 ms | 0.0651 ms | 0.1633 ms | 40.3076 ms | 299 |

These wall-clock samples include shared-container scheduling delays during other
validation; they are not hardware performance guarantees. The final release
Lantern Drift native overlay showed **60.1 presented FPS**, **7.43 ms average
paint + present**, **0.07 ms average VM update**, and **0 late VM frames** at
1440×900. An earlier run under heavy concurrent compilation presented fewer
frames; software rendering remains sensitive to CPU contention. The headless
benchmark does not measure audio output or UI rendering.

## Native Linux and Windows CI follow-up

[GitHub Actions run 41318900007](https://github.com/ItsKorayYT/YamiNet/actions/runs/41318900007)
passed for implementation commit `14820e9e825a69b900d3dcad49b5f4adab5af3cf`.
Both `validate (ubuntu-24.04)` and `validate (windows-latest)` completed
successfully. Their locked source verification included formatting, Clippy,
workspace tests, debug builds and release builds on the native runner.

- [Windows job](https://github.com/ItsKorayYT/YamiNet/actions/runs/41318900007/job/124884735333):
  native MSVC package verification and installer upgrade lifecycle passed.
- [Linux job](https://github.com/ItsKorayYT/YamiNet/actions/runs/41318900007/job/124884735520):
  native packaging, virtual X11 lifecycle and input smoke checks passed.
- Both jobs passed package regression checks, the independent-process YNS demo,
  and the loopback testnet topology rehearsal; validated packages and diagnostic
  evidence were uploaded by the workflow.

This adds native Windows automated build/test and package lifecycle coverage.
It does not establish interactive Windows game play or audible device testing.
[Recorded CI job results](games-03-ci.json) identify the verified code commit;
this documentation follow-up does not change its implementation.

## Testing limits

No physical Windows host, Windows runtime window, physical high-DPI display,
physical keyboard/mouse, real speakers/headphones, or owned remote Internet game
publisher was available. Windows was cross-compiled/checked in this container; native MSVC builds, tests
and package lifecycle checks subsequently passed on the Windows CI runner as
recorded above. The Linux ALSA build dependency is included in the CI workflow. X11 input was synthetic and high-DPI geometry was tested in
native rendering tests. There was no usable default ALSA audio device: gameplay
continued silently, exercising the unavailable-output path. Audio channel
lifecycle tests use native idle sinks; actual audible output, device removal and
driver behavior require physical follow-up. No microphone capability was added.
Pre-existing ignored manual/public-network tests remain ignored; no new ignored
tests were introduced. Internet routing and external deployments were not tested.
