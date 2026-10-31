use crate::{
    files, invalid, text, Result, WebsiteAddress, FORMAT_VERSION, MAX_INDEX_BYTES, MAX_PAGES,
    MAX_PAGE_TEXT, MAX_POSTINGS, MAX_QUERY_CHARS, MAX_QUERY_TERMS, MAX_RESULTS, MAX_TERMS,
    MAX_TOTAL_TEXT,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use yami_identity::NodeId;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub key: String,
    pub title: String,
    pub headings: String,
    pub text: String,
    pub address: Option<String>,
}
impl Page {
    /// Deterministic identity; local pages do not acquire a fabricated node ID.
    pub fn id(&self) -> String {
        self.address
            .clone()
            .unwrap_or_else(|| format!("local:{}", self.key))
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if !valid_key(&self.key)
            || self.title.is_empty()
            || self.title.len() > 4096
            || self.title.len() + self.headings.len() + self.text.len() > MAX_PAGE_TEXT
            || [&self.title, &self.headings, &self.text]
                .iter()
                .any(|s| s.chars().any(|c| c.is_control() && c != '\n'))
        {
            return Err(invalid("invalid page key/text or page budget exceeded"));
        }
        if let Some(address) = &self.address {
            let parsed = WebsiteAddress::parse(address).map_err(|e| invalid(e.to_string()))?;
            if parsed.key() != self.key || parsed.url() != *address {
                return Err(invalid("inconsistent canonical page address"));
            }
        }
        Ok(())
    }
}
pub(crate) fn valid_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    ylang::valid_key(key)
        && !["con", "prn", "aux", "nul"].contains(&lower.as_str())
        && !(lower.len() == 4
            && (lower.starts_with("com") || lower.starts_with("lpt"))
            && matches!(lower.as_bytes()[3], b'1'..=b'9'))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    root: Option<PathBuf>,
    identity: Option<String>,
    pages: Vec<Page>,
}

#[derive(Clone)]
pub struct Index {
    snapshot: Snapshot,
    postings: BTreeMap<String, BTreeMap<usize, u32>>,
}
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub page: Page,
    pub snippet: String,
    pub score: u32,
    pub source: String,
}
#[derive(Debug)]
pub struct Stats {
    pub pages: usize,
    pub terms: usize,
    pub postings: usize,
    pub text_bytes: usize,
}

