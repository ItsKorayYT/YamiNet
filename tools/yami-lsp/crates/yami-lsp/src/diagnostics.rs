//! Syntax diagnostics built directly from the shipped YLang/YSL parsers.

use crate::spec::Language;
use crate::text::{Position, Range, TextDocument};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: Range,
    pub source: &'static str,
    pub message: String,
    pub code: &'static str,
}

/// The parsers stop at the first error, so one diagnostic is accurate and
/// bounded. Keeping the constant explicit documents that contract.
pub const MAX_DIAGNOSTICS: usize = 1;

/// Parse an in-memory document and convert the parser's UTF-8 byte offsets to
/// LSP UTF-16 positions. A successful parse produces no diagnostics.
pub fn compute(language: Option<Language>, document: &TextDocument) -> Vec<Diagnostic> {
    match language {
        Some(Language::Ylang) => match ylang::parse(document.text()) {
            Ok(_) => Vec::new(),
            Err(error) => vec![Diagnostic {
                range: span_range(document, error.span.start, error.span.end),
                source: "ylang",
                message: error.message,
                code: "syntax",
            }],
        },
        Some(Language::Ysl) => match ysl::parse(document.text()) {
            Ok(_) => Vec::new(),
            Err(error) => {
                let end = error
                    .offset
                    .checked_add(
                        document
                            .text()
                            .get(error.offset..)
                            .and_then(|rest| rest.chars().next())
                            .map_or(0, char::len_utf8),
                    )
                    .unwrap_or(error.offset);
                vec![Diagnostic {
                    range: span_range(document, error.offset, end),
                    source: "ysl",
                    message: error.message,
                    code: "syntax",
                }]
            }
        },
        None => Vec::new(),
    }
}

fn span_range(document: &TextDocument, start: usize, end: usize) -> Range {
    let len = document.len();
    let start = start.min(len);
    let mut end = end.min(len).max(start);
    if start == end {
        if let Some(ch) = document
            .text()
            .get(start..)
            .and_then(|rest| rest.chars().next())
        {
            end = (start + ch.len_utf8()).min(len);
        }
    }
    document.range_between(start, end)
}

/// Diagnostic used when a document exceeds the editor-side memory bound and is
/// deliberately not retained or parsed.
pub fn too_large(limit: usize) -> Diagnostic {
    Diagnostic {
        range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        source: "yami-lsp",
        message: format!(
            "document exceeds the language-server limit of {} KiB and was not parsed",
            limit / 1024
        ),
        code: "limits",
    }
}

/// Diagnostic used when a `didChange` payload could not be applied. The old
/// text is retained rather than corrupting the buffer.
pub fn rejected_change(message: &str) -> Diagnostic {
    Diagnostic {
        range: Range::new(Position::new(0, 0), Position::new(0, 0)),
        source: "yami-lsp",
        message: format!("could not apply text change: {message}"),
        code: "sync",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_samples_parse_without_diagnostics() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        for relative in [
            "examples/welcome.ylang",
            "examples/default.ysl",
            "sites/blog/home.ylang",
            "sites/blog/about.ylang",
            "sites/blog/default.ysl",
            "sites/alice/home.ylang",
            "sites/showcase/home.ylang",
        ] {
            let text = std::fs::read_to_string(root.join(relative)).unwrap();
            let language = if relative.ends_with(".ysl") {
                Language::Ysl
            } else {
                Language::Ylang
            };
            let document = TextDocument::new(text);
            assert!(
                compute(Some(language), &document).is_empty(),
                "{relative} should parse cleanly"
            );
        }
    }

    #[test]
    fn ylang_diagnostic_uses_utf16_columns() {
        let document = TextDocument::new("@page 0.2\n  title T\n😀😀 [bad: value]\n");
        let diagnostics = compute(Some(Language::Ylang), &document);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].source, "ylang");
        assert_eq!(diagnostics[0].range.start, Position::new(2, 5));
        assert!(diagnostics[0].message.contains("expected"));
    }

    #[test]
    fn ysl_diagnostic_points_at_the_value() {
        let document = TextDocument::new("@style 0.2\ncanvas\n  width 9999\n");
        let diagnostics = compute(Some(Language::Ysl), &document);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].source, "ysl");
        assert_eq!(diagnostics[0].range.start, Position::new(2, 8));
        assert!(diagnostics[0].message.contains("240..1440"));
    }

    #[test]
    fn version_specific_syntax_is_reported_by_the_real_parsers() {
        let ylang_01 = TextDocument::new("@page 0.1\n  title T\n> /about | About\n");
        assert_eq!(compute(Some(Language::Ylang), &ylang_01).len(), 1);
        let ylang_02 = TextDocument::new("@page 0.2\n  title T\n> /about | About\n");
        assert!(compute(Some(Language::Ylang), &ylang_02).is_empty());

        let ysl_01 = TextDocument::new("@style 0.1\ncanvas\n  width 900\n");
        assert_eq!(compute(Some(Language::Ysl), &ysl_01).len(), 1);
        let ysl_02 = TextDocument::new("@style 0.2\ncanvas\n  width 900\n");
        assert!(compute(Some(Language::Ysl), &ysl_02).is_empty());
    }

    #[test]
    fn oversized_and_unknown_documents_are_safe() {
        let too_big = TextDocument::new("x".repeat(ylang::MAX_SOURCE + 1));
        let diagnostics = compute(Some(Language::Ylang), &too_big);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].range.start, Position::new(0, 0));
        assert!(compute(None, &too_big).is_empty());
        assert_eq!(too_large(1024 * 1024).code, "limits");
    }
}
