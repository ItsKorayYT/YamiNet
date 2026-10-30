//! Versioned YLang 0.1/0.2: an inert, indentation-based document language.
use std::fmt;
mod rich;
pub use rich::{parse_inline, valid_key, ContainerKind, Inline, InlineKind, TextRole, MAX_NODES};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V01,
    V02,
}

pub const MAX_SOURCE: usize = 256 * 1024;
pub const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// UTF-8 byte offsets, end exclusive; line and column are 1-based.
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}
impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: {}",
            self.span.line, self.span.column, self.message
        )
    }
}
impl std::error::Error for SyntaxError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub title: String,
    pub language: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub version: Version,
    pub metadata: Metadata,
    pub nodes: Vec<Node>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub span: Span,
    pub kind: NodeKind,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Heading {
        level: u8,
        text: String,
    },
    Section {
        name: String,
        children: Vec<Node>,
    },
    Paragraph(String),
    Link {
        target: String,
        label: String,
    },
    Text {
        role: TextRole,
        content: Vec<Inline>,
    },
    Container {
        kind: ContainerKind,
        name: String,
        children: Vec<Node>,
    },
    List {
        ordered: bool,
        items: Vec<Node>,
    },
    ListItem {
        content: Vec<Inline>,
        children: Vec<Node>,
    },
    Image {
        key: String,
        alt: String,
    },
    Separator,
}

struct Line<'a> {
    text: &'a str,
    indent: usize,
    span: Span,
}
impl Line<'_> {
    fn error(&self, message: &str) -> SyntaxError {
        SyntaxError {
            span: self.span.clone(),
            message: message.into(),
        }
    }
}
fn error_at(start: usize, end: usize, line: usize, column: usize, message: &str) -> SyntaxError {
    SyntaxError {
        span: Span {
            start,
            end,
            line,
            column,
        },
        message: message.into(),
    }
}

pub fn parse(source: &str) -> Result<Document, SyntaxError> {
    if source.len() > MAX_SOURCE {
        return Err(error_at(0, 0, 1, 1, "document exceeds 256 KiB"));
    }
    let mut lines = Vec::new();
    let mut offset = 0;
    for (i, raw) in source.split_inclusive('\n').enumerate() {
        let content = raw.strip_suffix('\n').unwrap_or(raw);
        let content = content.strip_suffix('\r').unwrap_or(content);
        if let Some((byte, ch)) = content.char_indices().find(|(_, ch)| ch.is_control()) {
            return Err(error_at(
                offset + byte,
                offset + byte + ch.len_utf8(),
                i + 1,
                content[..byte].chars().count() + 1,
                "control characters (including tabs) are forbidden",
            ));
        }
        if content.len() > 4096 {
            return Err(error_at(
                offset,
                offset + content.len(),
                i + 1,
                1,
                "line exceeds 4096 bytes",
            ));
        }
        let indent = content.bytes().take_while(|b| *b == b' ').count();
        if !content.trim().is_empty() {
            if indent % 2 != 0 {
                return Err(error_at(
                    offset + indent,
                    offset + indent,
                    i + 1,
                    indent + 1,
                    "indentation must use pairs of spaces",
                ));
            }
            lines.push(Line {
                text: content[indent..].trim_end(),
                indent,
                span: Span {
                    start: offset + indent,
                    end: offset + content.len(),
                    line: i + 1,
                    column: indent + 1,
                },
            });
        }
        offset += raw.len();
    }
    let first = lines
        .first()
        .ok_or_else(|| error_at(0, 0, 1, 1, "expected @page 0.1"))?;
    let version = match (first.indent, first.text) {
        (0, "@page 0.1") => Version::V01,
        (0, "@page 0.2") => Version::V02,
        _ => return Err(first.error("expected @page 0.1 or @page 0.2 at column 1")),
    };
    let mut cursor = 1;
    let mut title = None;
    let mut language = None;
    while cursor < lines.len() && lines[cursor].indent > 0 {
        let line = &lines[cursor];
        if line.indent != 2 {
            return Err(line.error("metadata must be indented two spaces"));
        }
        let (key, value) = line
            .text
            .split_once(' ')
            .ok_or_else(|| line.error("expected metadata key and value"))?;
        let value = value.trim();
        if value.is_empty() {
            return Err(line.error("metadata value cannot be empty"));
        }
        let slot = match key {
            "title" => &mut title,
            "language" => &mut language,
            _ => return Err(line.error("unknown metadata key; expected title or language")),
        };
        if slot.is_some() {
            return Err(line.error("duplicate metadata key"));
        }
        *slot = Some(value.to_string());
        cursor += 1;
    }
    let title = title.ok_or_else(|| first.error("missing title metadata"))?;
    let nodes = if version == Version::V01 {
        parse_block(&lines, &mut cursor, 0)?
    } else {
        rich::block(&lines, &mut cursor, 0, None, &mut 0)?
    };
    Ok(Document {
        version,
        metadata: Metadata { title, language },
        nodes,
    })
}

