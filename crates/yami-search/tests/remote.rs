//! Real loopback pinned TLS, signed DHT and relay tests. No physical internet.
use std::{
    collections::BTreeMap,
    fs, io,
    net::{SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use yami_dht::{Contact, Network, PeerRecord, Service};
use yami_identity::{NodeIdentity, RelayRoute};
use yami_search::{
    AuthenticatedFetcher, Index, Page, ProviderStatus, Publication, SearchStore, WebsiteAddress,
    MANIFEST_KEY,
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yami-search-remote-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[derive(Default)]
struct Docs {
    data: Mutex<BTreeMap<String, String>>,
    keys: Mutex<Vec<String>>,
    mode: AtomicUsize,
}
impl Docs {
    fn publication() -> Arc<Self> {
        let index = Index::from_pages(vec![Page {
            key: "home".into(),
            title: "Café Networks".into(),
            headings: "Independent".into(),
            text: "日本語 decentralized local content".into(),
            address: Some(
                "yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home".into(),
            ),
        }])
        .unwrap();
        let publication = Publication::build(&index).unwrap();
        let mut data = BTreeMap::from([(MANIFEST_KEY.into(), publication.manifest().into())]);
        data.extend(publication.shards().iter().cloned());
        Arc::new(Self {
            data: Mutex::new(data),
            ..Self::default()
        })
    }
}
impl yami_node::Documents for Docs {
    fn lookup(&self, key: &str) -> io::Result<Option<ynp::DocumentData>> {
        self.keys.lock().unwrap().push(key.into());
        if self.mode.load(Ordering::SeqCst) == 3 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete transfer",
            ));
        }
        let mut source = self.data.lock().unwrap().get(key).cloned();
        match self.mode.load(Ordering::SeqCst) {
            1 if key == MANIFEST_KEY => {
                source = Some("@page 0.2\n  title Invalid\n:: search-manifest 999\n".into())
            }
            2 if key != MANIFEST_KEY => source = Some("invalid shard".into()),
            _ => {}
        }
        Ok(source.map(|source| ynp::DocumentData {
            key: key.into(),
            source,
            style: String::new(),
        }))
    }
}
struct Server {
    identity: Arc<NodeIdentity>,
    endpoint: SocketAddrV4,
    service: Service,
    requests: Arc<AtomicUsize>,
    shutdown: yami_node::shutdown::Shutdown,
    task: Option<JoinHandle<()>>,
}
impl Server {
    fn new(path: &Path, identity: Arc<NodeIdentity>, docs: Arc<Docs>) -> Self {
        Self::with_clock(path, identity, docs, Arc::new(yami_dht::SystemClock))
    }
    fn with_clock(
        path: &Path,
        identity: Arc<NodeIdentity>,
        docs: Arc<Docs>,
        clock: Arc<dyn yami_dht::Clock>,
    ) -> Self {
        let listener =
            ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity.clone()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            panic!()
        };
        let service = Service::open_with_clock(
            path.join("server.cache"),
            *identity.id(),
            Network::Loopback,
            clock,
        )
        .unwrap();
        let worker_service = service.clone();
        let shutdown = yami_node::shutdown::Shutdown::default();
        let stop = shutdown.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let task = thread::spawn(move || {
            yami_node::serve_with_handler_until(
                &listener,
                2,
                &|connection| {
                    count.fetch_add(1, Ordering::SeqCst);
                    yami_node::serve_connection_with_dht(connection, &*docs, &worker_service)
                },
                &|_| {},
                &stop,
            )
            .unwrap();
        });
        Self {
            identity,
            endpoint,
            service,
            requests,
            shutdown,
            task: Some(task),
        }
    }
    fn contact(&self) -> Contact {
        Contact::new(*self.identity.id(), self.endpoint, Network::Loopback).unwrap()
    }
    fn stop(&mut self) {
        self.shutdown.stop();
        if let Some(task) = self.task.take() {
            task.join().unwrap();
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}
fn address(id: &NodeIdentity) -> WebsiteAddress {
    WebsiteAddress::parse(&format!("yami://{}/{MANIFEST_KEY}", id.id())).unwrap()
}
fn record(id: &NodeIdentity, endpoints: Vec<SocketAddrV4>, routes: Vec<RelayRoute>) -> PeerRecord {
    let now = yami_dht::now().unwrap();
    PeerRecord::sign_routes(id, endpoints, routes, now + 120, 1, Network::Loopback, now).unwrap()
}
fn fetcher(path: &Path, bootstrap: &Server, relays: bool) -> AuthenticatedFetcher {
    AuthenticatedFetcher::open_with_relays(
        path,
        Network::Loopback,
        vec![bootstrap.contact()],
        relays,
    )
    .unwrap()
}

#[test]
fn authenticated_download_offline_query_privacy_failures_and_multiple_providers() {
    let scratch = Scratch::new();
    let docs = Docs::publication();
    let owner = Arc::new(NodeIdentity::generate().unwrap());
    let mut server = Server::new(&scratch.0.join("node"), owner.clone(), docs.clone());
    server
        .service
        .accept(
            record(&owner, vec![server.endpoint], vec![]),
            yami_dht::now().unwrap(),
        )
        .unwrap();
    let fetcher = fetcher(&scratch.0.join("fetch"), &server, false);
    let state = scratch.0.join("state");
    let mut store = SearchStore::open(&state).unwrap();
    store.add("First", address(&owner)).unwrap();
    assert!(store
        .search("private secret query", None, 100)
        .unwrap()
        .is_empty());
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        0,
        "startup/add/query must not connect"
    );
    let cancel = AtomicBool::new(false);
    store.refresh("First", &fetcher, &cancel).unwrap();
    assert_eq!(store.search("cafe", None, 100).unwrap().len(), 1);
    let after_refresh = server.requests.load(Ordering::SeqCst);
    let keys = docs.keys.lock().unwrap().clone();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0], MANIFEST_KEY);
    assert!(keys[1].starts_with("search-0000-"));
    for mode in 1..=3 {
        docs.mode.store(mode, Ordering::SeqCst);
        assert!(store.refresh("First", &fetcher, &cancel).is_err());
        assert!(matches!(
            store.providers()[0].status(),
            ProviderStatus::Stale { .. }
        ));
        assert_eq!(
            SearchStore::open(&state)
                .unwrap()
                .search("cafe", None, 100)
                .unwrap()
                .len(),
            1
        );
    }
    docs.mode.store(0, Ordering::SeqCst);
    let second_owner = Arc::new(NodeIdentity::generate().unwrap());
    let second = Server::new(
        &scratch.0.join("second"),
        second_owner.clone(),
        docs.clone(),
    );
    server
        .service
        .accept(
            record(&second_owner, vec![second.endpoint], vec![]),
            yami_dht::now().unwrap(),
        )
        .unwrap();
    store.add("Second", address(&second_owner)).unwrap();
    store.refresh("Second", &fetcher, &cancel).unwrap();
    let merged = store.search("decentralized", None, 100).unwrap();
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].providers.len(), 2);
    assert_ne!(
        merged[0].hit.source,
        merged[0].hit.page.address.clone().unwrap()
    );
    assert_eq!(
        store.search("cafe", Some("Second"), 100).unwrap()[0]
            .providers
            .len(),
        1
    );
    let before_cancel = fs::read(state.join("search-state.json")).unwrap();
    cancel.store(true, Ordering::SeqCst);
    assert!(store.refresh("Second", &fetcher, &cancel).is_err());
    assert_eq!(
        fs::read(state.join("search-state.json")).unwrap(),
        before_cancel
    );
    cancel.store(false, Ordering::SeqCst);
    server.stop();
    drop(second);
    let requests_before_queries = server.requests.load(Ordering::SeqCst);
    let reopened = SearchStore::open(&state).unwrap();
    assert_eq!(reopened.search("日本", None, 100).unwrap().len(), 1);
    assert!(reopened
        .search("private secret query", None, 100)
        .unwrap()
        .is_empty());
    assert_eq!(
        server.requests.load(Ordering::SeqCst),
        requests_before_queries
    );
    assert!(after_refresh > 0);
    assert!(store.refresh("First", &fetcher, &cancel).is_err());
    assert_eq!(store.search("cafe", None, 100).unwrap().len(), 1);
}

