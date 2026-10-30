# Install the desktop alpha

YamiNet is experimental, not anonymous and not suitable for sensitive activity.
There are no accounts, telemetry, bundled public peers or mandatory services.
Use Windows 10/11 x86_64 or Linux x86_64 with an X11/XWayland desktop.

## Download and launch

A maintainer's explicitly published alpha prerelease or successful alpha workflow
contains portable packages and SHA-256 files. Development source is not an
installer. Verify the checksum before extracting: Windows PowerShell
`Get-FileHash .\YamiNet-...zip -Algorithm SHA256`; Linux
`sha256sum -c YamiNet-...tar.gz.sha256`. Compare with the published checksum.
Binaries are unsigned; there is no code-signing credential configured.

Windows: extract the entire ZIP to a user-owned application directory, then
double-click **Launch Browser.cmd** or `yami-browser.exe`. The optional NSIS
installer installs per user under `%LOCALAPPDATA%\Programs\YamiNet`, provides a
Start Menu shortcut and an uninstall entry. No administrator, firewall rule,
background service, public node or startup task is needed to browse.

Linux: extract the archive, run `./launch-browser.sh`. Keep the entire directory
together. `./install-desktop.sh` registers the current directory in your desktop
application menu. It does not copy the application; moving it requires rerunning
the script. Libraries: X11/XCB and XKB support from the distribution. Run
`ldd ./yami-browser` to diagnose missing libraries. The bundle is built on Ubuntu
24.04 in CI and may require a glibc at least as new as the build host. It is not
an AppImage or a promise of compatibility with every Linux distribution.

CLI commands remain available beside the browser: `yami-node`, `yami-relay`,
`yami-browser --help` and `--version`. Users of packages do not need Cargo.
Source build and package creation instructions are in [releases.md](releases.md).

## First launch and browsing

The native first-launch panel explains identities, peers, relays and privacy.
Choose **local only** to read the built-in Welcome/About pages and loopback
previews. Settings and optional history remain local. History starts disabled.
Startup does not discover nodes or contact public peers. **Settings & first
launch** reopens the panel; Escape dismisses it.

To browse a shared website, obtain the complete `yami://yn1-.../home` address and
trusted bootstrap `yn1-...@IPv4:port` pin from its operator. Compare identities
through a trusted channel. Enable network browsing in Settings, choose **Add
bootstrap peer**, paste the pin, press Enter, then enter the site's address.
No discovered peer is automatically trusted. A bootstrap supplies signed
locations, not trust in a website. A direct `ynps://ID@IPv4:port/home` address
can also connect to the exact pinned node. Remote content always requires TLS.
Relay fallback is off by default; enable it explicitly for sites with signed
relay routes. The browser displays both destination and relay identities.
TLS authentication does not establish the operator's reputation or anonymity.

If a site fails, check local-only mode, full ID/checksum, peer pins, node/relay
availability and clock. `yami-browser --network-status` reads local lookup state.
`yami-browser [--config FILE] --probe URL --asset cover` explicitly performs an
authenticated fetch; see [cross-network-validation.md](cross-network-validation.md).
Network config profiles and peer removal/replacement use the existing CLI in
[configuration.md](configuration.md); edits never happen through discovered ads.

## Data, backups and upgrades

Windows defaults: keys `%LOCALAPPDATA%\YamiNet\identity`; peer state
`%LOCALAPPDATA%\YamiNet\peers`; browser library below `peers\browser-library`.
Linux defaults: `${XDG_DATA_HOME:-$HOME/.local/share}/yaminet/identity` and sibling
`peers/browser-library`. Explicit `--config`, `YAMINET_PEER_DIR` or
`YAMINET_BROWSER_DATA_DIR` can select other directories. Keep these outside the
application and published site directories. The library uses an exclusive lock;
a second browser process may report it unavailable rather than overwrite data.
Corrupt settings are reported and preserved, not silently replaced.

Back up the complete identity directory (`node.key` and `node.id`) to private,
access-controlled offline storage. Anyone with the secret key can impersonate
the node. Do not email keys, put them in websites, screenshots, source control
or public diagnostics. Unix creation is owner-only; Windows inherits directory
ACLs, which the operator must inspect. See [SECURITY.md](../SECURITY.md).

Close running applications before upgrading. Back up identities and user data,
extract the new version into a new application directory (or run the per-user
installer), then launch it. Existing library settings/bookmarks and peer stores
are reused; the old library format loads with safe onboarding defaults. Keep the
previous application directory until basic browsing works. Older browsers may reject
the new settings row; restore a pre-upgrade library backup before downgrading
instead of deleting or rewriting identities. Settings reset clears
homepage, optional history and preferences, retaining bookmarks and keys. It does
not delete bootstrap pins; manage pins separately with the existing peer CLI.

## Uninstall

Windows installer: use Installed apps or the Start Menu Uninstall shortcut.
Portable Windows: delete the extracted application folder after closing programs.
Linux: run `./uninstall-desktop.sh`, then delete the extracted bundle. No service
or startup task was installed automatically. User data remains; to remove it,
first back up private identities and deliberately delete the separate per-user
state directories. An explicitly configured scheduled hosting task must be
removed separately by its owner. Uninstallation never changes firewall/router rules.

For 0.3.1, follow [getting started](getting-started-0.3.1.md) and the
[native release/rollback gate](release-candidate-0.3.1.md). Linux runtime loading
also uses X11 extensions through dynamic loading; `ldd` alone is not sufficient.
The Ubuntu 24.04 CI setup explicitly installs X11/XCB, XKB common + X11, XRandR,
XCursor, XI, Xinerama, XRender, XFixes and XCB shape/xfixes support, then exercises
an actual native window under Xvfb. Debian 13 is the local native environment.
Other distributions and physical desktops require their own recorded validation.
The diagnostic log is optional and disabled by default; see
[local diagnostics](diagnostics-0.3.1.md) for enable/report/delete commands.
