use crate::{invalid, Result, MAX_PAGE_TEXT};
use unicode_normalization::UnicodeNormalization;
use ylang::{Inline, Node, NodeKind, TextRole};

/// Compatibility decomposition plus Discover's case/accent folding. Original
/// text is retained separately; folded byte offsets never index original UTF-8.
pub fn normalize(text: &str) -> String {
    yami_discover::fold_text(&text.nfkd().collect::<String>())
}

pub fn tokens(text: &str) -> Vec<String> {
    normalize(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn extract(nodes: &[Node], headings: &mut String, body: &mut String) -> Result<()> {
    for node in nodes {
        match &node.kind {
            NodeKind::Heading { text, .. } => {
                append(headings, text);
                append(body, text);
            }
            NodeKind::Text { role, content } => {
                let text = inline(content);
                if matches!(role, TextRole::Heading(_)) {
                    append(headings, &text);
                }
                append(body, &text);
            }
            NodeKind::Paragraph(text) => append(body, text),
            NodeKind::Link { label, .. } => append(body, label),
            NodeKind::Image { alt, .. } => append(body, alt),
            NodeKind::Section { name, children } | NodeKind::Container { name, children, .. } => {
                append(headings, name);
                append(body, name);
                extract(children, headings, body)?;
            }
            NodeKind::List { items, .. } => extract(items, headings, body)?,
            NodeKind::ListItem { content, children } => {
                append(body, &inline(content));
                extract(children, headings, body)?;
            }
            NodeKind::Separator => {}
        }
        if body.len() + headings.len() > MAX_PAGE_TEXT {
            return Err(invalid("page text exceeds budget"));
        }
    }
    Ok(())
}
fn inline(parts: &[Inline]) -> String {
    parts.iter().map(|p| p.text.as_str()).collect()
}
fn append(to: &mut String, text: &str) {
    if !text.is_empty() {
        if !to.is_empty() {
            to.push('\n');
        }
        to.push_str(text);
    }
}

pub(crate) fn snippet(text: &str, terms: &[String]) -> String {
    // Map folded characters to original character indices, accounting for
    // expansions (ß -> ss), accents and all UTF-8 character boundaries.
    let chars: Vec<char> = text.chars().collect();
    let mut folded = String::new();
    let mut offsets = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        let part = normalize(&c.to_string());
        offsets.extend(std::iter::repeat_n(i, part.len()));
        folded.push_str(&part);
    }
    let position = terms
        .iter()
        .filter_map(|t| folded.find(t))
        .min()
        .and_then(|p| offsets.get(p).copied())
        .unwrap_or(0);
    let start = position.saturating_sub(48);
    let end = (start + 200).min(chars.len());
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        chars[start..end]
            .iter()
            .collect::<String>()
            .replace('\n', " "),
        if end < chars.len() { "…" } else { "" }
    )
}
