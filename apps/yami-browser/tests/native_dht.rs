//! The existing A/B/C loopback DHT topology, using the real node dispatcher,
//! signed publication, pinned TLS and browser model/worker pool directly.
use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use yami_browser::{
    model::{Browser, Loader, Location, PageState, Security},
    native::{self, Address, Resolver},
};
use yami_dht::{Contact, Network, PeerRecord, Service};
use yami_identity::{NodeId, NodeIdentity};
use yami_node::{peers::PeerStore, Documents, Site};
use ynp::{DocumentData, Incoming};
use ynp_tls::TlsListener;

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("yami-browser-native-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct CountedSite {
    site: Site,
    requests: Arc<AtomicUsize>,
}
impl Documents for CountedSite {
    fn lookup(&self, key: &str) -> std::io::Result<Option<DocumentData>> {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.site.lookup(key)
    }
}
struct Node {
    contact: Contact,
    service: Service,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Node {
    fn start(identity: Arc<NodeIdentity>, service: Service, site: &Path) -> Self {
        let listener =
            TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&identity)).unwrap();
        let SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        service
            .prepare_self(&identity, vec![endpoint], yami_dht::now().unwrap())
            .unwrap();
        let contact = Contact::new(*identity.id(), endpoint, Network::Loopback).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(AtomicUsize::new(0));
        let documents = CountedSite {
            site: Site::open(site).unwrap(),
            requests: Arc::clone(&requests),
        };
        let stopping = Arc::clone(&stop);
        let handler = service.clone();
        let task = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                if let Ok(mut connection) = listener.accept() {
                    if stopping.load(Ordering::SeqCst) {
                        break;
                    }
                    let _ = yami_node::serve_connection_with_dht(
                        &mut *connection,
                        &documents,
                        &handler,
                    );
                }
            }
        });
        Self {
            contact,
            service,
            requests,
            stop,
            task: Some(task),
        }
    }
    fn record(&self) -> PeerRecord {
        self.service
            .snapshot()
            .unwrap()
            .get(self.contact.id, yami_dht::now().unwrap())
            .unwrap()
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.contact.endpoint);
        self.task.take().unwrap().join().unwrap();
    }
}
fn service(root: &Path, name: &str, id: NodeId) -> Service {
    Service::open(root.join(format!("{name}.cache")), id, Network::Loopback).unwrap()
}
fn page(dir: &Path, key: &str, title: &str, links: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{key}.ylang")),
        format!("@page 0.1\n  title {title}\n= {title}\n:: Links\n{links}"),
    )
    .unwrap();
    fs::write(dir.join("style.ysl"), "").unwrap();
}
fn wait(browser: &mut Browser, loader: &Loader) {
    let deadline = Instant::now() + Duration::from_secs(25);
    while matches!(browser.active().state, PageState::Loading) {
        browser.poll(loader);
        assert!(Instant::now() < deadline, "bounded browser load");
        thread::sleep(Duration::from_millis(2));
    }
}
fn ready(browser: &mut Browser, loader: &Loader, title: &str, id: NodeId) {
    wait(browser, loader);
    assert_eq!(
        browser.active().title(),
        title,
        "{:?}",
        browser.active().state
    );
    assert_eq!(browser.active().security(), Some(&Security::Verified(id)));
}
fn error(browser: &mut Browser, loader: &Loader, text: &str) {
    wait(browser, loader);
    let PageState::Error(message) = &browser.active().state else {
        panic!("expected {text}: {:?}", browser.active().state)
    };
    assert!(message.contains(text), "{message}");
    assert!(!message.contains("node.key"));
    assert!(browser.active().security().is_none());
}
fn links(nodes: &[ylang::Node]) -> Vec<String> {
    let mut targets = Vec::new();
    for node in nodes {
        match &node.kind {
            ylang::NodeKind::Link { target, .. } => targets.push(target.clone()),
            ylang::NodeKind::Section { children, .. } => targets.extend(links(children)),
            _ => {}
        }
    }
    targets
}
fn loader(directory: &Path) -> (Arc<Resolver>, Loader) {
    let resolver = Arc::new(Resolver::new(directory.into(), Network::Loopback));
    let loader = Loader::with_resolver(Arc::new(|| {}), Arc::clone(&resolver)).unwrap();
    (resolver, loader)
}

