//! Browser navigation state and bounded off-thread document loading.
//!
//! The desktop adapter owns all interaction and drawing. This module never
//! touches a window and never performs a network request on its calling thread.
use crate::native::{self, Address, Resolver};
use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Condvar, Mutex, OnceLock,
    },
    thread,
    time::Instant,
};
use yami_identity::NodeId;
use ynp::{DocumentData, Protection, Transport};
use ynp_tls::{IdentityMismatch, TlsTransport};

pub const MAX_TABS: usize = 12;
const MAX_HISTORY: usize = 64;
const MAX_PENDING: usize = 16;
const WORKERS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    Home,
    Welcome,
    About,
    /// Legacy plaintext YNP on loopback.
    Ynp(String),
    /// YNP over TLS 1.3 to the node ID named in the address.
    Ynps(String),
    /// Stable identity address resolved through the existing DHT.
    Yami(Address),
}
impl Location {
    pub fn is_local(&self) -> bool {
        match self {
            Self::Home | Self::Welcome | Self::About | Self::Ynp(_) => true,
            Self::Ynps(url) => {
                ynp_tls::parse_url(url).is_ok_and(|(_, endpoint, _)| endpoint.ip().is_loopback())
            }
            Self::Yami(_) => false,
        }
    }
    pub fn parse(input: &str) -> Result<Self, String> {
        let address = input.trim();
        let unusable = |error: io::Error| {
            format!("Cannot open this address: {error}. Use yami://node-id/key, ynps://node-id@address:port/key, a localhost ynp:// address or local:welcome.")
        };
        match address {
            "" => Ok(Self::Home),
            "local:welcome" => Ok(Self::Welcome),
            "local:about" => Ok(Self::About),
            _ if address.starts_with(native::SCHEME) => Address::parse(input)
                .map(Self::Yami)
                .map_err(|error| format!("Cannot open this address: {error}")),
            _ if address.starts_with(ynp_tls::SCHEME) => {
                ynp_tls::parse_url(address).map_err(unusable)?;
                Ok(Self::Ynps(address.into()))
            }
            _ => {
                ynp::parse_url(address).map_err(unusable)?;
                Ok(Self::Ynp(address.into()))
            }
        }
    }
    pub fn address(&self) -> String {
        match self {
            Self::Home => String::new(),
            Self::Welcome => "local:welcome".into(),
            Self::About => "local:about".into(),
            Self::Ynp(address) | Self::Ynps(address) => address.clone(),
            Self::Yami(address) => address.url(),
        }
    }
    fn label(&self) -> &str {
        match self {
            Self::Home => "New tab",
            Self::Welcome => "Welcome to YamiNet",
            Self::About => "About YamiNet",
            Self::Ynp(_) | Self::Ynps(_) | Self::Yami(_) => "YNP document",
        }
    }
}

/// How the page on screen reached the browser, taken from the connection
/// that delivered it rather than from the address that was typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Security {
    /// Compiled into the browser; no network.
    BuiltIn,
    /// Legacy loopback YNP: not encrypted, sender not verified.
    Plaintext,
    /// TLS 1.3 in which the node proved this node ID before any request.
    Verified(NodeId),
    Relayed {
        destination: NodeId,
        relay: NodeId,
    },
}

