# Optional local diagnostics

Logging is disabled by default. Enable it with
`yami-browser --diagnostic-log ABSOLUTE-DIRECTORY`. The browser writes fixed
codes such as startup/shutdown, navigation completion/failure, settings outcome,
resize and tab close, with local timestamps. It never logs addresses, document
contents, private keys, free-text errors or telemetry. It sends no reports.

Storage is bounded to `events.log` and `events.previous.log`, each at most 64 KiB,
plus a zero-length lifetime lock. Files are owner-only on Unix; Windows inherits
the selected directory ACLs. Use a private directory and inspect its ACLs. Existing
symlinks, oversized files, malformed records and simultaneous writers are refused.
A write failure disables further logging with a generic local warning. These
logs are coarse troubleshooting events, not a complete browsing or security audit.

Close the browser before running:

```
yami-browser diagnostics report ABSOLUTE-DIRECTORY
yami-browser diagnostics clear ABSOLUTE-DIRECTORY
```

Report emits OS/version and counts of known events, omitting timestamps, paths,
addresses and contents. Invalid/injected records are refused rather than copied
into the report. Clear removes only the two owned event files, retaining unrelated
files and the empty lock. No report is uploaded automatically. Keep raw logs
private; send a redacted summary only after reviewing it yourself.

Native test automation uses `--desktop-smoke ABSOLUTE-NEW-REPORT` with a deliberately
isolated `YAMINET_BROWSER_DATA_DIR`. It opens a real native window, uses logical
actions/native clipboard, creates test bookmarks, and saves synthetic screenshots.
Do not point it at your normal profile. The package lifecycle harness creates a
fresh test profile, runs twice and distinguishes actual native frame presentation
from physical keyboard/IME and consumer Windows validation. Reports use PASS,
FAIL and NOT RUN; an absent graphical session does not count as a GUI pass.
