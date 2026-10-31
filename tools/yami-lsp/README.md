# yami-lsp — native YLang/YSL language server

A standalone Rust [Language Server Protocol](https://microsoft.github.io/language-server-protocol/)
implementation for YamiNet's native document languages: **YLang 0.1/0.2** and
**YSL 0.1/0.2**. It is a normal stdio LSP server, so any editor with an external
LSP client can launch it. No browser, JavaScript application or YamiNet node is
required.

The server reuses the repository's existing `ylang::parse` and `ysl::parse`
functions, so diagnostics and version behaviour are exactly those of YamiNet.
It does not define a second grammar.

## Features

| Feature | LSP method | Notes |
| --- | --- | --- |
| Syntax diagnostics | `textDocument/publishDiagnostics` | Parses the in-memory buffer on open/change; never reads the file from disk. |
| Autocompletion | `textDocument/completion` | Version-aware directives, metadata, headings, sections, cards/rows/groups, lists/steps, links, images, inline spans, YSL groups/properties/values and local page keys. |
| Hover documentation | `textDocument/hover` | Concise Markdown from the v0.1/v0.2 specifications with the same bounds as the parsers. |
| Local definitions | `textDocument/definition` | `> /about | About` resolves to `about.ylang` inside the selected site directory only. |
| Lifecycle | `initialize`, `initialized`, `shutdown`, `exit` | Standard LSP lifecycle, including `exit` code 0 after shutdown and 1 otherwise. |

Supported transport: JSON-RPC 2.0 over stdin/stdout with `Content-Length`
framing. Positions are **UTF-16 code units** as required by LSP, including
astral-plane characters such as emoji. Batch messages, notifications and
malformed messages are handled without terminating the session.

### Deliberate non-features

- No network requests, telemetry, analytics or private-data collection.
- No browser integration, HTML/CSS/JS rendering or asset loading.
- No arbitrary code execution, expressions, selectors or scripts.
- No modifications to YLang/YSL grammars or to the existing parsers.
- No file writes. The server only resolves local page *paths* for navigation.
- No formatting, rename, code actions or semantic tokens in this version.

## Build and run

From the repository root:

```sh
cargo build --release --manifest-path tools/yami-lsp/Cargo.toml --offline
```

The binary is `tools/yami-lsp/target/release/yami-lsp` (`.exe` on Windows).
Run it with no arguments; it speaks LSP on stdin/stdout:

```sh
./tools/yami-lsp/target/release/yami-lsp
```

`stderr` is reserved for fatal transport errors. `stdout` contains only
protocol frames.

The nested Cargo workspace is independent of the repository workspace, so the
root `Cargo.toml`, `Cargo.lock`, CI and other crates are untouched. The only
dependencies are path dependencies on `crates/ylang` and `crates/ysl`; the JSON
implementation and transport are in-tree and have no registry dependencies.

## Editor setup

See [docs/editors.md](docs/editors.md) for copy-paste configurations for
Neovim, Helix, Emacs (Eglot), Vim, Sublime Text and generic LSP clients.
Sample files are under [`editors/`](editors/), and TextMate/Vim syntax
definitions are under [`syntax/`](syntax/).

The two important client settings are:

- **File types**: `ylang` and `ysl`, mapped from `.ylang` and `.ysl`.
- **Initialization options**: `{ "siteRoot": "/absolute/path/to/website" }`.
  The site root is the directory containing `home.ylang`, `about.ylang`,
  `default.ysl`, `assets/`, and so on.

If `siteRoot` is omitted, the first workspace folder is used; if there is no
workspace, the directory of the opened document is used. `siteRoot` can also
be changed later with `workspace/didChangeConfiguration`:

```json
{ "settings": { "yami": { "siteRoot": "/absolute/path/to/website" } } }
```

## Local page navigation

YLang 0.2 local links are single-component page keys:

```ylang
> /about | About this site
```

The server resolves a definition only when all of the following hold:

1. The document is YLang 0.2 (0.1 links are `ynp://` only).
2. The key is a valid `ylang::valid_key` value: 1..128 ASCII letters, digits,
   `_` or `-`. This rejects `.`, `..`, slashes, backslashes, URLs, percent
   escapes and whitespace.
3. The site root is configured and exists.
4. `<siteRoot>/<key>.ylang` exists, is a regular file, and its canonical path is
   still inside the canonical site root.

Symlinks are resolved with `canonicalize` before the containment check, so a
symlink that points outside the site is never followed for definitions.
Page-key completion skips symlinks and hidden files, lists at most 512 keys and
never performs a network request. Remote `yami://`, `ynp://` and `ynps://`
targets are never resolved or fetched; a definition request for them returns
`null`.

Missing pages, unreadable directories, invalid keys and path traversal all
return `null` rather than an error. The parser itself reports traversal-shaped
links as syntax errors.

## Limits and resource bounds

| Resource | Bound |
| --- | --- |
| JSON-RPC message body | 4 MiB |
| Single header line / header block | 8 KiB / 16 KiB |
| Retained text per open document | 1 MiB (4× YLang's 256 KiB limit) |
| Open documents | 64 |
| Content changes per `didChange` | 64 |
| Batch requests | 64 |
| Completion items per response | 200 |
| Local page keys per site | 512 collected, at most 200 completion items per response |
| Diagnostics | The parser's first error (one per document) |

Documents that exceed 1 MiB are not retained; the server publishes a
`limits` diagnostic instead. A full-text `didChange` can shrink such a buffer
back within bounds. JSON nesting is limited to 64 levels and unknown methods
receive `MethodNotFound` rather than being executed.

## Protocol details

- Advertised sync kind: incremental (`change: 2`), open/close notifications.
- Advertised `positionEncoding`: `utf-16`.
- Completion trigger characters: `@ : > ! [ / = - + #` and space.
- Completion items use `textEdit` ranges, so typed prefixes are replaced
  instead of duplicated.
- A malformed JSON body returns JSON-RPC `Parse error` (`-32700`) with `id:
  null`; the session continues.
- A malformed header block is drained so the next valid frame can still be
  processed.
- Notifications before `initialize` (except `exit`) are ignored. Requests before
  `initialize` return `ServerNotInitialized` (`-32002`). Requests after
  `shutdown` return `InvalidRequest` (`-32600`).
- Unknown notifications are ignored; unknown requests return `-32601`.
- The server never logs document content, path keys or request data.

## Testing

```sh
cargo fmt --manifest-path tools/yami-lsp/Cargo.toml --all -- --check
cargo clippy --manifest-path tools/yami-lsp/Cargo.toml --all-targets --offline -- -D warnings
cargo test --manifest-path tools/yami-lsp/Cargo.toml --offline
```

The test suite covers:

- Diagnostics from both real parsers, including UTF-16 columns after emoji.
- Incremental document changes, unsaved buffers and version tracking.
- Completion/hover version gating (0.1 never receives 0.2-only suggestions).
- Local definition lookup, missing files, path traversal and symlink escapes.
- Malformed JSON-RPC bodies/framing and graceful shutdown, via a real spawned
  `yami-lsp` process.
- Every `.ylang` and `.ysl` file under `sites/` and `examples/` opens without
  diagnostics; the blog sample is exercised for navigation.

## Layout

```text
tools/yami-lsp/
├── Cargo.toml                 # independent nested workspace
├── crates/yami-lsp/
│   ├── src/
│   │   ├── main.rs            # process entry point
│   │   ├── server.rs          # lifecycle, dispatch, response shaping
│   │   ├── transport.rs       # Content-Length framing
│   │   ├── json.rs            # bounded JSON parser/serializer
│   │   ├── text.rs            # UTF-16 positions and edits
│   │   ├── documents.rs       # in-memory, bounded document store
│   │   ├── spec.rs            # versioned language facts and docs
│   │   ├── diagnostics.rs     # parser errors -> LSP diagnostics
│   │   ├── completion.rs      # context-aware completion
│   │   ├── hover.rs           # hover documentation
│   │   ├── definition.rs      # local page definitions
│   │   └── site.rs            # sandboxed site roots and file URIs
│   └── tests/                 # stdio protocol and sample-corpus tests
├── editors/                   # sample editor configurations
├── syntax/                    # TextMate and Vim grammars
└── docs/                      # editor integration notes
```

## License

GPL-3.0-only, matching the YamiNet repository.
