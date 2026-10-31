//! In-memory document store for unsaved editor buffers.
//!
//! Documents are identified by URI and never read from disk. Both the number of
//! open documents and the retained text size are bounded so a misbehaving client
//! cannot make the server grow without limit.

use std::collections::BTreeMap;
use std::fmt;

use crate::spec::{self, Language};
use crate::text::{TextChange, TextDocument};

/// Maximum number of simultaneously open editor documents.
pub const MAX_DOCUMENTS: usize = 64;
/// Maximum retained text per document. This is four times the YLang source
/// limit, leaving room for clients to send transiently oversized buffers while
/// keeping total retained memory below roughly 64 MiB.
pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
/// Maximum content changes accepted from one `didChange` notification.
pub const MAX_CHANGES_PER_NOTIFICATION: usize = 64;

#[derive(Clone, Debug)]
pub struct Document {
    uri: String,
    language_id: String,
    language: Option<Language>,
    version: i64,
    text: Option<TextDocument>,
}

impl Document {
    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn version(&self) -> i64 {
        self.version
    }

    pub fn language(&self) -> Option<Language> {
        self.language
    }

    pub fn text(&self) -> Option<&str> {
        self.text.as_ref().map(TextDocument::text)
    }

    pub fn text_document(&self) -> Option<&TextDocument> {
        self.text.as_ref()
    }