impl Security {
    pub fn destination(&self) -> Option<NodeId> {
        match self {
            Self::Verified(id)
            | Self::Relayed {
                destination: id, ..
            } => Some(*id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Page {
    pub document: ylang::Document,
    pub style: ysl::Stylesheet,
    pub security: Security,
    /// Endpoint of the authenticated native fetch, used only to upgrade old
    /// same-node numeric links while retaining the canonical identity address.
    pub endpoint: Option<SocketAddr>,
    pub images: yami_render::assets::Images,
    pub revision: u64,
    pub(crate) origin: Option<crate::resources::Origin>,
}
#[derive(Debug)]
pub enum PageState {
    Home,
    Loading,
    Ready(Box<Page>),
    Error(String),
    Game(Box<LocalGame>),
}
/// Local executable content has no document origin, remote URL or persisted history.
#[derive(Debug)]
pub struct LocalGame {
    pub path: std::path::PathBuf,
    pub session: yami_games::Session,
    pub snapshot: yami_games::Snapshot,
}
#[derive(Debug)]
pub struct Tab {
    pub id: u64,
    pub generation: u64,
    pub history: Vec<Location>,
    pub cursor: usize,
    pub state: PageState,
    /// User-editable address text, independent of the committed history entry.
    pub address: String,
    pub scroll: f64,
    pub target_scroll: f64,
    deadline: Option<Instant>,
}
impl Tab {
    fn new(id: u64) -> Self {
        Self {
            id,
            generation: 0,
            deadline: None,
            history: vec![Location::Home],
            cursor: 0,
            state: PageState::Home,
            address: String::new(),
            scroll: 0.0,
            target_scroll: 0.0,
        }
    }
    pub fn location(&self) -> &Location {
        &self.history[self.cursor]
    }
    pub fn title(&self) -> &str {
        match &self.state {
            PageState::Ready(page) => &page.document.metadata.title,
            PageState::Loading => "Loading…",
            PageState::Game(game) => &game.snapshot.title,
            PageState::Error(_) => "Could not open page",
            PageState::Home => self.location().label(),
        }
    }
    /// Only a loaded page has a connection to describe.
    pub fn security(&self) -> Option<&Security> {
        match &self.state {
            PageState::Ready(page) => Some(&page.security),
            _ => None,
        }
    }
    /// A link `ynp://A/key` on a page loaded from `ynps://N@A/...` names the
    /// same node, so it opens as `ynps://N@A/key` with the same pinned
    /// identity. YLang links are always written `ynp://`; this keeps a LAN
    /// site navigable without ever sending a plaintext request. Native pages
    /// likewise retain their ID for matching numeric links, and interpret full
    /// identity targets as native addresses. `Browser::follow` refuses other
    /// plaintext links from native pages.
    pub fn resolve_link(&self, target: &str) -> String {
        if let Some(key) = target
            .strip_prefix('/')
            .filter(|key| ynp::validate_key(key).is_ok())
        {
            return match self.location() {
                Location::Yami(address) => Address {
                    id: address.id,
                    key: key.into(),
                }
                .url(),
                Location::Ynps(url) => ynp_tls::parse_url(url)
                    .map(|(id, endpoint, _)| ynp_tls::format_url(&id, endpoint, key))
                    .unwrap_or_else(|_| target.into()),
                Location::Ynp(url) => ynp::parse_url(url)
                    .map(|(endpoint, _)| format!("ynp://{endpoint}/{key}"))
                    .unwrap_or_else(|_| target.into()),
                _ => target.into(),
            };
        }
        if let (Location::Yami(current), PageState::Ready(page)) = (self.location(), &self.state) {
            if page.security.destination() == Some(current.id) {
                if let Some(rest) = target.strip_prefix("ynp://") {
                    if let Ok(linked) = Address::parse(&format!("yami://{rest}")) {
                        return linked.url();
                    }
                    if let Ok((endpoint, key)) = ynp::parse_endpoint(rest) {
                        if page.endpoint == Some(endpoint) {
                            return Address {
                                id: current.id,
                                key,
                            }
                            .url();
                        }
                    }
                }
            }
        }
        if let (Location::Ynps(current), Some(rest)) =
            (self.location(), target.strip_prefix("ynp://"))
        {
            if let (Ok((id, address, _)), Ok((linked, key))) =
                (ynp_tls::parse_url(current), ynp::parse_endpoint(rest))
            {
                if linked == address {
                    return ynp_tls::format_url(&id, address, &key);
                }
            }
        }
        target.into()
    }
    pub fn can_back(&self) -> bool {
        self.cursor > 0
    }
    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.history.len()
    }
    fn begin(&mut self, loader: &Loader) {
        self.generation = self.generation.wrapping_add(1);
        self.address = self.location().address();
        self.scroll = 0.0;
        self.target_scroll = 0.0;
        loader.cancel(self.id);
        if self.location() == &Location::Home {
            self.state = PageState::Home;
            return;
        }
        self.state = PageState::Loading;
        let deadline = Instant::now() + native::FETCH_TIMEOUT;
        self.deadline = Some(deadline);
        let job = Job {
            tab_id: self.id,
            generation: self.generation,
            location: self.location().clone(),
            deadline,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        if let Err(message) = loader.submit(job) {
            self.state = PageState::Error(message);
        }
    }
}

pub struct Browser {
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_id: u64,
    pub library: Option<crate::library::Library>,
    pub library_error: Option<String>,
}
impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}
impl Browser {
    pub fn new() -> Self {
        Self {
            tabs: vec![Tab::new(1)],
            active: 0,
            next_id: 2,
            library: None,
            library_error: None,
        }
    }
    pub fn bookmark_current(&mut self, title: &str, folder: &str) -> io::Result<()> {
        let address = self.active().location().address();
        if !matches!(self.active().state, PageState::Ready(_)) {
            return Err(io::Error::other("Open a page before bookmarking it"));
        }
        self.library
            .as_mut()
            .ok_or_else(|| io::Error::other("Browser library is unavailable"))?
            .save_bookmark(&address, title, folder)
    }
    pub fn home(&mut self, loader: &Loader) {
        let home = self
            .library
            .as_ref()
            .map(|l| l.homepage().to_string())
            .unwrap_or_default();
        let _ = self.navigate(&home, loader);
    }
    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }
    pub fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }
    pub fn open_tab(&mut self) -> bool {
        if self.tabs.len() >= MAX_TABS {
            return false;
        }
        let Some(next) = self.next_id.checked_add(1) else {
            return false;
        };
        self.pause_active_game();
        self.tabs.push(Tab::new(self.next_id));
        self.next_id = next;
        self.active = self.tabs.len() - 1;
        true
    }
    /// Explicit user action only; document navigation and links never invoke this loader.
    pub fn open_game(
        &mut self,
        path: std::path::PathBuf,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), String> {
        if self.tabs.len() >= MAX_TABS {
            return Err("Close a tab before opening a game".into());
        }
        let session = yami_games::Session::launch(path.clone(), wake)?;
        if !self.open_tab() {
            return Err("Cannot open another tab".into());
        }
        let snapshot = session.snapshot();
        let tab = self.active_mut();
        tab.address = "Local WebAssembly game".into();
        tab.state = PageState::Game(Box::new(LocalGame {
            path,
            session,
            snapshot,
        }));
        Ok(())
    }
    pub fn poll_games(&mut self) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            if let PageState::Game(game) = &mut tab.state {
                let snapshot = game.session.snapshot();
                changed |= snapshot.frames != game.snapshot.frames
                    || snapshot.status != game.snapshot.status;
                game.snapshot = snapshot;
            }
        }
        changed
    }
    /// Keep one blank tab when the last tab closes. Pending results cannot
    /// target its replacement because tab IDs are never reused.
    pub fn close_tab_with_loader(&mut self, index: usize, loader: &Loader) {
        if let Some(tab) = self.tabs.get(index) {
            loader.cancel(tab.id);
        }
        self.close_tab(index);
    }
    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active = 0;
            self.tabs.push(Tab::new(self.next_id));
            self.next_id = self.next_id.saturating_add(1);
        } else if index < self.active {
            self.active -= 1;
        } else {
            self.active = self.active.min(self.tabs.len() - 1);
        }
    }
    pub fn pause_active_game(&self) {
        if let PageState::Game(game) = &self.active().state {
            game.session.pause();
        }
    }
    pub fn select(&mut self, index: usize) {
        if index < self.tabs.len() && index != self.active {
            self.pause_active_game();
            self.active = index;
        }
    }
    pub fn navigate(&mut self, address: &str, loader: &Loader) -> Result<(), String> {
        let location = match Location::parse(address) {
            Ok(location) => location,
            Err(message) => {
                let tab = self.active_mut();
                tab.generation = tab.generation.wrapping_add(1);
                loader.cancel(tab.id);
                tab.address = address.into();
                tab.state = PageState::Error(message.clone());
                tab.scroll = 0.0;
                tab.target_scroll = 0.0;
                return Err(message);
            }
        };
        self.navigate_location(location, loader);
        Ok(())
    }
    /// Open a link from the active page (see [`Tab::resolve_link`]).
    pub fn follow(&mut self, target: &str, loader: &Loader) -> Result<(), String> {
        let address = self.active().resolve_link(target);
        if matches!(self.active().location(), Location::Yami(_)) && address.starts_with("ynp://") {
            let message = "Refused plaintext link from a Yami page. Use a yami:// identity address or an explicit ynps:// address.".to_string();
            let tab = self.active_mut();
            tab.generation = tab.generation.wrapping_add(1);
            loader.cancel(tab.id);
            tab.state = PageState::Error(message.clone());
            return Err(message);
        }
        self.navigate(&address, loader)
    }
    pub fn navigate_location(&mut self, location: Location, loader: &Loader) {
        let tab = self.active_mut();
        if tab.location() != &location {
            tab.history.truncate(tab.cursor + 1);
            tab.history.push(location);
            if tab.history.len() > MAX_HISTORY {
                tab.history.remove(0);
            }
            tab.cursor = tab.history.len() - 1;
        }
        tab.begin(loader);
    }
    pub fn back(&mut self, loader: &Loader) -> bool {
        let tab = self.active_mut();
        if !tab.can_back() {
            return false;
        }
        tab.cursor -= 1;
        tab.begin(loader);
        true
    }
    pub fn forward(&mut self, loader: &Loader) -> bool {
        let tab = self.active_mut();
        if !tab.can_forward() {
            return false;
        }
        tab.cursor += 1;
        tab.begin(loader);
        true
    }
    pub fn reload(&mut self, loader: &Loader) {
        // An invalid typed address is shown as an error without entering the
        // validated history. Reload must retry that address rather than quietly
        // opening the previous history entry (or the start page).
        let tab = self.active();
        if matches!(tab.state, PageState::Error(_)) && tab.address != tab.location().address() {
            let address = tab.address.clone();
            let _ = self.navigate(&address, loader);
            return;
        }
        self.active_mut().begin(loader);
    }
    /// Apply results by stable tab ID and request generation, never by index or
    /// currently active tab. Closed tabs and superseded requests are discarded.
    pub fn poll(&mut self, loader: &Loader) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            if matches!(tab.state, PageState::Loading)
                && tab.deadline.is_some_and(|d| Instant::now() >= d)
            {
                tab.state = PageState::Error("Navigation deadline exceeded.".into());
                loader.cancel(tab.id);
                changed = true;
            }
        }
        while let Ok(result) = loader.results.try_recv() {
            changed |= self.complete(result);
        }
        while let Ok(update) = loader.resources.try_recv() {
            if let Some(tab) = self
                .tabs
                .iter_mut()
                .find(|tab| tab.id == update.tab_id && tab.generation == update.generation)
            {
                if let PageState::Ready(page) = &mut tab.state {
                    if let Some(image) = page.images.get_mut(&update.key) {
                        *image = update.state;
                        page.revision = page.revision.wrapping_add(1);
                        changed = true;
                    }
                }
            }
        }
        changed
    }
    fn complete(&mut self, result: Completion) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == result.tab_id) else {
            return false;
        };
        if tab.generation != result.generation || !matches!(tab.state, PageState::Loading) {
            return false;
        }
        tab.state = match result.page {
            Ok(page) => PageState::Ready(Box::new(page)),
            Err(error) => PageState::Error(error),
        };
        if let (Some(library), PageState::Ready(page)) = (&mut self.library, &tab.state) {
            // Titles are untrusted metadata, limited before optional disk storage.
            let title: String = page
                .document
                .metadata
                .title
                .chars()
                .filter(|c| !c.is_control())
                .take(40)
                .collect();
            if let Err(error) = library.record(&tab.location().address(), &title) {
                self.library_error = Some(error.to_string());
            }
        }
        true
    }
}