#[test]
fn wrong_node_identity_is_rejected_and_signed_expired_record_is_not_usable() {
    let scratch = Scratch::new();
    let claimed = Arc::new(NodeIdentity::generate().unwrap());
    let impostor_docs = Docs::publication();
    let impostor = Server::new(
        &scratch.0.join("impostor"),
        Arc::new(NodeIdentity::generate().unwrap()),
        impostor_docs.clone(),
    );
    let bootstrap = Server::new(
        &scratch.0.join("bootstrap"),
        Arc::new(NodeIdentity::generate().unwrap()),
        Arc::new(Docs::default()),
    );
    bootstrap
        .service
        .accept(
            record(&claimed, vec![impostor.endpoint], vec![]),
            yami_dht::now().unwrap(),
        )
        .unwrap();
    let fetch = fetcher(&scratch.0.join("fetch"), &bootstrap, false);
    let mut store = SearchStore::open(scratch.0.join("state")).unwrap();
    store.add("Wrong", address(&claimed)).unwrap();
    let error = store
        .refresh("Wrong", &fetch, &AtomicBool::new(false))
        .unwrap_err()
        .to_string();
    assert!(error.contains("identity"), "{error}");
    assert_eq!(
        impostor_docs.keys.lock().unwrap().len(),
        0,
        "identity mismatch must precede YNP request"
    );
    struct Past(u64);
    impl yami_dht::Clock for Past {
        fn now(&self) -> io::Result<u64> {
            Ok(self.0)
        }
    }
    let at = yami_dht::now().unwrap() - 60;
    let old = Server::with_clock(
        &scratch.0.join("old"),
        Arc::new(NodeIdentity::generate().unwrap()),
        Arc::new(Docs::default()),
        Arc::new(Past(at)),
    );
    let expired = PeerRecord::sign(
        &claimed,
        vec![impostor.endpoint],
        at + 30,
        1,
        Network::Loopback,
        at,
    )
    .unwrap();
    old.service.accept(expired, at).unwrap();
    let expired_fetch = fetcher(&scratch.0.join("expired-fetch"), &old, false);
    let error = expired_fetch
        .fetch_document(&address(&claimed))
        .unwrap_err();
    assert_eq!(error.kind, yami_discover::RemoteFetchErrorKind::Lookup);
}

