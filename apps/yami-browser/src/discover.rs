//! Native Discover state. Search reads only CatalogView; all store I/O and
//! explicit refreshes run on one bounded, non-queued worker at a time.
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread,
};
use yami_discover::{
    CatalogView, DiscoveryStore, Entry, RemoteDirectoryFetcher, RemoteFetchError, Source,
    SourceState, WebsiteAddress,
};

pub type FetcherFactory =
    dyn Fn(&Path, bool) -> Result<Box<dyn RemoteDirectoryFetcher>, String> + Send + Sync;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Search,
    Name,
    Address,
}

#[derive(Debug)]
pub struct Listing {
    pub entry: Entry,
    pub state: SourceState,
}

struct Loaded {
    sources: Vec<Source>,
    view: CatalogView,
    error: Option<String>,
}
enum Operation {
    Load,
    Add(String, WebsiteAddress),
    Remove(String),
    Refresh(String, Arc<FetcherFactory>, bool),
}

pub struct Discover {
    pub open: bool,
    pub directories: bool,
    pub query: String,
    pub name: String,
    pub address: String,
    pub sources: Vec<Source>,
    pub listings: Vec<Listing>,
    pub selected_source: Option<String>,
    pub selected_website: Option<String>,
    pub page: usize,
    pub error: Option<String>,
    pub query_error: Option<String>,
    pub busy: Option<&'static str>,
    view: Option<CatalogView>,
    directory: Result<PathBuf, String>,
    pending: Option<mpsc::Receiver<Result<Loaded, String>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Discover {
    /// Construction, startup and opening the panel never fetch directories.
    pub fn new(directory: Result<PathBuf, String>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            open: false,
            directories: false,
            query: String::new(),
            name: String::new(),
            address: String::new(),
            sources: Vec::new(),
            listings: Vec::new(),
            selected_source: None,
            selected_website: None,
            page: 0,
            error: None,
            query_error: None,
            busy: None,
            view: None,
            directory,
            pending: None,
            wake,
        }
    }
    pub fn default_directory() -> Result<PathBuf, String> {
        if let Some(path) = std::env::var_os("YAMINET_DISCOVER_DIR") {
            let path = PathBuf::from(path);
            return if path.is_absolute() {
                Ok(path)
            } else {
                Err("YAMINET_DISCOVER_DIR must be absolute".into())
            };
        }
        crate::library::Library::default_directory()
            .map(|directory| directory.join("discover"))
            .map_err(|error| error.to_string())
    }
    pub fn set_wake(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake = wake;
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.load();
        }
    }
    pub fn load(&mut self) {
        self.start(Operation::Load, "Loading saved listings…");
    }
    pub fn set_field(&mut self, field: Field, text: String) {
        match field {
            Field::Search => {
                self.query = text;
                self.page = 0;
                self.search();
            }
            Field::Name => self.name = text,
            Field::Address => self.address = text,
        }
    }
    fn search(&mut self) {
        self.query_error = None;
        self.listings.clear();
        self.selected_website = None;
        if let Some(view) = &self.view {
            match view.search(&self.query) {
                Ok(hits) => {
                    self.listings = hits
                        .into_iter()
                        .map(|hit| Listing {
                            entry: hit.entry.clone(),
                            state: view
                                .status_for(hit.entry.source())
                                .map_or(SourceState::NotBuilt, |status| status.state.clone()),
                        })
                        .collect();
                }
                Err(error) => self.query_error = Some(error.to_string()),
            }
        }
    }
    pub fn add(&mut self) {
        if self.busy.is_some() {
            return;
        }
        match WebsiteAddress::parse(self.address.trim()) {
            Ok(address) => self.start(
                Operation::Add(self.name.clone(), address),
                "Saving subscription (no fetch)…",
            ),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    pub fn remove_selected(&mut self) {
        if let Some(name) = &self.selected_source {
            self.start(Operation::Remove(name.clone()), "Removing directory…");
        }
    }
    pub fn refresh_selected(&mut self, factory: Arc<FetcherFactory>, relays: bool) {
        if let Some(name) = &self.selected_source {
            self.start(
                Operation::Refresh(name.clone(), factory, relays),
                "Refreshing selected directory…",
            );
        }
    }
    pub fn source_state(&self, name: &str) -> Option<&SourceState> {
        self.view
            .as_ref()?
            .status_for(name)
            .map(|status| &status.state)
    }
    pub fn select_website(&mut self, index: usize) {
        self.selected_website = self
            .listings
            .get(index)
            .map(|listing| listing.entry.address().url());
    }
    pub fn select_source(&mut self, index: usize) {
        self.selected_source = self.sources.get(index).map(|source| source.name().into());
    }
    pub fn change_page(&mut self, next: bool, size: usize) {
        let size = size.max(1);
        let count = if self.directories {
            self.sources.len()
        } else {
            self.listings.len()
        };
        self.page = self.page.min(count.saturating_sub(1) / size);
        if !next {
            self.page = self.page.saturating_sub(1);
        } else if (self.page + 1).saturating_mul(size) < count {
            self.page += 1;
        }
    }
    fn start(&mut self, operation: Operation, status: &'static str) {
        // No unbounded queues, concurrent writes or overlapping fetchers.
        if self.pending.is_some() {
            return;
        }
        let directory = match &self.directory {
            Ok(directory) => directory.clone(),
            Err(error) => {
                self.error = Some(error.clone());
                return;
            }
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let wake = self.wake.clone();
        match thread::Builder::new()
            .name("yami-discover-ui".into())
            .spawn(move || {
                let result = run(&directory, operation);
                let _ = sender.send(result);
                wake();
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.busy = Some(status);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot start Discover worker: {error}")),
        }
    }
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.pending else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Discover worker stopped unexpectedly".into())
            }
        };
        self.pending = None;
        self.busy = None;
        match result {
            Ok(loaded) => {
                self.sources = loaded.sources;
                self.view = Some(loaded.view);
                self.error = loaded.error;
                if !self
                    .sources
                    .iter()
                    .any(|source| Some(source.name()) == self.selected_source.as_deref())
                {
                    self.selected_source = None;
                }
                self.page = 0;
                self.search();
            }
            Err(error) => self.error = Some(error),
        }
        true
    }
}