#[test]
fn discover_refreshes_only_explicitly_and_opening_independently_authenticates() {
    use yami_browser::discover::{Discover, Field};
    use yami_discover::{Directory, DirectoryEntry, SourceState, WebsiteAddress};
    fn settle(panel: &mut Discover) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while panel.busy.is_some() {
            panel.poll();
            assert!(Instant::now() < deadline, "bounded directory refresh");
            thread::sleep(Duration::from_millis(2));
        }
    }
    let scratch = Scratch::new("discover");
    let site_id = Arc::new(NodeIdentity::generate().unwrap());
    let dir_id = Arc::new(NodeIdentity::generate().unwrap());
    let impostor = NodeIdentity::generate().unwrap();
    let website = format!("yami://{}/home", site_id.id());
    let forged = format!("yami://{}/home", impostor.id());
    let web_path = scratch.0.join("website");
    page(&web_path, "home", "Actual website", "");
    let web = Node::start(
        site_id.clone(),
        service(&scratch.0, "web", *site_id.id()),
        &web_path,
    );
    let dir_path = scratch.0.join("directory");
    fs::create_dir_all(&dir_path).unwrap();
    let document = Directory::new(
        "Untrusted source",
        vec![
            DirectoryEntry::new(
                "Café Operator Title",
                "Directory supplied description",
                WebsiteAddress::parse(&website).unwrap(),
                Some("Writing".into()),
            )
            .unwrap(),
            DirectoryEntry::new(
                "Trusted-looking fake title",
                "This listing does not authenticate its endpoint",
                WebsiteAddress::parse(&forged).unwrap(),
                None,
            )
            .unwrap(),
        ],
    )
    .unwrap();
    document.save_new(dir_path.join("directory.ylang")).unwrap();
    let directory = Node::start(
        dir_id.clone(),
        service(&scratch.0, "directory", *dir_id.id()),
        &dir_path,
    );
    yami_dht::publish(&web.service, &web.record(), &[directory.contact]).unwrap();
    let now = yami_dht::now().unwrap();
    let false_location = PeerRecord::sign(
        &impostor,
        vec![web.contact.endpoint],
        now + 120,
        1,
        Network::Loopback,
        now,
    )
    .unwrap();
    directory.service.accept(false_location, now).unwrap();
    let peer_path = scratch.0.join("peers");
    PeerStore::open(&peer_path)
        .unwrap()
        .add(directory.contact.id, directory.contact.endpoint.into())
        .unwrap();
    let (_, loader) = loader(&peer_path);
    let state = scratch.0.join("discover-state");
    let mut panel = Discover::new(Ok(state.clone()), Arc::new(|| {}));
    panel.toggle();
    settle(&mut panel);
    panel.name = "community".into();
    panel.address = format!("yami://{}/directory", dir_id.id());
    panel.add();
    settle(&mut panel);
    panel.set_field(Field::Search, "cafe".into());
    assert!(panel.listings.is_empty());
    assert_eq!(
        directory.requests.load(Ordering::SeqCst),
        0,
        "startup, add and search never fetch"
    );
    panel.select_source(0);
    // The worker also rechecks the browser privacy mode before constructing
    // a transport, even when callers bypass the disabled UI control.
    loader.set_local_only(true);
    panel.refresh_selected(loader.directory_fetcher(), false);
    settle(&mut panel);
    assert!(panel.error.as_deref().unwrap().contains("local-only"));
    assert_eq!(directory.requests.load(Ordering::SeqCst), 0);
    assert!(!state.join("dht-local-discover.cache").exists());
    loader.set_local_only(false);
    panel.refresh_selected(loader.directory_fetcher(), false);
    settle(&mut panel);
    assert!(panel.error.is_none(), "{:?}", panel.error);
    assert_eq!(panel.listings.len(), 1);
    assert_eq!(panel.listings[0].entry.title(), "Café Operator Title");
    assert_eq!(panel.listings[0].state, SourceState::Fresh);
    assert_eq!(directory.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        web.requests.load(Ordering::SeqCst),
        0,
        "refresh does not visit listed sites"
    );
    let mut browser = Browser::new();
    browser.library =
        Some(yami_browser::library::Library::open(scratch.0.join("library")).unwrap());
    browser
        .library
        .as_mut()
        .unwrap()
        .save_bookmark(&website, "My bookmark", "")
        .unwrap();
    browser.navigate("local:welcome", &loader).unwrap();
    wait(&mut browser, &loader);
    browser.open_tab();
    browser.navigate("local:about", &loader).unwrap();
    wait(&mut browser, &loader);
    panel.select_website(0);
    assert_eq!(browser.tabs.len(), 2);
    assert_eq!(browser.active().security(), Some(&Security::BuiltIn));
    browser
        .navigate(panel.selected_website.as_ref().unwrap(), &loader)
        .unwrap();
    ready(&mut browser, &loader, "Actual website", *site_id.id());
    browser.back(&loader);
    wait(&mut browser, &loader);
    assert_eq!(browser.active().security(), Some(&Security::BuiltIn));
    browser.forward(&loader);
    ready(&mut browser, &loader, "Actual website", *site_id.id());
    assert_eq!(browser.tabs.len(), 2);
    assert_eq!(
        browser.library.as_ref().unwrap().bookmarks()[0].title,
        "My bookmark"
    );
    assert!(!browser.library.as_ref().unwrap().history_enabled());
    assert!(browser.library.as_ref().unwrap().visits().is_empty());
    panel.set_field(Field::Search, "fake".into());
    panel.select_website(0);
    let before = web.requests.load(Ordering::SeqCst);
    browser
        .navigate(panel.selected_website.as_ref().unwrap(), &loader)
        .unwrap();
    wait(&mut browser, &loader);
    assert!(matches!(browser.active().state, PageState::Error(_)));
    assert!(browser.active().security().is_none());
    assert_eq!(
        web.requests.load(Ordering::SeqCst),
        before,
        "wrong TLS identity never receives a document request"
    );
    // A later invalid directory is an error with snapshot-backed results.
    fs::write(
        dir_path.join("directory.ylang"),
        "@page 0.1\n  title Not a directory\n= Wrong format\n",
    )
    .unwrap();
    panel.refresh_selected(loader.directory_fetcher(), false);
    settle(&mut panel);
    assert!(panel.error.is_some());
    assert!(matches!(
        panel.listings[0].state,
        SourceState::Cached { .. }
    ));
    let requests = directory.requests.load(Ordering::SeqCst);
    let mut reopened = Discover::new(Ok(state), Arc::new(|| {}));
    reopened.toggle();
    settle(&mut reopened);
    reopened.set_field(Field::Search, "cafe".into());
    assert_eq!(reopened.listings.len(), 1);
    assert!(matches!(
        reopened.listings[0].state,
        SourceState::Cached { .. }
    ));
    assert_eq!(directory.requests.load(Ordering::SeqCst), requests);
}

