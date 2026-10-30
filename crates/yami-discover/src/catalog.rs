//! Merged, deduplicated local catalog and bounded search.
use crate::{
    normalize::fold, Directory, DirectoryEntry, Error, Result, WebsiteAddress, MAX_QUERY_CHARS,
    MAX_RESULTS, MAX_SEARCH_TERMS, MAX_SOURCES, MAX_SOURCE_LABEL_CHARS, MAX_TOTAL_ENTRIES,
};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Normalized {
    title: String,
    description: String,
    category: String,
    address: String,
}

/// One imported website listing.
///
/// The canonical address is stored exactly as it would be shown to the user.
/// No signature, connection or trust state is attached: importing a directory
/// does not authenticate anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    title: String,
    description: String,
    address: WebsiteAddress,
    category: Option<String>,
    source: String,
    normalized: Normalized,
}

impl Entry {
    fn from_directory(entry: &DirectoryEntry, source: &str) -> Self {
        let category = entry.category().map(str::to_string);
        let normalized = Normalized {
            title: fold(entry.title()),
            description: fold(entry.description()),
            category: fold(entry.category().unwrap_or("")),
            // Leave the scheme out of the search index so that the common word
            // "yami" does not match every listing. Searching a pasted address
            // still works because query terms drop the same scheme prefix.
            address: fold(&format!(
                "{}/{}",
                entry.address().node_id(),
                entry.address().key()
            )),
        };
        Self {
            title: entry.title().to_string(),
            description: entry.description().to_string(),
            address: entry.address().clone(),
            category,
            source: source.to_string(),
            normalized,
        }
    }

    /// The human-readable title supplied by the directory operator.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The short description supplied by the directory operator.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The canonical destination. Opening it must still authenticate the node.
    pub fn address(&self) -> &WebsiteAddress {
        &self.address
    }

    /// The optional category supplied by the directory operator.
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    /// The label of the directory source that supplied this entry.
    pub fn source(&self) -> &str {
        &self.source
    }

    fn score(&self, terms: &[String]) -> Option<u32> {
        let mut score: u32 = 0;
        for term in terms {
            let term = term.as_str();
            let mut matched = false;
            if self.normalized.title.contains(term) {
                score += 60;
                matched = true;
            }
            if self.normalized.category.contains(term) {
                score += 35;
                matched = true;
            }
            if self.normalized.description.contains(term) {
                score += 20;
                matched = true;
            }
            if self.normalized.address.contains(term) {
                score += 10;
                matched = true;
            }
            if !matched {
                return None;
            }
            if self.normalized.title == term {
                score += 200;
            } else if self.normalized.title.starts_with(term) {
                score += 100;
            } else if self
                .normalized
                .title
                .split_whitespace()
                .any(|word| word.starts_with(term))
            {
                score += 40;
            }
        }
        Some(score)
    }
}

/// Bounded information about one imported directory source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    /// Importer-chosen unique label.
    pub label: String,
    /// Display name from the directory document.
    pub name: String,
    /// Directory format version.
    pub version: u32,
    /// Listings read from the document.
    pub entries: usize,
    /// Listings added to the merged catalog.
    pub added: usize,
    /// Listings skipped because their canonical address was already present.
    pub duplicates: usize,
}

/// Result of importing one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    /// What the source contained and how much of it was retained.
    pub source: SourceInfo,
    /// Total unique entries in the merged catalog after the import.
    pub catalog_entries: usize,
}

/// One local search match and its deterministic relevance score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchHit<'a> {
    /// The matching entry.
    pub entry: &'a Entry,
    /// Higher scores sort first. Scores are local ranking hints, not trust.
    pub score: u32,
}

/// A local, multi-source, deduplicated directory index.
///
/// Sources are merged in import order. When two sources list the same canonical
/// address, the first imported entry is retained and the later one is counted
/// as a duplicate. Duplicate entries inside one source are handled the same
/// way. The catalog is bounded by [`MAX_SOURCES`] and [`MAX_TOTAL_ENTRIES`].
///
/// The catalog performs no I/O except the explicit [`Catalog::import_file`]
/// call. It never opens a website and never contacts a directory operator.
#[derive(Debug, Default)]
pub struct Catalog {
    sources: Vec<SourceInfo>,
    entries: Vec<Entry>,
    addresses: HashSet<WebsiteAddress>,
}

impl Catalog {
    /// Creates an empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Imports one directory document.
    pub fn import_text(&mut self, label: &str, source: &str) -> Result<ImportReport> {
        let directory = Directory::parse(source)?;
        self.add_directory(label, directory)
    }

    /// Loads and imports one local directory file.
    pub fn import_file(
        &mut self,
        label: &str,
        path: impl AsRef<std::path::Path>,
    ) -> Result<ImportReport> {
        let directory = Directory::load(path)?;
        self.add_directory(label, directory)
    }

