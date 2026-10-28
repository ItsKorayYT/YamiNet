//! Native cell layout, terminal output and a software bitmap rasterizer.
//! No markup translation, browser engine, remote assets, or executable content.
use std::{
    fmt,
    io::{self, Write},
};
use ylang::{Document, Node, NodeKind};
use ysl::{Color, Stylesheet, TextStyle};

pub const MAX_ROWS: usize = 4096;
const MAX_PIXELS: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError(pub &'static str);
impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for RenderError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub x: usize,
    pub y: usize,
    pub text: String,
    pub ink: Color,
    pub target: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub width: usize,
    pub height: usize,
    pub paper: Color,
    pub runs: Vec<TextRun>,
}
impl Layout {
    /// Hit-testing uses cell coordinates, not pixels. Following a link is explicit.
    pub fn link_at(&self, x: usize, y: usize) -> Option<&str> {
        self.runs
            .iter()
            .find(|run| run.y == y && x >= run.x && x < run.x + run.text.chars().count())
            .and_then(|run| run.target.as_deref())
    }
    pub fn terminal(&self) -> String {
        let mut result = String::new();
        let mut row = 0;
        for run in &self.runs {
            while row < run.y {
                result.push('\n');
                row += 1;
            }
            result.push_str(&" ".repeat(run.x));
            result.push_str(&run.text);
            result.push('\n');
            row += 1;
        }
        result
    }
    /// P6 portable pixmap, 2x bitmap glyphs, six by eight cells. A standalone
    /// raster artifact; it is not an existing web or text rendering engine.
    pub fn write_ppm(&self, mut output: impl Write) -> io::Result<()> {
        let width = self
            .width
            .checked_mul(12)
            .ok_or_else(|| io::Error::other("image width overflow"))?;
        let height = self
            .height
            .checked_mul(16)
            .ok_or_else(|| io::Error::other("image height overflow"))?;
        let count = width
            .checked_mul(height)
            .filter(|n| *n <= MAX_PIXELS)
            .ok_or_else(|| io::Error::other("bitmap exceeds 16 megapixels"))?;
        let Color(r, g, b) = self.paper;
        let mut pixels = vec![0u8; count * 3];
        for pixel in pixels.chunks_exact_mut(3) {
            pixel.copy_from_slice(&[r, g, b]);
        }
        for run in &self.runs {
            for (column, ch) in run.text.chars().enumerate() {
                let glyph = glyph(ch);
                for (gy, bits) in glyph.iter().enumerate() {
                    for gx in 0..5 {
                        if bits & (1 << (4 - gx)) != 0 {
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    let x = (run.x + column) * 12 + gx * 2 + dx;
                                    let y = run.y * 16 + gy * 2 + dy;
                                    if x < width && y < height {
                                        let index = (y * width + x) * 3;
                                        let Color(r, g, b) = run.ink;
                                        pixels[index..index + 3].copy_from_slice(&[r, g, b]);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        write!(output, "P6\n{width} {height}\n255\n")?;
        output.write_all(&pixels)
    }
}

pub fn layout(document: &Document, sheet: &Stylesheet) -> Result<Layout, RenderError> {
    if !(20..=160).contains(&sheet.width)
        || sheet.padding > 8
        || [
            sheet.text.gap,
            sheet.heading.gap,
            sheet.section.gap,
            sheet.link.gap,
        ]
        .iter()
        .any(|n| *n > 3)
    {
        return Err(RenderError("stylesheet outside supported layout bounds"));
    }
    let mut builder = Builder {
        sheet,
        result: Layout {
            width: sheet.width,
            height: 0,
            paper: sheet.paper,
            runs: Vec::new(),
        },
        row: sheet.padding,
    };
    builder.nodes(&document.nodes, 0)?;
    builder.result.height = (builder.row + sheet.padding).clamp(1, MAX_ROWS);
    Ok(builder.result)
}
struct Builder<'a> {
    sheet: &'a Stylesheet,
    result: Layout,
    row: usize,
}
impl Builder<'_> {
    fn emit(
        &mut self,
        text: &str,
        depth: usize,
        style: TextStyle,
        target: Option<&str>,
    ) -> Result<(), RenderError> {
        if text.chars().any(char::is_control) {
            return Err(RenderError("control character in AST"));
        }
        let x = self.sheet.padding + (depth * 2).min(self.sheet.width - self.sheet.padding * 2 - 2);
        let width = self.sheet.width - self.sheet.padding - x;
        for line in wrap(text, width) {
            if self.row + self.sheet.padding >= MAX_ROWS {
                return Err(RenderError("layout exceeds 4096 rows"));
            }
            self.result.runs.push(TextRun {
                x,
                y: self.row,
                text: line,
                ink: style.ink,
                target: target.map(str::to_owned),
            });
            self.row += 1;
        }
        self.row += style.gap;
        Ok(())
    }
    fn nodes(&mut self, nodes: &[Node], depth: usize) -> Result<(), RenderError> {
        if depth > ylang::MAX_DEPTH {
            return Err(RenderError("AST nesting exceeds 32 levels"));
        }
        for node in nodes {
            match &node.kind {
                NodeKind::Heading { level, text } => self.emit(
                    &format!("{} {text}", "=".repeat(usize::from(*level).min(6))),
                    depth,
                    self.sheet.heading,
                    None,
                )?,
                NodeKind::Paragraph(text) => self.emit(text, depth, self.sheet.text, None)?,
                NodeKind::Section { name, children } => {
                    self.emit(&format!(":: {name}"), depth, self.sheet.section, None)?;
                    self.nodes(children, depth + 1)?;
                }
                NodeKind::Link { target, label } => self.emit(
                    &format!("> {label} ({target})"),
                    depth,
                    self.sheet.link,
                    Some(target),
                )?,
            }
        }
        Ok(())
    }
}
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut count = 0;
    for word in text.split_whitespace() {
        let length = word.chars().count();
        if count > 0 {
            if count + 1 + length <= width {
                current.push(' ');
                count += 1;
            } else {
                lines.push(std::mem::take(&mut current));
                count = 0;
            }
        }
        for ch in word.chars() {
            if count == width {
                lines.push(std::mem::take(&mut current));
                count = 0;
            }
            current.push(ch);
            count += 1;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

// Original tiny 5x7 monospace glyph table. Lowercase maps to uppercase;
// unsupported Unicode scalars use a visible box. Layout preserves Unicode text.
fn glyph(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 14],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 27, 17],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ' ' => [0; 7],
        '.' => [0, 0, 0, 0, 0, 6, 6],
        ',' => [0, 0, 0, 0, 6, 6, 4],
        ':' => [0, 6, 6, 0, 6, 6, 0],
        ';' => [0, 6, 6, 0, 6, 6, 4],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '>' => [16, 8, 4, 2, 4, 8, 16],
        '<' => [1, 2, 4, 8, 4, 2, 1],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '\\' => [16, 8, 8, 4, 2, 2, 1],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        '?' => [14, 17, 1, 2, 4, 0, 4],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '[' => [14, 8, 8, 8, 8, 8, 14],
        ']' => [14, 2, 2, 2, 2, 2, 14],
        '\'' => [4, 4, 2, 0, 0, 0, 0],
        '"' => [10, 10, 10, 0, 0, 0, 0],
        '#' => [10, 31, 10, 10, 31, 10, 0],
        '|' => [4, 4, 4, 4, 4, 4, 4],
        '@' => [14, 17, 23, 21, 23, 16, 14],
        _ => [31, 17, 17, 17, 17, 17, 31],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapping_nesting_styles_and_hit_testing() {
        let doc = ylang::parse("@page 0.1\n  title T\n= Hello\n:: Section\n  > ynp://127.0.0.1:9/a | A link\n  ééééééééééééééééééééééé").unwrap();
        let style = Stylesheet {
            width: 20,
            ..Stylesheet::default()
        };
        let result = layout(&doc, &style).unwrap();
        assert_eq!(result.runs[0].ink, style.heading.ink);
        assert_eq!(result.runs[0].x, 2);
        assert_eq!(result.runs[2].x, 4);
        for run in &result.runs {
            assert!(run.x + run.text.chars().count() <= 18);
        }
        let link = &result.runs[2];
        assert_eq!(result.link_at(link.x, link.y), Some("ynp://127.0.0.1:9/a"));
        assert_eq!(result.link_at(0, link.y), None);
        assert!(result.terminal().contains("Hello"));
    }
    #[test]
    fn raster_has_correct_dimensions_and_ink() {
        let doc = ylang::parse("@page 0.1\n  title T\nA").unwrap();
        let style = Stylesheet::default();
        let image = layout(&doc, &style).unwrap();
        let mut bytes = Vec::new();
        image.write_ppm(&mut bytes).unwrap();
        let header = format!("P6\n{} {}\n255\n", image.width * 12, image.height * 16);
        assert!(bytes.starts_with(header.as_bytes()));
        let pixels = &bytes[header.len()..];
        assert_eq!(pixels.len(), image.width * 12 * image.height * 16 * 3);
        assert_eq!(&pixels[..3], &[16, 24, 32]);
        assert!(pixels.chunks_exact(3).any(|p| p == [216, 226, 236]));
    }
    #[test]
    fn rejects_unbounded_layout_and_unsafe_ast() {
        let mut doc = ylang::parse("@page 0.1\n  title T\nA").unwrap();
        let style = Stylesheet {
            width: 0,
            ..Stylesheet::default()
        };
        assert!(layout(&doc, &style).is_err());
        doc.nodes[0].kind = NodeKind::Paragraph("\u{1b}[31m".into());
        assert!(layout(&doc, &Stylesheet::default()).is_err());
        doc.nodes[0].kind = NodeKind::Paragraph("word ".repeat(100_000));
        assert!(layout(&doc, &Stylesheet::default()).is_err());
    }
}
