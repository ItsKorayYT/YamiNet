//! Bounded, local-first website discovery for YamiNet 0.4.
//!
//! A *directory* is an ordinary YLang 0.2 document that lists websites by
//! title, short description, canonical `yami://` address and optional category.
//! Directory documents are inert data: this crate never fetches a page, never
//! contacts a directory operator and never sends a search query anywhere.
//!
//! [`Catalog`] imports one or more independent directory sources, merges their
//! entries and removes duplicate canonical addresses. [`Catalog::search`]
//! performs a bounded, Unicode-aware local search over the merged entries.
//!
//! # Trust boundary
//!
//! A directory entry is an **untrusted hint**. It is not a certificate, a
//! signature, a bookmark or a trust decision. The canonical `yami://` address
//! still names an identity, and the existing YamiNet networking stack must
//! authenticate that identity over TLS 1.3 when the page is opened. A
//! directory operator can list any address with any description, can omit
//! entries and can disagree with another operator. Nothing in this crate
//! changes TLS, DHT, relays or browser security.
//!
//! These types are independent of the browser UI so a later milestone can
//! integrate discovery without changing the format or trust model.
//!
//! Directories can also be authored locally: [`Directory::render`] produces a
//! canonical YLang 0.2 document, [`Directory::save`] and
//! [`Directory::save_new`] write it atomically, and the entry mutation methods
//! reject duplicate addresses and entry-count overflow.

mod address;
mod catalog;
mod directory;
mod normalize;
mod store;

pub use address::WebsiteAddress;
pub use catalog::{Catalog, Entry, ImportReport, SearchHit, SourceInfo};
pub use directory::{Directory, DirectoryEntry};

use std::{fmt, io};
use ylang::{Span, SyntaxError};

/// The only directory format version understood by this build.
pub const DIRECTORY_FORMAT_VERSION: u32 = 1;
/// Maximum size of one directory document in bytes.
///
/// This deliberately reuses YLang's document limit instead of inventing a
/// second, larger parser budget.
pub const MAX_DIRECTORY_BYTES: usize = ylang::MAX_SOURCE;
/// Maximum entries in one directory document.
pub const MAX_ENTRIES_PER_DIRECTORY: usize = 512;
/// Maximum unique entries retained by one merged [`Catalog`].
pub const MAX_TOTAL_ENTRIES: usize = 2048;
/// Maximum independent directory sources in one [`Catalog`].
pub const MAX_SOURCES: usize = 32;
/// Maximum characters in a directory display name.
pub const MAX_NAME_CHARS: usize = 80;
/// Maximum characters in an entry title.
pub const MAX_TITLE_CHARS: usize = 120;
/// Maximum characters in an entry description.
pub const MAX_DESCRIPTION_CHARS: usize = 320;
/// Maximum characters in an entry category.
pub const MAX_CATEGORY_CHARS: usize = 48;
/// Maximum characters in a source label chosen by the importer.
pub const MAX_SOURCE_LABEL_CHARS: usize = 80;
/// Maximum characters in one local search query.
pub const MAX_QUERY_CHARS: usize = 128;
/// Maximum whitespace-separated terms in one search query.
pub const MAX_SEARCH_TERMS: usize = 8;
/// Maximum results returned by one search, regardless of the requested limit.
pub const MAX_RESULTS: usize = 100;
/// Maximum canonical address size accepted from a directory document.
pub const MAX_ADDRESS_BYTES: usize = 200;

/// A 1-based source location, used for actionable validation errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl Location {
    pub const START: Self = Self { line: 1, column: 1 };

    fn from_span(span: &Span) -> Self {
        Self {
            line: span.line,
            column: span.column,
        }
    }
}

impl Default for Location {
    fn default() -> Self {
        Self::START
    }
}