#[test]
fn three_nodes_resolve_render_navigate_refresh_and_authenticate() {
    let scratch = Scratch::new("three");
    let mut ids: Vec<_> = (0..3)
        .map(|i| Arc::new(NodeIdentity::create(scratch.0.join(format!("identity-{i}"))).unwrap()))
        .collect();
    // Make C closer to A than B, deterministically exercising a stale reply
    // from C before the refresh reaches B's newer record.
    if yami_dht::distance(*ids[2].id(), *ids[0].id())
        > yami_dht::distance(*ids[1].id(), *ids[0].id())
    {
        ids.swap(1, 2);
    }
    let a_id = *ids[0].id();
    let c_id = *ids[2].id();
    let home = format!("yami://{a_id}/home");
    let about = format!("yami://{a_id}/about");
    let cross = format!("yami://{c_id}/home");
    let site = scratch.0.join("site");
    page(
        &site,
        "home",
        "Alice home",
        &format!("  > {about} | About\n  > {cross} | Other node\n"),
    );
    page(
        &site,
        "about",
        "Alice about",
        &format!("  > ynp://{a_id}/home | Home by identity\n"),
    );
    let a_state = service(&scratch.0, "a", a_id);
    let b = Node::start(
        Arc::clone(&ids[1]),
        service(&scratch.0, "b", *ids[1].id()),
        &site,
    );
    let a = Node::start(Arc::clone(&ids[0]), a_state.clone(), &site);
    let c = Node::start(Arc::clone(&ids[2]), service(&scratch.0, "c", c_id), &site);
    assert_eq!(
        yami_dht::publish(&a.service, &a.record(), &[b.contact]).unwrap(),
        1
    );
    assert_eq!(
        yami_dht::publish(&c.service, &c.record(), &[b.contact]).unwrap(),
        1
    );
    let directory = scratch.0.join("browser");
    PeerStore::open(&directory)
        .unwrap()
        .add(b.contact.id, b.contact.endpoint.into())
        .unwrap();
    let (resolver, loader) = loader(&directory);
    let mut browser = Browser::new();
    let started = Instant::now();
    browser.navigate(&home, &loader).unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(250),
        "navigation must only queue work"
    );
    ready(&mut browser, &loader, "Alice home", a_id);
    assert_eq!(browser.active().address, home);
    let PageState::Ready(loaded) = &browser.active().state else {
        unreachable!()
    };
    let targets = links(&loaded.document.nodes);
    assert_eq!(targets, [about.clone(), cross.clone()]);
    let layout = yami_render::layout(&loaded.document, &loaded.style).unwrap();
    assert!(layout.terminal().contains("Alice home"));
    assert!(layout.terminal().contains("Other node"));
    assert!(Service::read_snapshot(&resolver.state_path(), Network::Public).is_err());
    let stored = Service::read_snapshot(&resolver.state_path(), Network::Loopback).unwrap();
    assert_eq!(
        stored.get(a_id, yami_dht::now().unwrap()).unwrap(),
        a.record()
    );
    assert!(!directory.join("node.key").exists());

    // The shipped executable reads the documented peer/network environment,
    // resolves the identity directly, and supports the terminal/PPM tools too.
    let cli_directory = scratch.0.join("terminal-peers");
    PeerStore::open(&cli_directory)
        .unwrap()
        .add(b.contact.id, b.contact.endpoint.into())
        .unwrap();
    let ppm = scratch.0.join("native.ppm");
    let browser_config = scratch.0.join("browser.conf");
    fs::write(
        &browser_config,
        format!(
            "yaminet-config 1\nrole browser\nnetwork loopback\nstate {}\nbootstrap {}@{}\n",
            cli_directory.display(),
            b.contact.id,
            b.contact.endpoint
        ),
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_yami-browser"))
        .env("YAMINET_PEER_DIR", &cli_directory)
        .env("YAMINET_BROWSER_DHT_NETWORK", "local")
        .arg("--config")
        .arg(&browser_config)
        .args([&home, "--ppm"])
        .arg(&ppm)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Alice home"));
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("Verified node {a_id}")));
    assert!(fs::read(ppm).unwrap().starts_with(b"P6\n"));
    assert!(!cli_directory.join("node.key").exists());

    browser.follow(&targets[0], &loader).unwrap();
    ready(&mut browser, &loader, "Alice about", a_id);
    assert_eq!(browser.active().address, about);
    browser
        .follow(&format!("ynp://{a_id}/home"), &loader)
        .unwrap();
    ready(&mut browser, &loader, "Alice home", a_id);
    assert_eq!(browser.active().address, home);
    let numeric = format!("ynp://{}/about", a.contact.endpoint);
    browser.follow(&numeric, &loader).unwrap();
    ready(&mut browser, &loader, "Alice about", a_id);
    assert_eq!(browser.active().address, about);
    assert!(browser.back(&loader));
    ready(&mut browser, &loader, "Alice home", a_id);
    assert!(browser.forward(&loader));
    ready(&mut browser, &loader, "Alice about", a_id);
    browser.reload(&loader);
    ready(&mut browser, &loader, "Alice about", a_id);
    let history = browser.active().history.len();
    browser.active_mut().scroll = 120.0;
    browser.active_mut().target_scroll = 120.0;
    assert!(browser.open_tab());
    browser.follow(&cross, &loader).unwrap();
    ready(&mut browser, &loader, "Alice home", c_id);
    browser.select(0);
    assert_eq!(browser.active().scroll, 120.0);
    assert_eq!(browser.active().target_scroll, 120.0);
    assert_eq!(browser.active().history.len(), history);
    browser.close_tab(1);
    // An unrelated plaintext link cannot cause an implicit downgrade.
    let trap = TcpListener::bind("127.0.0.1:0").unwrap();
    trap.set_nonblocking(true).unwrap();
    assert!(browser
        .follow(
            &format!("ynp://{}/home", trap.local_addr().unwrap()),
            &loader
        )
        .is_err());
    error(&mut browser, &loader, "Refused plaintext");
    assert!(trap.accept().is_err());
    browser.reload(&loader);
    ready(&mut browser, &loader, "Alice about", a_id);

    // A validly signed lie is still not connection authentication. The wrong
    // identity must fail before the node's Documents implementation is called.
    let liar = NodeIdentity::generate().unwrap();
    let at = yami_dht::now().unwrap();
    let claim = PeerRecord::sign(
        &liar,
        vec![a.contact.endpoint],
        at + 120,
        1,
        Network::Loopback,
        at,
    )
    .unwrap();
    b.service.accept(claim, at).unwrap();
    let before = a.requests.load(Ordering::Relaxed);
    browser
        .navigate(&format!("yami://{}/home", liar.id()), &loader)
        .unwrap();
    error(&mut browser, &loader, "TLS identity mismatch");
    assert_eq!(a.requests.load(Ordering::Relaxed), before);
    browser.reload(&loader);
    error(&mut browser, &loader, "TLS identity mismatch");

    // Move A to another endpoint with the SAME identity and persistent sequence.
    // The browser first tries its cached old location, then refreshes through B.
    let previous = a.record();
    let old_endpoint = a.contact.endpoint;
    drop(a);
    let a = Node::start(Arc::clone(&ids[0]), a_state, &site);
    assert_ne!(a.contact.endpoint, old_endpoint);
    assert!(a.record().sequence() > previous.sequence());
    assert_eq!(
        yami_dht::publish(&a.service, &a.record(), &[b.contact]).unwrap(),
        1
    );
    assert!(b
        .service
        .accept(previous, yami_dht::now().unwrap())
        .is_err());
    browser.navigate(&home, &loader).unwrap();
    ready(&mut browser, &loader, "Alice home", a_id);
    let current = Service::read_snapshot(&resolver.state_path(), Network::Loopback).unwrap();
    assert_eq!(
        current.get(a_id, yami_dht::now().unwrap()).unwrap(),
        a.record()
    );
    assert_eq!(browser.active().address, home);

    // B is an ordinary bootstrap, not a mandatory central service. Warm cache
    // navigation keeps working after B goes offline.
    drop(b);
    browser.reload(&loader);
    ready(&mut browser, &loader, "Alice home", a_id);
    drop(a);
    browser.reload(&loader);
    error(&mut browser, &loader, "Node temporarily unreachable");
    assert_eq!(browser.active().address, home);
    // Cache-only success also works after reopening, with no bootstrap pins.
    fs::remove_file(directory.join("peers.tsv")).unwrap();
    drop(loader);
    drop(resolver);
    // Completion is published just before a worker drops its fetcher Arc.
    let deadline = Instant::now() + Duration::from_secs(2);
    let reopened = loop {
        match Service::open_lookup(directory.join("dht-local-browser.cache"), Network::Loopback) {
            Ok(service) => break service,
            Err(_) => {
                assert!(Instant::now() < deadline);
                thread::yield_now();
            }
        }
    };
    assert_eq!(reopened.snapshot().unwrap().local, stored.local);
    drop(reopened);
    let (_, reopened_loader) = self::loader(&directory);
    browser.navigate(&cross, &reopened_loader).unwrap();
    ready(&mut browser, &reopened_loader, "Alice home", c_id);
}