    pub fn is_too_large(&self) -> bool {
        self.text.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    TooManyDocuments,
    TooManyChanges,
    CannotApplyIncrementalChange,
    InvalidChange(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("document is not open"),
            Self::TooManyDocuments => f.write_str("too many open documents"),
            Self::TooManyChanges => f.write_str("too many changes in one notification"),
            Self::CannotApplyIncrementalChange => {
                f.write_str("document is too large to apply incremental changes")
            }
            Self::InvalidChange(message) => write!(f, "invalid change: {message}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeOutcome {
    Applied,
    /// The change was applied but the resulting buffer exceeded the retained
    /// text bound, so its content was deliberately dropped.
    TooLarge,
}

#[derive(Debug, Default)]
pub struct DocumentStore {
    documents: BTreeMap<String, Document>,
}

impl DocumentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(uri)
    }

    pub fn open(
        &mut self,
        uri: &str,
        language_id: &str,
        version: i64,
        text: String,
    ) -> Result<(), StoreError> {
        if !self.documents.contains_key(uri) && self.documents.len() >= MAX_DOCUMENTS {
            return Err(StoreError::TooManyDocuments);
        }
        let language = spec::detect_language(uri, language_id, &text);
        let retained = if text.len() <= MAX_DOCUMENT_BYTES {
            Some(TextDocument::new(text))
        } else {
            None
        };
        self.documents.insert(
            uri.to_string(),
            Document {
                uri: uri.to_string(),
                language_id: language_id.to_string(),
                language,
                version,
                text: retained,
            },
        );
        Ok(())
    }

    pub fn change(
        &mut self,
        uri: &str,
        version: i64,
        changes: &[TextChange],
    ) -> Result<ChangeOutcome, StoreError> {
        let document = self.documents.get_mut(uri).ok_or(StoreError::NotFound)?;
        document.version = version;
        if changes.len() > MAX_CHANGES_PER_NOTIFICATION {
            return Err(StoreError::TooManyChanges);
        }
        if changes.is_empty() {
            return Ok(ChangeOutcome::Applied);
        }

        let outcome = match document.text.as_mut() {
            Some(text_document) => {
                let mut candidate = text_document.clone();
                candidate
                    .apply_all(changes)
                    .map_err(|error| StoreError::InvalidChange(error.to_string()))?;
                if candidate.len() > MAX_DOCUMENT_BYTES {
                    document.text = None;
                    ChangeOutcome::TooLarge
                } else {
                    document.language =
                        spec::detect_language(uri, &document.language_id, candidate.text());
                    document.text = Some(candidate);
                    ChangeOutcome::Applied
                }
            }
            None => {
                // Incremental ranges require the old text, which was discarded
                // to respect the memory bound. A single full replacement can
                // still bring the buffer back within bounds.
                let full = changes
                    .iter()
                    .filter(|change| change.range.is_none())
                    .count();
                if changes.len() == 1 && full == 1 {
                    let text = changes[0].text.clone();
                    if text.len() <= MAX_DOCUMENT_BYTES {
                        document.language =
                            spec::detect_language(uri, &document.language_id, &text);
                        document.text = Some(TextDocument::new(text));
                        ChangeOutcome::Applied
                    } else {
                        ChangeOutcome::TooLarge
                    }
                } else {
                    return Err(StoreError::CannotApplyIncrementalChange);
                }
            }
        };
        Ok(outcome)
    }

    pub fn close(&mut self, uri: &str) -> bool {
        self.documents.remove(uri).is_some()
    }

    pub fn clear(&mut self) {
        self.documents.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{Position, Range};

    const BLOG_HOME: &str = include_str!("../../../../../sites/blog/home.ylang");

    #[test]
    fn opens_unsaved_buffers_without_disk_access() {
        let mut store = DocumentStore::new();
        store
            .open(
                "untitled:1",
                "ylang",
                1,
                "@page 0.2\n  title Unsaved".to_string(),
            )
            .unwrap();
        let document = store.get("untitled:1").unwrap();
        assert_eq!(document.version(), 1);
        assert_eq!(document.language(), Some(Language::Ylang));
        assert_eq!(document.text(), Some("@page 0.2\n  title Unsaved"));
        assert!(!document.is_too_large());
        assert!(store.close("untitled:1"));
        assert!(store.get("untitled:1").is_none());
        assert!(!store.close("untitled:1"));
    }

    #[test]
    fn applies_utf16_incremental_changes_and_tracks_versions() {
        let mut store = DocumentStore::new();
        store
            .open("file:///site/home.ylang", "ylang", 1, BLOG_HOME.to_string())
            .unwrap();
        let document = store.get("file:///site/home.ylang").unwrap();
        let text_document = document.text_document().unwrap();
        // Replace "Thoughts" on line 3 with "Unicode 😀".
        let line = 2;
        let start = text_document.offset_at(Position::new(line, 0));
        let end = text_document.offset_at(Position::new(line, 8));
        let range = Range::new(
            text_document.position_at(start),
            text_document.position_at(end),
        );
        let outcome = store
            .change(
                "file:///site/home.ylang",
                2,
                &[TextChange {
                    range: Some(range),
                    text: "Unicode 😀".to_string(),
                }],
            )
            .unwrap();
        assert_eq!(outcome, ChangeOutcome::Applied);
        let document = store.get("file:///site/home.ylang").unwrap();
        assert_eq!(document.version(), 2);
        assert!(document.text().unwrap().contains("Unicode 😀"));
    }

    #[test]
    fn full_replacements_can_shrink_an_oversized_buffer() {
        let mut store = DocumentStore::new();
        let huge = "x".repeat(MAX_DOCUMENT_BYTES + 1);
        store.open("untitled:big", "ylang", 1, huge).unwrap();
        assert!(store.get("untitled:big").unwrap().is_too_large());
        // Incremental edits cannot be applied without the old text.
        assert_eq!(
            store.change(
                "untitled:big",
                2,
                &[TextChange {
                    range: Some(Range::new(Position::new(0, 0), Position::new(0, 1))),
                    text: "y".into()
                }]
            ),
            Err(StoreError::CannotApplyIncrementalChange)
        );
        assert_eq!(
            store
                .change(
                    "untitled:big",
                    3,
                    &[TextChange {
                        range: None,
                        text: "@page 0.2\n  title Small".into()
                    }]
                )
                .unwrap(),
            ChangeOutcome::Applied
        );
        assert_eq!(
            store.get("untitled:big").unwrap().text(),
            Some("@page 0.2\n  title Small")
        );
        assert_eq!(store.get("untitled:big").unwrap().version(), 3);
    }

    #[test]
    fn invalid_and_excessive_change_payloads_leave_text_untouched() {
        let mut store = DocumentStore::new();
        store
            .open("untitled:1", "ylang", 1, "@page 0.2\n  title T".to_string())
            .unwrap();
        let original = store.get("untitled:1").unwrap().text().unwrap().to_string();
        let reversed = TextChange {
            range: Some(Range::new(Position::new(1, 5), Position::new(1, 1))),
            text: "x".into(),
        };
        assert!(matches!(
            store.change("untitled:1", 2, &[reversed]),
            Err(StoreError::InvalidChange(_))
        ));
        assert_eq!(
            store.get("untitled:1").unwrap().text(),
            Some(original.as_str())
        );
        let too_many = vec![
            TextChange {
                range: None,
                text: "x".into()
            };
            MAX_CHANGES_PER_NOTIFICATION + 1
        ];
        assert_eq!(
            store.change("untitled:1", 3, &too_many),
            Err(StoreError::TooManyChanges)
        );
        assert_eq!(
            store.get("untitled:1").unwrap().text(),
            Some(original.as_str())
        );
    }

    #[test]
    fn document_count_is_bounded() {
        let mut store = DocumentStore::new();
        for index in 0..MAX_DOCUMENTS {
            store
                .open(&format!("untitled:{index}"), "ylang", 0, String::new())
                .unwrap();
        }
        assert_eq!(store.len(), MAX_DOCUMENTS);
        assert_eq!(
            store.open("untitled:overflow", "ylang", 0, String::new()),
            Err(StoreError::TooManyDocuments)
        );
        // Re-opening an existing URI is allowed.
        store
            .open("untitled:0", "ylang", 1, "@page 0.1".to_string())
            .unwrap();
    }
}
