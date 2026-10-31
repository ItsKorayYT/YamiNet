//! Context-aware YLang/YSL completion.
//!
//! Completions are derived from the same tables as hover documentation and are
//! filtered by the explicitly selected grammar version. Unknown contexts return
//! an empty list rather than guessing at syntax the parser would reject.

use crate::site::SiteRoot;
use crate::spec::{self, ItemKind, Language, Version};
use crate::text::TextDocument;

pub const MAX_ITEMS: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: ItemKind,
    pub detail: String,
    pub documentation: String,
    /// Byte offsets in the document; the server converts them to UTF-16 ranges.
    pub edit_start: usize,
    pub edit_end: usize,
    pub new_text: String,
}

fn item(
    label: impl Into<String>,
    kind: ItemKind,
    detail: impl Into<String>,
    documentation: impl Into<String>,
    edit_start: usize,
    edit_end: usize,
    new_text: impl Into<String>,
) -> CompletionItem {
    CompletionItem {
        label: label.into(),
        kind,
        detail: detail.into(),
        documentation: documentation.into(),
        edit_start,
        edit_end,
        new_text: new_text.into(),
    }
}

fn push_unique(items: &mut Vec<CompletionItem>, candidate: CompletionItem) {
    if items.len() >= MAX_ITEMS {
        return;
    }
    if items
        .iter()
        .any(|existing| existing.label == candidate.label && existing.detail == candidate.detail)
    {
        return;
    }
    items.push(candidate);
}

struct Context<'a> {
    line: u32,
    line_start: usize,
    cursor: usize,
    indent: usize,
    prefix: &'a str,
    prefix_trim: &'a str,
}

impl<'a> Context<'a> {
    fn new(document: &'a TextDocument, offset: usize) -> Option<Self> {
        let line = document.position_at(offset).line;
        let (line_start, line_end) = document.line_bounds(line)?;
        let cursor = offset.clamp(line_start, line_end);
        let prefix = &document.text()[line_start..cursor];
        let indent = prefix.bytes().take_while(|byte| *byte == b' ').count();
        let prefix_trim = &prefix[indent..];
        Some(Self {
            line,
            line_start,
            cursor,
            indent,
            prefix,
            prefix_trim,
        })
    }

    fn word_start(&self) -> usize {
        self.line_start + self.indent
    }
}

/// Compute completion items for the cursor at `offset` (a UTF-8 byte offset).
pub fn complete(
    document: &TextDocument,
    offset: usize,
    language: Language,
    version: Version,
    site: Option<&SiteRoot>,
) -> Vec<CompletionItem> {
    let Some(context) = Context::new(document, offset) else {
        return Vec::new();
    };
    match language {
        Language::Ylang => complete_ylang(document, &context, version, site),
        Language::Ysl => complete_ysl(document, &context, version),
    }
}

fn complete_ylang(
    document: &TextDocument,
    context: &Context<'_>,
    version: Version,
    site: Option<&SiteRoot>,
) -> Vec<CompletionItem> {
    let first = first_nonblank(document);
    let nonblank_before = any_nonblank_before(document, context.line);

    // The header must be the first nonblank line. Offer it on any blank line
    // before existing content so leading blank lines stay valid.
    if !nonblank_before {
        if context.indent == 0
            && (context.prefix_trim.is_empty() || context.prefix_trim.starts_with('@'))
        {
            return header_items(spec::YLANG_HEADERS, context);
        }
        if context.prefix_trim.is_empty() {
            return Vec::new();
        }
    }
    let Some((header_line, _header_text)) = first else {
        return Vec::new();
    };
    if context.line == header_line {
        return if context.indent == 0 {
            header_items(spec::YLANG_HEADERS, context)
        } else {
            Vec::new()
        };
    }
    if context.prefix_trim.starts_with('@') {
        return Vec::new();
    }

    // Metadata is only valid before the first body block. On an unindented
    // blank line both metadata fields and the first body block are useful, so
    // offer both; once fields are used they are omitted.
    if let Some(used) = metadata_keys(document, header_line, context.line) {
        if context.prefix_trim.is_empty() {
            if context.indent < 2 {
                let mut items = metadata_items(context, true, &used);
                items.extend(block_starter_items(context, version));
                return items;
            }
            if context.indent == 2 {
                return metadata_items(context, false, &used);
            }
            return Vec::new();
        }
        if context.indent == 2 && !context.prefix_trim.contains(' ') {
            return metadata_items(context, false, &used);
        }
        if context.indent == 2 && context.prefix_trim.contains(' ') {
            return Vec::new();
        }
    }

    body_items(document, context, version, site)
}

