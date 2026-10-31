use super::{error_at, Line, Node, NodeKind, Span, SyntaxError, MAX_DEPTH};
pub const MAX_NODES: usize = 4096;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    Plain,
    Strong,
    Emphasis,
    Code,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inline {
    pub kind: InlineKind,
    pub text: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    Paragraph,
    Heading(u8),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    Card,
    Row,
    Group,
}
pub fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
pub fn parse_inline(text: &str, span: &Span) -> Result<Vec<Inline>, SyntaxError> {
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut offset = 0;
    let error = |at: usize, message: &str| {
        error_at(
            span.start + at,
            span.start + at + 1,
            span.line,
            span.column + text[..at].chars().count(),
            message,
        )
    };
    while offset < text.len() {
        let rest = &text[offset..];
        let ch = rest.chars().next().unwrap();
        if ch == '\\' {
            if let Some(escaped) = rest[1..]
                .chars()
                .next()
                .filter(|c| matches!(c, '[' | ']' | '\\'))
            {
                plain.push(escaped);
                offset += 1 + escaped.len_utf8();
                continue;
            }
        }
        if ch == ']' {
            return Err(error(
                offset,
                "unmatched inline closing bracket; escape literal brackets with \\",
            ));
        }
        if ch != '[' {
            plain.push(ch);
            offset += ch.len_utf8();
            continue;
        }
        if !plain.is_empty() {
            parts.push(Inline {
                kind: InlineKind::Plain,
                text: std::mem::take(&mut plain),
            });
        }
        let (kind, prefix) = if rest.starts_with("[strong: ") {
            (InlineKind::Strong, 9)
        } else if rest.starts_with("[em: ") {
            (InlineKind::Emphasis, 5)
        } else if rest.starts_with("[code: ") {
            (InlineKind::Code, 7)
        } else {
            return Err(error(
                offset,
                "expected [strong: text], [em: text] or [code: text]",
            ));
        };
        let start = offset;
        offset += prefix;
        let mut value = String::new();
        let mut closed = false;
        while offset < text.len() {
            let ch = text[offset..].chars().next().unwrap();
            if ch == ']' {
                offset += 1;
                closed = true;
                break;
            }
            if ch == '[' {
                return Err(error(
                    offset,
                    "inline spans cannot nest; escape literal brackets",
                ));
            }
            if ch == '\\' {
                if let Some(next) = text[offset + 1..]
                    .chars()
                    .next()
                    .filter(|c| matches!(c, '[' | ']' | '\\'))
                {
                    value.push(next);
                    offset += 1 + next.len_utf8();
                    continue;
                }
            }
            value.push(ch);
            offset += ch.len_utf8();
        }
        if !closed || value.trim().is_empty() {
            return Err(error(
                start,
                "inline span needs nonempty text and a closing ]",
            ));
        }
        parts.push(Inline { kind, text: value });
    }
    if !plain.is_empty() {
        parts.push(Inline {
            kind: InlineKind::Plain,
            text: plain,
        });
    }
    Ok(parts)
}
fn inline(line: &Line<'_>, text: &str) -> Result<Vec<Inline>, SyntaxError> {
    let mut span = line.span.clone();
    let prefix = line.text.len() - text.len();
    span.start += prefix;
    span.column += line.text[..prefix].chars().count();
    parse_inline(text, &span)
}
// Syntax only. Yami Browser performs complete checksum, signature, ownership
// and persistent binding checks before following any native address.
fn native_authority(s: &str) -> bool {
    let identity = |id: &str| {
        id.starts_with("yn1-")
            && id.len() == 62
            && id[4..]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || matches!(b, b'2'..=b'7'))
    };
    let label = |s: &str| {
        !s.is_empty()
            && s.len() <= 32
            && !s.starts_with("xn--")
            && s.as_bytes()[0].is_ascii_lowercase()
            && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    };
    let path = |s: &str| s.split('.').count() <= 8 && s.split('.').all(label);
    if identity(s) {
        return true;
    }
    if let Some(alias) = s.strip_prefix('@') {
        return label(alias);
    }
    if let Some(body) = s.strip_suffix(".yns") {
        let (p, owner) = body.rsplit_once('.').unwrap_or(("", body));
        return identity(owner) && (p.is_empty() || path(p));
    }
    s.strip_suffix(".yami").is_some_and(path)
}