/// Result type used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from parsing, importing or searching directory data.
#[derive(Debug)]
pub enum Error {
    /// A directory file could not be read.
    Io(io::Error),
    /// The input exceeded [`MAX_DIRECTORY_BYTES`].
    TooLarge { limit: usize },
    /// The embedded YLang parser rejected the document.
    Syntax(SyntaxError),
    /// A document or entry violated the directory format.
    Invalid {
        location: Option<Location>,
        message: String,
    },
    /// A bounded catalog resource would be exceeded.
    Limit { message: String },
    /// Two directory entries or sources cannot coexist.
    Conflict { message: String },
    /// A source label or source collection is invalid.
    Source { message: String },
    /// A local search query is invalid.
    Query { message: String },
}

impl Error {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            location: None,
            message: message.into(),
        }
    }

    pub(crate) fn at(span: &Span, message: impl Into<String>) -> Self {
        Self::Invalid {
            location: Some(Location::from_span(span)),
            message: message.into(),
        }
    }

    pub(crate) fn limit(message: impl Into<String>) -> Self {
        Self::Limit {
            message: message.into(),
        }
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    pub(crate) fn source(message: impl Into<String>) -> Self {
        Self::Source {
            message: message.into(),
        }
    }

    pub(crate) fn query(message: impl Into<String>) -> Self {
        Self::Query {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::TooLarge { limit } => write!(f, "input exceeds {limit} bytes"),
            Self::Syntax(error) => write!(f, "YLang syntax error: {error}"),
            Self::Invalid {
                location: Some(location),
                message,
            } => write!(
                f,
                "line {}, column {}: {message}",
                location.line, location.column
            ),
            Self::Invalid {
                location: None,
                message,
            } => f.write_str(message),
            Self::Limit { message } => write!(f, "resource limit: {message}"),
            Self::Conflict { message } => write!(f, "conflict: {message}"),
            Self::Source { message } => write!(f, "directory source: {message}"),
            Self::Query { message } => write!(f, "search query: {message}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Syntax(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<SyntaxError> for Error {
    fn from(error: SyntaxError) -> Self {
        Self::Syntax(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const COMMUNITY: &str = include_str!("../../../sites/directories/community.ylang");
    const GARDEN: &str = include_str!("../../../sites/directories/garden.ylang");
    const LAB: &str = include_str!("../../../sites/directories/lab.ylang");
    const ALICE_ID: &str = "yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re";

    fn sample_catalog() -> Catalog {
        let mut catalog = Catalog::new();
        catalog.import_text("community", COMMUNITY).unwrap();
        catalog.import_text("garden", GARDEN).unwrap();
        catalog.import_text("lab", LAB).unwrap();
        catalog
    }

    fn address(key: &str) -> WebsiteAddress {
        WebsiteAddress::parse(&format!("yami://{ALICE_ID}/{key}")).unwrap()
    }

    fn entry(index: usize) -> DirectoryEntry {
        DirectoryEntry::new(
            format!("Entry {index}"),
            "Description",
            address(&format!("p{index}")),
            None,
        )
        .unwrap()
    }

    fn directory(start: usize, count: usize) -> Directory {
        Directory::new(
            format!("Directory {start}"),
            (start..start + count).map(entry).collect(),
        )
        .unwrap()
    }

    fn document(body: &str) -> String {
        format!("@page 0.2\n  title Test Directory\n{body}")
    }

    fn card(body: &str) -> String {
        document(&format!(":: directory 1\n  :: card Entry\n{body}"))
    }

    #[test]
    fn sample_documents_parse_with_expected_metadata() {
        let community = Directory::parse(COMMUNITY).unwrap();
        assert_eq!(community.version(), 1);
        assert_eq!(community.name(), "YamiNet Community Directory");
        assert_eq!(community.entries().len(), 4);
        assert_eq!(community.entries()[0].title(), "Alice's Notebook");
        assert_eq!(community.entries()[0].category(), Some("Personal"));
        assert_eq!(
            community.entries()[0].address().to_string(),
            format!("yami://{ALICE_ID}/home")
        );

        let garden = Directory::parse(GARDEN).unwrap();
        assert_eq!(garden.entries().len(), 4);
        assert_eq!(garden.entries()[2].title(), "日本語の手帖");
        assert_eq!(garden.entries()[2].category(), Some("日記"));
        assert_eq!(garden.entries()[3].category(), None);
    }

    #[test]
    fn merging_is_deterministic_and_deduplicates_canonical_addresses() {
        let catalog = sample_catalog();
        assert_eq!(catalog.sources().len(), 3);
        assert_eq!(catalog.len(), 10);
        assert_eq!(catalog.sources()[0].added, 4);
        assert_eq!(catalog.sources()[1].added, 3);
        assert_eq!(catalog.sources()[1].duplicates, 1);
        assert_eq!(catalog.sources()[2].added, 3);
        assert_eq!(catalog.sources()[2].duplicates, 1);

        let alice = catalog
            .entries()
            .iter()
            .find(|entry| entry.title() == "Alice's Notebook")
            .expect("first source keeps the entry");
        assert_eq!(alice.source(), "community");
        assert!(catalog
            .entries()
            .iter()
            .all(|entry| entry.title() != "Alice's Field Notes"));
        let quiet = catalog
            .entries()
            .iter()
            .find(|entry| entry.title() == "The Quiet Corner")
            .unwrap();
        assert_eq!(quiet.source(), "community");
        assert_eq!(quiet.category(), Some("Journal"));
    }

    #[test]
    fn duplicate_within_one_source_is_counted_and_first_metadata_wins() {
        let mut catalog = Catalog::new();
        let source = format!(
            "@page 0.2\n  title Listing\n:: directory 1\n  :: card First\n    > yami://{ALICE_ID} | Open\n    First description\n\n  :: card Second\n    > yami://{ALICE_ID}/home | Open\n    Second description\n    :: category Other\n"
        );
        let report = catalog.import_text("mixed", &source).unwrap();
        assert_eq!(report.source.entries, 2);
        assert_eq!(report.source.added, 1);
        assert_eq!(report.source.duplicates, 1);
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.entries()[0].title(), "First");
    }

    #[test]
    fn local_search_handles_partial_case_and_unicode_matches() {
        let catalog = sample_catalog();
        let cafe = catalog.search("cafe").unwrap();
        assert_eq!(cafe.len(), 1);
        assert_eq!(cafe[0].entry.title(), "Café & Code");
        assert_eq!(
            catalog.search("CAFÉ").unwrap()[0].entry.title(),
            "Café & Code"
        );

        let japanese = catalog.search("日本").unwrap();
        assert_eq!(japanese.len(), 2);
        assert!(japanese
            .iter()
            .any(|hit| hit.entry.title() == "日本語の手帖"));
        assert_eq!(catalog.search("日記").unwrap().len(), 1);

        let partial = catalog.search("noteb").unwrap();
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].entry.title(), "Alice's Notebook");

        let multi = catalog.search("local first").unwrap();
        assert_eq!(multi.len(), 2);
        let titles: Vec<_> = multi.iter().map(|hit| hit.entry.title()).collect();
        assert!(titles.contains(&"Alice's Notebook"));
        assert!(titles.contains(&"Local-First Kitchen"));

        let address_query = catalog.search(&format!("yami://{ALICE_ID}/home")).unwrap();
        assert_eq!(address_query.len(), 1);
        assert_eq!(address_query[0].entry.title(), "Alice's Notebook");
        assert_eq!(catalog.search(&ALICE_ID[..10]).unwrap().len(), 1);
    }

    #[test]
    fn empty_queries_are_ordered_and_limited() {
        let catalog = sample_catalog();
        let all = catalog.search("").unwrap();
        assert_eq!(all.len(), 10);
        assert_eq!(all[0].entry.title(), "Alice's Notebook");
        assert_eq!(all[9].entry.title(), "日本語の手帖");
        assert_eq!(catalog.search_with_limit("", 3).unwrap().len(), 3);
        assert!(catalog.search_with_limit("", 0).unwrap().is_empty());
        assert_eq!(
            catalog.search_with_limit("", usize::MAX).unwrap().len(),
            catalog.len()
        );
    }

    #[test]
    fn search_queries_are_bounded() {
        let catalog = sample_catalog();
        assert!(catalog.search(&"x".repeat(MAX_QUERY_CHARS + 1)).is_err());
        assert!(catalog.search("line\nbreak").is_err());
        let terms = (0..=MAX_SEARCH_TERMS)
            .map(|index| format!("term{index}"))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(catalog.search(&terms).is_err());
        assert!(catalog.search("!!!").unwrap().is_empty());
    }

    #[test]
    fn malformed_documents_and_entries_are_rejected() {
        let valid = format!("    > yami://{ALICE_ID}/home | Open\n    Description\n");
        let cases = [
            (
                "legacy YLang version",
                format!("@page 0.1\n  title Test\n:: directory 1\n{valid}"),
            ),
            ("missing wrapper", document("")),
            (
                "unsupported version",
                document(":: directory 2\n"),
            ),
            (
                "extra top-level block",
                document(&format!(":: directory 1\n{valid}= Extra\n")),
            ),
            (
                "wrapper is not a section",
                document(":: card Entry\n"),
            ),
            (
                "entry is not a card",
                document(&format!(":: directory 1\n  :: row\n{valid}")),
            ),
            (
                "missing address",
                card("    Description\n"),
            ),
            (
                "missing description",
                card(&format!("    > yami://{ALICE_ID}/home | Open\n")),
            ),
            (
                "wrong scheme",
                card("    > ynps://example.com/home | Open\n    Description\n"),
            ),
            (
                "bad checksum",
                card(&format!("    > yami://yn1-{} | Open\n    Description\n", "a".repeat(58))),
            ),
            (
                "two links",
                card(&format!(
                    "    > yami://{ALICE_ID}/home | One\n    > yami://{ALICE_ID}/about | Two\n    Description\n"
                )),
            ),
            (
                "two descriptions",
                card(&format!("    > yami://{ALICE_ID}/home | Open\n    First\n\n    Second\n")),
            ),
            (
                "control character",
                card(&format!(
                    "    > yami://{ALICE_ID}/home | Open\n    Bad\ttext\n"
                )),
            ),
            (
                "heading inside entry",
                card(&format!("    > yami://{ALICE_ID}/home | Open\n    = Heading\n    Description\n")),
            ),
            (
                "unknown section",
                card(&format!("    > yami://{ALICE_ID}/home | Open\n    Description\n    :: group\n")),
            ),
            (
                "empty category",
                card(&format!("    > yami://{ALICE_ID}/home | Open\n    Description\n    :: category\n")),
            ),
            (
                "category with child",
                card(&format!(
                    "    > yami://{ALICE_ID}/home | Open\n    Description\n    :: category Personal\n      Child\n"
                )),
            ),
            (
                "two categories",
                card(&format!(
                    "    > yami://{ALICE_ID}/home | Open\n    Description\n    :: category One\n    :: category Two\n"
                )),
            ),
            (
                "oversized title",
                document(&format!(
                    ":: directory 1\n  :: card {}\n{valid}",
                    "x".repeat(MAX_TITLE_CHARS + 1)
                )),
            ),
            (
                "oversized description",
                card(&format!(
                    "    > yami://{ALICE_ID}/home | Open\n    {}\n",
                    "x".repeat(MAX_DESCRIPTION_CHARS + 1)
                )),
            ),
            (
                "oversized category",
                card(&format!(
                    "    > yami://{ALICE_ID}/home | Open\n    Description\n    :: category {}\n",
                    "x".repeat(MAX_CATEGORY_CHARS + 1)
                )),
            ),
            (
                "oversized directory title",
                format!(
                    "@page 0.2\n  title {}\n:: directory 1\n",
                    "x".repeat(MAX_NAME_CHARS + 1)
                ),
            ),
        ];
        for (name, source) in cases {
            assert!(Directory::parse(&source).is_err(), "{name}");
        }

        let error = Directory::parse(&card("    Description\n")).unwrap_err();
        assert!(matches!(
            error,
            Error::Invalid {
                location: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn oversized_documents_and_entry_counts_are_rejected() {
        assert!(matches!(
            Directory::parse(&"x".repeat(MAX_DIRECTORY_BYTES + 1)),
            Err(Error::TooLarge { .. })
        ));
        let mut source = String::from("@page 0.2\n  title Too Many\n:: directory 1\n");
        for index in 0..=MAX_ENTRIES_PER_DIRECTORY {
            source.push_str(&format!(
                "  :: card Entry {index}\n    > yami://{ALICE_ID}/p{index} | Open\n    Description\n"
            ));
        }
        let error = Directory::parse(&source).unwrap_err().to_string();
        assert!(error.contains("exceeds 512 entries"), "{error}");
    }

    #[test]
    fn catalog_source_and_entry_budgets_are_enforced_atomically() {
        let empty = || Directory::new("Empty", vec![]).unwrap();
        let mut catalog = Catalog::new();
        for index in 0..MAX_SOURCES {
            catalog
                .add_directory(&format!("source-{index}"), empty())
                .unwrap();
        }
        assert!(matches!(
            catalog.add_directory("one-too-many", empty()),
            Err(Error::Limit { .. })
        ));
        assert!(matches!(
            catalog.add_directory("source-0", empty()),
            Err(Error::Source { .. })
        ));

        let mut catalog = Catalog::new();
        for index in 0..4 {
            catalog
                .add_directory(&format!("source-{index}"), directory(index * 512, 512))
                .unwrap();
        }
        assert_eq!(catalog.len(), MAX_TOTAL_ENTRIES);
        let extra = Directory::new("Extra", vec![entry(MAX_TOTAL_ENTRIES)]).unwrap();
        assert!(matches!(
            catalog.add_directory("extra", extra),
            Err(Error::Limit { .. })
        ));
        assert_eq!(catalog.len(), MAX_TOTAL_ENTRIES);
    }

    #[test]
    fn result_limit_is_capped() {
        let mut catalog = Catalog::new();
        catalog
            .add_directory("many", directory(0, MAX_RESULTS + 1))
            .unwrap();
        assert_eq!(catalog.search("").unwrap().len(), MAX_RESULTS);
        assert_eq!(catalog.search_with_limit("", 3).unwrap().len(), 3);
        assert_eq!(
            catalog.search_with_limit("", usize::MAX).unwrap().len(),
            MAX_RESULTS
        );
    }

    #[test]
    fn empty_directories_and_inline_descriptions_are_supported() {
        let empty = Directory::parse("@page 0.2\n  title Empty\n:: directory 1\n").unwrap();
        assert!(empty.entries().is_empty());
        let source = card(&format!(
            "    > yami://{ALICE_ID}/home | Open\n    A [strong: bold] and [code: code] description.\n"
        ));
        let directory = Directory::parse(&source).unwrap();
        assert_eq!(
            directory.entries()[0].description(),
            "A bold and code description."
        );
    }

    #[test]
    fn files_are_loaded_with_bounds_and_directory_paths_are_refused() {
        let directory =
            std::env::temp_dir().join(format!("yami-discover-load-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let file = directory.join("community.ylang");
        fs::write(&file, COMMUNITY).unwrap();
        assert_eq!(Directory::load(&file).unwrap().entries().len(), 4);
        let mut catalog = Catalog::new();
        let report = catalog.import_file("community", &file).unwrap();
        assert_eq!(report.source.entries, 4);
        assert!(Directory::load(&directory).is_err());

        fs::write(&file, "x".repeat(MAX_DIRECTORY_BYTES + 1)).unwrap();
        assert!(matches!(
            Directory::load(&file),
            Err(Error::TooLarge { .. })
        ));
        let _ = fs::remove_dir_all(&directory);
    }
}