fn complete_ysl(
    document: &TextDocument,
    context: &Context<'_>,
    version: Version,
) -> Vec<CompletionItem> {
    let first = first_nonblank(document);
    let nonblank_before = any_nonblank_before(document, context.line);
    if !nonblank_before {
        if context.indent == 0
            && (context.prefix_trim.is_empty() || context.prefix_trim.starts_with('@'))
        {
            return header_items(spec::YSL_HEADERS, context);
        }
        return Vec::new();
    }
    let Some((header_line, _)) = first else {
        return Vec::new();
    };
    if context.line <= header_line {
        return Vec::new();
    }
    if context.prefix_trim.starts_with('@') {
        return Vec::new();
    }

    if context.indent == 0 {
        if context.prefix_trim.contains(' ') {
            return Vec::new();
        }
        let mut items = Vec::new();
        for group in spec::groups_for(version) {
            if !group.name.starts_with(context.prefix_trim) {
                continue;
            }
            push_unique(
                &mut items,
                item(
                    group.name,
                    ItemKind::Keyword,
                    group.detail,
                    group.doc,
                    context.word_start(),
                    context.cursor,
                    group.name,
                ),
            );
        }
        return items;
    }
    if context.indent != 2 {
        return Vec::new();
    }

    let Some(group) = current_group(document, header_line, context.line, version) else {
        return Vec::new();
    };
    let content = &context.prefix[context.indent..];
    if let Some(whitespace) = content.find(char::is_whitespace) {
        let name = &content[..whitespace];
        let after = &content[whitespace..];
        let value_offset = whitespace + (after.len() - after.trim_start().len());
        let value_start = context.line_start + context.indent + value_offset;
        let Some(property) = spec::property_spec(group, name, version) else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for suggestion in spec::value_suggestions(property) {
            push_unique(
                &mut items,
                item(
                    suggestion.label,
                    suggestion.kind,
                    suggestion.detail,
                    suggestion.doc,
                    value_start,
                    context.cursor,
                    suggestion.insert,
                ),
            );
        }
        return items;
    }

    let mut items = Vec::new();
    for property in spec::properties_for(group, version) {
        if !property.name.starts_with(content) {
            continue;
        }
        let detail = format!("{}.{} ({})", group, property.name, property.default);
        push_unique(
            &mut items,
            item(
                property.name,
                ItemKind::Property,
                detail,
                property.doc,
                context.word_start(),
                context.cursor,
                format!("{} ", property.name),
            ),
        );
    }
    items
}

fn body_items(
    document: &TextDocument,
    context: &Context<'_>,
    version: Version,
    site: Option<&SiteRoot>,
) -> Vec<CompletionItem> {
    // YLang 0.2 inline spans can appear inside any text-bearing block.
    if version == Version::V02 {
        if let Some((start, content)) = unclosed_bracket(context.prefix) {
            let mut items = Vec::new();
            for inline in spec::YLANG_INLINE {
                if !inline_matches(inline.marker, content) {
                    continue;
                }
                push_unique(
                    &mut items,
                    item(
                        inline.label,
                        ItemKind::Snippet,
                        format!("[{}: text]", inline.marker),
                        inline.doc,
                        context.line_start + start,
                        context.cursor,
                        inline.insert,
                    ),
                );
            }
            if !items.is_empty() {
                return items;
            }
        }
    }

    // Direct children of `:: list` / `:: steps` use marker items.
    if let Some(ordered) = list_parent(document, context.line, context.indent) {
        return list_item_items(context, version, ordered);
    }

    if context.prefix_trim.starts_with("::") {
        let mut items = Vec::new();
        for block in spec::blocks_for(version) {
            if !block.insert.starts_with("::") || !block.insert.starts_with(context.prefix_trim) {
                continue;
            }
            push_unique(
                &mut items,
                item(
                    block.label,
                    block.kind,
                    block.detail,
                    block.doc,
                    context.word_start(),
                    context.cursor,
                    block.insert,
                ),
            );
        }
        return items;
    }

    if context.prefix_trim.starts_with('=') {
        let equals = context
            .prefix_trim
            .bytes()
            .take_while(|byte| *byte == b'=')
            .count();
        if equals > 0 && context.prefix_trim[equals..].trim().is_empty() {
            let mut items = Vec::new();
            for level in 1..=6u8 {
                let insert = format!("{} ", "=".repeat(level as usize));
                if !insert.starts_with(context.prefix_trim) {
                    continue;
                }
                push_unique(
                    &mut items,
                    item(
                        format!("{} Heading {level}", "=".repeat(level as usize)),
                        ItemKind::Keyword,
                        format!("heading level {level}"),
                        spec::DOC_HEADING,
                        context.word_start(),
                        context.cursor,
                        insert,
                    ),
                );
            }
            return items;
        }
        return Vec::new();
    }

    if context.prefix_trim.starts_with('>') {
        return link_items(context, version, site);
    }

    if context.prefix_trim.starts_with('!') {
        if version == Version::V02 && context.prefix_trim.trim() == "!" {
            for block in spec::blocks_for(version) {
                if block.insert.starts_with('!') {
                    return vec![item(
                        block.label,
                        block.kind,
                        block.detail,
                        block.doc,
                        context.word_start(),
                        context.cursor,
                        block.insert,
                    )];
                }
            }
        }
        return Vec::new();
    }

    if version == Version::V02 && context.prefix_trim == "-" {
        if let Some(block) = spec::blocks_for(version)
            .into_iter()
            .find(|block| block.insert == "---")
        {
            return vec![item(
                block.label,
                block.kind,
                block.detail,
                block.doc,
                context.word_start(),
                context.cursor,
                block.insert,
            )];
        }
        return Vec::new();
    }

    if context.prefix_trim.is_empty() {
        return block_starter_items(context, version);
    }

    Vec::new()
}