impl Index {
    /// Index the same flat `<key>.ylang` website layout served by yami-node.
    /// Unrelated files and subdirectories are never opened.
    pub fn build(root: impl AsRef<Path>, identity: Option<NodeId>) -> Result<Self> {
        files::check_path(root.as_ref())?;
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(invalid("website must be a directory"));
        }
        let mut paths = Vec::new();
        for (count, entry) in fs::read_dir(&root)?.enumerate() {
            if count >= 4096 {
                return Err(invalid("website inventory exceeds 4096 entries"));
            }
            let entry = entry?;
            if entry.path().extension().is_some_and(|s| s == "ylang") {
                if !entry.file_type()?.is_file() {
                    return Err(invalid("YLang pages must be regular non-symlink files"));
                }
                paths.push(entry.path());
                if paths.len() > MAX_PAGES {
                    return Err(invalid("too many pages"));
                }
            }
        }
        paths.sort();
        let mut pages = Vec::new();
        let mut total = 0;
        for path in paths {
            let key = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| invalid("non-UTF8 key"))?;
            if !valid_key(key) {
                return Err(invalid("unsafe page key"));
            }
            let source = files::read(&path, ylang::MAX_SOURCE)?;
            let doc = ylang::parse(source.strip_prefix('\u{feff}').unwrap_or(&source))
                .map_err(|e| invalid(format!("{key}: {e}")))?;
            let mut page = Page {
                key: key.into(),
                title: doc.metadata.title,
                headings: String::new(),
                text: String::new(),
                address: identity.map(|id| format!("yami://{id}/{key}")),
            };
            text::extract(&doc.nodes, &mut page.headings, &mut page.text)?;
            page.validate()?;
            total += page.title.len() + page.headings.len() + page.text.len();
            if total > MAX_TOTAL_TEXT {
                return Err(invalid("site text exceeds budget"));
            }
            pages.push(page);
        }
        Self::from_snapshot(Snapshot {
            version: FORMAT_VERSION,
            root: Some(root),
            identity: identity.map(|id| id.to_string()),
            pages,
        })
    }

    /// Validated imported pages, with no local rebuild path.
    pub fn from_pages(pages: Vec<Page>) -> Result<Self> {
        Self::from_snapshot(Snapshot {
            version: FORMAT_VERSION,
            root: None,
            identity: None,
            pages,
        })
    }
    fn from_snapshot(mut snapshot: Snapshot) -> Result<Self> {
        if snapshot.version != FORMAT_VERSION || snapshot.pages.len() > MAX_PAGES {
            return Err(invalid("unsupported index version or page limit"));
        }
        let identity = snapshot
            .identity
            .as_deref()
            .map(str::parse::<NodeId>)
            .transpose()
            .map_err(|_| invalid("invalid index identity"))?;
        if let Some(root) = &snapshot.root {
            if !root.is_absolute() || root.as_os_str().len() > 4096 {
                return Err(invalid("invalid rebuild root"));
            }
        }
        snapshot.pages.sort_by_key(Page::id);
        let mut ids = BTreeSet::new();
        let mut total = 0;
        let mut postings = BTreeMap::<String, BTreeMap<usize, u32>>::new();
        let mut posting_count = 0;
        for (i, page) in snapshot.pages.iter().enumerate() {
            page.validate()?;
            if snapshot.root.is_some()
                && page.address != identity.map(|id| format!("yami://{id}/{}", page.key))
            {
                return Err(invalid("page address differs from supplied index identity"));
            }
            if !ids.insert(page.id()) {
                return Err(invalid("duplicate page identifier"));
            }
            total += page.title.len() + page.headings.len() + page.text.len();
            if total > MAX_TOTAL_TEXT {
                return Err(invalid("index text exceeds budget"));
            }
            for (field, weight) in [(&page.title, 12), (&page.headings, 6), (&page.text, 1)] {
                for term in text::tokens(field) {
                    if term.len() > 256 {
                        return Err(invalid("token exceeds 256 bytes"));
                    }
                    let posting = postings.entry(term).or_default();
                    if !posting.contains_key(&i) {
                        posting_count += 1;
                    }
                    let score = posting.entry(i).or_default();
                    *score = score.saturating_add(weight).min(1024);
                    if postings.len() > MAX_TERMS || posting_count > MAX_POSTINGS {
                        return Err(invalid("vocabulary/posting budget exceeded"));
                    }
                }
            }
        }
        Ok(Self { snapshot, postings })
    }
    pub fn pages(&self) -> &[Page] {
        &self.snapshot.pages
    }
    pub fn source(&self) -> String {
        self.snapshot.identity.clone().unwrap_or_else(|| {
            self.snapshot
                .root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "imported".into())
        })
    }
    pub fn stats(&self) -> Stats {
        Stats {
            pages: self.pages().len(),
            terms: self.postings.len(),
            postings: self.postings.values().map(BTreeMap::len).sum(),
            text_bytes: self
                .pages()
                .iter()
                .map(|p| p.text.len() + p.title.len() + p.headings.len())
                .sum(),
        }
    }
    pub fn save(&self, directory: impl AsRef<Path>) -> Result<()> {
        files::directory(directory.as_ref())?;
        let text = serde_json::to_string(&self.snapshot).map_err(|e| invalid(e.to_string()))?;
        if text.len() > MAX_INDEX_BYTES {
            return Err(invalid("serialized index exceeds byte budget"));
        }
        files::write(&directory.as_ref().join("index.json"), &text)
    }
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        let text = files::read(&directory.as_ref().join("index.json"), MAX_INDEX_BYTES)?;
        let snapshot =
            serde_json::from_str(&text).map_err(|e| invalid(format!("invalid index: {e}")))?;
        Self::from_snapshot(snapshot)
    }
    pub fn rebuild(&self) -> Result<Self> {
        let root = self
            .snapshot
            .root
            .as_ref()
            .ok_or_else(|| invalid("imported indexes have no rebuild root"))?;
        let identity = self
            .snapshot
            .identity
            .as_deref()
            .map(str::parse::<NodeId>)
            .transpose()
            .map_err(|_| invalid("invalid identity"))?;
        Self::build(root, identity)
    }
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let terms = query_terms(query)?;
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut scores: Option<BTreeMap<usize, u32>> = None;
        for term in &terms {
            let mut matches = BTreeMap::<usize, u32>::new();
            // Partial matching scans a bounded vocabulary; postings avoid page rescans.
            for (word, posting) in &self.postings {
                if word.contains(term) {
                    for (&page, &weight) in posting {
                        let value = matches.entry(page).or_default();
                        *value = (*value).max(weight * if word == term { 2 } else { 1 });
                    }
                }
            }
            scores = Some(match scores {
                None => matches,
                Some(previous) => previous
                    .into_iter()
                    .filter_map(|(p, s)| matches.get(&p).map(|w| (p, s + w)))
                    .collect(),
            });
        }
        let mut ranked: Vec<_> = scores.unwrap_or_default().into_iter().collect();
        ranked.sort_by(|(a, sa), (b, sb)| {
            sb.cmp(sa)
                .then_with(|| self.pages()[*a].id().cmp(&self.pages()[*b].id()))
        });
        Ok(ranked
            .into_iter()
            .take(limit.min(MAX_RESULTS))
            .map(|(i, score)| {
                let page = self.pages()[i].clone();
                let snippet = text::snippet(
                    if page.text.is_empty() {
                        &page.title
                    } else {
                        &page.text
                    },
                    &terms,
                );
                SearchHit {
                    page,
                    snippet,
                    score,
                    source: self.source(),
                }
            })
            .collect())
    }
}
pub(crate) fn query_terms(query: &str) -> Result<Vec<String>> {
    if query.chars().count() > MAX_QUERY_CHARS || query.chars().any(char::is_control) {
        return Err(invalid(
            "query exceeds character limit or contains controls",
        ));
    }
    let terms: BTreeSet<_> = text::tokens(query).into_iter().collect();
    if terms.len() > MAX_QUERY_TERMS {
        return Err(invalid("too many query terms"));
    }
    Ok(terms.into_iter().collect())
}