fn link_target(target: &str) -> bool {
    if let Some(key) = target.strip_prefix("game:") {
        return valid_key(key) && key.len() <= 64;
    }
    if let Some(key) = target.strip_prefix('/') {
        return valid_key(key);
    }
    if target.chars().any(char::is_whitespace) {
        return false;
    }
    if let Some(rest) = target.strip_prefix("yami://") {
        let (id, key) = rest.split_once('/').unwrap_or((rest, "home"));
        return native_authority(id) && valid_key(key);
    }
    ["ynp://", "ynps://"].iter().any(|scheme| {
        target.strip_prefix(scheme).is_some_and(|v| {
            v.split_once('/')
                .is_some_and(|(address, key)| !address.is_empty() && valid_key(key))
        })
    })
}
pub(super) fn block(
    lines: &[Line<'_>],
    cursor: &mut usize,
    indent: usize,
    list: Option<bool>,
    count: &mut usize,
) -> Result<Vec<Node>, SyntaxError> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut last_paragraph_line = None;
    if indent / 2 > MAX_DEPTH {
        return Err(lines[*cursor - 1].error("nesting exceeds 32 levels"));
    }
    while let Some(line) = lines.get(*cursor) {
        if line.indent < indent {
            break;
        }
        if line.indent != indent {
            return Err(
                line.error("unexpected indentation; children require a container or list item")
            );
        }
        *count += 1;
        if *count > MAX_NODES {
            return Err(line.error("0.2 document exceeds 4096 nodes"));
        }
        let text = line.text;
        let span = line.span.clone();
        *cursor += 1;
        let kind = if let Some(ordered) = list {
            let content = text
                .strip_prefix(if ordered { "+ " } else { "- " })
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| line.error("list items require - text; steps require + text"))?;
            let content = inline(line, content)?;
            let children = block(lines, cursor, indent + 2, None, count)?;
            NodeKind::ListItem { content, children }
        } else if matches!(text, ":: list" | ":: steps") {
            let ordered = text == ":: steps";
            let items = block(lines, cursor, indent + 2, Some(ordered), count)?;
            if items.is_empty() {
                return Err(line.error("list requires at least one item"));
            }
            NodeKind::List { ordered, items }
        } else if let Some(name) = text.strip_prefix(":: ") {
            if name.trim().is_empty() {
                return Err(line.error("section needs a name"));
            }
            let children = block(lines, cursor, indent + 2, None, count)?;
            let semantic = [
                ("card", ContainerKind::Card),
                ("row", ContainerKind::Row),
                ("group", ContainerKind::Group),
            ]
            .into_iter()
            .find_map(|(key, kind)| {
                if name == key {
                    Some((kind, ""))
                } else {
                    name.strip_prefix(&format!("{key} "))
                        .map(|title| (kind, title))
                }
            });
            if let Some((kind, name)) = semantic {
                if kind == ContainerKind::Row && children.len() > 4 {
                    return Err(line.error("row supports at most four child blocks"));
                }
                NodeKind::Container {
                    kind,
                    name: name.into(),
                    children,
                }
            } else {
                NodeKind::Section {
                    name: name.into(),
                    children,
                }
            }
        } else if text.starts_with('=') {
            let level = text.bytes().take_while(|b| *b == b'=').count();
            if level > 6 || !text[level..].starts_with(' ') || text[level..].trim().is_empty() {
                return Err(line.error("heading requires 1..6 equals signs and text"));
            }
            NodeKind::Text {
                role: TextRole::Heading(level as u8),
                content: inline(line, &text[level + 1..])?,
            }
        } else if let Some(value) = text.strip_prefix("> ") {
            let (target, label) = value.split_once(" | ").ok_or_else(|| {
                line.error("link requires > /page or yami://identity/page | label")
            })?;
            if !link_target(target) || label.trim().is_empty() {
                return Err(line.error("invalid local/native link target or empty label"));
            }
            NodeKind::Link {
                target: target.into(),
                label: if target.starts_with("game:") {
                    format!("Play game: {}", label.trim())
                } else {
                    label.trim().into()
                },
            }
        } else if let Some(value) = text.strip_prefix("! ") {
            let (key, alt) = value.split_once(" | ").ok_or_else(|| {
                line.error("image requires ! asset-key | meaningful alternative text")
            })?;
            if !valid_key(key) || key.len() > 64 || alt.trim().is_empty() {
                return Err(line.error(
                    "image needs a restricted same-node key (1..64) and nonempty alt text",
                ));
            }
            NodeKind::Image {
                key: key.into(),
                alt: alt.trim().into(),
            }
        } else if text == "---" {
            NodeKind::Separator
        } else if let Some(literal) = text.strip_prefix('\\') {
            if literal.is_empty() {
                return Err(line.error("literal escape requires text"));
            }
            NodeKind::Text {
                role: TextRole::Paragraph,
                content: vec![Inline {
                    kind: InlineKind::Plain,
                    text: literal.into(),
                }],
            }
        } else if text.starts_with(['@', ':', '>', '!', '-', '+']) {
            return Err(line.error("unsupported 0.2 directive; list items require :: list/:: steps; escape literal prefixes with \\"));
        } else {
            let content = inline(line, text)?;
            if let Some(previous) = nodes.last_mut() {
                if let NodeKind::Text {
                    role: TextRole::Paragraph,
                    content: old,
                } = &mut previous.kind
                {
                    if last_paragraph_line == Some(line.span.line - 1) {
                        if old.iter().map(|part| part.text.len()).sum::<usize>() + text.len() + 1
                            > 16 * 1024
                        {
                            return Err(line.error("paragraph exceeds 16 KiB"));
                        }
                        old.push(Inline {
                            kind: InlineKind::Plain,
                            text: " ".into(),
                        });
                        old.extend(content);
                        previous.span.end = span.end;
                        last_paragraph_line = Some(line.span.line);
                        continue;
                    }
                }
            }
            NodeKind::Text {
                role: TextRole::Paragraph,
                content,
            }
        };
        last_paragraph_line = if matches!(
            kind,
            NodeKind::Text {
                role: TextRole::Paragraph,
                ..
            }
        ) && !text.starts_with('\\')
        {
            Some(line.span.line)
        } else {
            None
        };
        nodes.push(Node { span, kind });
    }
    Ok(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse, Version};
    #[test]
    fn rich_blocks_inline_unicode_lists_and_flow_are_typed() {
        let source="@page 0.2\n  title Café\n= Hello [strong: world]\nA [em: thoughtful] paragraph\ncontinues with [code: Rust].\n\nA separate paragraph.\n:: card Details\n  :: list\n    - First\n      :: steps\n        + Déjà vu\n        + Next\n  > /about | About\n! cover | A mountain reflected in a lake\n---\n";
        let doc = parse(source).unwrap();
        assert_eq!(doc.version, Version::V02);
        let NodeKind::Text { content, .. } = &doc.nodes[0].kind else {
            panic!("heading")
        };
        assert_eq!(content[1].kind, InlineKind::Strong);
        let NodeKind::Text { content, .. } = &doc.nodes[1].kind else {
            panic!("paragraph")
        };
        assert!(crate::plain_text(content).contains("paragraph continues"));
        assert_eq!(doc.nodes[1].span.column, 1);
        assert!(matches!(
            doc.nodes[3].kind,
            NodeKind::Container {
                kind: ContainerKind::Card,
                ..
            }
        ));
        assert!(matches!(doc.nodes[4].kind, NodeKind::Image { .. }));
        assert!(matches!(doc.nodes[5].kind, NodeKind::Separator));
    }
    #[test]
    fn rich_node_paragraph_row_and_nesting_budgets_have_explicit_errors() {
        let nodes = format!(
            "@page 0.2\n  title Too many\n{}",
            "---\n".repeat(MAX_NODES + 1)
        );
        assert!(parse(&nodes).unwrap_err().message.contains("4096 nodes"));
        let paragraph = format!(
            "@page 0.2\n  title Long\n{}",
            format!("{}\n", "a".repeat(4000)).repeat(5)
        );
        assert!(parse(&paragraph).is_err());
        let row="@page 0.2\n  title Row\n:: row\n  One\n  > /two | Two\n  ---\n  > /three | Three\n  ---\n";
        assert!(parse(row).is_err());
        let mut nested = String::from("@page 0.2\n  title Deep\n");
        for n in 0..34 {
            nested.push_str(&format!("{}:: group\n", "  ".repeat(n)));
        }
        assert!(parse(&nested).is_err());
        let escaped = parse("@page 0.2\n  title Literal\n\\[strong: plain]\n").unwrap();
        assert!(
            matches!(&escaped.nodes[0].kind,NodeKind::Text{content,..} if content.len()==1 && content[0].kind==InlineKind::Plain && content[0].text=="[strong: plain]")
        );
    }
    #[test]
    fn legacy_markers_are_not_reinterpreted_and_new_errors_keep_source_positions() {
        let old=parse("@page 0.1\n  title Legacy\n[strong: literal]\n:: card\n  - literal list text\n! literal image\n---\n").unwrap();
        assert!(
            matches!(&old.nodes[0].kind,NodeKind::Paragraph(text) if text=="[strong: literal]")
        );
        assert!(matches!(old.nodes[1].kind, NodeKind::Section { .. }));
        for body in [
            ":: list\n  + wrong",
            ":: steps\n  - wrong",
            "! ../key | Bad",
            "! cover | ",
            "[unknown: no]",
            "[strong: [em: nested]]",
            "[code: unclosed",
            "- not in list",
            "> /../home | Bad",
            "> https://example.com/a | Bad",
        ] {
            assert!(
                parse(&format!("@page 0.2\n  title Bad\n{body}\n")).is_err(),
                "{body}"
            );
        }
        let error = parse("@page 0.2\n  title T\n:: card\n  α [bad: value]\n").unwrap_err();
        assert_eq!((error.span.line, error.span.column), (4, 5));
        let error = parse("@page 0.3\n  title Unsupported\n").unwrap_err();
        assert_eq!((error.span.line, error.span.column), (1, 1));
    }
}

