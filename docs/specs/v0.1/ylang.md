# YLang 0.1 draft

This remains the unchanged 0.1 specification. See the explicit [0.2 extension](../v0.2/ylang.md) for rich documents/styles or PNG asset frames; old documents and wire-v1 document messages remain supported.


Status: experimental. Normative for the `ylang` parser in YamiNet 0.1.0.
YLang is an original inert text language; it has no embedded execution or asset
loading. Syntax and byte limits are independent of the transport.

## Encoding and limits

Input MUST be UTF-8, at most 262144 bytes. LF and CRLF terminate lines; a final
newline is optional. A terminal CR is stripped as well. Other Unicode control
characters, including tabs and embedded CR, MUST be rejected. A physical line
MUST contain at most 4096 UTF-8 bytes excluding its terminator. Blank lines
are ignored, trailing whitespace is ignored, and indentation MUST be pairs of
ASCII spaces. At most 32 nested section headers are supported; a deepest
section can still contain leaf nodes. Exceeding a limit is a syntax error.

## Grammar

The first nonblank line MUST be unindented `@page 0.1`. It is immediately
followed by metadata lines indented exactly two spaces. Metadata ends at the
first nonblank unindented body line. `title <text>` is required exactly once;
`language <text>` is optional and cannot repeat. Values are trimmed and cannot
be empty. Language is descriptive text, not a validated language tag. Unknown
metadata keys and unsupported page versions are errors.

Body lines have the following forms:

| Form | AST node |
| --- | --- |
| `= Text` through `====== Text` | Heading with level 1..6 and nonempty text |
| `:: Name` | Named section with zero or more child nodes |
| `> ynp://address/key | Label` | Link with target and nonempty label |
| `Ordinary text` | Paragraph containing one source line |
| `\@literal` | Literal paragraph after removing one leading backslash |

Only sections can contain child nodes. Indentation increases by exactly two
spaces on entering children and returns to an existing level on dedent.
Headings and links are leaves. Paragraph source lines do not merge.
Reserved initial `@`, `:`, `>` and `=` characters MUST use valid syntax or be
escaped with a leading backslash. A bare backslash is an error. Text is plain;
there are no inline tags, entities, interpolation, attributes or comments.

A link delimiter is the first literal ` | `. At parse time its target MUST
start with `ynp://`, have a nonempty remainder and contain no whitespace.
The document parser intentionally does not resolve or connect to targets.
At navigation time the YNP URL rules apply: numeric loopback address, nonzero
port and a valid document key. Invalid destinations can therefore be represented
in an AST but cannot be fetched. No links are fetched automatically.

## AST and diagnostics

`Document` contains typed `Metadata` and a list of `Node`s. `NodeKind` is an enum:
`Heading { level, text }`, `Section { name, children }`, `Paragraph(text)`, or
`Link { target, label }`. All text is owned UTF-8.

Each node has a `Span`: absolute UTF-8 byte `start`/exclusive `end`, and 1-based
Unicode-scalar `line` and `column`. It covers the physical content line after
indentation, including any original trailing whitespace; section spans cover
only the section header. A `SyntaxError` contains a span and descriptive
message. Structural errors point to the offending content line; indentation
and control errors point at the actual invalid position. Missing metadata points
to the page directive. Empty/oversized documents point to byte 0, line 1,
column 1. CRLF bytes count toward absolute offsets.

See `examples/welcome.ylang` for a complete document.