fn parse_block(
    lines: &[Line<'_>],
    cursor: &mut usize,
    indent: usize,
) -> Result<Vec<Node>, SyntaxError> {
    let mut nodes = Vec::new();
    while let Some(line) = lines.get(*cursor) {
        if line.indent < indent {
            break;
        }
        if line.indent != indent {
            return Err(
                line.error("only sections may contain children; indentation must increase by two")
            );
        }
        let text = line.text;
        let span = line.span.clone();
        *cursor += 1;
        let kind = if let Some(name) = text.strip_prefix(":: ") {
            if name.trim().is_empty() {
                return Err(line.error("section name cannot be empty"));
            }
            if indent / 2 >= MAX_DEPTH {
                return Err(line.error("section nesting exceeds 32 levels"));
            }
            let children = parse_block(lines, cursor, indent + 2)?;
            NodeKind::Section {
                name: name.into(),
                children,
            }
        } else if text.starts_with('=') {
            let level = text.bytes().take_while(|b| *b == b'=').count();
            if level > 6 || !text[level..].starts_with(' ') || text[level..].trim().is_empty() {
                return Err(line.error("heading requires 1..6 equals signs, a space, and text"));
            }
            NodeKind::Heading {
                level: level as u8,
                text: text[level + 1..].trim().into(),
            }
        } else if let Some(link) = text.strip_prefix("> ") {
            let (target, label) = link
                .split_once(" | ")
                .ok_or_else(|| line.error("link requires > ynp://address/key | label"))?;
            if !target.starts_with("ynp://")
                || target.len() <= 6
                || target.chars().any(char::is_whitespace)
                || label.trim().is_empty()
            {
                return Err(line
                    .error("link requires a YNP target without whitespace and a nonempty label"));
            }
            NodeKind::Link {
                target: target.into(),
                label: label.trim().into(),
            }
        } else if let Some(literal) = text.strip_prefix('\\') {
            if literal.is_empty() {
                return Err(line.error("literal escape requires text"));
            }
            NodeKind::Paragraph(literal.into())
        } else if text.starts_with(['@', ':', '>']) {
            return Err(line.error("invalid directive; use \\ to escape reserved prefixes"));
        } else {
            NodeKind::Paragraph(text.into())
        };
        nodes.push(Node { span, kind });
    }
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sample_and_nested_ast() {
        let doc = parse(include_str!("../../../examples/welcome.ylang")).unwrap();
        assert_eq!(doc.metadata.title, "Welcome to YamiNet");
        let NodeKind::Section { children, .. } = &doc.nodes[2].kind else {
            panic!("expected section")
        };
        assert!(matches!(children[1].kind, NodeKind::Section { .. }));
    }
    #[test]
    fn precise_errors_and_crlf_spans() {
        let source = "@page 0.1\r\n  title café\r\n= OK\r\n   bad\r\n";
        let error = parse(source).unwrap_err();
        assert_eq!((error.span.line, error.span.column), (4, 4));
        assert_eq!(error.span.start, source.find("bad").unwrap());
        let doc = parse("@page 0.1\n  title é\nhello").unwrap();
        assert_eq!(
            &"@page 0.1\n  title é\nhello"[doc.nodes[0].span.start..doc.nodes[0].span.end],
            "hello"
        );
    }
    #[test]
    fn rejects_invalid_grammar() {
        for body in [
            "=",
            "======= bad",
            "> http://a | b",
            "> ynp://a/b",
            "@script no",
            "text\n  child",
            ":: root\n    jump",
            "\tbad",
            ":: ",
        ] {
            assert!(
                parse(&format!("@page 0.1\n  title Test\n{body}")).is_err(),
                "{body}"
            );
        }
        assert!(parse("@page 0.3\n  title Test").is_err());
        assert!(parse("@page 0.1\n  title A\n  title B").is_err());
        assert!(parse("@page 0.1\nbody").is_err());
    }
    #[test]
    fn bounds_and_literal_text() {
        assert!(parse(&"x".repeat(MAX_SOURCE + 1)).is_err());
        assert!(parse(&format!("@page 0.1\n  title T\n{}", "x".repeat(4097))).is_err());
        let deep = (0..34)
            .map(|n| format!("{}:: child\n", "  ".repeat(n)))
            .collect::<String>();
        assert!(parse(&format!("@page 0.1\n  title T\n{deep}")).is_err());
        assert!(matches!(
            parse("@page 0.1\n  title T\n\\@literal").unwrap().nodes[0].kind,
            NodeKind::Paragraph(_)
        ));
    }
}

impl NodeKind {
    pub fn children(&self) -> &[Node] {
        match self {
            Self::Section { children, .. }
            | Self::Container { children, .. }
            | Self::ListItem { children, .. } => children,
            Self::List { items, .. } => items,
            _ => &[],
        }
    }
}
pub fn plain_text(content: &[Inline]) -> String {
    content.iter().map(|part| part.text.as_str()).collect()
}