#[cfg(test)]
mod game_tests {
    #[test]
    fn play_links_are_inert_same_owner_keys_with_a_fixed_consent_label() {
        let doc =
            crate::parse("@page 0.2\n  title Arcade\n> game:prism | Prism Breakout\n").unwrap();
        assert!(
            matches!(&doc.nodes[0].kind,crate::NodeKind::Link {target,label} if target=="game:prism" && label=="Play game: Prism Breakout")
        );
        for target in [
            "game:",
            "game:../prism",
            "game:yami://other/game",
            "game:prism.wasm",
        ] {
            assert!(
                crate::parse(&format!("@page 0.2\n  title Arcade\n> {target} | Launch\n")).is_err()
            );
        }
        assert!(crate::parse("@page 0.1\n  title Arcade\n> game:prism | Launch\n").is_err());
    }
}

#[cfg(test)]
mod naming_tests {
    #[test]
    fn readable_native_links_are_inert_and_syntactically_scoped() {
        let owner = "yn1-lbtgmztgmztgmztgmztgmztgmztgmztgmztgmztgmztgmztgmztfk4s3km";
        for authority in [
            "@blog".into(),
            "welcome.yami".into(),
            format!("blog.{owner}.yns"),
            format!("{owner}.yns"),
        ] {
            let target = format!("yami://{authority}/home");
            let document =
                crate::parse(&format!("@page 0.2\n  title Names\n> {target} | Open\n")).unwrap();
            assert!(
                matches!(&document.nodes[0].kind,crate::NodeKind::Link { target: found, .. } if found==&target)
            );
        }
        for authority in [
            "@Blog",
            "@xn--blog",
            "blog",
            "welcome.YAMI",
            "welcоme.yami",
            "blog.yn1-short.yns",
        ] {
            assert!(crate::parse(&format!(
                "@page 0.2\n  title Names\n> yami://{authority}/home | Open\n"
            ))
            .is_err());
        }
    }
}
