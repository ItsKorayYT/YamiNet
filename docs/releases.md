# Building experimental alpha packages

Use Rust 1.90 and Python 3.12 on the native target OS. Run the four workspace
checks in README, then `cargo build --workspace --release --locked` and
`python packaging/package.py --platform linux` (or `windows`). Output is in
a fresh `dist/` directory. The script refuses to reuse a staging directory.
The distribution includes the browser, node, relay, embedded and loose licensed
fonts, examples, config templates, docs, version and SHA-256 manifest. No user
state is copied. Linux requires an x86_64 desktop with X11 (or XWayland) and
system X11 libraries; this is a portable directory, not a static universal binary.

Windows packaging uses native MSVC binaries. Install NSIS 3; pass the absolute
staging directory and version to `makensis /DSTAGE=... /DVERSION=0.3.0-alpha.1
packaging/windows/installer.nsi`. The per-user installer has Start Menu and
uninstall entries, no elevation, firewall, services, autorun or identity actions.
Executables and installers are unsigned. SmartScreen may warn; do not claim signing.

The alpha workflow validates Linux and Windows, builds release binaries and
uploads downloadable workflow artifacts. A `v0.3.0-alpha.1` tag must match the
workspace version. Tag builds do not publish a GitHub release or a stable release.
A maintainer must inspect successful jobs, download artifacts, verify SHA-256,
complete manual alpha checks, and explicitly create an alpha prerelease. Failed
jobs never count as validated packages; do not distribute partial staging output.
NSIS must exist on the Windows runner; a missing tool is an explicit job failure.

Do not put identities inside the application folder. Upgrades replace only the
application bundle; keep the existing user state directory. Uninstallation keeps
identities, bootstrap pins, bookmarks and optional history. Back up private keys
before manually deleting user data. See installation.md and alpha-checklist.md.