#[test]
fn signed_relay_fallback_downloads_all_shards_with_destination_pinning() {
    let scratch = Scratch::new();
    let destination = Arc::new(NodeIdentity::generate().unwrap());
    let relay_identity = Arc::new(NodeIdentity::generate().unwrap());
    let relay_id = *relay_identity.id();
    let relay = Arc::new(
        yami_relay::Relay::bind(
            "127.0.0.1:0".parse().unwrap(),
            relay_identity,
            yami_relay::Limits::default(),
        )
        .unwrap(),
    );
    let SocketAddr::V4(endpoint) = relay.local_addr().unwrap() else {
        panic!()
    };
    let stop = Arc::new(AtomicBool::new(false));
    let relay_stop = stop.clone();
    let running_relay = relay.clone();
    let relay_task = thread::spawn(move || running_relay.run(&relay_stop).unwrap());
    let route = yami_relay::Route {
        relay: relay_id,
        endpoint: endpoint.into(),
        destination: *destination.id(),
    };
    let host_stop = stop.clone();
    let host_identity = destination.clone();
    let docs = Docs::publication();
    let host_task = thread::spawn(move || {
        let _ = yami_relay::run_host(
            route,
            host_identity,
            &host_stop,
            &|connection| yami_node::serve_connection(connection, &*docs).map(|_| ()),
            &|_| {},
        );
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while relay.statistics().registrations == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(relay.statistics().registrations > 0);
    let bootstrap = Server::new(
        &scratch.0.join("bootstrap"),
        Arc::new(NodeIdentity::generate().unwrap()),
        Arc::new(Docs::default()),
    );
    bootstrap
        .service
        .accept(
            record(
                &destination,
                vec!["127.0.0.1:9".parse().unwrap()],
                vec![RelayRoute {
                    relay: relay_id,
                    endpoint,
                }],
            ),
            yami_dht::now().unwrap(),
        )
        .unwrap();
    let mut store = SearchStore::open(scratch.0.join("state")).unwrap();
    store.add("Relay", address(&destination)).unwrap();
    let disabled = fetcher(&scratch.0.join("disabled"), &bootstrap, false);
    assert!(store
        .refresh("Relay", &disabled, &AtomicBool::new(false))
        .is_err());
    assert_eq!(relay.statistics().forwarded_bytes, 0);
    let enabled = fetcher(&scratch.0.join("enabled"), &bootstrap, true);
    store
        .refresh("Relay", &enabled, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(store.search("cafe", None, 100).unwrap().len(), 1);
    assert!(relay.statistics().forwarded_bytes > 0);
    stop.store(true, Ordering::SeqCst);
    host_task.join().unwrap();
    relay_task.join().unwrap();
}