#[test]
fn browser_reports_missing_bootstrap_unknown_offline_and_expired_records() {
    let scratch = Scratch::new("errors");
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let url = format!("yami://{}/home", identity.id());
    let (_, empty) = loader(&scratch.0.join("empty"));
    let mut browser = Browser::new();
    browser.navigate(&url, &empty).unwrap();
    error(&mut browser, &empty, "No bootstrap peers configured");

    let site = scratch.0.join("site");
    page(&site, "home", "Bootstrap", "");
    let b_id = Arc::new(NodeIdentity::generate().unwrap());
    let b = Node::start(
        Arc::clone(&b_id),
        service(&scratch.0, "b", *b_id.id()),
        &site,
    );
    let directory = scratch.0.join("configured");
    PeerStore::open(&directory)
        .unwrap()
        .add(b.contact.id, b.contact.endpoint.into())
        .unwrap();
    let (_, configured) = loader(&directory);
    browser.navigate(&url, &configured).unwrap();
    error(&mut browser, &configured, "Unknown node");
    drop(b);
    browser.reload(&configured);
    error(&mut browser, &configured, "Node temporarily unreachable");

    let directory = scratch.0.join("expired");
    let resolver = Arc::new(Resolver::new(directory.clone(), Network::Loopback));
    let at = yami_dht::now().unwrap();
    let expired = PeerRecord::sign(
        &identity,
        vec!["127.0.0.1:1".parse().unwrap()],
        at - 1,
        7,
        Network::Loopback,
        at - 10,
    )
    .unwrap();
    let state = Service::open_lookup(resolver.state_path(), Network::Loopback).unwrap();
    state.accept(expired, at - 10).unwrap();
    drop(state);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    browser.navigate(&url, &loader).unwrap();
    error(&mut browser, &loader, "Expired or invalid peer record");
    browser.navigate("local:welcome", &loader).unwrap();
    wait(&mut browser, &loader);
    assert_eq!(browser.active().security(), Some(&Security::BuiltIn));
}

