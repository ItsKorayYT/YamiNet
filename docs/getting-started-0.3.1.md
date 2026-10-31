# First steps with the 0.3.1 desktop alpha

Use disposable content. YamiNet is experimental, unsigned, not anonymous and not
suitable for sensitive activity. Download a validated alpha artifact and compare
its SHA-256. Keep the application folder separate from identities and user data.
See [installation](installation.md) for OS dependencies and data locations.

1. Windows: extract the ZIP or run the per-user installer; use **Launch Browser**.
   Linux: extract the bundle, run `./launch-browser.sh`; optionally run
   `./install-desktop.sh`. No Cargo is needed at runtime.
2. The browser opens directly without a welcome popup. **Local only** is enabled
   by default, and optional history is disabled. Open **Settings** in the sidebar
   to change browsing preferences.
3. Open the built-in Welcome/About pages. For a bundled rich example, from the
   package run `yami-node site preview sites/personal --identity PRIVATE-DIRECTORY`
   (PowerShell: `.\yami-node.exe ...`). Keep that directory outside the package.
   Paste the printed `ynps://ID@127.0.0.1:PORT/home` address in the browser.
4. Create your own site outside the package:

   ```sh
   ./yami-node site setup /path/to/my-site --new personal --identity /private/my-key --output /private/my-host.conf --preview
   ```

   Windows PowerShell:

   ```powershell
   .\yami-node.exe site setup "$env:USERPROFILE\Documents\My Site" --new personal --identity "$env:LOCALAPPDATA\YamiNet\my-key" --output "$env:LOCALAPPDATA\YamiNet\my-host.conf" --preview
   ```

   Preview is loopback TLS, not public hosting. Stop it with Ctrl+C. Later use
   `yami-node --config PRIVATE-CONFIG` to start the explicitly selected host.
5. For a remote site, independently compare its complete identity and a trusted
   bootstrap pin. Enable network browsing deliberately. **Add bootstrap peer**
   accepts `full-node-ID@IPv4:port`. Paste the site's `yami://ID/page` address.
   Relay fallback is a separate opt-in. Both destination and relay remain pinned.
6. Ctrl+L selects the address; Ctrl+F searches rendered text locally. Enter/
   Shift+Enter select matches; Escape closes find/dialogs. Ctrl+D saves a
   bookmark; edit its title in the address editor and press Enter.
7. Close and reopen the browser. Use **Bookmarks & history** to reopen the saved
   site. No remote tabs or search terms are restored automatically on startup.
   If the library is locked, close the other browser instance. Invalid settings
   are preserved and reported. A failed settings save keeps the settings panel visible.

To browse saved website directories, choose **Discover websites**. Search under
**Websites · offline search**; queries stay local. In **Directory subscriptions**,
add a label and a canonical `yami://FULL-NODE-ID/directory-key`, select the row,
and explicitly refresh it after enabling network browsing. Adding a subscription
or restarting never fetches it. Cached listings survive a failed refresh. Every
listing remains untrusted: **Open selected website · verify identity** uses normal
TLS-authenticated navigation. See the [Discover guide](desktop-browser.md#discover-websites-and-directory-subscriptions)
for provenance, local-only behavior, relay opt-in and sharing a CLI store.

![Native browser rendering image-heavy test content](images/getting-started-031-browser.png)

These are framebuffer captures from an actually presented native winit/softbuffer
window on Debian 13 under Xvfb. The second uses synthetic public smoke content.
The test resized the window, exercised logical shortcuts and the native
clipboard, saved a bookmark, exited and reopened it. This is virtual-X11 native
window validation, not physical Windows/Linux keyboard, IME or display testing.

If something fails, enable local event logging explicitly:
`yami-browser --diagnostic-log ABSOLUTE-DIRECTORY`. After closing the browser,
`yami-browser diagnostics report ABSOLUTE-DIRECTORY` prints a redacted JSON
summary. `diagnostics clear ABSOLUTE-DIRECTORY` deletes the two log files only.
See [diagnostics](diagnostics-0.3.1.md) and [network trials](network-trials-0.3.1.md).
