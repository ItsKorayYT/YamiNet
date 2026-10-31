# Editor integration

`yami-lsp` is an ordinary stdio language server. Editors only need to:

1. Start the `yami-lsp` executable with no arguments.
2. Associate `*.ylang` with language id `ylang` and `*.ysl` with `ysl`.
3. Optionally pass `initializationOptions.siteRoot` (absolute path or `file://`
   URI) so local page links resolve against the intended website directory.

Build the release binary first:

```sh
cargo build --release --manifest-path tools/yami-lsp/Cargo.toml --offline
```

Then use the absolute path to `tools/yami-lsp/target/release/yami-lsp`
(`yami-lsp.exe` on Windows). All snippets below use `yami-lsp` on `PATH`; replace
it with the absolute path if the binary is not installed globally.

The server has no editor-specific runtime and no JavaScript component. If an
editor lacks a built-in LSP client, any generic client that can launch a
command and speak LSP over stdio works.

## Neovim (0.11+)

```lua
-- ~/.config/nvim/init.lua
vim.filetype.add({
  extension = { ylang = "ylang", ysl = "ysl" },
})

vim.lsp.config("yami_lsp", {
  cmd = { "yami-lsp" },
  filetypes = { "ylang", "ysl" },
  root_markers = { "default.ysl", ".git" },
  init_options = {
    siteRoot = vim.fn.getcwd(), -- or an explicit website directory
  },
})
vim.lsp.enable("yami_lsp")
```

For Neovim 0.10, use `vim.lsp.start` from a `FileType` autocmd:

```lua
vim.filetype.add({
  extension = { ylang = "ylang", ysl = "ysl" },
})

vim.api.nvim_create_autocmd("FileType", {
  pattern = { "ylang", "ysl" },
  callback = function(args)
    local root = vim.fs.root(args.buf, { "default.ysl", ".git" })
      or vim.fn.getcwd()
    vim.lsp.start({
      name = "yami-lsp",
      cmd = { "yami-lsp" },
      root_dir = root,
      init_options = { siteRoot = root },
    }, { bufnr = args.buf })
  end,
})
```

Copy [`editors/neovim/yami.lua`](../editors/neovim/yami.lua) if you prefer a
separate file and `require` it. The Vim syntax files in
[`syntax/vim/`](../syntax/vim/) provide basic highlighting.

## Helix

Merge into `~/.config/helix/languages.toml`:

```toml
[language-server.yami-lsp]
command = "yami-lsp"

[language-server.yami-lsp.config]
siteRoot = "."

[[language]]
name = "ylang"
scope = "source.ylang"
file-types = ["ylang"]
language-servers = ["yami-lsp"]

[[language]]
name = "ysl"
scope = "source.ysl"
file-types = ["ysl"]
language-servers = ["yami-lsp"]
```

Helix uses tree-sitter for highlighting. No tree-sitter grammar is bundled
here; without one, Helix still runs the language server but may fall back to
plain text. The TextMate and Vim grammars in `syntax/` can be adapted if you
want highlighting. A ready-to-copy file is at
[`editors/helix/languages.toml`](../editors/helix/languages.toml).

## Emacs (Eglot, built in since Emacs 29)

```elisp
(define-derived-mode ylang-mode prog-mode "YLang"
  "Major mode for YamiNet YLang documents."
  (setq-local indent-tabs-mode nil))

(define-derived-mode ysl-mode prog-mode "YSL"
  "Major mode for YamiNet YSL stylesheets."
  (setq-local indent-tabs-mode nil))

(add-to-list 'auto-mode-alist '("\\.ylang\\'" . ylang-mode))
(add-to-list 'auto-mode-alist '("\\.ysl\\'" . ysl-mode))
(add-to-list 'eglot-server-programs '(ylang-mode . ("yami-lsp")))
(add-to-list 'eglot-server-programs '(ysl-mode . ("yami-lsp")))

;; Optional: tell the server which website directory to use. Eglot delivers
;; this through workspace/didChangeConfiguration after initialization.
(setq-default eglot-workspace-configuration
              '(:yami (:siteRoot "/absolute/path/to/website")))
```

