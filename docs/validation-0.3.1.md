# 0.3.1 stabilization evidence

Development date: 2026-10-30; final evidence recorded 2026-10-31. Base milestone
`760398d`; version `0.3.1-alpha.1`. Native CI verified code revision `f74a0eb`;
the subsequent evidence-only commit changes this report.
The final result remains an experimental alpha, not stable or security-certified.

## Implemented features and concrete fixes

- Native Windows MSVC and Linux source/build/test jobs; strict complete package
  manifests/resources/version checks; executable runtime in a non-development
  working directory with Rust/Cargo absent from PATH.
- Real NSIS baseline-to-current installation/upgrade/uninstall harness in paths
  containing spaces. It checks shortcuts, uninstall registration, binary versions
  and exact persistent identity, bookmark and peer bytes. Existing user installs
  cause refusal; no firewall/service/startup changes are performed.
- Linux launcher/dependency/user-data lifecycle plus two actual native-window
  launches under virtual X11. Window resizing, native Unicode clipboard, logical
  shortcuts, tabs, long/image-heavy rendering, local find and bookmark restart
  are exercised. A separate synthetic X11/XTEST test drives actual native keyboard
  events through the winit adapter and checks titles/navigation/shutdown. Actual
  framebuffer captures are in the getting-started guide.
- Reproduced Find/bootstrap-to-address editor mode leak fixed. First-run save
  failures keep the dialog open. Reset cancels stale loading states rather than
  leaving an apparently active request. Significant fixes have regressions.
- Windows-only Clippy failure in snapshot persistence corrected while preserving
  the installed/high-water flag before parent-directory sync on Unix.
- Opt-in fixed-code local logging: two 64 KiB files, locking, Unix owner-only
  creation, symlink/bounds checks, redacted count reports and user-controlled
  clearing. No addresses, contents, keys, free-text errors, telemetry or upload.
- Structured network trial on existing TLS/DHT/relay/PNG APIs: exact peer pins,
  application-level DHT FIND responses, fresh returned signed publication,
  canonical resolution, document/PNG validation and route classification. Local
  host snapshots are optional and distinguished from authenticated delivery.
  Controlled relay cycling is operator-assisted and explicitly enabled.
- Fresh lookup cannot pass using a cached record or replace an older wire record
  with a newer local cache as false evidence of current publication. Regression
  tests cover both. Probes reuse the browser's native-link compatibility adapter;
  a reproduced valid 0.1 native-link rejection was fixed and tested through the
  existing four-process relay/recovery fixture, now also fetching a PNG/report.
- CI exposed a peer-store reopen race when a process spawn briefly retained an
  inherited open file description. Owner Drop now explicitly unlocks before close;
  a deterministic duplicate-description regression preserves exclusivity and verifies
  that a stale descriptor cannot release a new owner's lock.
- Failed lifecycle runs preserve completed stages and bounded synthetic child
  stderr. NSIS's unquoted final `/D=` tail is handled explicitly by the harness.

## Passed tests

| Check | Actual evidence/status |
| --- | --- |
| Local fmt, warning-denied Clippy, workspace tests/build and release build | PASS on final code: 242 passed, 0 failed, 8 ignored |
| Deterministic peer-lock regression and internet/peer integration tests | PASS after explicit unlock fix |
| Packaging completeness/traversal/lifecycle regressions | PASS: 6 tests |
| Debian 13 native bundle CLI + launcher/shared-library/data isolation | PASS; ordinary UID 1000; no Cargo/Rust in child PATH |
| Two Debian native-window runs under Xvfb | PASS; resized, clipboard, logical actions, rendering, bookmark reopening and clean exit |
| Separate X11/XTEST event adapter test | PASS; address typing, history, tabs, find/address focus and Alt+F4 clean shutdown |
| Fresh signed-publication and rollback diagnostics | PASS, including actual four-process relayed document/PNG report and recovery fixture |
| Ubuntu 24.04 native CI | PASS at f74a0eb; run 41117836166, job 124276564661: fmt, Clippy, tests (242 passed, 0 failed, 8 ignored), debug/release builds, package verification, launcher/data/window lifecycle and separate XTEST input test |
| Windows Server 2025 native MSVC CI | PASS at f74a0eb; same run, job 124276565154: fmt, Clippy, tests (235 passed, 0 failed, 8 ignored), debug/release builds, portable ZIP and current/baseline NSIS compilation |
| Corrected native Windows installer lifecycle | PASS: actual 0.3.0-alpha.1 baseline installed, upgraded to 0.3.1-alpha.1, verified and uninstalled in a path containing spaces; shortcuts/registry removed and identity/bookmark/peer bytes preserved |
| Windows native GUI | PASS: two native-window launches with rendered frames, resize, Unicode clipboard, tabs, find, long/image-heavy documents, bookmark reopening and clean exits; logical actions, physical keyboard/IME NOT RUN |

