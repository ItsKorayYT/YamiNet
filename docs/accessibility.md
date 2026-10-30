# Native accessibility status

The alpha uses winit, softbuffer and cosmic-text, with software-drawn controls.
It does not currently expose a semantic accessibility tree. Windows UI Automation,
Linux AT-SPI and screen-reader support have not been implemented or tested.
AccessKit can bridge a semantic tree into these native APIs, but integrating it
requires stable control/document node IDs, focus events, text ranges and platform
validation; painting pixels or adding labels alone does not provide that bridge.
No screen-reader compatibility is claimed.

Implemented: Tab/Shift+Tab through native chrome, modal controls and currently
visible document links; Enter activation; high-contrast control outlines;
address editor selection, clipboard and IME; keyboard scrolling; local Unicode
find with match scrolling; Escape dismisses dialogs. Search is exact and
case-sensitive; Unicode normalization/case folding is not currently provided.
The bundled fonts have limited script coverage. Missing glyphs may appear as boxes.
Document text selection/copy and platform accessible text ranges are unsupported.
Links outside the viewport become keyboard targets after scrolling into view.
Page YSL colors are author-selected; the browser does not promise accessible
contrast for arbitrary websites.

Manual release checks must cover keyboard-only use on Windows and Linux, focus
visibility, IME, display scaling and high-contrast themes. Those checks are
separate from headless drawing/model tests. A future native accessibility bridge
must be tested with actual assistive technology before claiming compatibility.