fn block_starter_items(context: &Context<'_>, version: Version) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for block in spec::blocks_for(version) {
        push_unique(
            &mut items,
            item(
                block.label,
                block.kind,
                block.detail,
                block.doc,
                context.word_start(),
                context.cursor,
                block.insert,
            ),
        );
    }
    items
}

fn list_item_items(context: &Context<'_>, version: Version, ordered: bool) -> Vec<CompletionItem> {
    // Nested blocks are valid inside a list item.
    if context.prefix_trim.starts_with("::") {
        let mut items = Vec::new();
        for block in spec::blocks_for(version) {
            if !block.insert.starts_with("::") || !block.insert.starts_with(context.prefix_trim) {
                continue;
            }
            push_unique(
                &mut items,
                item(
                    block.label,
                    block.kind,
                    block.detail,
                    block.doc,
                    context.word_start(),
                    context.cursor,
                    block.insert,
                ),
            );
        }
        return items;
    }
    if context.prefix_trim.starts_with('=') {
        return Vec::new();
    }
    let marker = if ordered { '+' } else { '-' };
    let typed = context.prefix_trim.strip_prefix(marker);
    let marker_empty = typed.is_some_and(|rest| rest.trim().is_empty());
    if context.prefix_trim.is_empty() || marker_empty {
        let doc = if ordered {
            spec::DOC_STEPS
        } else {
            spec::DOC_LIST
        };
        return vec![item(
            format!("{marker} Item"),
            ItemKind::Snippet,
            if ordered { "step item" } else { "list item" },
            doc,
            context.word_start(),
            context.cursor,
            format!("{marker} "),
        )];
    }
    Vec::new()
}

