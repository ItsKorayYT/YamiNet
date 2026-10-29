# Architecture and rendering 0.1 draft

The dependency flow is deliberately small:

```text
yami-browser -> ynp -> transport connection (explicit loopback fetch only)
             -> ynp-tls -> TLS 1.3 to the node ID in a ynps:// address
             -> ylang -> typed Document
             -> ysl -> typed Stylesheet
             -> yami-render -> Layout -> terminal / P6 bitmap

yami-node -> ynp -> in-memory Store or directory Site -> YNP response
          -> ynp-tls -> TLS 1.3 listener/client pinned to a node ID (optional)
          -> yami-identity -> persistent node key, signed site manifest (local files only)

ynp-tls -> rustls (TLS 1.3, ring) + rcgen (certificate encoding)
        -> yami-identity -> node key signs certificate and handshake
        -> ynp -> unchanged frames; loopback policy, or private IPv4 LAN by opt-in
```

The node treats source and style as bounded UTF-8 blobs. A successful fetch
therefore does not assert that a document parses. Parsing and layout happen in
the viewer. A malformed document/style fails visibly with source location; no
fallback interpretation as another markup or execution language is attempted.
All application messages between peers, including missing keys, are YNP.

Layout is a fixed-cell vertical flow. Width and padding come from YSL. Heading
levels retain `=` prefixes, sections retain `::`, links display label and target,
and paragraphs preserve text after whitespace wrapping. Each section increases
child indentation by two cells. Indentation is capped to leave at least two
content cells. Words wrap to the available width, splitting long words on
Unicode scalar boundaries. No hyphenation, grapheme segmentation, bidirectional
layout or Unicode width/shaping algorithm is included. Adjacent source lines
remain distinct paragraph nodes. A style's `gap` inserts rows after a node;
section gaps precede its children. One row/cell unit is shared by layout and
link hit-testing. Links are inert until a caller explicitly chooses to fetch.

`Layout` contains positioned `TextRun`s, ink colors, background, dimensions and
optional link targets on wrapped runs. `link_at(x, y)` resolves an occupied cell
to its target. The terminal backend emits plain text with spacing and indentation;
terminal colors and mouse/keyboard navigation are not implemented. The software
rasterizer uses its own five-by-seven bitmap glyph table in six-by-eight cells,
scaled 2x, with foreground/background colors. Lowercase maps to uppercase,
unsupported characters to a box; the terminal retains the original Unicode.
It writes a standalone P6 PPM with a width of `cells * 12`, height `rows * 16`,
RGB bytes and no external assets or rendering libraries.

Rendering rejects control characters even in manually constructed AST nodes,
invalid style bounds, nesting beyond 32 and layouts exceeding 4096 rows.
Bitmap allocation is checked and limited to 16777216 pixels. A layout can be
valid for terminal output yet exceed the bitmap budget; export then fails.
There is no paging, scrolling or clipping-based recovery in this MVP.

The demo uses two stores/listeners and two node threads. Each node fetches its
peer's document through a distinct loopback TCP connection and serves its own.
The received document and style bytes are compared with the originals. Persistent
sample processes use ports 7301 and 7302 as shown in the README; the automated
demo and tests use ephemeral ports to avoid collisions.

This is a local architecture prototype, not a distributed or secure network.
Node identities and an authenticated TLS channel exist for loopback and, by
explicit opt-in, one private IPv4 LAN address, and the browser opens `ynps://`
addresses through it ([ynp-tls.md](ynp-tls.md)). Address policy is a
transport concern (`ynp_tls::Scope`), so other transports, such as privacy
routing, can be added beside it. Routing semantics, robust resource quotas,
discovery and trust distribution would still be needed before any broader
network use. No existing web engine, markup translation or script runtime is
part of the architecture.
