# YSL 0.1 draft

This remains the unchanged 0.1 specification. See the explicit [0.2 extension](../v0.2/ysl.md) for rich documents/styles or PNG asset frames; old documents and wire-v1 document messages remain supported.


Status: experimental. YSL is an original line-oriented presentation language,
separate from YLang. It has no cascading selectors, imports, expressions,
executable code or external resources.

Input MUST be UTF-8 and at most 16384 bytes. LF/CRLF, final CR handling, blank
lines and control-character rejection follow YLang. The first nonblank line
MUST be unindented `@style 0.1`. Remaining unindented lines name groups;
properties MUST be indented exactly two ASCII spaces and follow a group.
Each group and each property within that group may occur only once.
Unknown groups/properties, malformed values and unsupported versions are errors.
There are no comments or inline declarations.

```text
@style 0.1
canvas
  width 76
  padding 2
  paper #101820
heading
  ink #61dafb
  gap 1
```

| Group | Properties and bounds | Defaults |
| --- | --- | --- |
| `canvas` | `width` 20..160 cells; `padding` 0..8 cells; `paper` color | 76; 2; #101820 |
| `text` | `ink` color; `gap` 0..3 rows after each node | #d8e2ec; 1 |
| `heading` | `ink` color; `gap` 0..3 | #61dafb; 1 |
| `section` | `ink` color; `gap` 0..3 | #b4d273; 0 |
| `link` | `ink` color; `gap` 0..3 | #ffa970; 1 |

Integers are parsed as Rust unsigned decimal integers (a leading `+` is
accepted), without fractional units. Colors MUST use `#RRGGBB` with six ASCII
hexadecimal digits, case-insensitive. Omitted groups/properties use defaults;
a header-only sheet is valid. Whitespace separates property and value.

The typed `Stylesheet` contains canvas settings and four `TextStyle`s with
RGB `Color`s and row gaps. Syntax errors report a message, 1-based line and
Unicode-scalar column, and absolute UTF-8 byte offset. Value errors point to
the value; unknown and duplicate properties point to their name.

YNP bundles YSL source with a document. An empty bundled style selects defaults.
A local viewer override may replace it. Parsing styles never initiates network
requests. Rendering validates bounds again for programmatically created styles.
