//! Bounded discovery worker and transient, explicitly untrusted UI state.
use std::{
    sync::{mpsc, Arc},
    thread,
};
use ynp_discovery::{Advertisement, Config, LanInterface};

pub struct Discovery {
    pub open: bool,
    pub interfaces: Vec<LanInterface>,
    pub interface: Option<usize>,
    pub results: Vec<Advertisement>,
    pub selected: Option<usize>,
    pub page: usize,
    pub status: String,
    pending: Option<mpsc::Receiver<Result<Vec<Advertisement>, String>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Discovery {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            open: false,
            interfaces: Vec::new(),
            interface: None,
            results: Vec::new(),
            selected: None,
            page: 0,
            status: "Choose a private LAN interface, then search.".into(),
            pending: None,
            wake,
        }
    }
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open && !self.searching() {
            let address = self
                .interface
                .and_then(|i| self.interfaces.get(i))
                .map(LanInterface::address);
            match ynp_discovery::interfaces() {
                Ok(interfaces) => {
                    self.interface = interfaces.iter().position(|i| Some(i.address()) == address);
                    self.interfaces = interfaces;
                    if self.interfaces.is_empty() {
                        self.status = "No private IPv4 LAN interface is available.".into();
                    }
                }
                Err(error) => {
                    self.interfaces.clear();
                    self.interface = None;
                    self.status = error.to_string();
                }
            }
        }
    }
    pub fn searching(&self) -> bool {
        self.pending.is_some()
    }
    pub fn choose_next_interface(&mut self) {
        if self.searching() || self.interfaces.is_empty() {
            return;
        }
        self.interface = Some(
            self.interface
                .map_or(0, |i| (i + 1) % self.interfaces.len()),
        );
        self.results.clear();
        self.selected = None;
        self.page = 0;
        self.status = "Ready. Search sends one query on this interface.".into();
    }
    pub fn search(&mut self) {
        if self.searching() {
            return;
        }
        let Some(interface) = self.interface.and_then(|i| self.interfaces.get(i)).cloned() else {
            return;
        };
        self.results.clear();
        self.selected = None;
        self.page = 0;
        let config = match Config::from_env() {
            Ok(config) => config,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let wake = Arc::clone(&self.wake);
        match thread::Builder::new()
            .name("browser-discovery".into())
            .spawn(move || {
                let result =
                    ynp_discovery::discover(&interface, config, ynp_discovery::SEARCH_TIME)
                        .map_err(|error| error.to_string());
                let _ = sender.send(result);
                wake();
            }) {
            Ok(_) => {
                self.pending = Some(receiver);
                self.status = "Searching for 3 seconds...".into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }
    pub fn poll(&mut self) -> bool {
        let Some(receiver) = &self.pending else {
            return false;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err("Discovery worker stopped.".into()),
        };
        self.pending = None;
        match result {
            Ok(results) => {
                self.status = if results.is_empty() {
                    "No nodes found. Check Private-network firewall rules.".into()
                } else {
                    format!(
                        "{} untrusted node(s). Select one to inspect.",
                        results.len()
                    )
                };
                self.results = results;
            }
            Err(error) => self.status = format!("Search failed: {error}"),
        }
        true
    }
    pub fn selected_url(&self) -> Option<String> {
        self.selected
            .and_then(|i| self.results.get(i))
            .map(Advertisement::url)
    }
}
