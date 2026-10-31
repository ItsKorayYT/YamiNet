//! LSP text coordinates and incremental edits.
//!
//! LSP positions are zero-based lines and **UTF-16 code units**. YLang and YSL
//! parsers report UTF-8 byte offsets, so every conversion in the server goes
//! through this module. Positions that point inside a surrogate pair or past
//! the end of a line are safely clamped to a character boundary.

/// An LSP position: zero-based line and UTF-16 character offset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// An LSP range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Self {
        Self { start, end }
    }
}

/// A `textDocument/didChange` content change. `range == None` replaces the
/// whole document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextChange {
    pub range: Option<Range>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextChangeError {
    RangeReversed,
    SizeOverflow,
}

impl std::fmt::Display for TextChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RangeReversed => f.write_str("range start is after range end"),
            Self::SizeOverflow => f.write_str("resulting document is too large"),
        }
    }
}

/// A UTF-8 string with a precomputed line index for fast LSP conversions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDocument {
    text: String,
    /// Byte offset of the start of every line. Always contains at least `0`.
    line_starts: Vec<usize>,
}

impl TextDocument {
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Self { text, line_starts }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn line_count(&self) -> u32 {
        u32::try_from(self.line_starts.len()).unwrap_or(u32::MAX)
    }

    pub fn line_start(&self, line: u32) -> Option<usize> {
        self.line_starts.get(usize::try_from(line).ok()?).copied()
    }

