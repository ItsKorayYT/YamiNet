//! Full-text Discover state. One worker, no queue, no automatic network work.
use crate::discover::Field;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
};
use yami_search::{
    AuthenticatedDocumentFetcher, CachedHit, SearchStore, Subscription, WebsiteAddress,
};

pub type FetcherFactory =
    dyn Fn(&Path, bool) -> Result<Box<dyn AuthenticatedDocumentFetcher>, String> + Send + Sync;
enum Operation {
    Query,
    Add(String, WebsiteAddress),
    Remove(String),
    Refresh(String, Arc<FetcherFactory>, bool),
}
struct Loaded {
    sources: Vec<Subscription>,
    hits: Vec<CachedHit>,
    error: Option<String>,
}
struct Pending {
    receiver: mpsc::Receiver<Result<Loaded, String>>,
    generation: u64,
    cancel: Arc<AtomicBool>,
}
pub struct PageSearch {
    pub query: String,
    pub name: String,
    pub address: String,
    pub sources: Vec<Subscription>,
    pub hits: Vec<CachedHit>,
    pub selected_source: Option<String>,
    pub selected_page: Option<String>,
    pub page: usize,
    pub busy: Option<&'static str>,
    pub error: Option<String>,
    directory: Result<PathBuf, String>,
    pending: Option<Pending>,
    generation: u64,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl PageSearch {
    pub fn new(directory: Result<PathBuf, String>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            query: String::new(),
            name: String::new(),
            address: String::new(),
            sources: Vec::new(),
            hits: Vec::new(),
            selected_source: None,
            selected_page: None,
            page: 0,
            busy: None,
            error: None,
            directory,
            pending: None,
            generation: 0,
            wake,
        }
    }
    pub fn set_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake = wake;
    }
    pub fn load(&mut self) {
        self.start(Operation::Query, "Loading cached pages…");
    }
    pub fn set_field(&mut self, field: Field, value: String) {
        match field {
            Field::Search => {
                self.query = value;
                self.generation = self.generation.wrapping_add(1);
                self.hits.clear();
                self.selected_page = None;
                self.page = 0;
                self.start(Operation::Query, "Searching cached pages…");
            }
            Field::Name => self.name = value,
            Field::Address => self.address = value,
        }
    }
    pub fn add(&mut self) {
        match WebsiteAddress::parse(self.address.trim()) {
            Ok(address) => self.start(
                Operation::Add(self.name.clone(), address),
                "Saving provider (no fetch)…",
            ),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    pub fn remove_selected(&mut self) {
        if let Some(name) = self.selected_source.clone() {
            self.start(Operation::Remove(name), "Removing search provider…");
        }
    }
    pub fn refresh_selected(&mut self, factory: Arc<FetcherFactory>, relays: bool) {
        if let Some(name) = self.selected_source.clone() {
            self.start(
                Operation::Refresh(name, factory, relays),
                "Downloading search index…",
            );
        }
    }
    /// No detached replacement workers: wait for the bounded operation to stop.
    /// Generation invalidation also prevents cancelled work repainting results.
    pub fn cancel(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::SeqCst);
        }
        self.generation = self.generation.wrapping_add(1);
        self.hits.clear();
        self.selected_page = None;
    }
    pub fn change_page(&mut self, next: bool, size: usize, sources: bool) {
        let size = size.max(1);
        let count = if sources {
            self.sources.len()
        } else {
            self.hits.len()
        };
        self.page = self.page.min(count.saturating_sub(1) / size);
        if !next {
            self.page = self.page.saturating_sub(1);
        } else if (self.page + 1) * size < count {
            self.page += 1;
        }
    }
    pub fn move_selection(&mut self, next: bool, size: usize) {
        if self.hits.is_empty() {
            return;
        }
        let current = self
            .hits
            .iter()
            .position(|h| h.hit.page.address == self.selected_page);
        let index = match current {
            None => 0,
            Some(i) if next => (i + 1).min(self.hits.len() - 1),
            Some(i) => i.saturating_sub(1),
        };
        self.selected_page = self.hits[index].hit.page.address.clone();
        self.page = index / size.max(1);
    }
    fn start(&mut self, operation: Operation, status: &'static str) {
        if self.pending.is_some() {
            return;
        }
        let directory = match &self.directory {
            Ok(p) => p.clone(),
            Err(e) => {
                self.error = Some(e.clone());
                return;
            }
        };
        let query = self.query.clone();
        let wake = self.wake.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("yami-page-search".into())
            .spawn(move || {
                let result = run(&directory, &query, operation, &worker_cancel);
                let _ = sender.send(result);
                wake();
            }) {
            Ok(_) => {
                self.pending = Some(Pending {
                    receiver,
                    generation: self.generation,
                    cancel,
                });
                self.busy = Some(status);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot start search worker: {error}")),
        }
    }
    pub fn poll(&mut self) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        let result = match pending.receiver.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err("Search worker stopped".into()),
        };
        let stale = pending.generation != self.generation;
        let cancelled = pending.cancel.load(Ordering::SeqCst);
        self.pending = None;
        self.busy = None;
        match result {
            Ok(loaded) => {
                self.sources = loaded.sources;
                if !self
                    .sources
                    .iter()
                    .any(|s| Some(&s.name) == self.selected_source.as_ref())
                {
                    self.selected_source = None;
                }
                if !stale {
                    self.hits = loaded.hits;
                    self.selected_page = None;
                    self.page = 0;
                }
                self.error = loaded.error;
            }
            Err(error) if !cancelled => self.error = Some(error),
            Err(_) => {}
        }
        if stale {
            self.start(Operation::Query, "Searching latest query…");
        }
        true
    }
}
impl Drop for PageSearch {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::SeqCst);
        }
    }
}