fn link_items(
    context: &Context<'_>,
    version: Version,
    site: Option<&SiteRoot>,
) -> Vec<CompletionItem> {
    let after = &context.prefix_trim[1..];
    let leading = after.len() - after.trim_start().len();
    let target = after.trim_start();
    let target_start = context.word_start() + 1 + leading;

    if target.contains(" | ") {
        return Vec::new();
    }

    if let Some(typed) = target.strip_prefix('/') {
        if version != Version::V02 {
            return Vec::new();
        }
        let key_start = target_start + 1;
        let mut items = Vec::new();
        if let Some(site) = site {
            for key in site.page_keys() {
                if !key.starts_with(typed) {
                    continue;
                }
                let label = page_label(&key);
                push_unique(
                    &mut items,
                    item(
                        format!("/{key}"),
                        ItemKind::File,
                        "local page",
                        format!(
                            "Resolves to `{key}.ylang` in the selected website directory. \
                             Only local, single-component page keys are listed."
                        ),
                        key_start,
                        context.cursor,
                        format!("{key} | {label}"),
                    ),
                );
            }
        }
        return items;
    }

    if target.is_empty() {
        let mut items = Vec::new();
        for block in spec::blocks_for(version) {
            if !block.insert.starts_with("> ") {
                continue;
            }
            push_unique(
                &mut items,
                item(
                    block.label,
                    block.kind,
                    block.detail,
                    block.doc,
                    context.word_start(),
                    context.cursor,
                    block.insert,
                ),
            );
        }
        if version == Version::V02 {
            if let Some(site) = site {
                for key in site.page_keys() {
                    let label = page_label(&key);
                    push_unique(
                        &mut items,
                        item(
                            format!("/{key}"),
                            ItemKind::File,
                            "local page",
                            format!(
                                "Resolves to `{key}.ylang` in the selected website directory. \
                                 Only local, single-component page keys are listed."
                            ),
                            context.word_start(),
                            context.cursor,
                            format!("> /{key} | {label}"),
                        ),
                    );
                }
            }
        }
        return items;
    }

    let mut items = Vec::new();
    for block in spec::blocks_for(version) {
        if !block.insert.starts_with("> ") {
            continue;
        }
        let target_text = block.insert.trim_start_matches("> ");
        if !target_text.starts_with(target) {
            continue;
        }
        push_unique(
            &mut items,
            item(
                block.label,
                block.kind,
                block.detail,
                block.doc,
                context.word_start(),
                context.cursor,
                block.insert,
            ),
        );
    }
    items
}

fn page_label(key: &str) -> String {
    let mut label = String::new();
    for part in key.split(['-', '_']) {
        if !label.is_empty() {
            label.push(' ');
        }
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            label.extend(first.to_uppercase());
            label.push_str(chars.as_str());
        }
    }
    if label.is_empty() {
        key.to_string()
    } else {
        label
    }
}

fn header_items(specs: &'static [spec::HeaderSpec], context: &Context<'_>) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for header in specs {
        if !context.prefix_trim.is_empty() && !header.label.starts_with(context.prefix_trim) {
            continue;
        }
        push_unique(
            &mut items,
            item(
                header.label,
                ItemKind::Snippet,
                header.detail,
                header.doc,
                context.word_start(),
                context.cursor,
                header.insert,
            ),
        );
    }
    items
}

fn metadata_items(
    context: &Context<'_>,
    insert_indent: bool,
    used: &[&str],
) -> Vec<CompletionItem> {
    let mut items = Vec::new();
    for field in spec::META_FIELDS {
        if used.contains(&field.name) {
            continue;
        }
        if !context.prefix_trim.is_empty() && !field.name.starts_with(context.prefix_trim) {
            continue;
        }
        let (edit_start, new_text) = if insert_indent {
            (context.line_start, format!("  {}", field.insert))
        } else {
            (context.word_start(), field.insert.to_string())
        };
        let detail = if field.required {
            format!("{} {}", field.detail, "· required")
        } else {
            field.detail.to_string()
        };
        push_unique(
            &mut items,
            item(
                field.name,
                ItemKind::Property,
                detail,
                field.doc,
                edit_start,
                context.cursor,
                new_text,
            ),
        );
    }
    items
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

fn any_nonblank_before(document: &TextDocument, line: u32) -> bool {
    (0..line).any(|index| {
        document
            .line_text(index)
            .is_some_and(|text| !text.trim().is_empty())
    })
}

fn metadata_keys(
    document: &TextDocument,
    header_line: u32,
    current_line: u32,
) -> Option<Vec<&'static str>> {
    if current_line <= header_line {
        return None;
    }
    let mut used = Vec::new();
    for line in header_line + 1..current_line {
        let text = document.line_text(line)?;
        if text.trim().is_empty() {
            continue;
        }
        let indent = text.bytes().take_while(|byte| *byte == b' ').count();
        if indent != 2 {
            return None;
        }
        let content = &text[indent..];
        let key = content.split_whitespace().next().unwrap_or("");
        let field = spec::meta_field(key)?;
        used.push(field.name);
    }
    Some(used)
}

/// Return the ordered flag when the current line is a direct child of a list.
fn list_parent(document: &TextDocument, line: u32, indent: usize) -> Option<bool> {
    if indent < 2 {
        return None;
    }
    let mut index = line;
    while index > 0 {
        index -= 1;
        let text = document.line_text(index)?;
        if text.trim().is_empty() {
            continue;
        }
        let previous_indent = text.bytes().take_while(|byte| *byte == b' ').count();
        if previous_indent >= indent {
            continue;
        }
        if previous_indent + 2 != indent {
            return None;
        }
        return match text[previous_indent..].trim_end() {
            ":: list" => Some(false),
            ":: steps" => Some(true),
            _ => None,
        };
    }
    None
}