Open a page and run `M-x eglot`. A complete sample is at
[`editors/emacs/yami-lsp.el`](../editors/emacs/yami-lsp.el).

## Vim (vim-lsp)

```vim
" ~/.vimrc, after vim-lsp is installed
let g:lsp_settings = {
\  'yami-lsp': {
\    'command': ['yami-lsp'],
\    'initialization_options': { 'siteRoot': getcwd() },
\  },
\}
let g:lsp_settings_filetype_ylang = ['yami-lsp']
let g:lsp_settings_filetype_ysl = ['yami-lsp']

augroup yami_filetypes
  autocmd!
  autocmd BufRead,BufNewFile *.ylang setfiletype ylang
  autocmd BufRead,BufNewFile *.ysl setfiletype ysl
augroup END
```

The `siteRoot` value is resolved when the server starts. If you work with
multiple websites in one Vim session, configure it per project or start the
server from the website directory.

## Sublime Text (LSP package)

Install the [LSP](https://packagecontrol.io/packages/LSP) package, then add the
settings from
[`editors/sublime-lsp/LSP-yami.sublime-settings`](../editors/sublime-lsp/LSP-yami.sublime-settings):

```json
{
  "clients": {
    "yami-lsp": {
      "command": ["/absolute/path/to/yami-lsp"],
      "selector": "source.ylang, source.ysl",
      "enabled": true,
      "initializationOptions": {
        "siteRoot": "/absolute/path/to/website"
      }
    }
  }
}
```

The TextMate grammars under [`syntax/textmate/`](../syntax/textmate/) can be
installed in a Sublime syntax package so `source.ylang` / `source.ysl` selectors
match.

## Generic client configuration

Any client that accepts a command plus LSP `initializationOptions` can use:

```json
{
  "command": ["/absolute/path/to/yami-lsp"],
  "args": [],
  "filetypes": ["ylang", "ysl"],
  "initializationOptions": {
    "siteRoot": "/absolute/path/to/website"
  }
}
```

The server specifies its capabilities in the `initialize` result; nothing else
is required. `siteRoot` may also be a `file:///...` URI. It is optional: the
first workspace folder is used when present, otherwise the directory of the
opened document.

## VS Code

VS Code requires an LSP client extension for external language servers, but
**this repository ships no VS Code extension and no JavaScript application**.
Use any generic external-LSP extension and configure:

- command: absolute path to `yami-lsp`
- file extensions: `.ylang`, `.ysl`
- initialization options: `{ "siteRoot": "${workspaceFolder}" }` (or the
  website directory)

Most such extensions expand `${workspaceFolder}`; otherwise use a literal
absolute path. The TextMate grammars can be registered by the same extension
or a syntax-only extension.

## Site directory selection

Definitions resolve local page keys relative to, in order:

1. `initializationOptions.siteRoot` (or `siteRootUri`).
2. The first workspace folder (`rootUri`, `rootPath` or `workspaceFolders`).
3. The opened document's directory.

For a repository checkout where the website is in `sites/blog`, set
`siteRoot` to `.../sites/blog`; opening the repository root as the workspace
would otherwise make `/about` resolve to `<repo>/about.ylang` (which does not
exist).

## Troubleshooting

- **No diagnostics**: verify the file extension/language id and that the
  client sends `textDocument/didOpen` with the buffer text. The server never
  reads the file from disk.
- **No local definitions**: confirm the document header is `@page 0.2`, the
  link target is a single key such as `/about`, and `siteRoot` contains
  `about.ylang`. YLang 0.1 has no local page links.
- **Binary not found**: use an absolute path; editors do not always inherit your
  shell `PATH`.
- **Windows paths**: use an absolute Windows path or a `file:///C:/...` URI for
  `siteRoot`.
