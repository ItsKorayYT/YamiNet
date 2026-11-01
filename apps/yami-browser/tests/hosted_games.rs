//! Hosted delivery through the actual browser model, YNP dispatcher, TLS and DHT.
use std::{
    fs, io,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use yami_browser::{
    model::{Browser, Loader, PageState, Security},
    native::Resolver,
};
use yami_dht::{Network, Service};
use yami_identity::NodeIdentity;
use yami_node::{shutdown::Shutdown, Documents, Site};
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("yami-rich-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("site/assets")).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Tracked {
    site: Site,
    assets: Arc<AtomicUsize>,
    slow: Arc<AtomicBool>,
}
impl Documents for Tracked {
    fn lookup(&self, key: &str) -> io::Result<Option<ynp::DocumentData>> {
        self.site.lookup(key)
    }
    fn game(&self, key: &str, resource: ynp::GameResource) -> io::Result<Option<Vec<u8>>> {
        self.assets.fetch_add(1, Ordering::SeqCst);
        if self.slow.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(600));
        }
        self.site.game(key, resource)
    }
}
struct Host {
    id: yami_identity::NodeId,
    endpoint: SocketAddr,
    stop: Shutdown,
    thread: Option<thread::JoinHandle<()>>,
    requests: Arc<AtomicUsize>,
    slow: Arc<AtomicBool>,
}
impl Host {
    fn start(scratch: &Scratch) -> Self {
        let identity = Arc::new(NodeIdentity::create(scratch.0.join("identity")).unwrap());
        let id = *identity.id();
        let listener =
            ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity.clone()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let service = Service::open(scratch.0.join("host.cache"), id, Network::Loopback).unwrap();
        let SocketAddr::V4(ip) = endpoint else {
            unreachable!()
        };
        service
            .prepare_self(&identity, vec![ip], yami_dht::now().unwrap())
            .unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let slow = Arc::new(AtomicBool::new(false));
        let tracked = Tracked {
            site: Site::open(scratch.0.join("site")).unwrap(),
            assets: requests.clone(),
            slow: slow.clone(),
        };
        let stop = Shutdown::default();
        let signal = stop.clone();
        let thread = thread::spawn(move || {
            yami_node::serve_with_handler_until(
                &listener,
                4,
                &|connection| yami_node::serve_connection_with_dht(connection, &tracked, &service),
                &|_| {},
                &signal,
            )
            .unwrap()
        });
        Self {
            id,
            endpoint,
            stop,
            thread: Some(thread),
            requests,
            slow,
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.stop();
        self.thread.take().unwrap().join().unwrap();
    }
}
fn wait(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(8);
    while !ready() {
        assert!(Instant::now() < until, "rich site operation timed out");
        thread::sleep(Duration::from_millis(5));
    }
}
fn poll(browser: &mut Browser, loader: &Loader) {
    wait(|| {
        browser.poll(loader);
        !matches!(browser.active().state, PageState::Loading)
    })
}
fn prepare(scratch: &Scratch) {
    fs::create_dir_all(scratch.0.join("site/games")).unwrap();
    fs::write(scratch.0.join("site/home.ylang"),"@page 0.2\n  title Hosted arcade\n= Prism Breakout\n> game:prism | Prism Breakout\n> /about | About\n").unwrap();
    fs::write(
        scratch.0.join("site/about.ylang"),
        "@page 0.2\n  title About arcade\nGames start only from Play.\n",
    )
    .unwrap();
    let bytes = include_bytes!("../../../games/breakout/breakout.wasm");
    let hash = yami_identity::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    fs::write(scratch.0.join("site/games/prism.wasm"), bytes).unwrap();
    fs::write(scratch.0.join("site/games/prism.json"),serde_json::to_vec(&serde_json::json!({"version":1,"abi":1,"title":"Prism Breakout","description":"Test hosted game","module":{"key":"prism","bytes":bytes.len(),"sha256":hash}})).unwrap()).unwrap();
}
fn setup(host: &Host, scratch: &Scratch) -> (Browser, Loader) {
    let state = scratch.0.join("browser");
    yami_node::peers::PeerStore::open(&state)
        .unwrap()
        .add(host.id, host.endpoint)
        .unwrap();
    let resolver = Arc::new(Resolver::new(state, Network::Loopback));
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(&format!("yami://{}/home", host.id), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(host.id))
    );
    (browser, loader)
}
fn game_wait(browser: &mut Browser, check: impl Fn(&yami_games::Snapshot) -> bool) {
    wait(|| {
        browser.poll_games();
        matches!(&browser.active().state,PageState::Game(game) if check(&game.snapshot))
    });
}
#[test]
fn authenticated_page_is_inert_until_play_and_game_uses_normal_tabs_and_controls() {
    let scratch = Scratch::new("game-consent");
    prepare(&scratch);
    let host = Host::start(&scratch);
    let (mut browser, loader) = setup(&host, &scratch);
    thread::sleep(Duration::from_millis(100));
    browser.poll(&loader);
    assert_eq!(host.requests.load(Ordering::SeqCst), 0);
    assert!(browser.play_hosted("unoffered", Arc::new(|| {})).is_err());
    assert_eq!(browser.tabs.len(), 1);
    // Link navigation, address navigation and restored history are never consent.
    assert!(browser.follow("game:prism", &loader).is_err());
    assert_eq!(host.requests.load(Ordering::SeqCst), 0);
    browser
        .navigate(&format!("yami://{}/home", host.id), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    browser.play_hosted("prism", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        s.frames > 1 && s.status == yami_games::Status::Running
    });
    assert_eq!(browser.tabs.len(), 2);
    assert_eq!(host.requests.load(Ordering::SeqCst), 2);
    assert_eq!(browser.active().title(), "Prism Breakout");
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(host.id))
    );
    browser.select(0);
    browser.select(1);
    game_wait(&mut browser, |s| s.status == yami_games::Status::Paused);
    let PageState::Game(game) = &browser.active().state else {
        panic!()
    };
    game.session.resume();
    game_wait(&mut browser, |s| s.status == yami_games::Status::Running);
    let PageState::Game(game) = &browser.active().state else {
        panic!()
    };
    game.session.stop();
    game_wait(&mut browser, |s| s.status == yami_games::Status::Stopped);
    let PageState::Game(game) = &mut browser.active_mut().state else {
        panic!()
    };
    game.session = game.source.launch(Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        s.frames > 1 && s.status == yami_games::Status::Running
    });
    assert_eq!(host.requests.load(Ordering::SeqCst), 4);
    assert!(browser.back(&loader));
    assert!(!matches!(browser.active().state, PageState::Game(_)));
    assert!(browser.forward(&loader));
    poll(&mut browser, &loader);
    assert!(matches!(browser.active().state, PageState::Ready(_)));
    assert_eq!(host.requests.load(Ordering::SeqCst), 4);
    browser.close_tab_with_loader(1, &loader);
    assert_eq!(browser.active().title(), "Hosted arcade");
    // The executable module was never written into the player profile.
    assert!(!scratch.0.join("browser/games").exists());
}
#[test]
fn failed_integrity_missing_malformed_and_cancelled_transfers_never_execute() {
    let scratch = Scratch::new("game-failure");
    prepare(&scratch);
    let host = Host::start(&scratch);
    let (mut browser, loader) = setup(&host, &scratch);
    fs::write(scratch.0.join("site/games/prism.wasm"), b"corrupt").unwrap();
    browser.play_hosted("prism", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        matches!(&s.status,yami_games::Status::Failed(e) if e.contains("integrity"))
            && s.frames == 0
    });
    assert!(browser.active().security().is_none());
    browser.close_tab(1);
    fs::remove_file(scratch.0.join("site/games/prism.wasm")).unwrap();
    browser.play_hosted("prism", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        matches!(&s.status, yami_games::Status::Failed(_)) && s.frames == 0
    });
    browser.close_tab(1);
    fs::write(scratch.0.join("site/games/prism.json"), b"{}").unwrap();
    browser.play_hosted("prism", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        matches!(&s.status, yami_games::Status::Failed(_)) && s.frames == 0
    });
    browser.close_tab(1);
    prepare(&scratch);
    host.slow.store(true, Ordering::SeqCst);
    let before = host.requests.load(Ordering::SeqCst);
    browser.play_hosted("prism", Arc::new(|| {})).unwrap();
    wait(|| host.requests.load(Ordering::SeqCst) > before);
    let now = Instant::now();
    browser.close_tab_with_loader(1, &loader);
    browser.follow("/about", &loader).unwrap();
    poll(&mut browser, &loader);
    assert!(now.elapsed() < Duration::from_millis(400));
    assert_eq!(browser.active().title(), "About arcade");
    thread::sleep(Duration::from_millis(700));
    assert_eq!(host.requests.load(Ordering::SeqCst), before + 1);
}
#[test]
fn plaintext_parent_and_changed_destination_identity_cannot_deliver_a_game() {
    let scratch = Scratch::new("game-identity");
    prepare(&scratch);
    let host = Host::start(&scratch);
    let (mut browser, loader) = setup(&host, &scratch);
    // Use a real second node identity at the resource endpoint: TLS must reject it.
    let wrong = NodeIdentity::generate().unwrap();
    let origin = yami_browser::resources::Origin {
        destination: *wrong.id(),
        endpoint: host.endpoint,
        relay: None,
        public_relay: false,
    };
    assert!(
        yami_browser::hosted_games::fetch(&origin, "prism", &AtomicBool::new(false))
            .unwrap_err()
            .contains("authenticate")
    );
    assert_eq!(host.requests.load(Ordering::SeqCst), 0);
    let listener = ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let site = Site::open(scratch.0.join("site")).unwrap();
    let task = thread::spawn(move || yami_node::serve_once(&listener, &site).unwrap());
    browser
        .navigate(&format!("ynp://{endpoint}/home"), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    task.join().unwrap();
    assert!(browser
        .play_hosted("prism", Arc::new(|| {}))
        .unwrap_err()
        .contains("authenticated"));
    assert_eq!(host.requests.load(Ordering::SeqCst), 0);
}
#[test]
fn interrupted_tls_transfer_fails_without_a_frame() {
    use std::io::Write;
    use ynp::Incoming;
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let id = *identity.id();
    let listener = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let host = thread::spawn(move || {
        let mut connection = listener.accept().unwrap();
        ynp::read_frame(&mut *connection).unwrap();
        let mut frame = Vec::new();
        ynp::write_frame(
            &mut frame,
            &ynp::Frame {
                request_id: 3,
                message: ynp::Message::Game {
                    key: "prism".into(),
                    resource: ynp::GameResource::Manifest,
                    bytes: vec![b' '; 100],
                },
            },
        )
        .unwrap();
        connection.write_all(&frame[..40]).unwrap();
        connection.flush().unwrap();
    });
    let source = yami_browser::hosted_games::Source::Hosted {
        origin: yami_browser::resources::Origin {
            destination: id,
            endpoint,
            relay: None,
            public_relay: false,
        },
        key: "prism".into(),
    };
    let session = source.launch(Arc::new(|| {})).unwrap();
    wait(|| matches!(session.snapshot().status, yami_games::Status::Failed(_)));
    assert_eq!(session.snapshot().frames, 0);
    host.join().unwrap();
}
#[test]
fn relay_games_pin_both_relay_and_publisher_and_run_the_existing_vm() {
    let scratch = Scratch::new("relay-games");
    prepare(&scratch);
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let id = *identity.id();
    let relay_key = Arc::new(NodeIdentity::generate().unwrap());
    let relay = Arc::new(
        yami_relay::Relay::bind(
            "127.0.0.1:0".parse().unwrap(),
            relay_key.clone(),
            yami_relay::Limits::default(),
        )
        .unwrap(),
    );
    let endpoint = relay.local_addr().unwrap();
    let stop = Shutdown::default();
    let relay_stop = stop.clone();
    let running = relay.clone();
    let relay_task = thread::spawn(move || running.run(relay_stop.flag()).unwrap());
    let route = yami_relay::Route {
        relay: *relay_key.id(),
        endpoint,
        destination: id,
    };
    let host_stop = stop.clone();
    let publisher = identity.clone();
    let site = Site::open(scratch.0.join("site")).unwrap();
    let host_task = thread::spawn(move || {
        yami_relay::run_host(
            route,
            identity,
            host_stop.flag(),
            &|c| yami_node::serve_connection(c, &site).map(|_| ()),
            &|_| {},
        )
        .unwrap()
    });
    struct Tasks(Shutdown, Vec<thread::JoinHandle<()>>);
    impl Drop for Tasks {
        fn drop(&mut self) {
            self.0.stop();
            for task in self.1.drain(..) {
                task.join().unwrap();
            }
        }
    }
    let _tasks = Tasks(stop, vec![relay_task, host_task]);
    wait(|| relay.statistics().registrations == 1);
    let origin = yami_browser::resources::Origin {
        destination: id,
        endpoint,
        relay: Some(*relay_key.id()),
        public_relay: false,
    };
    let resolver = Arc::new(Resolver::new(
        scratch.0.join("relay-browser"),
        Network::Loopback,
    ));
    resolver.set_relay_enabled(true);
    let now = yami_dht::now().unwrap();
    let SocketAddr::V4(ip) = endpoint else {
        unreachable!()
    };
    let record = yami_dht::PeerRecord::sign_routes(
        &publisher,
        vec![],
        vec![yami_dht::RelayRoute {
            relay: *relay_key.id(),
            endpoint: ip,
        }],
        now + 120,
        1,
        Network::Loopback,
        now,
    )
    .unwrap();
    let cache = Service::open_lookup(resolver.state_path(), Network::Loopback).unwrap();
    cache.accept(record, now).unwrap();
    drop(cache);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(&format!("yami://{id}/home"), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    let security = Security::Relayed {
        destination: id,
        relay: *relay_key.id(),
    };
    assert_eq!(browser.active().security(), Some(&security));
    browser.play_hosted("prism", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        s.frames > 1 && s.status == yami_games::Status::Running
    });
    assert_eq!(browser.active().title(), "Prism Breakout");
    assert_eq!(browser.active().security(), Some(&security));
    drop(browser);
    let mut wrong = origin.clone();
    wrong.relay = Some(*NodeIdentity::generate().unwrap().id());
    assert!(yami_browser::hosted_games::fetch(&wrong, "prism", &AtomicBool::new(false)).is_err());
    let mut wrong = origin;
    wrong.destination = *NodeIdentity::generate().unwrap().id();
    assert!(yami_browser::hosted_games::fetch(&wrong, "prism", &AtomicBool::new(false)).is_err());
}

#[test]
fn abi2_authenticated_assets_instant_play_integrity_and_no_guest_requests() {
    let scratch = Scratch::new("abi2");
    prepare(&scratch);
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../games/lantern-drift");
    let local: serde_json::Value =
        serde_json::from_slice(&fs::read(source.join("game.yami-game.json")).unwrap()).unwrap();
    let module = fs::read(source.join("lantern-drift.wasm")).unwrap();
    let hash: String = yami_identity::sha256(&module)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut manifest = local.clone();
    manifest["module"] = serde_json::json!({"key":"lantern","bytes":module.len(),"sha256":hash});
    let directory = scratch.0.join("site/games");
    fs::write(
        directory.join("lantern.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(directory.join("lantern.wasm"), module).unwrap();
    for asset in local["assets"].as_array().unwrap() {
        let file = format!(
            "{}.{}",
            asset["key"].as_str().unwrap(),
            asset["kind"].as_str().unwrap()
        );
        fs::copy(source.join(&file), directory.join(&file)).unwrap();
    }
    fs::write(
        scratch.0.join("site/home.ylang"),
        "@page 0.2\n  title Hosted arcade\n> game:lantern | Lantern Drift\n",
    )
    .unwrap();
    let checked = yami_node::authoring::check(scratch.0.join("site")).unwrap();
    assert_eq!(checked.game_assets.len(), 4);
    let built = scratch.0.join("built-site");
    assert_eq!(
        yami_node::authoring::build(scratch.0.join("site"), &built)
            .unwrap()
            .game_assets
            .len(),
        4
    );
    let host = Host::start(&scratch);
    let (mut browser, loader) = setup(&host, &scratch);
    assert_eq!(host.requests.load(Ordering::SeqCst), 0);
    browser.play_hosted("lantern", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        s.frames > 2 && s.status == yami_games::Status::Running
    });
    assert_eq!(host.requests.load(Ordering::SeqCst), 6);
    let PageState::Game(game) = &browser.active().state else {
        panic!()
    };
    assert_eq!(game.snapshot.title, "Lantern Drift");
    game.session.key(yami_games::START, true);
    game.session.key(yami_games::START, false);
    game.session.pointer(70, 140);
    game.session.key(yami_games::MOUSE_LEFT, true);
    game_wait(&mut browser, |s| s.frames > 30);
    assert_eq!(
        host.requests.load(Ordering::SeqCst),
        6,
        "guest must never initiate resource transfers"
    );
    browser.close_tab(1);
    // Corruption of an authenticated publisher's resource still prevents any execution.
    let mut png = fs::read(directory.join("sprites.png")).unwrap();
    png[45] ^= 1;
    fs::write(directory.join("sprites.png"), png).unwrap();
    browser.play_hosted("lantern", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        s.frames == 0
            && matches!(&s.status,yami_games::Status::Failed(e) if e.contains("integrity"))
    });
    browser.close_tab(1);
    manifest["assets"][0]["key"] = serde_json::json!("yami://wrong-owner/sprites");
    fs::write(
        directory.join("lantern.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let before = host.requests.load(Ordering::SeqCst);
    browser.play_hosted("lantern", Arc::new(|| {})).unwrap();
    game_wait(&mut browser, |s| {
        s.frames == 0 && matches!(&s.status, yami_games::Status::Failed(_))
    });
    assert_eq!(
        host.requests.load(Ordering::SeqCst),
        before + 1,
        "reject foreign asset keys before requesting them"
    );
    drop(loader);
}
