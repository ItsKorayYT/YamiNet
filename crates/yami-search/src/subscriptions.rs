//! A single atomic state transaction keeps subscriptions, provenance, failure
//! status and complete validated snapshots together. Queries never fetch.
use crate::{
    engine::query_terms, files, invalid, AuthenticatedDocumentFetcher, Index, Manifest, Page,
    Result, SearchHit, WebsiteAddress, FORMAT_VERSION, MAX_RESULTS,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const MAX_PROVIDERS: usize = 8;
const MAX_STATE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHE_PAGES: usize = 512;
const MAX_CACHE_TEXT: usize = 8 * 1024 * 1024;
pub const REFRESH_BUDGET: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    pub name: String,
    pub address: String,
    pub last_success: Option<u64>,
    pub last_error: Option<String>,
    snapshot: Option<Vec<Page>>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderStatus {
    NotRefreshed,
    Cached { last_success: u64 },
    Stale { last_success: u64, error: String },
    Unavailable { error: String },
}
impl ProviderStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotRefreshed => "not refreshed",
            Self::Cached { .. } => "cached",
            Self::Stale { .. } => "cached/stale",
            Self::Unavailable { .. } => "unavailable",
        }
    }
}
impl Subscription {
    pub fn status(&self) -> ProviderStatus {
        match (self.last_success, &self.last_error, &self.snapshot) {
            (Some(last_success), Some(error), Some(_)) => ProviderStatus::Stale {
                last_success,
                error: error.clone(),
            },
            (Some(last_success), _, Some(_)) => ProviderStatus::Cached { last_success },
            (_, Some(error), _) => ProviderStatus::Unavailable {
                error: error.clone(),
            },
            _ => ProviderStatus::NotRefreshed,
        }
    }
}
#[derive(Clone, Debug)]
pub struct CachedHit {
    pub hit: SearchHit,
    /// Canonical authenticated publisher addresses, independent of page claims.
    pub providers: Vec<String>,
    pub status: ProviderStatus,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    providers: Vec<Subscription>,
}
pub struct SearchStore {
    directory: PathBuf,
    state: State,
}
impl SearchStore {
    /// Opens bounded local state only; never constructs a network client.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        let directory = directory.as_ref().to_path_buf();
        files::check_path(&directory)?;
        let path = directory.join("search-state.json");
        let state = match files::read(&path, MAX_STATE_BYTES) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| invalid(format!("corrupt search cache: {e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State {
                version: FORMAT_VERSION,
                providers: Vec::new(),
            },
            Err(e) => return Err(e),
        };
        validate_state(&state)?;
        Ok(Self { directory, state })
    }
    pub fn providers(&self) -> &[Subscription] {
        &self.state.providers
    }
    pub fn add(&mut self, name: &str, address: WebsiteAddress) -> Result<()> {
        let mut state = self.state.clone();
        state.providers.push(Subscription {
            name: name.into(),
            address: address.url(),
            last_success: None,
            last_error: None,
            snapshot: None,
        });
        state.providers.sort_by(|a, b| a.name.cmp(&b.name));
        self.commit(state)
    }
    pub fn remove(&mut self, name: &str) -> Result<()> {
        if !self.state.providers.iter().any(|p| p.name == name) {
            return Err(invalid("unknown provider"));
        }
        let mut state = self.state.clone();
        state.providers.retain(|p| p.name != name);
        self.commit(state)
    }
    /// Failed/cancelled/partial transfers never replace the last valid snapshot.
    /// Only this explicit operation is allowed to call the authenticated fetcher.
    pub fn refresh(
        &mut self,
        name: &str,
        fetcher: &dyn AuthenticatedDocumentFetcher,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let position = self
            .state
            .providers
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| invalid("unknown provider"))?;
        let address = WebsiteAddress::parse(&self.state.providers[position].address)
            .map_err(|e| invalid(e.to_string()))?;
        let until = Instant::now() + REFRESH_BUDGET;
        let result = download(&address, fetcher, cancel, until);
        if cancel.load(Ordering::SeqCst) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "search refresh cancelled",
            ));
        }
        if let Err(error) = check_active(cancel, until) {
            return self.record_failure(position, error.to_string());
        }
        let mut state = self.state.clone();
        match result {
            Ok(index) => {
                let p = &mut state.providers[position];
                p.snapshot = Some(index.pages().to_vec());
                p.last_success = Some(now()?);
                p.last_error = None;
                // If the global cache budget is exceeded, keep the old snapshot.
                if let Err(e) = validate_state(&state) {
                    return self.record_failure(position, e.to_string());
                }
                self.commit(state)
            }
            Err(error) => self.record_failure(position, error.to_string()),
        }
    }
    fn record_failure(&mut self, position: usize, error: String) -> Result<()> {
        let message: String = error
            .chars()
            .filter(|c| !c.is_control())
            .take(240)
            .collect();
        let mut state = self.state.clone();
        state.providers[position].last_error = Some(message.clone());
        self.commit(state)?;
        Err(invalid(message))
    }
    /// Offline AND-term search. Merge canonical addresses; highest score wins,
    /// then lexicographic provider URL. Preserve every matching publisher.
    pub fn search(
        &self,
        query: &str,
        selected: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CachedHit>> {
        query_terms(query)?;
        let mut merged = BTreeMap::<String, CachedHit>::new();
        for provider in &self.state.providers {
            if selected.is_some_and(|name| name != provider.name) {
                continue;
            }
            let Some(pages) = &provider.snapshot else {
                continue;
            };
            for mut hit in Index::from_pages(pages.clone())?.search(query, MAX_RESULTS)? {
                hit.source = provider.address.clone();
                let id = hit
                    .page
                    .address
                    .clone()
                    .ok_or_else(|| invalid("cached page lacks address"))?;
                match merged.get_mut(&id) {
                    None => {
                        merged.insert(
                            id,
                            CachedHit {
                                hit,
                                providers: vec![provider.address.clone()],
                                status: provider.status(),
                            },
                        );
                    }
                    Some(previous) => {
                        previous.providers.push(provider.address.clone());
                        if hit.score > previous.hit.score
                            || (hit.score == previous.hit.score && hit.source < previous.hit.source)
                        {
                            previous.hit = hit;
                            previous.status = provider.status();
                        }
                    }
                }
            }
        }
        let mut hits: Vec<_> = merged.into_values().collect();
        for hit in &mut hits {
            hit.providers.sort();
            hit.providers.dedup();
        }
        hits.sort_by(|a, b| {
            b.hit
                .score
                .cmp(&a.hit.score)
                .then_with(|| a.hit.page.id().cmp(&b.hit.page.id()))
        });
        hits.truncate(limit.min(MAX_RESULTS));
        Ok(hits)
    }
    fn commit(&mut self, state: State) -> Result<()> {
        validate_state(&state)?;
        let text = serde_json::to_string(&state).map_err(|e| invalid(e.to_string()))?;
        if text.len() > MAX_STATE_BYTES {
            return Err(invalid("search state exceeds disk budget"));
        }
        files::directory(&self.directory)?;
        files::write(&self.directory.join("search-state.json"), &text)?;
        self.state = state;
        Ok(())
    }
}
fn validate_state(state: &State) -> Result<()> {
    if state.version != FORMAT_VERSION || state.providers.len() > MAX_PROVIDERS {
        return Err(invalid(
            "unsupported search state version or provider limit",
        ));
    }
    let mut names = BTreeSet::new();
    let mut addresses = BTreeSet::new();
    let mut count = 0;
    let mut bytes = 0;
    for provider in &state.providers {
        let address =
            WebsiteAddress::parse(&provider.address).map_err(|e| invalid(e.to_string()))?;
        if provider.name.trim() != provider.name
            || provider.name.is_empty()
            || provider.name.chars().count() > 80
            || provider.name.chars().any(char::is_control)
            || !names.insert(&provider.name)
            || !addresses.insert(&provider.address)
            || address.url() != provider.address
            || provider
                .last_error
                .as_ref()
                .is_some_and(|e| e.chars().count() > 240 || e.chars().any(char::is_control))
        {
            return Err(invalid("invalid or duplicate search provider"));
        }
        if provider.snapshot.is_some() != provider.last_success.is_some() {
            return Err(invalid("inconsistent cache freshness"));
        }
        if let Some(pages) = &provider.snapshot {
            let index = Index::from_pages(pages.clone())?;
            if pages.iter().any(|p| p.address.is_none()) {
                return Err(invalid("cached page lacks canonical address"));
            }
            let stats = index.stats();
            count += stats.pages;
            bytes += stats.text_bytes;
            if count > MAX_CACHE_PAGES || bytes > MAX_CACHE_TEXT {
                return Err(invalid("global search cache budget exceeded"));
            }
        }
    }
    Ok(())
}
fn check_active(cancel: &AtomicBool, until: Instant) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "search refresh cancelled",
        ));
    }
    if Instant::now() >= until {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "search refresh time budget exceeded",
        ));
    }
    Ok(())
}
fn download(
    address: &WebsiteAddress,
    fetcher: &dyn AuthenticatedDocumentFetcher,
    cancel: &AtomicBool,
    until: Instant,
) -> Result<Index> {
    let fetch = |address: &WebsiteAddress| -> Result<String> {
        check_active(cancel, until)?;
        let doc = fetcher
            .fetch_document(address)
            .map_err(|e| invalid(e.to_string()))?;
        check_active(cancel, until)?;
        if doc.key != address.key() || doc.source.len() > ylang::MAX_SOURCE || !doc.style.is_empty()
        {
            return Err(invalid(
                "wrong document key, oversized source or non-inert stylesheet",
            ));
        }
        Ok(doc.source)
    };
    let manifest = Manifest::parse(&fetch(address)?)?;
    let mut payload = Vec::with_capacity(manifest.bytes);
    for (i, shard) in manifest.shards.iter().enumerate() {
        let shard_address =
            WebsiteAddress::parse(&format!("yami://{}/{}", address.node_id(), shard.key))
                .map_err(|e| invalid(e.to_string()))?;
        payload.extend(manifest.validate_shard(i, &fetch(&shard_address)?)?);
    }
    check_active(cancel, until)?;
    manifest.decode(&payload)
}
fn now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| invalid("clock before epoch"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registry_versions_duplicates_limits_and_corruption_fail_closed() {
        let root =
            std::env::temp_dir().join(format!("yami-search-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut store = SearchStore::open(&root).unwrap();
        assert!(!root.exists());
        for i in 0..MAX_PROVIDERS {
            let address = WebsiteAddress::parse(&format!(
                "yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/p{i}"
            ))
            .unwrap();
            store.add(&format!("Provider {i}"), address).unwrap();
        }
        let previous = std::fs::read(root.join("search-state.json")).unwrap();
        let duplicate = WebsiteAddress::parse(&store.providers()[0].address).unwrap();
        assert!(store.add("Duplicate", duplicate).is_err());
        assert_eq!(
            std::fs::read(root.join("search-state.json")).unwrap(),
            previous
        );
        let bad = String::from_utf8(previous)
            .unwrap()
            .replace("\"version\":1", "\"version\":2");
        std::fs::write(root.join("search-state.json"), &bad).unwrap();
        assert!(SearchStore::open(&root).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("search-state.json")).unwrap(),
            bad
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