#[test]
fn dht_timeout_does_not_block_other_browser_tabs() {
    let scratch = Scratch::new("timeout");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let id = *NodeIdentity::generate().unwrap().id();
    PeerStore::open(&scratch.0)
        .unwrap()
        .add(id, listener.local_addr().unwrap())
        .unwrap();
    let (accepted, observed) = std::sync::mpsc::channel();
    let task = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        accepted.send(()).unwrap();
        // Wait for the client's bounded TLS timeout/close, without sleeping.
        stream
            .set_read_timeout(Some(Duration::from_secs(8)))
            .unwrap();
        let _ = std::io::copy(&mut stream, &mut std::io::sink());
    });
    let (_, loader) = loader(&scratch.0);
    let mut browser = Browser::new();
    browser
        .navigate(&format!("yami://{id}/home"), &loader)
        .unwrap();
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(matches!(browser.active().state, PageState::Loading));
    assert!(browser.open_tab());
    browser.navigate("local:welcome", &loader).unwrap();
    wait(&mut browser, &loader);
    assert_eq!(browser.active().title(), "Welcome to YamiNet");
    browser.select(0);
    error(&mut browser, &loader, "DHT lookup timeout");
    task.join().unwrap();
}

#[test]
fn invalid_signed_and_malformed_records_are_reported_by_the_browser() {
    for malformed in [false, true] {
        let scratch = Scratch::new(if malformed { "malformed" } else { "invalid" });
        let identity = Arc::new(NodeIdentity::generate().unwrap());
        let listener =
            TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&identity)).unwrap();
        PeerStore::open(&scratch.0)
            .unwrap()
            .add(*identity.id(), listener.local_addr().unwrap())
            .unwrap();
        let owner = NodeIdentity::generate().unwrap();
        let at = yami_dht::now().unwrap();
        let mut invalid = PeerRecord::sign(
            &owner,
            vec!["127.0.0.1:1".parse().unwrap()],
            at + 60,
            1,
            Network::Loopback,
            at,
        )
        .unwrap()
        .encode();
        *invalid.last_mut().unwrap() ^= 1;
        if malformed {
            invalid.pop();
        }
        let task = thread::spawn(move || {
            let mut connection = listener.accept().unwrap();
            let mut request = [0; yami_dht::MAX_FRAME];
            connection.read_exact(&mut request).unwrap();
            // A malicious authenticated bootstrap relays a structurally valid but
            // forged record. The production wire decoder and Service validate it.
            let mut reply = [0; yami_dht::MAX_FRAME];
            reply[..24].copy_from_slice(&request[..24]);
            reply[5] = 3;
            reply[22..24].copy_from_slice(&((3 + invalid.len()) as u16).to_be_bytes());
            reply[24] = 1;
            reply[25..27].copy_from_slice(&(invalid.len() as u16).to_be_bytes());
            reply[27..27 + invalid.len()].copy_from_slice(&invalid);
            connection.write_all(&reply).unwrap();
            connection.flush().unwrap();
        });
        let (resolver, loader) = loader(&scratch.0);
        let mut browser = Browser::new();
        browser
            .navigate(&format!("yami://{}/home", owner.id()), &loader)
            .unwrap();
        error(&mut browser, &loader, "Expired or invalid peer record");
        task.join().unwrap();
        assert!(
            !resolver.state_path().exists(),
            "forged records must never be persisted"
        );
    }
}