#[derive(Clone)]
struct Job {
    tab_id: u64,
    generation: u64,
    location: Location,
    deadline: Instant,
    cancel: Arc<AtomicBool>,
}
struct Completion {
    tab_id: u64,
    generation: u64,
    page: Result<Page, String>,
}
struct ResourceCompletion {
    tab_id: u64,
    generation: u64,
    key: String,
    state: yami_render::assets::ImageState,
}
#[derive(Default)]
struct Pending {
    jobs: VecDeque<Job>,
    active: Vec<(u64, Arc<AtomicBool>)>,
    closed: bool,
}
struct Shared {
    pending: Mutex<Pending>,
    changed: Condvar,
}
type Bootstrap = dyn Fn(&str) -> Result<(), String> + Send + Sync;
type Fetcher = dyn Fn(&Location, Instant) -> Result<Page, String> + Send + Sync;

/// A fixed worker pool with bounded request and completion queues. Dropping it
/// closes the queues without joining a potentially slow socket on the UI thread.
/// In-flight obsolete work is bounded to WORKERS; queued work for the same tab
/// is replaced by its latest request. No loader task ever follows page links.
pub struct Loader {
    local_only: Arc<AtomicBool>,
    shared: Arc<Shared>,
    results: mpsc::Receiver<Completion>,
    resources: mpsc::Receiver<ResourceCompletion>,
    relay_enabled: Arc<AtomicBool>,
    bootstrap: Arc<Bootstrap>,
    directory_fetcher: Arc<crate::discover::FetcherFactory>,
    search_fetcher: Arc<crate::page_search::FetcherFactory>,
}
impl Loader {
    /// The returned factory is invoked on Discover's worker. Recheck privacy
    /// there as well as at the UI action, including requests not yet started.
    pub fn directory_fetcher(&self) -> Arc<crate::discover::FetcherFactory> {
        let factory = self.directory_fetcher.clone();
        let local_only = self.local_only.clone();
        Arc::new(move |state, relays| {
            if local_only.load(Ordering::SeqCst) {
                return Err("Directory refresh is blocked by local-only mode.".into());
            }
            factory(state, relays)
        })
    }
    pub fn search_fetcher(&self) -> Arc<crate::page_search::FetcherFactory> {
        let factory = self.search_fetcher.clone();
        let local_only = self.local_only.clone();
        Arc::new(move |state, relays| {
            if local_only.load(Ordering::SeqCst) {
                return Err("Search refresh is blocked by local-only mode.".into());
            }
            factory(state, relays)
        })
    }
    pub fn set_local_only(&self, enabled: bool) {
        self.local_only.store(enabled, Ordering::SeqCst);
        if enabled {
            let mut pending = self
                .shared
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            for (_, flag) in &pending.active {
                flag.store(true, Ordering::SeqCst);
            }
            for job in &pending.jobs {
                job.cancel.store(true, Ordering::SeqCst);
            }
            pending.jobs.clear();
        }
    }
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> io::Result<Self> {
        let resolver = OnceLock::new();
        let relay_enabled = Arc::new(AtomicBool::new(
            std::env::var("YAMINET_BROWSER_RELAYS").as_deref() == Ok("1"),
        ));
        let control = relay_enabled.clone();
        let mut loader = Self::with_fetcher(
            wake,
            Arc::new(move |location, deadline| {
                if matches!(location, Location::Yami(_)) {
                    let resolver = resolver
                        .get_or_init(|| {
                            Resolver::from_environment().map(|mut resolver| {
                                resolver.relay_enabled = control.clone();
                                resolver
                            })
                        })
                        .as_ref()
                        .map_err(Clone::clone)?;
                    load_native_page(location, resolver, deadline)
                } else {
                    load_page(location)
                }
            }),
        )?;
        loader.relay_enabled = relay_enabled;
        Ok(loader)
    }
    pub fn relay_enabled(&self) -> bool {
        self.relay_enabled.load(Ordering::SeqCst)
    }
    pub fn set_relay_enabled(&self, enabled: bool) {
        self.relay_enabled.store(enabled, Ordering::SeqCst);
    }
    pub fn with_resolver(
        wake: Arc<dyn Fn() + Send + Sync>,
        resolver: Arc<Resolver>,
    ) -> io::Result<Self> {
        let relay_enabled = resolver.relay_enabled.clone();
        let bootstrap_resolver = resolver.clone();
        let directory_resolver = resolver.clone();
        let search_resolver = resolver.clone();
        let mut loader = Self::with_fetcher(
            wake,
            Arc::new(move |location, deadline| {
                if matches!(location, Location::Yami(_)) {
                    load_native_page(location, &resolver, deadline)
                } else {
                    load_page(location)
                }
            }),
        )?;
        loader.relay_enabled = relay_enabled;
        loader.bootstrap = Arc::new(move |pin| bootstrap_resolver.add_bootstrap(pin));
        loader.directory_fetcher =
            Arc::new(move |state, relays| directory_resolver.directory_fetcher(state, relays));
        loader.search_fetcher =
            Arc::new(move |state, relays| search_resolver.search_fetcher(state, relays));
        Ok(loader)
    }
    pub fn add_bootstrap(&self, pin: &str) -> Result<(), String> {
        (self.bootstrap)(pin)
    }
    fn with_fetcher(wake: Arc<dyn Fn() + Send + Sync>, fetcher: Arc<Fetcher>) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending::default()),
            changed: Condvar::new(),
        });
        let (sender, results) = mpsc::sync_channel(MAX_PENDING);
        let (resource_sender, resources) = mpsc::sync_channel(MAX_PENDING);
        let loader = Self {
            local_only: Arc::new(AtomicBool::new(false)),
            shared: Arc::clone(&shared),
            results,
            resources,
            relay_enabled: Arc::new(AtomicBool::new(false)),
            bootstrap: Arc::new(native::add_bootstrap_from_environment),
            search_fetcher: Arc::new(|state, relays| {
                Resolver::from_environment()?.search_fetcher(state, relays)
            }),
            directory_fetcher: Arc::new(|state, relays| {
                Resolver::from_environment()?.directory_fetcher(state, relays)
            }),
        };
        let image_cache = Arc::new(crate::image_cache::DecodeCache::default());
        for index in 0..WORKERS {
            let shared = Arc::clone(&shared);
            let sender = sender.clone();
            let resource_sender = resource_sender.clone();
            let wake = Arc::clone(&wake);
            let fetcher = Arc::clone(&fetcher);
            let local_only = loader.local_only.clone();
            let image_cache = image_cache.clone();
            thread::Builder::new()
                .name(format!("yami-loader-{index}"))
                .spawn(move || loop {
                    let job = {
                        let mut pending = shared.pending.lock().unwrap_or_else(|e| e.into_inner());
                        while pending.jobs.is_empty() && !pending.closed {
                            pending = shared
                                .changed
                                .wait(pending)
                                .unwrap_or_else(|e| e.into_inner());
                        }
                        if pending.closed {
                            return;
                        }
                        let job = pending.jobs.pop_front().expect("nonempty queue");
                        pending.active.push((job.tab_id, job.cancel.clone()));
                        job
                    };
                    let page =
                        if Instant::now() >= job.deadline || job.cancel.load(Ordering::SeqCst) {
                            Err("Navigation deadline exceeded or cancelled while queued.".into())
                        } else if local_only.load(Ordering::SeqCst) && !job.location.is_local() {
                            Err("Local-only mode blocks network browsing. Open Settings to enable explicitly configured peers.".into())
                        } else {
                            fetcher(&job.location, job.deadline)
                        };
                    let resources_page = page
                        .as_ref()
                        .ok()
                        .filter(|page| !page.images.is_empty())
                        .cloned();
                    let result = Completion {
                        tab_id: job.tab_id,
                        generation: job.generation,
                        page,
                    };
                    if sender.send(result).is_err() {
                        return;
                    }
                    wake();
                    if let Some(page) = resources_page {
                        crate::resources::load(&page, &image_cache, &job.cancel, job.deadline, |key, state| {
                            if job.cancel.load(Ordering::SeqCst) {
                                return false;
                            }
                            let sent = resource_sender
                                .send(ResourceCompletion {
                                    tab_id: job.tab_id,
                                    generation: job.generation,
                                    key,
                                    state,
                                })
                                .is_ok();
                            if sent {
                                wake();
                            }
                            sent
                        });
                    }
                    shared
                        .pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .active
                        .retain(|(_, flag)| !Arc::ptr_eq(flag, &job.cancel));
                })?;
        }
        Ok(loader)
    }
    fn submit(&self, job: Job) -> Result<(), String> {
        if self.local_only.load(Ordering::SeqCst) && !job.location.is_local() {
            return Err("Local-only mode: open Settings to enable network browsing. No connection was attempted.".into());
        }
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if pending.closed {
            return Err("Document loader is shutting down.".into());
        }
        pending.jobs.retain(|old| old.tab_id != job.tab_id);
        if pending.jobs.len() >= MAX_PENDING {
            return Err("The document queue is busy. Please try Reload shortly.".into());
        }
        pending.jobs.push_back(job);
        self.shared.changed.notify_one();
        Ok(())
    }
    fn cancel(&self, tab_id: u64) {
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for job in &pending.jobs {
            if job.tab_id == tab_id {
                job.cancel.store(true, Ordering::SeqCst);
            }
        }
        for (id, flag) in &pending.active {
            if *id == tab_id {
                flag.store(true, Ordering::SeqCst);
            }
        }
        pending.jobs.retain(|job| job.tab_id != tab_id);
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pending.closed = true;
        for job in &pending.jobs {
            job.cancel.store(true, Ordering::SeqCst);
        }
        for (_, flag) in &pending.active {
            flag.store(true, Ordering::SeqCst);
        }
        pending.jobs.clear();
        self.shared.changed.notify_all();
    }
}
fn load_page(location: &Location) -> Result<Page, String> {
    let (source, style, security) = match location {
        Location::Home => return Err("The start page does not require document loading.".into()),
        Location::Welcome => (
            include_str!("../../../examples/welcome.ylang").to_string(),
            include_str!("../../../examples/default.ysl").to_string(),
            Security::BuiltIn,
        ),
        Location::About => (
            include_str!("../../../examples/about.ylang").to_string(),
            include_str!("../../../examples/default.ysl").to_string(),
            Security::BuiltIn,
        ),
        Location::Ynp(address) => {
            // Validate again at the transport boundary even when constructed
            // programmatically rather than through Location::parse.
            let (endpoint, key) = ynp::parse_url(address).map_err(|error| error.to_string())?;
            let data = ynp::fetch(&ynp::LoopbackTcp, endpoint, &key).map_err(|error| {
                format!("The local node could not return this document: {error}")
            })?;
            (data.source, data.style, Security::Plaintext)
        }
        Location::Yami(_) => return Err("Native loading requires a browser DHT resolver.".into()),
        Location::Ynps(address) => {
            let (data, id) = fetch_verified(address)?;
            (data.source, data.style, Security::Verified(id))
        }
    };
    let mut page = parse_page(&source, &style, security, None)?;
    if let Location::Ynps(url) = location {
        let (id, endpoint, _) = ynp_tls::parse_url(url).map_err(|e| e.to_string())?;
        page.origin = Some(crate::resources::Origin {
            destination: id,
            endpoint,
            relay: None,
            public_relay: false,
        });
    }
    Ok(page)
}
fn load_native_page(
    location: &Location,
    resolver: &Resolver,
    deadline: Instant,
) -> Result<Page, String> {
    let Location::Yami(address) = location else {
        unreachable!()
    };
    let (data, info) = resolver.fetch_until(address, deadline)?;
    let mut page = parse_page(
        &data.source,
        &data.style,
        info.relay
            .map_or(Security::Verified(address.id), |relay| Security::Relayed {
                destination: address.id,
                relay,
            }),
        if info.relay.is_some() {
            None
        } else {
            Some(info.endpoint)
        },
    )?;
    page.origin = Some(crate::resources::Origin {
        destination: address.id,
        endpoint: info.endpoint,
        relay: info.relay,
        public_relay: resolver.public_network(),
    });
    Ok(page)
}
fn parse_page(
    source: &str,
    style: &str,
    security: Security,
    endpoint: Option<SocketAddr>,
) -> Result<Page, String> {
    let document = native::parse_document(source).map_err(|error| format!("YLang {error}"))?;
    let style = if style.is_empty() {
        ysl::Stylesheet::default()
    } else {
        ysl::parse(style).map_err(|error| format!("YSL {error}"))?
    };
    let images = yami_render::assets::image_keys(&document)
        .into_iter()
        .map(|key| (key, yami_render::assets::ImageState::Pending))
        .collect();
    Ok(Page {
        images,
        revision: 0,
        origin: None,
        document,
        style,
        security,
        endpoint,
    })
}
/// Fetch a `ynps://` address from exactly the node ID it names. Nothing is
/// requested unless the TLS 1.3 handshake proved that identity, and a failed
/// handshake is reported, never retried in plaintext.
pub fn fetch_verified(address: &str) -> Result<(DocumentData, NodeId), String> {
    let (id, endpoint, key) = ynp_tls::parse_url(address).map_err(|error| error.to_string())?;
    let transport = TlsTransport::new(id).map_err(|error| error.to_string())?;
    let mut connection = transport
        .connect(endpoint)
        .map_err(|error| handshake_failure(&id, endpoint, &error))?;
    if connection.protection() != Protection::Tls13PinnedServer {
        return Err(
            "Authentication failed: the connection is not TLS 1.3 to a pinned node.".into(),
        );
    }
    let data = ynp::request(&mut *connection, &key)
        .map_err(|error| format!("Verified node {id} could not return this document: {error}"))?;
    Ok((data, id))
}
fn handshake_failure(id: &NodeId, endpoint: SocketAddr, error: &io::Error) -> String {
    match error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<IdentityMismatch>())
    {
        Some(mismatch) => format!(
            "Authentication failed at {endpoint}: {mismatch}. Nothing was requested from it."
        ),
        None => format!(
            "Could not open an authenticated TLS 1.3 connection to node {id} at {endpoint}: {error}. Nothing was requested, and YNP never falls back to plaintext."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn page(title: &str) -> Page {
        Page {
            document: ylang::parse(&format!("@page 0.1\n  title {title}\n= {title}")).unwrap(),
            style: ysl::Stylesheet::default(),
            security: Security::BuiltIn,
            endpoint: None,
            images: yami_render::assets::Images::new(),
            revision: 0,
            origin: None,
        }
    }
    #[test]
    fn local_only_blocks_remote_before_calling_fetcher() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        let loader = Loader::with_fetcher(
            Arc::new(|| {}),
            Arc::new(move |_, _| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(page("Local"))
            }),
        )
        .unwrap();
        loader.set_local_only(true);
        let id = *yami_identity::NodeIdentity::generate().unwrap().id();
        let mut browser = Browser::new();
        browser
            .navigate(&format!("yami://{id}/home"), &loader)
            .unwrap();
        assert!(matches!(browser.active().state, PageState::Error(_)));
        browser
            .navigate(&format!("ynps://{id}@192.168.1.2:7443/home"), &loader)
            .unwrap();
        assert!(matches!(browser.active().state, PageState::Error(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        browser.navigate("local:welcome", &loader).unwrap();
        wait(&mut browser, &loader);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn loader() -> Loader {
        Loader::with_fetcher(
            Arc::new(|| {}),
            Arc::new(|location, _| Ok(page(location.label()))),
        )
        .unwrap()
    }
    fn wait(browser: &mut Browser, loader: &Loader) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(browser.active().state, PageState::Loading) {
            browser.poll(loader);
            assert!(Instant::now() < deadline, "loader did not complete");
            thread::yield_now();
        }
    }
    #[test]
    fn queued_navigation_deadline_expires_without_waiting_for_completion() {
        let loader = loader();
        let mut browser = Browser::new();
        browser.navigate("local:welcome", &loader).unwrap();
        browser.active_mut().deadline = Some(Instant::now());
        browser.poll(&loader);
        assert!(matches!(&browser.active().state,PageState::Error(e) if e.contains("deadline")));
        assert!(browser.active().security().is_none());
    }
    #[test]
    fn addresses_are_explicit_and_loopback_only() {
        for invalid in [
            "yami://home",
            "https://example.com",
            "ynp://example.com:7301/a",
            "ynp://192.0.2.1:7301/a",
            "ynp://127.0.0.1:0/a",
            "ynp://127.0.0.1:7301/../secret",
        ] {
            assert!(Location::parse(invalid).is_err(), "{invalid}");
        }
        for address in ["local:welcome", "local:about", "ynp://[::1]:7301/welcome"] {
            assert_eq!(Location::parse(address).unwrap().address(), address);
        }
        assert_eq!(Location::parse("").unwrap(), Location::Home);
    }
    #[test]
    fn history_branches_and_reload_does_not_add_an_entry() {
        let loader = loader();
        let mut browser = Browser::new();
        browser.navigate_location(Location::Welcome, &loader);
        browser.navigate_location(Location::About, &loader);
        assert!(browser.back(&loader));
        assert_eq!(browser.active().location(), &Location::Welcome);
        assert!(browser.active().can_forward());
        browser.navigate_location(Location::Ynp("ynp://127.0.0.1:9/a".into()), &loader);
        assert!(!browser.active().can_forward());
        assert_eq!(browser.active().history.len(), 3);
        browser.reload(&loader);
        assert_eq!(browser.active().history.len(), 3);
        assert!(browser.back(&loader));
        assert!(browser.back(&loader));
        assert!(!browser.back(&loader));
        assert!(matches!(browser.active().state, PageState::Home));
        assert!(browser.forward(&loader));
        assert_eq!(browser.active().location(), &Location::Welcome);
    }
    #[test]
    fn stale_responses_and_closed_tabs_cannot_replace_current_page() {
        let loader = loader();
        let mut browser = Browser::new();
        browser.navigate_location(Location::Welcome, &loader);
        let id = browser.active().id;
        let generation = browser.active().generation;
        browser.navigate_location(Location::About, &loader);
        assert!(!browser.complete(Completion {
            tab_id: id,
            generation,
            page: Ok(page("stale"))
        }));
        let generation = browser.active().generation;
        browser.open_tab();
        assert!(browser.complete(Completion {
            tab_id: id,
            generation,
            page: Ok(page("correct tab"))
        }));
        assert_eq!(browser.tabs[0].title(), "correct tab");
        assert!(matches!(browser.active().state, PageState::Home));
        browser.close_tab(0);
        assert!(!browser.complete(Completion {
            tab_id: id,
            generation,
            page: Ok(page("closed"))
        }));
        assert!(matches!(browser.active().state, PageState::Home));
    }
    #[test]
    fn tabs_history_and_pending_work_are_bounded() {
        let loader = loader();
        let mut browser = Browser::new();
        for _ in 1..MAX_TABS {
            assert!(browser.open_tab());
        }
        assert!(!browser.open_tab());
        for number in 0..100 {
            browser.navigate_location(
                Location::Ynp(format!("ynp://127.0.0.1:9/key{number}")),
                &loader,
            );
        }
        assert_eq!(browser.active().history.len(), MAX_HISTORY);
        assert!(loader.shared.pending.lock().unwrap().jobs.len() <= MAX_PENDING);
        for _ in 0..MAX_TABS {
            browser.close_tab(0);
        }
        assert_eq!(browser.tabs.len(), 1);
        assert!(matches!(browser.active().state, PageState::Home));
        assert!(browser.active().id > MAX_TABS as u64);
    }
    #[test]
    fn background_loading_and_errors_complete_without_blocking_navigation() {
        let (started, began) = mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_gate = Arc::clone(&gate);
        let loader = Loader::with_fetcher(
            Arc::new(|| {}),
            Arc::new(move |_, _| {
                started.send(()).unwrap();
                let (lock, changed) = &*worker_gate;
                let mut released = lock.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                Err("Node unavailable".into())
            }),
        )
        .unwrap();
        let mut browser = Browser::new();
        browser.navigate_location(Location::Welcome, &loader);
        began.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(browser.active().state, PageState::Loading));
        assert!(browser.open_tab());
        browser.select(0);
        let (lock, changed) = &*gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
        wait(&mut browser, &loader);
        assert!(
            matches!(&browser.active().state, PageState::Error(message) if message == "Node unavailable")
        );
        browser.navigate_location(Location::Home, &loader);
        assert!(matches!(browser.active().state, PageState::Home));
    }
    #[test]
    fn builtin_pages_use_real_parsers_and_invalid_navigation_cancels_loading() {
        let welcome = load_page(&Location::Welcome).unwrap();
        assert_eq!(welcome.document.metadata.title, "Welcome to YamiNet");
        assert_eq!(
            load_page(&Location::About).unwrap().document.metadata.title,
            "About YamiNet"
        );
        let loader = loader();
        let mut browser = Browser::new();
        browser.navigate_location(Location::Welcome, &loader);
        let old_generation = browser.active().generation;
        assert!(browser.navigate("yami://home", &loader).is_err());
        assert!(matches!(browser.active().state, PageState::Error(_)));
        assert!(!browser.complete(Completion {
            tab_id: browser.active().id,
            generation: old_generation,
            page: Ok(page("old"))
        }));
    }

    fn links(nodes: &[ylang::Node]) -> Vec<String> {
        nodes
            .iter()
            .flat_map(|node| match &node.kind {
                ylang::NodeKind::Link { target, .. } => vec![target.clone()],
                ylang::NodeKind::Section { children, .. } => links(children),
                _ => Vec::new(),
            })
            .collect()
    }
    fn ready_title(browser: &mut Browser, loader: &Loader) -> String {
        wait(browser, loader);
        match &browser.active().state {
            PageState::Ready(page) => page.document.metadata.title.clone(),
            other => panic!("expected a page, got {other:?}"),
        }
    }
    /// Two directory-backed nodes, the production loader and real link targets:
    /// same-node and cross-node links, history, reload after an edit, NotFound.
    #[test]
    fn navigates_multi_page_sites_across_two_nodes() {
        let base =
            std::env::temp_dir().join(format!("yaminet-browser-sites-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (alice, bob) = (base.join("alice"), base.join("bob"));
        std::fs::create_dir_all(&alice).unwrap();
        std::fs::create_dir_all(&bob).unwrap();
        let bind = || ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let (alice_listener, bob_listener) = (bind(), bind());
        let (a, b) = (
            alice_listener.local_addr().unwrap(),
            bob_listener.local_addr().unwrap(),
        );
        let write = |dir: &std::path::Path, key: &str, title: &str, link: String| {
            let source =
                format!("@page 0.1\n  title {title}\n= {title}\n:: Links\n  > {link} | Next\n");
            std::fs::write(dir.join(format!("{key}.ylang")), source).unwrap();
        };
        write(&alice, "home", "Alice home", format!("ynp://{a}/about"));
        write(&alice, "about", "Alice about", format!("ynp://{b}/home"));
        write(&bob, "home", "Bob home", format!("ynp://{a}/home"));
        std::fs::write(
            alice.join("default.ysl"),
            include_str!("../../../sites/alice/default.ysl"),
        )
        .unwrap();
        for (listener, root) in [(alice_listener, alice.clone()), (bob_listener, bob.clone())] {
            let site = yami_node::Site::open(root).unwrap();
            thread::spawn(move || yami_node::serve_concurrently(&listener, &site, 2, &|_| {}));
        }

        let loader = Loader::new(Arc::new(|| {})).unwrap();
        let mut browser = Browser::new();
        browser
            .navigate(&format!("ynp://{a}/home"), &loader)
            .unwrap();
        assert_eq!(ready_title(&mut browser, &loader), "Alice home");
        // Plaintext pages are never described as verified.
        assert_eq!(browser.active().security(), Some(&Security::Plaintext));
        for expected in ["Alice about", "Bob home", "Alice home"] {
            let PageState::Ready(page) = &browser.active().state else {
                unreachable!()
            };
            let target = links(&page.document.nodes).remove(0);
            browser.navigate(&target, &loader).unwrap();
            assert_eq!(ready_title(&mut browser, &loader), expected);
        }
        assert!(browser.back(&loader));
        assert_eq!(ready_title(&mut browser, &loader), "Bob home");
        write(&bob, "home", "Bob home edited", format!("ynp://{a}/home"));
        browser.reload(&loader);
        assert_eq!(ready_title(&mut browser, &loader), "Bob home edited");
        // Start page plus four documents; reload adds no entry.
        assert_eq!(browser.active().history.len(), 5);

        browser
            .navigate(&format!("ynp://{b}/missing"), &loader)
            .unwrap();
        wait(&mut browser, &loader);
        assert!(
            matches!(&browser.active().state, PageState::Error(message) if message.contains("not found"))
        );
        assert!(browser.back(&loader));
        assert_eq!(ready_title(&mut browser, &loader), "Bob home edited");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn reload_of_invalid_address_preserves_the_error_and_does_not_open_history() {
        let loader = loader();
        let mut browser = Browser::new();
        browser.navigate_location(Location::Welcome, &loader);
        assert!(browser.navigate("yami://not-implemented", &loader).is_err());
        let previous_generation = browser.active().generation;
        let history = browser.active().history.clone();

        browser.reload(&loader);

        assert_eq!(browser.active().address, "yami://not-implemented");
        assert!(
            matches!(&browser.active().state, PageState::Error(message) if message.contains("Cannot open this address"))
        );
        assert_eq!(browser.active().history, history);
        assert_eq!(browser.active().location(), &Location::Welcome);
        assert!(browser.active().generation > previous_generation);
        assert!(!browser.complete(Completion {
            tab_id: browser.active().id,
            generation: previous_generation,
            page: Ok(page("superseded")),
        }));
        assert!(browser.back(&loader));
        assert!(matches!(browser.active().state, PageState::Home));
    }

    fn node_id() -> NodeId {
        *yami_identity::NodeIdentity::generate().unwrap().id()
    }
    #[test]
    fn ynps_addresses_are_validated_and_plaintext_never_leaves_loopback() {
        let id = node_id();
        for address in [
            format!("ynps://{id}@192.168.1.50:7443/home"),
            format!("ynps://{id}@10.0.0.7:7443/home"),
            format!("ynps://{id}@127.0.0.1:7443/home"),
            format!("ynps://{id}@8.8.8.8:7443/home"),
        ] {
            assert_eq!(
                Location::parse(&address).unwrap(),
                Location::Ynps(address.clone())
            );
        }
        for invalid in [
            "ynps://192.168.1.50:7443/home".to_owned(),
            format!(
                "ynps://{}@192.168.1.50:7443/home",
                id.to_string().to_uppercase()
            ),
            format!("ynps://{id}@198.51.100.1:7443/home"),
            format!("ynps://{id}@0.0.0.0:7443/home"),
            format!("ynps://{id}@lan-host:7443/home"),
            format!("ynps://{id}@192.168.1.50:0/home"),
            format!("ynps://{id}@192.168.1.50:7443/../home"),
            // A LAN address is reachable only through authenticated TLS.
            "ynp://192.168.1.50:7443/home".to_owned(),
        ] {
            let error = Location::parse(&invalid).unwrap_err();
            assert!(error.contains("Cannot open this address"), "{invalid}");
        }
    }
    #[test]
    fn links_on_a_verified_page_stay_on_the_same_pinned_node() {
        let id = node_id();
        let mut tab = Tab::new(1);
        tab.history = vec![Location::Ynps(format!(
            "ynps://{id}@192.168.1.50:7443/home"
        ))];
        assert_eq!(
            tab.resolve_link("ynp://192.168.1.50:7443/about"),
            format!("ynps://{id}@192.168.1.50:7443/about")
        );
        // Another node, another port or a loopback node keeps its own address,
        // which for a LAN address means it is refused rather than downgraded.
        for other in [
            "ynp://192.168.1.51:7443/about",
            "ynp://192.168.1.50:7444/about",
            "ynp://127.0.0.1:7301/about",
            "ynp://192.168.1.50:7443/../about",
        ] {
            assert_eq!(tab.resolve_link(other), other);
        }
        // Plaintext pages never produce verified addresses.
        tab.history = vec![Location::Ynp("ynp://127.0.0.1:7443/home".into())];
        assert_eq!(
            tab.resolve_link("ynp://127.0.0.1:7443/about"),
            "ynp://127.0.0.1:7443/about"
        );
    }

    /// A private IPv4 address of this machine, so browser tests reach a real
    /// LAN listener when one exists; loopback otherwise.
    fn test_address() -> std::net::IpAddr {
        if let Ok(text) = std::env::var("YAMINET_TEST_LAN_ADDRESS") {
            return text.parse().unwrap();
        }
        // Connecting a UDP socket sends nothing; it only selects a route.
        std::net::UdpSocket::bind("0.0.0.0:0")
            .and_then(|socket| {
                socket.connect("192.168.0.1:9")?;
                socket.local_addr()
            })
            .map(|local| local.ip())
            .ok()
            .filter(|ip| ynp_tls::is_private_lan(*ip))
            .unwrap_or_else(|| {
                eprintln!("no private IPv4 address; using loopback for TLS browser tests");
                [127, 0, 0, 1].into()
            })
    }
    /// Serve `dir` over TLS as a fresh identity, on the LAN when possible.
    fn tls_site(dir: &std::path::Path) -> (SocketAddr, NodeId) {
        let identity = Arc::new(yami_identity::NodeIdentity::generate().unwrap());
        let id = *identity.id();
        let ip = test_address();
        let listener = if ip.is_loopback() {
            ynp_tls::TlsListener::bind((ip, 0).into(), identity)
        } else {
            ynp_tls::TlsListener::bind_lan((ip, 0).into(), identity)
        }
        .unwrap();
        let address = listener.local_addr().unwrap();
        let site = yami_node::Site::open(dir).unwrap();
        thread::spawn(move || yami_node::serve_concurrently(&listener, &site, 2, &|_| {}));
        (address, id)
    }
    fn site_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yaminet-browser-tls-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn write_page(dir: &std::path::Path, key: &str, title: &str, link: &str) {
        let source =
            format!("@page 0.1\n  title {title}\n= {title}\n:: Links\n  > {link} | Next\n");
        std::fs::write(dir.join(format!("{key}.ylang")), source).unwrap();
    }

    /// Public-listener policy exercised locally, without claiming internet reach.
    #[test]
    fn public_mode_navigation_preserves_pins_tabs_and_error_recovery() {
        let dir = site_dir("public-mode");
        let identity = Arc::new(yami_identity::NodeIdentity::generate().unwrap());
        let id = *identity.id();
        let listener =
            ynp_tls::TlsListener::bind_public("127.0.0.2:0".parse().unwrap(), identity).unwrap();
        let address = listener.local_addr().unwrap();
        write_page(
            &dir,
            "home",
            "Public mode home",
            &format!("ynp://{address}/about"),
        );
        write_page(
            &dir,
            "about",
            "Public mode about",
            &format!("ynp://{address}/home"),
        );
        let site = yami_node::Site::open(&dir).unwrap();
        let server = thread::spawn(move || {
            for _ in 0..5 {
                let _ = yami_node::serve_once(&listener, &site);
            }
        });
        let loader = Loader::new(Arc::new(|| {})).unwrap();
        let mut browser = Browser::new();
        browser.navigate("local:welcome", &loader).unwrap();
        wait(&mut browser, &loader);
        browser.open_tab();
        let home = ynp_tls::format_url(&id, address, "home");
        browser.navigate(&home, &loader).unwrap();
        assert_eq!(ready_title(&mut browser, &loader), "Public mode home");
        browser
            .follow(&format!("ynp://{address}/about"), &loader)
            .unwrap();
        assert_eq!(ready_title(&mut browser, &loader), "Public mode about");
        assert_eq!(browser.active().security(), Some(&Security::Verified(id)));
        browser.back(&loader);
        assert_eq!(ready_title(&mut browser, &loader), "Public mode home");
        browser.reload(&loader);
        assert_eq!(ready_title(&mut browser, &loader), "Public mode home");
        let wrong = node_id();
        browser
            .navigate(&ynp_tls::format_url(&wrong, address, "home"), &loader)
            .unwrap();
        wait(&mut browser, &loader);
        assert!(
            matches!(&browser.active().state, PageState::Error(error) if error.contains("Authentication failed"))
        );
        assert!(browser.active().security().is_none());
        browser.navigate("local:about", &loader).unwrap();
        wait(&mut browser, &loader);
        assert!(matches!(browser.active().state, PageState::Ready(_)));
        assert_eq!(browser.tabs.len(), 2);
        assert_eq!(browser.tabs[0].address, "local:welcome");
        server.join().unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[ignore = "requires an owned reachable public endpoint in YAMINET_TEST_REMOTE_URL"]
    fn browser_opens_owned_remote_site_over_pinned_tls() {
        let address =
            std::env::var("YAMINET_TEST_REMOTE_URL").expect("set an owned remote ynps:// URL");
        let (id, endpoint, _) = ynp_tls::parse_url(&address).unwrap();
        assert!(matches!(endpoint.ip(), std::net::IpAddr::V4(ip) if ynp_tls::is_public_ipv4(ip)));
        let loader = Loader::new(Arc::new(|| {})).unwrap();
        let mut browser = Browser::new();
        browser.navigate("local:welcome", &loader).unwrap();
        wait(&mut browser, &loader);
        browser.open_tab();
        browser.navigate(&address, &loader).unwrap();
        wait(&mut browser, &loader);
        assert!(
            matches!(&browser.active().state, PageState::Ready(_)),
            "{:?}",
            browser.active().state
        );
        assert_eq!(browser.active().security(), Some(&Security::Verified(id)));
        let wrong = node_id();
        browser
            .navigate(
                &address.replace(&id.to_string(), &wrong.to_string()),
                &loader,
            )
            .unwrap();
        wait(&mut browser, &loader);
        assert!(
            matches!(&browser.active().state, PageState::Error(error) if error.contains("Authentication failed"))
        );
        assert_eq!(browser.tabs[0].address, "local:welcome");
    }

    /// The production loader against a real TLS node: typed address, links
    /// written as `ynp://` on the node's own address, history, reload, tabs.
    #[test]
    fn navigates_a_site_over_verified_tls() {
        let dir = site_dir("navigate");
        let (address, id) = tls_site(&dir);
        write_page(
            &dir,
            "home",
            "Secure home",
            &format!("ynp://{address}/about"),
        );
        write_page(
            &dir,
            "about",
            "Secure about",
            &format!("ynp://{address}/home"),
        );
        let loader = Loader::new(Arc::new(|| {})).unwrap();
        let mut browser = Browser::new();
        let home = ynp_tls::format_url(&id, address, "home");
        browser.navigate(&home, &loader).unwrap();
        assert_eq!(ready_title(&mut browser, &loader), "Secure home");
        assert_eq!(browser.active().security(), Some(&Security::Verified(id)));

        let PageState::Ready(page) = &browser.active().state else {
            unreachable!()
        };
        let link = links(&page.document.nodes).remove(0);
        assert_eq!(link, format!("ynp://{address}/about"));
        browser.follow(&link, &loader).unwrap();
        assert_eq!(ready_title(&mut browser, &loader), "Secure about");
        assert_eq!(
            browser.active().location(),
            &Location::Ynps(ynp_tls::format_url(&id, address, "about"))
        );
        assert_eq!(browser.active().security(), Some(&Security::Verified(id)));

        assert!(browser.back(&loader));
        assert_eq!(ready_title(&mut browser, &loader), "Secure home");
        write_page(
            &dir,
            "home",
            "Secure home edited",
            &format!("ynp://{address}/about"),
        );
        browser.reload(&loader);
        assert_eq!(ready_title(&mut browser, &loader), "Secure home edited");
        assert!(browser.forward(&loader));
        assert_eq!(ready_title(&mut browser, &loader), "Secure about");
        assert_eq!(browser.active().history.len(), 3);

        // A second tab loads independently; closing it returns to the first.
        assert!(browser.open_tab());
        browser.navigate(&home, &loader).unwrap();
        assert_eq!(ready_title(&mut browser, &loader), "Secure home edited");
        browser.close_tab(1);
        assert_eq!(browser.active().title(), "Secure about");

        browser
            .navigate(&ynp_tls::format_url(&id, address, "missing"), &loader)
            .unwrap();
        wait(&mut browser, &loader);
        assert!(
            matches!(&browser.active().state, PageState::Error(message) if message.contains("not found"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_authentication_is_shown_and_the_browser_keeps_working() {
        let dir = site_dir("refused");
        let (address, id) = tls_site(&dir);
        write_page(
            &dir,
            "home",
            "Pinned home",
            &format!("ynp://{address}/home"),
        );
        let loader = Loader::new(Arc::new(|| {})).unwrap();
        let mut browser = Browser::new();

        // The node at this address is not the one named in the URL.
        let expected = node_id();
        let impostor = ynp_tls::format_url(&expected, address, "home");
        browser.navigate(&impostor, &loader).unwrap();
        wait(&mut browser, &loader);
        let PageState::Error(message) = &browser.active().state else {
            panic!("an unexpected identity loaded a page")
        };
        assert!(message.starts_with("Authentication failed"), "{message}");
        assert!(message.contains(&expected.to_string()), "{message}");
        assert!(message.contains(&id.to_string()), "{message}");
        assert_eq!(browser.active().security(), None);
        assert_eq!(browser.active().title(), "Could not open page");

        // Reload checks again and fails again; nothing falls back.
        browser.reload(&loader);
        wait(&mut browser, &loader);
        assert!(matches!(browser.active().state, PageState::Error(_)));

        // A plaintext node at a ynps:// address is an error, never plaintext.
        let legacy = ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let legacy_address = legacy.local_addr().unwrap();
        let site = yami_node::Site::open(&dir).unwrap();
        thread::spawn(move || yami_node::serve_concurrently(&legacy, &site, 1, &|_| {}));
        browser
            .navigate(&ynp_tls::format_url(&id, legacy_address, "home"), &loader)
            .unwrap();
        wait(&mut browser, &loader);
        assert!(
            matches!(&browser.active().state, PageState::Error(message) if message.contains("never falls back to plaintext"))
        );
        assert_eq!(browser.active().security(), None);

        // The right node ID at the same address then works normally.
        browser
            .navigate(&ynp_tls::format_url(&id, address, "home"), &loader)
            .unwrap();
        assert_eq!(ready_title(&mut browser, &loader), "Pinned home");
        assert_eq!(browser.active().security(), Some(&Security::Verified(id)));
        assert!(browser.back(&loader));
        wait(&mut browser, &loader);
        assert!(matches!(browser.active().state, PageState::Error(_)));
        assert!(browser.forward(&loader));
        assert_eq!(ready_title(&mut browser, &loader), "Pinned home");
        browser.navigate("local:welcome", &loader).unwrap();
        wait(&mut browser, &loader);
        assert_eq!(browser.active().security(), Some(&Security::BuiltIn));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