    /// Byte range of the line content, excluding `\n` and a preceding `\r`.
    pub fn line_bounds(&self, line: u32) -> Option<(usize, usize)> {
        let index = usize::try_from(line).ok()?;
        let start = *self.line_starts.get(index)?;
        let raw_end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len());
        let mut end = raw_end;
        if end > start && self.text.as_bytes()[end - 1] == b'\n' {
            end -= 1;
            if end > start && self.text.as_bytes()[end - 1] == b'\r' {
                end -= 1;
            }
        }
        Some((start, end))
    }

    /// Line content without its terminator.
    pub fn line_text(&self, line: u32) -> Option<&str> {
        let (start, end) = self.line_bounds(line)?;
        Some(&self.text[start..end])
    }

    /// Convert an LSP position to a UTF-8 byte offset, clamping out-of-range
    /// positions to the end of the line (or document).
    pub fn offset_at(&self, position: Position) -> usize {
        let Some((start, end)) = self.line_bounds(position.line) else {
            return self.text.len();
        };
        let target = position.character as usize;
        if target == 0 {
            return start;
        }
        let mut units = 0usize;
        for (byte, ch) in self.text[start..end].char_indices() {
            let width = ch.len_utf16();
            if units + width > target {
                // The client asked for a position inside a surrogate pair:
                // snap to the start of that character.
                return start + byte;
            }
            units += width;
            if units == target {
                return start + byte + ch.len_utf8();
            }
        }
        end
    }

    /// Convert a UTF-8 byte offset to an LSP position. Offsets inside a
    /// multi-byte character are clamped to the previous boundary.
    pub fn position_at(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let mut offset = offset;
        while offset > 0 && !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        let index = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let start = self.line_starts[index];
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len());
        let content_end = if end > start && self.text.as_bytes()[end - 1] == b'\n' {
            let mut value = end - 1;
            if value > start && self.text.as_bytes()[value - 1] == b'\r' {
                value -= 1;
            }
            value
        } else {
            end
        };
        let effective = offset.clamp(start, content_end);
        let character = self.text[start..effective]
            .chars()
            .map(|ch| ch.len_utf16())
            .sum::<usize>();
        Position {
            line: u32::try_from(index).unwrap_or(u32::MAX),
            character: u32::try_from(character).unwrap_or(u32::MAX),
        }
    }

    pub fn range_between(&self, start: usize, end: usize) -> Range {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        Range::new(self.position_at(start), self.position_at(end))
    }

    /// Resolve a range to byte offsets.
    pub fn offsets_for(&self, range: Range) -> (usize, usize) {
        (self.offset_at(range.start), self.offset_at(range.end))
    }

    /// Apply one LSP content change. Returns an error without modifying the
    /// document if the range is reversed or the result would overflow.
    pub fn apply(&mut self, change: &TextChange) -> Result<(), TextChangeError> {
        let (start, end) = match change.range {
            None => (0, self.text.len()),
            Some(range) => {
                let (start, end) = self.offsets_for(range);
                if start > end {
                    return Err(TextChangeError::RangeReversed);
                }
                (start, end)
            }
        };
        let new_len = self
            .text
            .len()
            .checked_sub(end - start)
            .and_then(|value| value.checked_add(change.text.len()))
            .ok_or(TextChangeError::SizeOverflow)?;
        let mut text = String::with_capacity(new_len);
        text.push_str(&self.text[..start]);
        text.push_str(&change.text);
        text.push_str(&self.text[end..]);
        *self = Self::new(text);
        Ok(())
    }

    /// Apply a sequence of changes in order, atomically. On error the original
    /// document is unchanged.
    pub fn apply_all(&mut self, changes: &[TextChange]) -> Result<(), TextChangeError> {
        let mut candidate = self.clone();
        for change in changes {
            candidate.apply(change)?;
        }
        *self = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_cover_emoji_and_combining_text() {
        let doc = TextDocument::new("a😀b\néx\n");
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.offset_at(Position::new(0, 0)), 0);
        assert_eq!(doc.offset_at(Position::new(0, 1)), 1);
        // Inside the surrogate pair: snap before the emoji.
        assert_eq!(doc.offset_at(Position::new(0, 2)), 1);
        assert_eq!(doc.offset_at(Position::new(0, 3)), 5);
        assert_eq!(doc.offset_at(Position::new(0, 4)), 6);
        assert_eq!(doc.position_at(5), Position::new(0, 3));
        assert_eq!(doc.position_at(6), Position::new(0, 4));
        // 'é' is one UTF-16 unit and two UTF-8 bytes.
        assert_eq!(doc.offset_at(Position::new(1, 1)), 9);
        assert_eq!(doc.position_at(9), Position::new(1, 1));
        assert_eq!(doc.offset_at(Position::new(1, 99)), 10);
        assert_eq!(doc.offset_at(Position::new(99, 0)), doc.len());
    }

    #[test]
    fn crlf_line_endings_exclude_terminators() {
        let doc = TextDocument::new("a\r\nb\r\n");
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.line_text(0), Some("a"));
        assert_eq!(doc.line_text(1), Some("b"));
        assert_eq!(doc.line_text(2), Some(""));
        assert_eq!(doc.offset_at(Position::new(0, 1)), 1);
        assert_eq!(doc.offset_at(Position::new(1, 0)), 3);
        assert_eq!(doc.position_at(4), Position::new(1, 1));
        // A byte offset inside the CRLF clamps to the line end.
        assert_eq!(doc.position_at(2), Position::new(0, 1));
    }

    #[test]
    fn incremental_changes_are_atomic_and_utf16_aware() {
        let mut doc = TextDocument::new("@page 0.2\n  title A\n= 😀 hello");
        let range = doc.range_between(
            doc.offset_at(Position::new(2, 5)),
            doc.offset_at(Position::new(2, 10)),
        );
        doc.apply(&TextChange {
            range: Some(range),
            text: "world".into(),
        })
        .unwrap();
        assert_eq!(doc.text(), "@page 0.2\n  title A\n= 😀 world");

        // Insert before the emoji using a UTF-16 range.
        let mut emoji_doc = TextDocument::new("😀");
        emoji_doc
            .apply(&TextChange {
                range: Some(Range::new(Position::new(0, 2), Position::new(0, 2))),
                text: "!".into(),
            })
            .unwrap();
        assert_eq!(emoji_doc.text(), "😀!");

        let before = doc.clone();
        let reversed = Range::new(Position::new(2, 5), Position::new(2, 1));
        assert_eq!(
            doc.apply(&TextChange {
                range: Some(reversed),
                text: "x".into()
            }),
            Err(TextChangeError::RangeReversed)
        );
        assert_eq!(doc, before);

        let mut empty = TextDocument::new("abc");
        empty
            .apply(&TextChange {
                range: None,
                text: "full".into(),
            })
            .unwrap();
        assert_eq!(empty.text(), "full");
    }
}