#[test]
fn native_addresses_are_strict_and_document_adapter_preserves_ylang() {
    let id = *NodeIdentity::generate().unwrap().id();
    for suffix in ["", "/", "/home", "/Key_2-x"] {
        let address = Address::parse(&format!("yami://{id}{suffix}")).unwrap();
        assert_eq!(address.id, id);
        assert_eq!(
            Location::parse(&address.url()).unwrap(),
            Location::Yami(address)
        );
    }
    let mut damaged = id.to_string();
    damaged.replace_range(5..6, if &damaged[5..6] == "a" { "b" } else { "a" });
    for text in [
        format!("yami://{damaged}/home"),
        format!("yami://{}/home", id.to_string().to_uppercase()),
        format!("yami://{id}:7443/home"),
        format!("yami://{id}@127.0.0.1/home"),
        format!("yami://{id}/home?x=1"),
        format!("yami://{id}/home#x"),
        format!("yami://{id}//home"),
        format!("yami://{id}/../home"),
        format!("yami://{id}/%68ome"),
        format!("yami://{id}/home/"),
        format!("yami://{id}/{}", "a".repeat(129)),
        format!("yami://{id}/ho me"),
        format!("YAMI://{id}/home"),
        "yami://alice/home".into(),
        "yami://127.0.0.1:7443/home".into(),
        format!("yami://{id}/home\n"),
    ] {
        assert!(Address::parse(&text).is_err(), "{text}");
        assert!(Location::parse(&text).is_err(), "{text}");
    }
    let legacy = "@page 0.1\n  title Test\n> ynp:///untouched | Existing\n";
    assert_eq!(
        native::parse_document(legacy).unwrap(),
        ylang::parse(legacy).unwrap()
    );
    let source = format!("@page 0.1\n  title Test\n:: Section\n  > yami://{id}/home | Native\n  > ynp:///untouched | Existing\n");
    let parsed = native::parse_document(&source).unwrap();
    assert_eq!(
        links(&parsed.nodes),
        [format!("yami://{id}/home"), "ynp:///untouched".into()]
    );
    assert_eq!(
        parsed.nodes[0].span.start,
        source.find(":: Section").unwrap()
    );
}
