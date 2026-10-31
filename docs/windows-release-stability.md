# Windows release uninstall failure

Base: `main` at `f7355454643e9fe3f7128b942c85847c5ea9f25d`.
Fix branch: `fix/windows-release-stability`.

## Observed failure

The actual Windows job logs report:

```text
packaging/lifecycle.py, line 100
ValueError: uninstall left application directory or shortcuts
```

This repeats in the [first run after the game runtime merge](https://github.com/ItsKorayYT/YamiNet/actions/runs/41194566458/job/124518022850),
the [browser polish PR run](https://github.com/ItsKorayYT/YamiNet/actions/runs/41199470876/job/124532223826),
and [current main](https://github.com/ItsKorayYT/YamiNet/actions/runs/41200280692/job/124534557798).
Current main's [Linux job passed](https://github.com/ItsKorayYT/YamiNet/actions/runs/41200280692/job/124534557969).

The Windows failure is specifically after installation and upgrade, during
uninstall. The logs and lifecycle control flow distinguish these stages:

| Stage | Original Windows result |
| --- | --- |
| Rust formatting, Clippy, workspace tests, debug/release MSVC compilation | Passed |
| Package staging, exact manifest/resource verification, native CLI without Rust | Passed; `rust_on_path: False` |
| Current NSIS installer creation | Passed |
| Immutable prior source `760398d` compilation, packaging and NSIS creation | Passed |
| Prior installation and current-version upgrade | Reached and passed installed-package verification |
| Uninstallation and complete application-directory removal | Failed |
| Later packaged GUI smoke | Not reached |

Commit `12e93e0` added the bundled local game to packaging and completeness
verification. It did not update `packaging/windows/installer.nsi`. That script
removed the previous known application subdirectories, but not `games/`, so its
final non-recursive `RMDir "$INSTDIR"` could not remove the installation.
The browser and game UI changes are not the cause of this installer mismatch.

## Fix and regression

Add `games/` to the existing application-directory uninstall allowlist. Keep the
non-recursive final directory removal; do not replace the allowlist with a wipe
of the installation root or user profile. Identities, libraries, settings and
peer data remain outside the installed application directory.

The new cross-platform packaging regression stages the real Windows resource
layout with fixture executables, interprets the explicit NSIS uninstall
allowlist, and checks that it covers every staged path plus the generated
uninstaller. On the original script it fails with exactly:

```text
games/
games/README.md
games/breakout/
games/breakout/breakout.wasm
games/breakout/game.yami-game.json
```

It passes with the fix. This is an allowlist/layout regression on Linux, not a
claim that Linux executed the Windows installer. It also checks that unrelated
operator files and separate user-state fixtures survive. Completeness tests now
reject a missing game manifest or Wasm module even if the manifest is rewritten.

## Stronger native package verification

All existing release gates remain enabled, including native MSVC compilation,
full bundle verification, the real prior-to-current NSIS upgrade, complete
uninstall/shortcut/registry removal and protected user-data hash comparisons.
No workflow step, test, security restriction or completeness requirement is
removed or skipped to pass CI.

The lifecycle report now separates baseline installation, upgrade, installed
package verification, installed GUI and uninstall. An uninstall failure records
bounded relative leftover paths, never private state contents. The upgraded
registry version must match the verified executable/package version.

When a graphical session is available, the upgraded installed application is
launched before uninstall, in addition to the existing portable launches. Both
use fresh and reopened isolated profiles. The native smoke harness now exercises
the shipped game through its existing launcher and local Wasm loader, and the
package harness supplies a synthetic cached search fixture. It requires checks
for cached full-text results and keyboard selection, tabs, game rendering,
pause/background pause, resume, reset, stop and close. Missing checks fail the
lifecycle. These fixtures are only used by explicit smoke mode; normal browser
behavior, UI, networking and Wasm restrictions are unchanged.

Executable verification and native-window smoke use a non-development working
directory containing spaces with `cargo` and `rustc` absent from PATH. The game
manifest and Wasm are loaded from the extracted or installed package, not the
source checkout. Existing font/license, document, site/image and configuration
resources remain covered by the exact checksum manifest.

## Verification and evidence

Reproduce with Rust 1.90.0 and the checked-in lockfile:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
cargo build --workspace --release --locked
python -m unittest discover -s packaging -p 'test_*.py' -v
```

Native Linux package checks:

```sh
python packaging/package.py --platform linux
python packaging/verify.py dist/YamiNet-0.3.1-alpha.1-linux-x86_64 --expected-version 0.3.1-alpha.1
xvfb-run -a python packaging/lifecycle.py --archive dist/YamiNet-0.3.1-alpha.1-linux-x86_64.tar.gz --report dist/linux-lifecycle.json --gui
```

Native Windows checks, in a disposable Windows user profile after release build:

```powershell
$stage = python packaging/package.py --platform windows
python packaging/verify.py "$stage" --expected-version 0.3.1-alpha.1
./packaging/windows/verify-native.ps1 -Stage $stage -Version 0.3.1-alpha.1
```

The PR records the final source SHA, the exact Actions run and both matrix job
results. Acceptance requires successful Linux and Windows jobs on that final
branch revision, including the unchanged installer upgrade gate. The uploaded
`windows-lifecycle.json` records actual native installation, upgrade, CLI,
uninstall and data-survival results. In an interactive Windows session it also
records `installed_gui_runs` and portable `gui_runs`, with per-run search/game
checks and screenshots. A non-interactive session retains explicit NOT RUN for
GUI, as before; a failed attempted GUI check fails the job.

Automated X11 and Windows-runner checks are not physical-machine validation.
Physical Windows 10/11 keyboard/IME, display scaling/GPU, SmartScreen, ordinary
non-administrator desktop installation and ACL inspection still require the
manual gate in [release-candidate-0.3.1.md](release-candidate-0.3.1.md).
