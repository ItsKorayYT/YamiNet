//! Hover documentation for YLang directives and YSL groups/properties.

use crate::site::{local_key, SiteRoot};
use crate::spec::{self, Language, Version};
use crate::text::TextDocument;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub markdown: String,
    /// Absolute UTF-8 byte range to highlight.
    pub start: usize,
    pub end: usize,
}

fn hover(start: usize, end: usize, markdown: impl Into<String>) -> Hover {
    Hover {
        markdown: markdown.into(),
        start,
        end: end.max(start),
    }
}

struct LineView<'a> {
    line: u32,
    start: usize,
    end: usize,
    indent: usize,
    text: &'a str,
}

impl<'a> LineView<'a> {
    fn new(document: &'a TextDocument, offset: usize) -> Option<(Self, usize)> {
        let line = document.position_at(offset).line;
        let (start, end) = document.line_bounds(line)?;
        let text = &document.text()[start..end];
        let indent = text.bytes().take_while(|byte| *byte == b' ').count();
        Some((
            Self {
                line,
                start,
                end,
                indent,
                text,
            },
            offset.clamp(start, end) - start,
        ))
    }

    fn trailing(&self) -> &'a str {
        &self.text[self.indent..]
    }
}

/// Compute hover content for the cursor at a UTF-8 byte offset.
pub fn hover_at(
    document: &TextDocument,
    offset: usize,
    language: Language,
    version: Version,
    site: Option<&SiteRoot>,
) -> Option<Hover> {
    let (view, cursor) = LineView::new(document, offset)?;
    match language {
        Language::Ylang => hover_ylang(document, &view, cursor, version, site),
        Language::Ysl => hover_ysl(document, &view, version),
    }
}

