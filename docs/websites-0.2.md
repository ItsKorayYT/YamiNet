# Native websites in YamiNet 0.2

YLang documents, separate YSL styles, YNP document/PNG frames and the existing
Rust text shaping and software drawing engine form one static website platform.
A browser resolves `yami://FULL-NODE-ID/home`, tries direct TLS before enabled
relay alternatives, verifies the exact destination, and parses the document and
style. Only then do its existing loader workers request referenced PNG keys from
that same authenticated destination. Relays carry the independent end-to-end
TLS stream and cannot decrypt documents or assets. There are no external asset
URLs, font downloads, scripts, tracking services or central publishing server.

Version selection is explicit. Existing `@page 0.1` / `@style 0.1` files keep
original parsing/rendering semantics. To adopt rich syntax, change the header
and escape any literal brackets as documented in the new grammar. An old browser
can reject 0.2 documents; this is not content negotiation or a downgrade path.
YNP v1 document bytes remain unchanged. The separate wire-v2 PNG messages fail
closed on old nodes and produce a visible alternative-text fallback.

Read the complete [YLang grammar](specs/v0.2/ylang.md),
[YSL property reference](specs/v0.2/ysl.md), [YNP asset wire specification](specs/v0.2/ynp-assets.md),
[publishing guide](publishing-sites.md) and [browser controls](desktop-browser.md).
The checked-in [portfolio](../sites/showcase), [personal homepage](../sites/personal)
and [blog](../sites/blog) are working templates, with original PNG illustrations.
The portfolio demonstrates rich text, nested lists, multiple pages, cards,
responsive rows, an image and local links. For an explicit external link, replace
`FULL-NODE-ID` with the other node's complete checked identity:

```text
> yami://FULL-NODE-ID/home | Another creator
```

External links require user activation and a new authenticated navigation.
The site checker validates full identity checksums; advertised labels never
establish trust. No remote site has to be contacted to check/build a site.

## Exact limits

| Resource | Limit / behavior |
| --- | --- |
| YLang source / source line / nesting | 256 KiB / 4096 bytes / 32 levels |
| YLang 0.2 nodes / paragraph | 4096 / 16 KiB |
| YSL source | 16 KiB; bounded unsigned values; duplicate properties rejected |
| Native layout | 4096 nodes and shaped lines; 256 KiB aggregate text; finite 131072 logical-pixel flow height |
| Columns | Up to four equal columns per row; stack below minimum width |
| Typography | Sizes 12..72; weights 400/600; bundled faces; glyph availability depends on installed/bundled fallback fonts |
| PNG format | Static PNG only; palette/grayscale/RGB/RGBA supported, 16-bit samples reduced to 8-bit; APNG rejected |
| One PNG | 256 KiB compressed, width/height <=2048, <=1048576 pixels, <=4 MiB retained RGBA |
| PNG decoder | 8 MiB internal decoder allocation limit; dimensions/output checked before output allocation; metadata is not executed |
| One page | Eight distinct image requests, 1 MiB incoming asset bytes including malformed responses/framing, 16 MiB retained decoded pixels |
| Image concurrency | Sequential per page on the existing two loader workers; no extra resource threads |
| Image retry / deadline | Two connection attempts at most, sharing three seconds per key and the original thirty-second native navigation deadline |
| Cancellation | Queued work removed; flags checked between stages and retries; an in-flight socket can take up to its remaining three-second asset budget |
| Site tooling | 256 pages and 256 assets; missing/invalid references fail validation; exports omit unrelated files |
| Local browser data | 256 bookmarks, 512 optional history entries, 256 KiB file, one writer |

The authoring checker reserves framing overhead conservatively when checking
page PNG byte budgets. Repeated references share a decoded image on that page;
there is no persistent image/document cache. Assets reuse the selected parent
route; route failure shows alternative text until reload rather than new asset
discovery. Missing images remain visible with
their meaningful alternative text. PNGs scale while preserving aspect ratio;
scaling currently uses nearest-neighbor sampling. Layout may move as images
arrive; there is no authored fixed-size placeholder or animation system.

The terminal and P6/PPM debugging backends keep conservative text/alternative-text
fallbacks. Rich cards, inline fonts and PNG painting require the native renderer.
There are no arbitrary selectors, per-element style classes, downloaded fonts,
absolute positioning, forms, audio/video, script runtime, graphical editor,
upload service or full accessibility tree. Unsupported Unicode glyphs can show
a missing-glyph box; Unicode source text is preserved. Bookmarks/history are
not encrypted on disk. Public operation remains experimental and not audited.

## Reproducible validation

See the [completed Linux validation record](validation-0.2.md) for results and
unexecuted physical-network/Windows scenarios.

```sh
cargo test -p ylang -p ysl -p ynp --locked
cargo test -p yami-render --all-features --locked
cargo test -p yami-node --test site_cli --test assets --locked
cargo test -p yami-browser --test rich_site --locked
cargo test -p yami-browser --lib --bins --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

`site_cli` spawns the shipped CLI, creates/checks/exports a template, starts a
real loopback TLS preview process and fetches document and PNG frames.
`rich_site` uses the production browser workers and native renderer against real
TLS sockets and a local DHT; it covers direct and reverse-relay PNG loading,
identity changes between document and asset connections, explicit cross-node
links, broken images, responsive image painting, template/build/bookmark restart,
slow assets, bounded attempts and navigation cancellation. Renderer tests cover
clipping, scrolling, aspect ratio, hover hit regions and layout bounds. Decoder
regressions include valid but excessive dimensions/pixel counts, animated PNG,
truncation, corruption and unsupported data. Library tests exercise exclusive
locking, restarts, folders/titles, bounded optional history and failed writes.
Existing TLS/DHT/relay/reliability scenarios remain in the workspace suite.

For an optional native content capture (no window required):

```sh
YAMINET_CAPTURE_SITE=/tmp/yami-showcase.png cargo test -p yami-browser --test rich_site authored_site_build_identity_bookmark_restart_and_native_render --locked
```

These are local automated simulations, not proof of cross-network reachability.
Windows interactive runtime, physical NATs, Internet asset delivery and provider
firewalls require the [cross-network validation plan](cross-network-validation.md).
No such physical-network or Windows runtime result is claimed for 0.2.
