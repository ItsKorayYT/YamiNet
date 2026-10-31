# YLang 0.2 grammar

`@page 0.2` explicitly selects this grammar. `@page 0.1` retains its original
semantics, including literal new-looking markers. Titles/language metadata,
two-space indentation, bounded source (256 KiB), lines (4096 bytes) and nesting
(32 levels) remain. Version 0.2 adds a 4096-node and 16 KiB paragraph limit.

```text
@page 0.2
  title My original website
  language en
= Hello [strong: YamiNet]
This paragraph uses [em: emphasis] and [code: literal code].
Adjacent ordinary lines flow into the same paragraph.

A blank line starts another paragraph.
:: card Welcome
  A semantic card with its own bounded YSL styling.
  > /about | About this site
:: list
  - Unordered item
  - Another item
    :: steps
      + First step
      + Next step
:: row Projects
  :: card One
    First project.
  :: card Two
    Second project.
! cover | A descriptive account of the cover image
---
```

`:: list` contains `- text` items; `:: steps` contains `+ text` items numbered by
the renderer. Items may contain nested blocks. Markers outside their explicit
list blocks are errors; this is not Markdown interpretation. Inline spans are
`[strong: text]`, `[em: text]` and `[code: text]`, cannot nest and need nonempty
text. Escape literal brackets/backslashes as `\[`, `\]`, `\\`. The original
leading `\` line escape produces literal paragraph text without inline parsing.
Headings and list items support the same typed inline spans. Links have plain,
nonempty labels. `---` is a separator, not an arbitrary directive.

`:: card`, `:: row` and `:: group`, with an optional plain title, are typed
semantic containers. Other `:: Name` sections retain their section meaning.
Rows contain at most four direct child blocks and stack on compact viewports.
There are no arbitrary class names, selectors, scripts or executable nodes.

`> /about | About` names a page key on the current node. `> yami://FULL-ID/home |
Other site` explicitly names another identity; the browser performs full ID and
transport verification on activation. Explicit `ynps://` and legacy loopback
`ynp://` links remain subject to browser policy. Parser shape validation is not
a cryptographic identity proof. Author checks validate full identity checksums.

Images use `! asset-key | meaningful alternative text`. Keys contain 1..64 ASCII
letters, digits, underscores or hyphens; no paths, extensions, remote identities
or URLs. The key selects `assets/<key>.png` on the authenticated parent node.
Images are inert references; the browser's bounded same-node resource policy
controls requests. Missing images retain alternative text. No external resources,
fonts, scripts or tracking endpoints are implied by this grammar.

YNS 0.1 also permits explicit `yami://@alias/home`,
`yami://label.FULL-OWNER-ID.yns/home`, and `yami://service.yami/home` link targets.
These links remain inert until selected. Syntax acceptance establishes no trust:
the browser validates names and resolves them to full authenticated identities.
See [YNS](../../yns.md) for canonical spelling and trust rules.