fn hover_ylang(
    document: &TextDocument,
    view: &LineView<'_>,
    cursor: usize,
    version: Version,
    site: Option<&SiteRoot>,
) -> Option<Hover> {
    let text = view.trailing();
    let base = view.start + view.indent;

    if view.indent == 2 && metadata_line(document, view.line) {
        let key = text.split_whitespace().next().unwrap_or("");
        if let Some(field) = spec::meta_field(key) {
            return Some(hover(
                base,
                base + key.len(),
                format!(
                    "### `{}` metadata\n\n{}\n\n{}",
                    field.name,
                    field.doc,
                    if field.required {
                        "**Required** exactly once."
                    } else {
                        "**Optional.**"
                    }
                ),
            ));
        }
    }

    if text.starts_with("@page") {
        let markdown = format!(
            "{}\n\nSupported page headers: `@page 0.1` and `@page 0.2`. \
             The header must be the first nonblank line, at column 1.",
            spec::page_doc(version)
        );
        return Some(hover(view.start, view.end, markdown));
    }

    if version == Version::V02 {
        if let Some((start, end, inline)) = inline_at(text, cursor) {
            return Some(hover(
                view.start + start,
                view.start + end,
                format!("### Inline `{}`\n\n{}", inline.label, inline.doc),
            ));
        }
    }

    if let Some(rest) = text.strip_prefix("::") {
        let rest = rest.trim_start();
        let name = rest.split_whitespace().next().unwrap_or("");
        let (doc, title) = if version == Version::V02 {
            match name {
                "card" => (spec::DOC_CARD, "Typed card"),
                "row" => (spec::DOC_ROW, "Typed row"),
                "group" => (spec::DOC_GROUP, "Typed group"),
                "list" => (spec::DOC_LIST, "Unordered list"),
                "steps" => (spec::DOC_STEPS, "Ordered step list"),
                _ => (spec::DOC_SECTION, "Named section"),
            }
        } else {
            (spec::DOC_SECTION, "Named section")
        };
        let take = text
            .len()
            .min(2 + (rest.len() - rest.trim_start().len()) + name.len());
        return Some(hover(
            view.start,
            view.start + take,
            format!("### {title}\n\n{doc}"),
        ));
    }

    if let Some(target) = link_target(text) {
        let markdown = if version == Version::V01 {
            if !target.starts_with("ynp://") {
                return None;
            }
            format!(
                "### YLang 0.1 link\n\n{}\n\nThe language server never fetches a link target.",
                spec::DOC_LINK_YNP
            )
        } else if let Some(key) = local_key(target) {
            let resolution = match site {
                Some(site) => match site.resolve_page(key) {
                    Some(_) => format!("Local page `/{key}` resolves to `{key}.ylang`."),
                    None => {
                        format!("No `{key}.ylang` page exists in the selected website directory.")
                    }
                },
                None => {
                    "No website directory is configured, so this local page cannot be resolved."
                        .to_string()
                }
            };
            format!(
                "### Local page link\n\n{}\n\n{}\n\nThe language server performs no network requests.",
                spec::DOC_LINK_LOCAL, resolution
            )
        } else if target.starts_with("yami://") {
            format!(
                "### Native identity link\n\n{}\n\nThe language server never resolves or fetches remote targets.",
                spec::DOC_LINK_NATIVE
            )
        } else if target.starts_with("ynp://") || target.starts_with("ynps://") {
            format!(
                "### YNP link\n\n{}\n\nThe language server never fetches a link target.",
                spec::DOC_LINK_YNP
            )
        } else {
            return None;
        };
        let start = view.start + text.find(target).unwrap_or(0);
        return Some(hover(start, start + target.len(), markdown));
    }

    if version == Version::V02 && text.starts_with('!') {
        return Some(hover(
            view.start,
            view.end,
            format!("### Same-node image\n\n{}", spec::DOC_IMAGE),
        ));
    }

    if version == Version::V02 && text == "---" {
        return Some(hover(
            view.start,
            view.end,
            format!("### Separator\n\n{}", spec::DOC_SEPARATOR),
        ));
    }

    if text.starts_with('=') {
        return Some(hover(
            view.start,
            view.end,
            format!("### Heading\n\n{}", spec::DOC_HEADING),
        ));
    }

    if text.starts_with('\\') {
        return Some(hover(
            view.start,
            view.end,
            format!("### Literal text\n\n{}", spec::DOC_ESCAPE),
        ));
    }

    if version == Version::V02 {
        if text.starts_with("- ") {
            return Some(hover(
                view.start,
                view.end,
                format!("### List item\n\n{}", spec::DOC_LIST),
            ));
        }
        if text.starts_with("+ ") {
            return Some(hover(
                view.start,
                view.end,
                format!("### Step item\n\n{}", spec::DOC_STEPS),
            ));
        }
    }

    None
}

fn hover_ysl(document: &TextDocument, view: &LineView<'_>, version: Version) -> Option<Hover> {
    let text = view.trailing();
    let base = view.start + view.indent;

    if text.starts_with("@style") {
        return Some(hover(
            view.start,
            view.end,
            format!(
                "{}\n\nSupported stylesheet headers: `@style 0.1` and `@style 0.2`. \
                 The header must be the first nonblank line, at column 1.",
                spec::style_doc(version)
            ),
        ));
    }

    if view.indent == 0 {
        let name = text.trim_end();
        let group = spec::group_spec(name, version)?;
        return Some(hover(
            base,
            base + name.len(),
            format!(
                "### `{}` (YSL {})\n\n{}",
                group.name,
                version.label(),
                group.doc
            ),
        ));
    }

    if view.indent == 2 {
        let name = text.split_whitespace().next().unwrap_or("");
        if name.is_empty() {
            return None;
        }
        let group = current_group(document, view.line, version)?;
        let property = spec::property_spec(group.name, name, version)?;
        return Some(hover(
            base,
            base + name.len(),
            format!(
                "### `{}.{}` (YSL {})\n\n{}\n\n**Default:** `{}`.",
                group.name,
                property.name,
                version.label(),
                property.doc,
                property.default
            ),
        ));
    }
    None
}

