# 0.3.1 alpha / release-candidate gate

The version remains `0.3.1-alpha.1` until native platform and physical-network
blockers are cleared. No stable release, release tag or signing is automatic.
Only a maintainer may explicitly approve a matching `v0.3.1-alpha.1` (or later
RC) tag. GitHub Actions validates tags but never creates them or a GitHub release.

## Reproduce and inspect

With Rust 1.90.0 and the checked-in lockfile:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
cargo build --workspace --release --locked
python -m unittest discover -s packaging -p 'test_*.py' -v
```

Stage with `python packaging/package.py --platform linux` or `windows`. Run
`python packaging/verify.py STAGE --expected-version 0.3.1-alpha.1` natively. The
verifier requires every binary, font/license, example/config resource, a valid
version and an exact complete manifest; deleted files with a rewritten manifest,
extras, traversal and symlinks fail. Executables run in a non-development working
directory with Cargo/Rust removed from PATH, and their reported versions must
match. `--no-execute` is manifest-only cross-build inspection, not runtime proof.

The Windows workflow builds native MSVC current and immutable `760398d` baseline
sources, packages both, and uses the **real** prior installer to test upgrade to
the current version. It extracts/installs in paths with spaces, creates a real
identity and peer store plus a bookmark fixture outside the application, and
checks exact data bytes after install, upgrade and uninstall. It verifies the
Start Menu shortcut, uninstall entry and complete removal. NSIS runs per user;
no firewall/service/autorun action occurs. A pre-existing YamiNet uninstall entry
or Start Menu folder aborts the test to protect an operator's existing install.
Use a disposable Windows user profile. A same-build reinstall is explicitly
NOT RUN for cross-version upgrade; it must never stand in for the baseline test.

On Linux run:

```
xvfb-run -a -s '-screen 0 1600x1000x24 -nolisten tcp' python packaging/lifecycle.py --archive dist/YamiNet-0.3.1-alpha.1-linux-x86_64.tar.gz --report dist/linux-lifecycle.json --gui
```

This checks actual launcher install/removal, runtime libraries, user-data
isolation, non-development execution and two native window launches with a
fresh profile. Xvfb provides a virtual X11 display, not physical keyboard/IME
validation. The additional Linux input-smoke.py harness sends synthetic X11
keyboard events through the actual event adapter and checks navigation, tabs,
find/address focus and graceful shutdown. It requires xdotool. Windows runs the native-window test only in a non-service interactive
session; otherwise GUI remains NOT RUN. A GUI attempt that fails is a failure,
not silently converted into a skip. Failure logs and structured reports are
uploaded separately from successfully validated package artifacts.

## Upgrade and rollback

1. Close browsers/hosts. Back up the full private identity directory and local
   library/peer data in access-controlled storage. Record full public identities.
2. Keep sites, identities, peer state and optional logs outside the application.
3. Extract a fresh portable version (or run the per-user installer). Inspect
   version/checksums and reopen the existing user profile. Compare identity IDs,
   bookmark contents and trusted pins before resuming hosting.
4. On a regression, close the new app and reinstall/reselect the previous known
   version. Preserve the latest identity and DHT anti-rollback/high-water state;
   never restore an old sequence cache or regenerate keys as a shortcut.
   0.3.0/0.3.1 use the same library/state formats. Much older versions may reject
   new settings rows; use a separate restored library backup rather than erase
   the live profile. If a state format becomes incompatible, stop and document
   migration/rollback rules before release.
5. Uninstall removes application files/shortcuts, preserving separate user data.
   Explicitly remove an operator-created Task Scheduler/service entry separately.
   Logs are cleared only by the user's diagnostic clear command or deliberate
   private data deletion. Never include private keys in bug reports.

## Maintainer manual gate (record PASS / FAIL / NOT RUN)

- [ ] Download artifacts from the exact source revision and verify SHA-256.
- [ ] Review native Windows MSVC + installer old-to-new lifecycle report and logs.
- [ ] Repeat on an ordinary non-administrator Windows 10/11 account; inspect ACLs,
      Start Menu, uninstall, SmartScreen behavior, package paths and data survival.
- [ ] Test Linux desktop on each explicitly claimed distribution; record glibc,
      X11/XWayland, normal UID, launcher behavior, display scaling and shutdown.
- [ ] Physical keyboard, clipboard, IME/composition, resizing, focus/Tab, find,
      bookmarks/history, rapid navigation and close-during-loading on both OSes.
- [ ] Work through [getting started](getting-started-0.3.1.md) without Rust on PATH.
- [ ] Run the [VPS/home/independent-client trial](network-trials-0.3.1.md), including
      signed discovery, document/PNG, route identities and an explicit relay cycle.
- [ ] Review failures, all NOT RUN items, known glyph/accessibility/render limits,
      diagnostic redaction and unsigned experimental security warnings.
- [ ] Explicitly approve the version/tag and a prerelease only after recording
      remaining blockers. No code-signing claim without actual credentials.