// Feed setup failures through DiscoveryStore's existing cached/unavailable
// fallback instead of reproducing that logic in the browser.
struct ConfiguredFetcher(Result<Box<dyn RemoteDirectoryFetcher>, String>);
impl RemoteDirectoryFetcher for ConfiguredFetcher {
    fn fetch(
        &self,
        address: &WebsiteAddress,
    ) -> Result<yami_discover::Directory, RemoteFetchError> {
        match &self.0 {
            Ok(fetcher) => fetcher.fetch(address),
            Err(error) => Err(RemoteFetchError::configuration(error)),
        }
    }
}
fn run(directory: &Path, operation: Operation) -> Result<Loaded, String> {
    let mut store = DiscoveryStore::open(directory).map_err(|error| error.to_string())?;
    let mut remote_error = None;
    let view = match operation {
        Operation::Load => store.load_last(),
        Operation::Add(name, address) => store
            .register_remote(&name, address)
            .and_then(|_| store.load_last()),
        Operation::Remove(name) => store.remove_source(&name).and_then(|_| store.load_last()),
        Operation::Refresh(name, factory, relays) => {
            let remote = store
                .registry()
                .get(&name)
                .is_some_and(|source| source.remote_address().is_some());
            if remote {
                let fetcher = ConfiguredFetcher(factory(directory, relays));
                store.refresh_source_with(&name, &fetcher).map(|report| {
                    remote_error = report.remote_error.map(|error| error.to_string());
                    report.view
                })
            } else {
                store.refresh_source(&name)
            }
        }
    }
    .map_err(|error| error.to_string())?;
    Ok(Loaded {
        sources: store.registry().sources().to_vec(),
        view,
        error: remote_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Mutex,
        },
        time::{Duration, Instant},
    };
    use yami_discover::{Directory, DirectoryEntry};
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            Self(
                std::env::temp_dir()
                    .join(format!("browser-discover-{name}-{}", std::process::id())),
            )
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn address() -> WebsiteAddress {
        WebsiteAddress::parse(&format!(
            "yami://{}/home",
            yami_identity::NodeIdentity::generate().unwrap().id()
        ))
        .unwrap()
    }
    fn sample() -> Directory {
        Directory::new(
            "An untrusted directory",
            vec![DirectoryEntry::new(
                "Café 日本",
                "An operator supplied description",
                address(),
                Some("Writing".into()),
            )
            .unwrap()],
        )
        .unwrap()
    }
    fn wait(panel: &mut Discover) {
        let until = Instant::now() + Duration::from_secs(10);
        while panel.busy.is_some() {
            panel.poll();
            assert!(Instant::now() < until, "Discover worker did not complete");
            thread::sleep(Duration::from_millis(1));
        }
    }
    struct Mock {
        data: Directory,
        calls: Arc<AtomicUsize>,
        fail: Arc<AtomicBool>,
    }
    impl RemoteDirectoryFetcher for Mock {
        fn fetch(&self, _: &WebsiteAddress) -> Result<Directory, RemoteFetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail.load(Ordering::SeqCst) {
                Err(RemoteFetchError::unreachable("directory offline"))
            } else {
                Ok(self.data.clone())
            }
        }
    }
    #[test]
    fn subscription_search_cache_restart_and_removal_use_existing_store() {
        let scratch = Scratch::new("lifecycle");
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(AtomicBool::new(false));
        let data = sample();
        let factory: Arc<FetcherFactory> = {
            let calls = calls.clone();
            let fail = fail.clone();
            let data = data.clone();
            Arc::new(move |_, relays| {
                assert!(relays, "UI relay opt-in must reach the transport factory");
                Ok(Box::new(Mock {
                    data: data.clone(),
                    calls: calls.clone(),
                    fail: fail.clone(),
                }))
            })
        };
        let mut panel = Discover::new(Ok(scratch.0.clone()), Arc::new(|| {}));
        assert!(
            !scratch.0.exists(),
            "construction does not read or create state"
        );
        panel.toggle();
        wait(&mut panel);
        assert!(panel.sources.is_empty());
        panel.name = "community".into();
        panel.address = "https://example.com".into();
        panel.add();
        assert!(panel.error.is_some());
        assert!(panel.busy.is_none());
        let remote = address();
        panel.address = format!("yami://{}", remote.node_id());
        panel.add();
        wait(&mut panel);
        assert!(panel.error.is_none(), "{:?}", panel.error);
        assert_eq!(panel.sources[0].remote_address(), Some(&remote));
        assert_eq!(
            panel.source_state("community"),
            Some(&SourceState::InactiveRemote)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "adding never fetches");
        panel.select_source(0);
        panel.refresh_selected(factory.clone(), true);
        wait(&mut panel);
        assert!(panel.error.is_none(), "{:?}", panel.error);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        panel.set_field(Field::Search, "CAFE 日本".into());
        assert_eq!(panel.listings.len(), 1);
        assert_eq!(
            panel.listings[0].entry.description(),
            "An operator supplied description"
        );
        assert_eq!(panel.listings[0].entry.category(), Some("Writing"));
        assert_eq!(panel.listings[0].entry.source(), "community");
        assert_eq!(panel.listings[0].state, SourceState::Fresh);
        panel.set_field(
            Field::Search,
            "x".repeat(yami_discover::MAX_QUERY_CHARS + 1),
        );
        assert!(panel.query_error.is_some());
        panel.set_field(Field::Search, "café".into());
        assert!(panel.query_error.is_none());
        assert_eq!(panel.listings.len(), 1);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "query edits are entirely offline"
        );
        fail.store(true, Ordering::SeqCst);
        panel.refresh_selected(factory, true);
        wait(&mut panel);
        assert!(panel
            .error
            .as_deref()
            .unwrap()
            .contains("directory offline"));
        assert!(matches!(
            panel.listings[0].state,
            SourceState::Cached { .. }
        ));
        let mut reopened = Discover::new(Ok(scratch.0.clone()), Arc::new(|| {}));
        reopened.toggle();
        wait(&mut reopened);
        assert_eq!(reopened.listings.len(), 1);
        assert!(matches!(
            reopened.listings[0].state,
            SourceState::Cached { .. }
        ));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "startup and reopening never refresh"
        );
        reopened.select_source(0);
        reopened.remove_selected();
        wait(&mut reopened);
        assert!(reopened.sources.is_empty() && reopened.listings.is_empty());
        assert!(DiscoveryStore::open(&scratch.0)
            .unwrap()
            .load_last()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn slow_refresh_does_not_block_search_or_allow_overlapping_writes() {
        let scratch = Scratch::new("worker");
        let data = sample();
        let mut store = DiscoveryStore::open(&scratch.0).unwrap();
        store.register_remote("slow", address()).unwrap();
        let mut panel = Discover::new(Ok(scratch.0.clone()), Arc::new(|| {}));
        panel.load();
        wait(&mut panel);
        panel.select_source(0);
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let release_rx = Mutex::new(release_rx);
        let ui_thread = thread::current().id();
        let factory: Arc<FetcherFactory> = Arc::new(move |_, relays| {
            assert_ne!(thread::current().id(), ui_thread);
            assert!(!relays);
            started_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            Ok(Box::new(Mock {
                data: data.clone(),
                calls: Arc::new(AtomicUsize::new(0)),
                fail: Arc::new(AtomicBool::new(false)),
            }))
        });
        panel.refresh_selected(factory, false);
        started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        panel.set_field(Field::Search, "cafe".into());
        assert!(!panel.poll());
        panel.remove_selected(); // must not overlap the active store mutation
        assert_eq!(panel.busy, Some("Refreshing selected directory…"));
        release_tx.send(()).unwrap();
        wait(&mut panel);
        assert_eq!(panel.sources.len(), 1);
        assert_eq!(panel.listings.len(), 1);
    }
    #[test]
    fn corrupt_registry_surfaces_an_error_without_replacing_saved_state() {
        let scratch = Scratch::new("corrupt");
        fs::create_dir_all(&scratch.0).unwrap();
        fs::write(scratch.0.join("sources.conf"), "corrupt registry").unwrap();
        let mut panel = Discover::new(Ok(scratch.0.clone()), Arc::new(|| {}));
        panel.load();
        wait(&mut panel);
        assert!(panel.error.is_some());
        assert_eq!(
            fs::read_to_string(scratch.0.join("sources.conf")).unwrap(),
            "corrupt registry"
        );
        assert!(panel.sources.is_empty() && panel.listings.is_empty());
    }
}