fn current_group(
    document: &TextDocument,
    line: u32,
    version: Version,
) -> Option<&'static spec::GroupSpec> {
    for index in (0..line).rev() {
        let text = document.line_text(index)?;
        if text.trim().is_empty() {
            continue;
        }
        let indent = text.bytes().take_while(|byte| *byte == b' ').count();
        if indent != 0 {
            continue;
        }
        return spec::group_spec(text.trim_end(), version);
    }
    None
}

/// Metadata keys are only valid in the block between the header and the first
/// body block. This prevents a paragraph that happens to start with "title "
/// from showing metadata documentation.
fn metadata_line(document: &TextDocument, line: u32) -> bool {
    let Some((header_line, _)) = first_nonblank(document) else {
        return false;
    };
    if line <= header_line {
        return false;
    }
    for index in header_line + 1..line {
        let Some(text) = document.line_text(index) else {
            return false;
        };
        if text.trim().is_empty() {
            continue;
        }
        let indent = text.bytes().take_while(|byte| *byte == b' ').count();
        if indent != 2 {
            return false;
        }
        let content = &text[indent..];
        let key = content.split_whitespace().next().unwrap_or("");
        if spec::meta_field(key).is_none() {
            return false;
        }
    }
    true
}

fn first_nonblank(document: &TextDocument) -> Option<(u32, &str)> {
    for line in 0..document.line_count() {
        let text = document.line_text(line)?;
        if !text.trim().is_empty() {
            return Some((line, text));
        }
    }
    None
}

