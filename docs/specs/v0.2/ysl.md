# YSL 0.2 native presentation

`@style 0.2` explicitly selects pixel-based native presentation. `@style 0.1`
keeps its original cell widths, row gaps, defaults and rendering path. The new
syntax remains the original group/property syntax with two-space indentation,
strict duplicate rejection, 16 KiB source bounds and line/column diagnostics.
No selectors, positioning, expressions, scripts, external fonts or compatibility
with another styling language are provided.

```text
@style 0.2
canvas
  width 960
  padding 24
  paper #101820
text
  ink #d8e2ec
  size 18
  weight 400
  align left
  line-height 155
  gap 14
heading
  ink #61dafb
  size 40
  weight 600
  gap 20
section
  ink #b4d273
  size 23
link
  ink #ffa970
  hover #ffd5a0
  active #61dafb
card
  paper #192531
  ink #d8e2ec
  border #405970
  border-width 1
  radius 14
  padding 20
  gap 20
row
  gap 20
  min-width 240
image
  width 800
  height 1024
  align center
  gap 16
```

| Group/property | Supported values (logical pixels unless stated) |
| --- | --- |
| canvas width / padding | 240..1440 / 0..64; actual content always fits viewport |
| canvas paper | #RRGGBB |
| text, heading, section, link ink | #RRGGBB |
| text, heading, section, link size | 12..72 |
| text, heading, section, link weight | 400 regular or 600 strong; bundled fonts |
| text, heading, section, link align | left, center, right |
| text, heading, section, link line-height | 100..220 percent of font size |
| text, heading, section, link gap | 0..64 |
| link hover / active | #RRGGBB, applied to the actual shaped link rectangles |
| card paper / ink / border | #RRGGBB |
| card border-width / radius / padding / gap | 0..4 / 0..32 / 0..48 / 0..64 |
| row gap / min-width | 0..64 / 160..640 |
| image width / height | maximum 16..1440 / 16..1536, preserving aspect ratio |
| image align / gap | left, center, right / 0..64 |

Heading levels progressively reduce the configured heading size (minimum 12).
Containers have deterministic vertical flow. A `:: row` has at most four equal
columns and stacks when each column cannot meet min-width plus bounded gaps.
No absolute positioning or overlapping arbitrary boxes are possible. Cards use
one shared declarative style; arbitrary per-node selectors/classes are out of
scope. Nested cards are limited by document depth and layout budgets.

Inline strong uses bundled Open Sans semibold; emphasis and code use bundled,
licensed DejaVu oblique and monospace faces. No font downloads occur. Content is
shaped with the existing cosmic-text engine; visual glyph bounds determine link
hit-testing. The native display list is bounded by node/text/line budgets and
finite document height. The terminal/P6 tool remains a conservative text fallback
for 0.2 documents; rich cards, inline font changes and images use native rendering.
