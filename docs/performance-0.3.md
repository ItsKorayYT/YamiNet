# Alpha performance and resource scope

Run `cargo bench -p yami-browser --bench desktop --locked`. The standalone Rust
benchmark uses the production shaper, layout, rasterizer, PNG decoder and RAM
cache; it requires no network or graphical display. It reports simple layout,
600-paragraph layout/scrolled drawing, eight-image layout/drawing and repeated
PNG decode. These are local CPU timings, not network/page-load or UI latency claims.

Observed Linux x86_64 release results (2026-10-30): Rust 1.90.0,
Debian 13 container, AMD EPYC 9V74, 3 visible CPUs, 1000×800 viewport, bundled
fonts, checked-in cover PNG. Average over 10–100 iterations, no statistical
confidence interval or isolated CPU scheduling:

| Operation | Microseconds/iteration |
| --- | ---: |
| Simple layout | 33.540 |
| Long layout (600 paragraphs) | 34602.083 |
| PNG decode | 366.693 |
| Cached PNG decode | 0.155 |
| Eight-image layout | 0.685 (no text shaping in this fixture) |
| Eight-image draw | 6631.607 |
| Long page draw at scroll 10000 | 863.243 |

The decode comparison supports reusing an already-decoded identical PNG. The
cache is per browser Loader, shared by its two workers, limited to 16 entries and
16 MiB including encoded+decoded bytes. Keys contain the exact cryptographic
owner, resource key and fetched encoded bytes. Network responses still undergo
authentication and fresh retrieval before cache lookup, so a same-key content
change cannot return stale pixels. Cache hits still count against each page's
image budget. No document/image data is persisted to disk. Cache entries may
outlive tabs within the stated bound; the entire cache drops with its workers.

One native layout is retained for the active document/width/DPI/revision. Blank,
loading and closed-tab states release the old layout and its image references.
Other tabs retain bounded model pages (at most 12 tabs, existing per-page decoded
image bound 16 MiB); tab close cancels queued/resource work. In-flight sockets
retain existing deadlines, and obsolete results cannot replace current content.
Cache eviction has a weak-reference cleanup regression. Duplicate image keys on
a page are already deduplicated; requests in different tabs are not coalesced.

Mouse movement redraws only on changed hover targets or active scrollbar drag;
idle windows use winit Wait, and scrolling animates until settled. Cached layouts
avoid shaping on each scroll frame, and the renderer paints only visible text
blocks/images. Long-document initial layout still runs on the UI thread and may
briefly stall near maximum bounds; remote retrieval, parsing and image decoding
remain on bounded loader workers. The benchmark does not establish responsiveness
on all hardware. Full-window rasterization, nearest-neighbor image sampling and
relayout when switching tabs remain alpha limitations. No rendering engine was
replaced, and no persistent content cache was introduced.
