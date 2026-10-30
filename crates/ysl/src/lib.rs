//! YSL 0.1: original line-oriented presentation settings, with no execution.
use std::{collections::HashSet, fmt};
mod rich;
pub use rich::{Align, CardStyle, ImageStyle, RichStyle, Typography};

pub const MAX_SOURCE: usize = 16 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextStyle {
    pub ink: Color,
    pub gap: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub rich: Option<RichStyle>,
    pub width: usize,
    pub padding: usize,
    pub paper: Color,
    pub text: TextStyle,
    pub heading: TextStyle,
    pub section: TextStyle,
    pub link: TextStyle,
}
impl Default for Stylesheet {
    fn default() -> Self {
        Self {
            rich: None,
            width: 76,
            padding: 2,
            paper: Color(16, 24, 32),
            text: TextStyle {
                ink: Color(216, 226, 236),
                gap: 1,
            },
            heading: TextStyle {
                ink: Color(97, 218, 251),
                gap: 1,
            },
            section: TextStyle {
                ink: Color(180, 210, 115),
                gap: 0,
            },
            link: TextStyle {
                ink: Color(255, 169, 112),
                gap: 1,
            },
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// 1-based Unicode-scalar column and line; UTF-8 byte offset.
    pub line: usize,
    pub column: usize,
    pub offset: usize,
    pub message: String,
}
impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}
impl std::error::Error for SyntaxError {}

pub fn parse(source: &str) -> Result<Stylesheet, SyntaxError> {
    let mut sheet = Stylesheet::default();
    let mut group = "";
    let mut header = false;
    let mut groups = HashSet::new();
    let mut properties = HashSet::new();
    let mut offset = 0;
    if source.len() > MAX_SOURCE {
        return Err(SyntaxError {
            line: 1,
            column: 1,
            offset: 0,
            message: "stylesheet exceeds 16 KiB".into(),
        });
    }
    for (i, raw) in source.split_inclusive('\n').enumerate() {
        let content = raw.strip_suffix('\n').unwrap_or(raw);
        let content = content.strip_suffix('\r').unwrap_or(content);
        let indent = content.bytes().take_while(|b| *b == b' ').count();
        let text = content.trim();
        let err = |column: usize, byte: usize, message: &str| SyntaxError {
            line: i + 1,
            column,
            offset: offset + byte,
            message: message.into(),
        };
        if let Some((byte, _)) = content.char_indices().find(|(_, c)| c.is_control()) {
            return Err(err(
                content[..byte].chars().count() + 1,
                byte,
                "control characters are forbidden",
            ));
        }
        if !text.is_empty() {
            if !header {
                if indent != 0 || !matches!(text, "@style 0.1" | "@style 0.2") {
                    return Err(err(1, 0, "expected @style 0.1 or @style 0.2 at column 1"));
                }
                if text == "@style 0.2" {
                    sheet.rich = Some(RichStyle::default());
                }
                header = true;
            } else if indent == 0 {
                if !matches!(text, "canvas" | "text" | "heading" | "section" | "link")
                    && !(sheet.rich.is_some() && matches!(text, "card" | "image" | "row"))
                {
                    return Err(err(1, 0, "unknown style group"));
                }
                if !groups.insert(text) {
                    return Err(err(1, 0, "duplicate style group"));
                }
                group = text;
            } else {
                if indent != 2 || group.is_empty() {
                    return Err(err(
                        indent + 1,
                        indent,
                        "property requires a group and two spaces",
                    ));
                }
                let Some((name, value)) = text.split_once(' ') else {
                    return Err(err(3, 2, "expected property and value"));
                };
                let value = value.trim();
                let value_byte = indent + text.find(value).unwrap_or(text.len());
                let value_error = |msg: &str| err(value_byte + 1, value_byte, msg);
                if !properties.insert((group, name)) {
                    return Err(err(3, 2, "duplicate property"));
                }
                let number = |min: usize, max: usize| -> Result<usize, SyntaxError> {
                    let n = value
                        .parse::<usize>()
                        .map_err(|_| value_error("expected bounded unsigned integer"))?;
                    if !(min..=max).contains(&n) {
                        return Err(value_error("number outside supported range"));
                    }
                    Ok(n)
                };
                let color = || -> Result<Color, SyntaxError> {
                    if value.len() != 7
                        || !value.starts_with('#')
                        || !value[1..].bytes().all(|b| b.is_ascii_hexdigit())
                    {
                        return Err(value_error("expected color #RRGGBB"));
                    }
                    let n = u32::from_str_radix(&value[1..], 16)
                        .map_err(|_| value_error("invalid color"))?;
                    Ok(Color((n >> 16) as u8, (n >> 8) as u8, n as u8))
                };
                let handled = if let Some(rich) = &mut sheet.rich {
                    rich.set(group, name, value)
                        .map_err(|message| value_error(&message))?
                } else {
                    false
                };
                if handled {
                    offset += raw.len();
                    continue;
                }
                if matches!(group, "card" | "image" | "row") {
                    return Err(err(3, 2, "unknown property for this 0.2 group"));
                }
                if group == "canvas" {
                    match name {
                        "width" => sheet.width = number(20, 160)?,
                        "padding" => sheet.padding = number(0, 8)?,
                        "paper" => sheet.paper = color()?,
                        _ => return Err(err(3, 2, "unknown canvas property")),
                    }
                } else {
                    let target = match group {
                        "heading" => &mut sheet.heading,
                        "section" => &mut sheet.section,
                        "link" => &mut sheet.link,
                        _ => &mut sheet.text,
                    };
                    match name {
                        "ink" => target.ink = color()?,
                        "gap" => target.gap = number(0, 3)?,
                        _ => return Err(err(3, 2, "unknown text property")),
                    }
                }
            }
        }
        offset += raw.len();
    }
    if !header {
        return Err(SyntaxError {
            line: 1,
            column: 1,
            offset: 0,
            message: "expected @style 0.1".into(),
        });
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sample_and_defaults() {
        assert_eq!(
            parse(include_str!("../../../examples/default.ysl")).unwrap(),
            Stylesheet::default()
        );
        assert_eq!(parse("@style 0.1").unwrap(), Stylesheet::default());
        assert_eq!(parse("@style 0.1\ncanvas\n  width 30").unwrap().width, 30);
    }
    #[test]
    fn errors_are_located_and_strict() {
        let err = parse("@style 0.1\ncanvas\n  width 0").unwrap_err();
        assert_eq!((err.line, err.column, err.offset), (3, 9, 26));
        for source in [
            "@style 2",
            "@style 0.1\nunknown",
            "@style 0.1\n  ink #ffffff",
            "@style 0.1\ntext\n   gap 1",
            "@style 0.1\ntext\n  ink #ffzzff",
            "@style 0.1\ntext\n  gap 4",
            "@style 0.1\ncanvas\n  width 80\n  width 90",
            "@style 0.1\ntext\ntext",
            "@style 0.1\ntext\n  script yes",
            "@style 0.1\n\ttext",
        ] {
            assert!(parse(source).is_err(), "{source}");
        }
        assert!(parse(&"x".repeat(MAX_SOURCE + 1)).is_err());
    }
}
