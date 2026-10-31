# Syntax highlighting grammars

These files are optional; language-server features do not depend on them.

## TextMate

- [`textmate/ylang.tmLanguage.json`](textmate/ylang.tmLanguage.json) — scope
  `source.ylang`, file type `ylang`
- [`textmate/ysl.tmLanguage.json`](textmate/ysl.tmLanguage.json) — scope
  `source.ysl`, file type `ysl`

Use them with TextMate, Sublime Text, or any editor/extension that loads
TextMate grammars. Register the scope names as `source.ylang` and `source.ysl`
so the sample LSP client selectors match.

The grammars highlight:

- `@page`/`@style` headers and version numbers
- YLang metadata, headings, sections, cards/rows/groups/lists/steps, links,
  images, separators and inline spans
- YSL groups, properties, colors and common keyword/number values

They are deliberately small and do not implement every parser error case.

## Vim / Neovim

Copy or link the contents of [`vim/`](vim/) into a runtimepath directory:

```sh
mkdir -p ~/.vim/syntax ~/.vim/ftdetect
cp vim/syntax/ylang.vim vim/syntax/ysl.vim ~/.vim/syntax/
cp vim/ftdetect/yami.vim ~/.vim/ftdetect/
```

For Neovim, use `~/.config/nvim/syntax/` and `~/.config/nvim/ftdetect/`
instead. The `ftdetect` file maps `.ylang` and `.ysl` to filetypes `ylang` and
`ysl`.

## Helix / tree-sitter

No tree-sitter grammar is bundled. Helix users can still run the language
server (see [`../editors/helix/languages.toml`](../editors/helix/languages.toml));
without a grammar, highlighting falls back to plain text.