fn current_group(
    document: &TextDocument,
    header_line: u32,
    current_line: u32,
    version: Version,
) -> Option<&'static str> {
    for line in (header_line + 1..current_line).rev() {
        let Some(text) = document.line_text(line) else {
            break;
        };
        if text.trim().is_empty() {
            continue;
        }
        let indent = text.bytes().take_while(|byte| *byte == b' ').count();
        if indent != 0 {
            continue;
        }
        let name = text.trim_end();
        return spec::group_spec(name, version).map(|group| group.name);
    }
    None
}

/// Find the last unclosed `[` in the line prefix and return its byte offset and
/// the content typed since it. Escaped brackets are ignored.
fn unclosed_bracket(prefix: &str) -> Option<(usize, &str)> {
    let mut open = None;
    let mut escaped = false;
    for (index, ch) in prefix.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' => open = Some(index),
            ']' => open = None,
            _ => {}
        }
    }
    open.map(|index| (index, &prefix[index + 1..]))
}

fn inline_matches(marker: &str, content: &str) -> bool {
    if marker.starts_with(content) {
        return true;
    }
    let Some(rest) = content.strip_prefix(marker) else {
        return false;
    };
    rest.is_empty() || (rest.starts_with(':') && rest[1..].trim().is_empty() && rest.len() <= 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn blog_site() -> SiteRoot {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        SiteRoot::open(&root.join("sites/blog")).unwrap()
    }

    fn item_labels(items: &[CompletionItem]) -> Vec<&str> {
        items.iter().map(|item| item.label.as_str()).collect()
    }

    fn complete_at(
        text: &str,
        line: u32,
        character: u32,
        language: Language,
        version: Version,
        site: Option<&SiteRoot>,
    ) -> Vec<CompletionItem> {
        let document = TextDocument::new(text);
        let offset = document.offset_at(crate::text::Position::new(line, character));
        complete(&document, offset, language, version, site)
    }

    #[test]
    fn header_completions_offer_both_versions_only() {
        let items = complete_at("", 0, 0, Language::Ylang, Version::V02, None);
        assert_eq!(item_labels(&items), ["@page 0.1", "@page 0.2"]);
        let items = complete_at("@p", 0, 2, Language::Ylang, Version::V02, None);
        assert_eq!(items.len(), 2);
        let items = complete_at("", 0, 0, Language::Ysl, Version::V02, None);
        assert_eq!(item_labels(&items), ["@style 0.1", "@style 0.2"]);
        assert!(complete_at("hello", 0, 5, Language::Ylang, Version::V02, None).is_empty());
    }

    #[test]
    fn metadata_completions_stop_after_the_first_body_block() {
        let text = "@page 0.2\n  ";
        let items = complete_at(text, 1, 2, Language::Ylang, Version::V02, None);
        assert!(item_labels(&items).contains(&"title"));
        assert!(item_labels(&items).contains(&"language"));
        let text = "@page 0.2\n  title A\n= Heading\n";
        let items = complete_at(text, 3, 0, Language::Ylang, Version::V02, None);
        assert!(!item_labels(&items).contains(&"title"));
    }

    #[test]
    fn ylang_01_never_suggests_02_features() {
        let text = "@page 0.1\n  title A\n";
        let items = complete_at(text, 2, 0, Language::Ylang, Version::V01, None);
        let labels = item_labels(&items);
        assert!(labels.contains(&"= Heading"));
        assert!(labels.contains(&":: Section"));
        assert!(labels.contains(&"> YNP link"));
        assert!(!labels.contains(&":: card Title"));
        assert!(!labels.contains(&"! Image"));
        assert!(!labels.contains(&"--- Separator"));
        assert!(!labels.contains(&":: list"));

        let text = "@page 0.2\n  title A\n";
        let items = complete_at(text, 2, 0, Language::Ylang, Version::V02, None);
        let labels = item_labels(&items);
        assert!(labels.contains(&":: card Title"));
        assert!(labels.contains(&"! Image"));
        assert!(labels.contains(&"--- Separator"));
        assert!(labels.contains(&":: list"));
    }

    #[test]
    fn inline_spans_complete_only_in_02() {
        let text = "@page 0.2\n  title A\nText [str";
        let items = complete_at(text, 2, 11, Language::Ylang, Version::V02, None);
        assert_eq!(item_labels(&items), ["strong"]);
        let text = "@page 0.1\n  title A\nText [str";
        let items = complete_at(text, 2, 11, Language::Ylang, Version::V01, None);
        assert!(items.is_empty());
        let text = "@page 0.2\n  title A\nText [strong: hello";
        let items = complete_at(text, 2, 21, Language::Ylang, Version::V02, None);
        assert!(items.is_empty());
    }

    #[test]
    fn list_items_are_offered_only_with_the_right_marker() {
        let text = "@page 0.2\n  title A\n:: list\n  ";
        let items = complete_at(text, 3, 2, Language::Ylang, Version::V02, None);
        assert_eq!(item_labels(&items), ["- Item"]);
        let text = "@page 0.2\n  title A\n:: steps\n  ";
        let items = complete_at(text, 3, 2, Language::Ylang, Version::V02, None);
        assert_eq!(item_labels(&items), ["+ Item"]);
        let text = "@page 0.2\n  title A\n:: list\n  - existing\n  ";
        let items = complete_at(text, 4, 2, Language::Ylang, Version::V02, None);
        assert_eq!(item_labels(&items), ["- Item"]);
    }

    #[test]
    fn local_page_keys_complete_from_the_selected_site() {
        let site = blog_site();
        let text = "@page 0.2\n  title A\n> /";
        let items = complete_at(text, 2, 3, Language::Ylang, Version::V02, Some(&site));
        let labels = item_labels(&items);
        assert!(labels.contains(&"/about"));
        assert!(labels.contains(&"/home"));
        assert!(!labels.iter().any(|label| label.contains("default")));
        let item = items.iter().find(|item| item.label == "/about").unwrap();
        assert_eq!(item.new_text, "about | About");
        let text = "@page 0.2\n  title A\n> /ab";
        let items = complete_at(text, 2, 5, Language::Ylang, Version::V02, Some(&site));
        assert_eq!(item_labels(&items), ["/about"]);
        // YLang 0.1 has no local page links.
        let items = complete_at(
            "@page 0.1\n  title A\n> /",
            2,
            3,
            Language::Ylang,
            Version::V01,
            Some(&site),
        );
        assert!(items.is_empty());
    }

    #[test]
    fn ysl_groups_and_properties_respect_the_version() {
        let text = "@style 0.1\n";
        let items = complete_at(text, 1, 0, Language::Ysl, Version::V01, None);
        let labels = item_labels(&items);
        assert_eq!(labels, ["canvas", "text", "heading", "section", "link"]);
        assert!(!labels.contains(&"card"));

        let text = "@style 0.1\ntext\n  ";
        let items = complete_at(text, 2, 2, Language::Ysl, Version::V01, None);
        assert_eq!(item_labels(&items), ["ink", "gap"]);
        let text = "@style 0.2\ntext\n  ";
        let items = complete_at(text, 2, 2, Language::Ysl, Version::V02, None);
        let labels = item_labels(&items);
        assert!(labels.contains(&"size"));
        assert!(labels.contains(&"line-height"));
        assert!(labels.contains(&"align"));

        let text = "@style 0.2\ntext\n  align ";
        let items = complete_at(text, 2, 8, Language::Ysl, Version::V02, None);
        assert_eq!(item_labels(&items), ["left", "center", "right"]);
        let text = "@style 0.2\ntext\n  weight ";
        let items = complete_at(text, 2, 9, Language::Ysl, Version::V02, None);
        assert_eq!(item_labels(&items), ["400", "600"]);
        let text = "@style 0.2\ncanvas\n  width ";
        let items = complete_at(text, 2, 8, Language::Ysl, Version::V02, None);
        assert_eq!(item_labels(&items), ["960"]);
        let text = "@style 0.1\ncanvas\n  width ";
        let items = complete_at(text, 2, 8, Language::Ysl, Version::V01, None);
        assert_eq!(item_labels(&items), ["76"]);
    }

    #[test]
    fn completions_are_bounded() {
        let text = format!(
            "@page 0.2\n  title A\n{}",
            (0..1000).map(|_| "=\n").collect::<String>()
        );
        let items = complete_at(&text, 2, 0, Language::Ylang, Version::V02, None);
        assert!(items.len() <= MAX_ITEMS);
    }
}