    /// Merges an already parsed directory under a unique source label.
    pub fn add_directory(&mut self, label: &str, directory: Directory) -> Result<ImportReport> {
        let label = source_label(label)?;
        if self.sources.iter().any(|source| source.label == label) {
            return Err(Error::source(format!(
                "source {label:?} is already imported; choose a unique label"
            )));
        }
        if self.sources.len() >= MAX_SOURCES {
            return Err(Error::limit(format!(
                "at most {MAX_SOURCES} directory sources can be imported"
            )));
        }

        let mut pending = HashSet::new();
        let mut additions = 0usize;
        for entry in directory.entries() {
            if !self.addresses.contains(entry.address()) && pending.insert(entry.address().clone())
            {
                additions += 1;
            }
        }
        if self.entries.len().saturating_add(additions) > MAX_TOTAL_ENTRIES {
            return Err(Error::limit(format!(
                "merged catalog would exceed {MAX_TOTAL_ENTRIES} unique entries"
            )));
        }

        let source_entries = directory.entries().len();
        let mut added = Vec::with_capacity(additions);
        let mut source_addresses = HashSet::new();
        for entry in directory.entries() {
            if self.addresses.contains(entry.address())
                || !source_addresses.insert(entry.address().clone())
            {
                continue;
            }
            added.push(Entry::from_directory(entry, &label));
        }
        debug_assert_eq!(added.len(), additions);

        let first_new = self.entries.len();
        self.entries.extend(added);
        for entry in &self.entries[first_new..] {
            self.addresses.insert(entry.address().clone());
        }
        let source = SourceInfo {
            label,
            name: directory.name().to_string(),
            version: directory.version(),
            entries: source_entries,
            added: additions,
            duplicates: source_entries - additions,
        };
        self.sources.push(source.clone());
        Ok(ImportReport {
            source,
            catalog_entries: self.entries.len(),
        })
    }

    /// The imported sources in import order.
    pub fn sources(&self) -> &[SourceInfo] {
        &self.sources
    }

    /// The merged unique entries in import order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Number of unique entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no entries have been imported.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Searches locally, returning at most [`MAX_RESULTS`] matches.
    ///
    /// Matching is case- and common-accent-insensitive substring matching over
    /// title, description, category and canonical address. Multiple
    /// whitespace-separated query terms must all match somewhere in an entry.
    /// An empty query lists all entries in deterministic relevance/title order.
    /// No part of the query leaves the process.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit<'_>>> {
        self.search_with_limit(query, MAX_RESULTS)
    }

    /// Searches locally with an explicit limit, capped at [`MAX_RESULTS`].
    pub fn search_with_limit(&self, query: &str, limit: usize) -> Result<Vec<SearchHit<'_>>> {
        let terms = query_terms(query)?;
        let limit = limit.min(MAX_RESULTS);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for entry in &self.entries {
            if terms.is_empty() {
                hits.push(SearchHit { entry, score: 0 });
            } else if let Some(score) = entry.score(&terms) {
                hits.push(SearchHit { entry, score });
            }
        }
        hits.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| {
                    left.entry
                        .normalized
                        .title
                        .cmp(&right.entry.normalized.title)
                })
                .then_with(|| {
                    left.entry
                        .normalized
                        .address
                        .cmp(&right.entry.normalized.address)
                })
                .then_with(|| left.entry.source.cmp(&right.entry.source))
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

pub(crate) fn source_label(label: &str) -> Result<String> {
    let label = label.trim();
    if label.is_empty() {
        return Err(Error::source("label cannot be empty"));
    }
    if label.chars().count() > MAX_SOURCE_LABEL_CHARS {
        return Err(Error::source(format!(
            "label exceeds {MAX_SOURCE_LABEL_CHARS} characters"
        )));
    }
    if label.chars().any(char::is_control) {
        return Err(Error::source("label contains control characters"));
    }
    Ok(label.to_string())
}

fn query_terms(query: &str) -> Result<Vec<String>> {
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(Error::query(format!(
            "query exceeds {MAX_QUERY_CHARS} characters"
        )));
    }
    if query.chars().any(char::is_control) {
        return Err(Error::query("query contains control characters"));
    }
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| {
            let folded = fold(term);
            folded
                .strip_prefix("yami://")
                .unwrap_or(&folded)
                .to_string()
        })
        .filter(|term| !term.is_empty())
        .collect();
    if terms.is_empty() {
        return Err(Error::query("query has no searchable terms"));
    }
    if terms.len() > MAX_SEARCH_TERMS {
        return Err(Error::query(format!(
            "query has {} terms; the maximum is {MAX_SEARCH_TERMS}",
            terms.len()
        )));
    }
    Ok(terms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_labels_and_queries_are_bounded() {
        assert_eq!(source_label("  local  ").unwrap(), "local");
        assert!(source_label(" ").is_err());
        assert!(source_label(&"x".repeat(MAX_SOURCE_LABEL_CHARS + 1)).is_err());
        assert!(query_terms("").unwrap().is_empty());
        assert!(query_terms("yami://").is_err());
        assert_eq!(
            query_terms("YAMI://ABC/Home").unwrap(),
            ["abc/home".to_string()]
        );
        assert!(query_terms(&"x".repeat(MAX_QUERY_CHARS + 1)).is_err());
        assert!(query_terms(&"x ".repeat(MAX_SEARCH_TERMS + 1)).is_err());
        assert!(query_terms("line\nbreak").is_err());
    }
}