Local development uses Rust 1.90.0 on Debian 13 x86_64, with debug symbols disabled
for the final test rebuild to fit the managed disk quota; debug assertions remain
enabled. CI uses native default profiles. Existing tests remain enabled; no
failures are hidden by skipping substantive checks. Run/job IDs distinguish
earlier attempts from the final code and platform. See actual workflow artifacts
for the final source revision, toolchain, per-command logs, JSON and screenshots.

### Downloadable native CI evidence

All four artifacts below belong to the successful
[run 41117836166](https://github.com/ItsKorayYT/YamiNet/actions/runs/41117836166)
at `f74a0eb`. Package archives contain the report as it existed before this
evidence-only update. They are CI artifacts, not a published GitHub release.

- [Linux portable package and SHA-256](https://github.com/ItsKorayYT/YamiNet/actions/runs/41117836166/artifacts/13005214797)
- [Linux logs, lifecycle/input JSON and native screenshots](https://github.com/ItsKorayYT/YamiNet/actions/runs/41117836166/artifacts/13007196147)
- [Windows native MSVC ZIP, NSIS installer and SHA-256](https://github.com/ItsKorayYT/YamiNet/actions/runs/41117836166/artifacts/13007881039)
- [Windows logs, upgrade/lifecycle JSON and native screenshots](https://github.com/ItsKorayYT/YamiNet/actions/runs/41117836166/artifacts/13006683757)

Download while GitHub retains the artifacts; authentication may be required.
Compare `source-revision.txt`, `toolchain.txt`, checksum files and lifecycle JSON
before using a package. The seven-test difference between Linux and Windows is
existing Unix-only coverage, including filesystem permissions and descriptor
locking; no failing tests were disabled.

## Failed tests and fixes

- Prior 0.3.0 Windows CI stopped at warning-denied Clippy in DHT snapshot storage.
  Reworked cfg-specific sync sequencing; native MSVC source checks subsequently
  completed successfully.
- New focus/save regressions failed against the prior implementation, then passed
  after the editor-mode and first-run/reset corrections.
- New four-process network report initially rejected a valid legacy native link:
  raw parser versus production adapter mismatch. Now shares the native adapter.
- First stabilization Linux CI failed its window child with exit 101 after native
  CLI checks passed. Child stderr was previously discarded. Required dynamically
  loaded X11 extension libraries and retained failure logs were added for reruns.
- First stabilization Windows CI built current/baseline MSVC binaries and both
  NSIS installers, then failed to find the requested installation path/shortcut.
  Python's normal argv quoting is incorrect for NSIS's special `/D=` tail; the
  actual raw final-tail syntax is now used. The final native rerun passed the
  real baseline upgrade, uninstall, data-preservation and GUI stages.
- A subsequent Ubuntu run hit the existing peer-store reopen/lock race; this was
  fixed with explicit owner unlock and a regression rather than disabling parallel
  tests or weakening lock behavior.

## Not run and remaining release blockers

- Physical Linux VPS + Windows home-router host + independently connected browser
  trial NOT RUN. No public-network or NAT validation is claimed. No infrastructure,
  router or firewall was provisioned/changed.
- Physical keyboard/IME, consumer Windows 10/11, ordinary non-admin Windows ACLs,
  real display scaling/high-contrast and untested Linux distributions remain
  manual checks. Virtual X11/native CI do not substitute for these environments.
- Linux cross-version installation upgrade NOT RUN by the Linux lifecycle
  harness; it checks launcher installation/removal and persistent-data isolation.
  Only Windows has an automated actual prior-version installer upgrade result.
- Operator-assisted relay stop/restart on real infrastructure NOT RUN. Existing
  automated loopback relay outage/recovery simulations remain separate evidence.
- No signing credentials or independent security review. No screen-reader tree,
  anonymity, production certification or sensitive-activity suitability claim.

## Exact remaining maintainer actions

1. Download artifacts/logs for the final source SHA from Actions; verify SHA-256
   and each stage in Windows/Linux lifecycle JSON. Any failed job blocks claiming
   that package validated.
2. In a disposable ordinary Windows 10/11 user account, repeat native installer
   old-to-new lifecycle, inspect ACLs/shortcuts/uninstall and compare saved identity,
   bookmark and peer bytes. Keep data outside the installation.
3. On each supported physical desktop run the getting-started guide without Rust
   on PATH; exercise IME, clipboard, keyboard focus/find, resize, tabs, failures and
   repeated close/reopen. Record the OS/display environment and actual observations.
4. Follow network-trials-0.3.1.md on three independently connected machines. Run
   strict preflight, the JSON trial with a PNG, an explicitly enabled operator
   relay cycle, then actual native rendering/navigation. Fill the report template.
5. Review blockers, unsigned/security/accessibility limits and the rollback gate.
   Only then explicitly approve any alpha/RC tag or prerelease publication. This
   task creates no release tag and publishes no GitHub release.