fn link_target(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("> ")?;
    let target = rest.split(" | ").next()?.trim();
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

fn inline_at(text: &str, cursor: usize) -> Option<(usize, usize, &'static spec::InlineSpec)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] != b'[' {
            index += 1;
            continue;
        }
        for inline in spec::YLANG_INLINE {
            let marker = format!("[{}: ", inline.marker);
            if text[index..].starts_with(&marker) {
                let content_start = index + marker.len();
                let mut end = content_start;
                let mut escaped = false;
                while end < bytes.len() {
                    if escaped {
                        escaped = false;
                        end += 1;
                        continue;
                    }
                    match bytes[end] {
                        b'\\' => {
                            escaped = true;
                            end += 1;
                        }
                        b']' => {
                            end += 1;
                            if cursor >= index && cursor <= end {
                                return Some((index, end, inline));
                            }
                            break;
                        }
                        _ => end += 1,
                    }
                }
                if end >= bytes.len() && cursor >= index && cursor <= bytes.len() {
                    return Some((index, bytes.len(), inline));
                }
            }
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn blog_site() -> SiteRoot {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        SiteRoot::open(&root.join("sites/blog")).unwrap()
    }

    fn hover_doc(
        text: &str,
        line: u32,
        character: u32,
        language: Language,
        version: Version,
        site: Option<&SiteRoot>,
    ) -> Option<Hover> {
        let document = TextDocument::new(text);
        let offset = document.offset_at(crate::text::Position::new(line, character));
        hover_at(&document, offset, language, version, site)
    }

    #[test]
    fn ylang_headers_metadata_and_containers_are_documented() {
        let doc = hover_doc(
            "@page 0.2\n  title T\n:: row Title\n",
            0,
            2,
            Language::Ylang,
            Version::V02,
            None,
        )
        .unwrap();
        assert!(doc.markdown.contains("YLang 0.2"));
        assert!(doc.markdown.contains("4096"));

        let doc = hover_doc(
            "@page 0.2\n  title T\n:: row Title\n",
            1,
            4,
            Language::Ylang,
            Version::V02,
            None,
        )
        .unwrap();
        assert!(doc.markdown.contains("title"));

        // A paragraph inside a section that happens to start with "title " is
        // not metadata.
        let nested = "@page 0.2\n  title T\n:: Section\n  title not metadata\n";
        assert!(hover_doc(nested, 3, 4, Language::Ylang, Version::V02, None).is_none());

        let doc = hover_doc(
            "@page 0.2\n  title T\n:: row Title\n",
            2,
            4,
            Language::Ylang,
            Version::V02,
            None,
        )
        .unwrap();
        assert!(doc.markdown.contains("four"));

        let doc = hover_doc(
            "@page 0.1\n  title T\n:: row Title\n",
            2,
            4,
            Language::Ylang,
            Version::V01,
            None,
        )
        .unwrap();
        assert!(doc.markdown.contains("Named section"));
        assert!(!doc.markdown.contains("four"));
    }

    #[test]
    fn inline_and_image_hover_follow_the_version() {
        let text = "@page 0.2\n  title T\nA [strong: value] line\n! cover | Alt\n";
        let doc = hover_doc(text, 2, 6, Language::Ylang, Version::V02, None).unwrap();
        assert!(doc.markdown.contains("Inline `strong`"));
        assert!(doc.markdown.contains("cannot nest"));
        let doc = hover_doc(text, 3, 2, Language::Ylang, Version::V02, None).unwrap();
        assert!(doc.markdown.contains("64"));

        let old = "@page 0.1\n  title T\nA [strong: value] line\n! cover | Alt\n";
        assert!(hover_doc(old, 2, 6, Language::Ylang, Version::V01, None).is_none());
        assert!(hover_doc(old, 3, 2, Language::Ylang, Version::V01, None).is_none());
    }

    #[test]
    fn local_link_hover_uses_the_site_root_without_network_access() {
        let site = blog_site();
        let text = "@page 0.2\n  title T\n> /about | About\n";
        let doc = hover_doc(text, 2, 5, Language::Ylang, Version::V02, Some(&site)).unwrap();
        assert!(doc.markdown.contains("about.ylang"));
        let text = "@page 0.2\n  title T\n> /missing | Missing\n";
        let doc = hover_doc(text, 2, 5, Language::Ylang, Version::V02, Some(&site)).unwrap();
        assert!(doc.markdown.contains("No `missing.ylang`"));
        let text = "@page 0.2\n  title T\n> yami://yn1-abc/home | Other\n";
        let doc = hover_doc(text, 2, 5, Language::Ylang, Version::V02, Some(&site)).unwrap();
        assert!(doc.markdown.contains("never resolves or fetches"));
        // A local-looking target is not a valid 0.1 link and is not documented
        // as one.
        let old = "@page 0.1\n  title T\n> /about | About\n";
        assert!(hover_doc(old, 2, 5, Language::Ylang, Version::V01, Some(&site)).is_none());
    }

    #[test]
    fn ysl_group_and_property_docs_are_version_specific() {
        let text = "@style 0.2\ncard\n  radius 14\n";
        let doc = hover_doc(text, 1, 1, Language::Ysl, Version::V02, None).unwrap();
        assert!(doc.markdown.contains("`card`"));
        let doc = hover_doc(text, 2, 3, Language::Ysl, Version::V02, None).unwrap();
        assert!(doc.markdown.contains("0..32"));
        assert!(doc.markdown.contains("Default 14"));

        let text = "@style 0.1\ncanvas\n  width 76\n";
        let doc = hover_doc(text, 2, 3, Language::Ysl, Version::V01, None).unwrap();
        assert!(doc.markdown.contains("20..160"));
        let text = "@style 0.1\ntext\n  size 18\n";
        assert!(hover_doc(text, 2, 3, Language::Ysl, Version::V01, None).is_none());
        let text = "@style 0.2\ntext\n  size 18\n";
        let doc = hover_doc(text, 2, 3, Language::Ysl, Version::V02, None).unwrap();
        assert!(doc.markdown.contains("12..72"));
    }
}
