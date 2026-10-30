# 0.3 desktop alpha release checklist

Record PASS / FAIL / NOT RUN and evidence for every item. A generated package is
not proof of native runtime, external reachability or independent security review.

- [ ] Confirm workspace version and `v0.3.0-alpha.1` tag match; no stable publish.
- [ ] Run fmt, warning-free Clippy, workspace tests and workspace build with lockfile.
- [ ] Windows and Linux CI succeed; review failures before distributing artifacts.
- [ ] Build release binaries, bundles, manifests and installer; verify SHA-256.
- [ ] Native Windows fresh install, upgrade and uninstall: existing identity and library byte-for-byte retained.
- [ ] Native Linux launcher install/uninstall and startup from directories containing spaces.
- [ ] Start without Cargo installed, in a non-development working directory.
- [ ] First launch makes no external requests; local-only blocks remote navigation.
- [ ] Onboarding, settings persistence/reset, bootstrap editing and relay toggle work.
- [ ] History remains disabled until explicit opt-in; no telemetry or accounts.
- [ ] Keyboard shortcuts, Tab links, focus outline, IME, Escape, clipboard and find on both OSes.
- [ ] Simple, long and image-heavy documents; tab close cancels resources and releases layout.
- [ ] v0.1/v0.2 documents/styles, direct/relay identities and mismatch errors preserved.
- [ ] Guided setup creates/reuses a key; preview and explicit host start; no public autorun.
- [ ] Record benchmark environment/results without claiming universal performance.
- [ ] Complete physical VPS + Windows NAT host + independent client document/PNG trial.
- [ ] Native visual rendering/navigation checklist, unavailable peer/relay and recovery.
- [ ] Document unsigned executables, limited glyphs, no semantic accessibility API and remaining limits.
- [ ] Review SECURITY.md and do not claim anonymity, production security or code signing.
- [ ] Maintainer explicitly approves an experimental GitHub prerelease with checksums.