struct ConfiguredFetcher(Result<Box<dyn AuthenticatedDocumentFetcher>, String>);
impl AuthenticatedDocumentFetcher for ConfiguredFetcher {
    fn fetch_document(
        &self,
        address: &WebsiteAddress,
    ) -> Result<ynp::DocumentData, yami_discover::RemoteFetchError> {
        match &self.0 {
            Ok(fetcher) => fetcher.fetch_document(address),
            Err(e) => Err(yami_discover::RemoteFetchError::configuration(e)),
        }
    }
    fn fetch_document_before(
        &self,
        address: &WebsiteAddress,
        until: std::time::Instant,
    ) -> Result<ynp::DocumentData, yami_discover::RemoteFetchError> {
        match &self.0 {
            Ok(fetcher) => fetcher.fetch_document_before(address, until),
            Err(e) => Err(yami_discover::RemoteFetchError::configuration(e)),
        }
    }
}
fn run(
    directory: &Path,
    query: &str,
    operation: Operation,
    cancel: &AtomicBool,
) -> Result<Loaded, String> {
    let mut store = SearchStore::open(directory).map_err(|e| e.to_string())?;
    let mut error = None;
    match operation {
        Operation::Query => {}
        Operation::Add(name, address) => store.add(&name, address).map_err(|e| e.to_string())?,
        Operation::Remove(name) => store.remove(&name).map_err(|e| e.to_string())?,
        Operation::Refresh(name, factory, relays) => {
            if let Err(e) = store.refresh(
                &name,
                &ConfiguredFetcher(factory(directory, relays)),
                cancel,
            ) {
                error = Some(e.to_string());
            }
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("Search operation cancelled; previous cache retained".into());
    }
    let hits = match store.search(query, None, 100) {
        Ok(hits) => hits,
        Err(e) => {
            error = Some(e.to_string());
            Vec::new()
        }
    };
    Ok(Loaded {
        sources: store.providers().to_vec(),
        hits,
        error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        fs,
        sync::atomic::AtomicUsize,
        time::{Duration, Instant},
    };
    use yami_search::{Index, Page, Publication};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Fixture {
        path: PathBuf,
        docs: Arc<BTreeMap<String, String>>,
    }
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "browser-page-search-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let pages = ["Café Networks", "Gardening"].into_iter().enumerate().map(|(i, title)| Page { key: format!("p{i}"), title: title.into(), headings: String::new(), text: title.into(),
                address: Some(format!("yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/p{i}")) }).collect();
            let publication = Publication::build(&Index::from_pages(pages).unwrap()).unwrap();
            let mut docs =
                BTreeMap::from([("search-manifest".into(), publication.manifest().into())]);
            docs.extend(publication.shards().iter().cloned());
            Self {
                path,
                docs: Arc::new(docs),
            }
        }
        fn panel(&self) -> PageSearch {
            PageSearch::new(Ok(self.path.clone()), Arc::new(|| {}))
        }
        fn add(&self, panel: &mut PageSearch) {
            panel.name = "Independent".into();
            panel.address = "yami://yn1-sp2hcdqey6gfx62bqb3vzymxh4hjsjv2ocfqk4hecwplc77pkh2yatmgrq/search-manifest".into();
            panel.add();
            settle(panel);
            panel.selected_source = Some("Independent".into());
        }
        fn factory(&self) -> Arc<FetcherFactory> {
            let docs = self.docs.clone();
            Arc::new(move |_, _| Ok(Box::new(Local(docs.clone()))))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
    // Local state tests only; real TLS, DHT and relay coverage lives in
    // yami-search/tests/remote.rs and browser native_dht integration tests.
    struct Local(Arc<BTreeMap<String, String>>);
    impl AuthenticatedDocumentFetcher for Local {
        fn fetch_document(
            &self,
            address: &WebsiteAddress,
        ) -> Result<ynp::DocumentData, yami_discover::RemoteFetchError> {
            Ok(ynp::DocumentData {
                key: address.key().into(),
                source: self.0[address.key()].clone(),
                style: String::new(),
            })
        }
    }
    fn settle(panel: &mut PageSearch) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while panel.busy.is_some() {
            panel.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn explicit_provider_persistence_offline_search_and_latest_query() {
        let fixture = Fixture::new();
        let mut panel = fixture.panel();
        assert!(panel.busy.is_none());
        assert!(!fixture.path.exists());
        fixture.add(&mut panel);
        assert_eq!(
            SearchStore::open(&fixture.path).unwrap().providers().len(),
            1
        );
        panel.refresh_selected(fixture.factory(), false);
        settle(&mut panel);
        panel.set_field(Field::Search, "cafe".into());
        panel.set_field(Field::Search, "garden".into());
        settle(&mut panel);
        assert_eq!(panel.hits.len(), 1);
        assert_eq!(panel.hits[0].hit.page.title, "Gardening");
        panel.move_selection(true, 1);
        assert!(panel.selected_page.is_some());
        let mut reopened = fixture.panel();
        reopened.query = "CAFÉ".into();
        reopened.load();
        settle(&mut reopened);
        assert_eq!(reopened.hits.len(), 1);
        assert_eq!(reopened.sources.len(), 1);
        assert!(reopened.hits[0].status.label().contains("cached"));
        reopened.selected_source = Some("Independent".into());
        reopened.remove_selected();
        settle(&mut reopened);
        assert!(reopened.hits.is_empty());
        assert!(SearchStore::open(&fixture.path)
            .unwrap()
            .providers()
            .is_empty());
    }
    #[test]
    fn cancellation_preserves_cache_and_discards_stale_results() {
        let fixture = Fixture::new();
        let mut panel = fixture.panel();
        fixture.add(&mut panel);
        panel.refresh_selected(fixture.factory(), false);
        settle(&mut panel);
        let before = fs::read(fixture.path.join("search-state.json")).unwrap();
        let gate = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(AtomicBool::new(false));
        struct Blocked {
            local: Local,
            gate: Arc<AtomicBool>,
            entered: Arc<AtomicBool>,
        }
        impl AuthenticatedDocumentFetcher for Blocked {
            fn fetch_document(
                &self,
                address: &WebsiteAddress,
            ) -> Result<ynp::DocumentData, yami_discover::RemoteFetchError> {
                self.entered.store(true, Ordering::SeqCst);
                let until = Instant::now() + Duration::from_secs(5);
                while !self.gate.load(Ordering::SeqCst) && Instant::now() < until {
                    thread::sleep(Duration::from_millis(1));
                }
                self.local.fetch_document(address)
            }
        }
        let docs = fixture.docs.clone();
        let worker_gate = gate.clone();
        let worker_entered = entered.clone();
        panel.refresh_selected(
            Arc::new(move |_, _| {
                Ok(Box::new(Blocked {
                    local: Local(docs.clone()),
                    gate: worker_gate.clone(),
                    entered: worker_entered.clone(),
                }))
            }),
            false,
        );
        let until = Instant::now() + Duration::from_secs(5);
        while !entered.load(Ordering::SeqCst) {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
        panel.set_field(Field::Search, "cafe".into());
        panel.cancel();
        panel.set_field(Field::Search, "garden".into());
        gate.store(true, Ordering::SeqCst);
        settle(&mut panel);
        assert_eq!(
            fs::read(fixture.path.join("search-state.json")).unwrap(),
            before
        );
        assert_eq!(panel.hits.len(), 1);
        assert_eq!(panel.hits[0].hit.page.title, "Gardening");
    }
    #[test]
    fn invalid_provider_and_corrupt_cache_are_reported_without_overwrite() {
        let fixture = Fixture::new();
        let mut panel = fixture.panel();
        panel.address = "https://invalid".into();
        panel.add();
        assert!(panel.error.is_some());
        assert!(!fixture.path.exists());
        fs::create_dir_all(&fixture.path).unwrap();
        fs::write(fixture.path.join("search-state.json"), "corrupt").unwrap();
        panel.load();
        settle(&mut panel);
        assert!(panel.error.as_deref().unwrap().contains("corrupt"));
        assert_eq!(
            fs::read_to_string(fixture.path.join("search-state.json")).unwrap(),
            "corrupt"
        );
    }
}
