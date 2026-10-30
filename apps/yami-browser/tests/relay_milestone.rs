//! Four independent OS processes on loopback. Models outbound-only NAT hosting;
//! does not claim physical cross-network, firewall or public Internet validation.
use std::{
    fs,
    net::{SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use yami_browser::{
    model::{Browser, Loader, PageState, Security},
    native::{Address, Resolver},
};
use yami_dht::{Network, PeerRecord, RelayRoute, Service};
use yami_identity::NodeIdentity;
use yami_node::{
    maintenance::{Config, Maintainer, Worker},
    peers::PeerStore,
    relay_host::{ActiveRoutes, Hosts},
    shutdown::Shutdown,
    Site,
};
use yami_relay::{Limits, Relay};
use yami_render::native::{NativeDocument, TextEngine};
use ynp_tls::TlsListener;

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "yami-relay-milestone-{name}-{}",
            std::process::id()
        ));
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
struct Process(Child);
impl Process {
    fn spawn(base: &Path, role: &str) -> Self {
        Self(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "process_fixture", "--nocapture"])
                .env("YAMINET_MILESTONE_FIXTURE", base)
                .env("YAMINET_MILESTONE_ROLE", role)
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        )
    }
    fn success(&mut self) {
        wait(|| self.0.try_wait().unwrap().is_some());
        assert!(self.0.wait().unwrap().success());
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(25);
    while !ready() {
        assert!(Instant::now() < deadline, "milestone condition timed out");
        thread::sleep(Duration::from_millis(50));
    }
}
fn id(base: &Path, role: &str) -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::load(base.join(role)).unwrap())
}
fn endpoint(base: &Path, role: &str) -> SocketAddrV4 {
    fs::read_to_string(base.join(format!("{role}.endpoint")))
        .unwrap()
        .parse()
        .unwrap()
}
fn stop_when_requested(base: &Path, role: &str, signal: Shutdown) {
    let path = base.join(format!("{role}.stop"));
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(180);
        while !path.exists() && Instant::now() < deadline && !signal.is_stopped() {
            thread::sleep(Duration::from_millis(25));
        }
        signal.stop();
    });
}
fn stored(base: &Path) -> Option<PeerRecord> {
    Service::read_snapshot(&base.join("B.cache"), Network::Loopback)
        .ok()?
        .get(*id(base, "A").id(), yami_dht::now().unwrap())
}
fn page_ready(browser: &mut Browser, loader: &Loader) {
    wait(|| {
        browser.poll(loader);
        !matches!(browser.active().state, PageState::Loading)
    });
}
fn client(base: &Path) {
    let host = *id(base, "A").id();
    let relay = *id(base, "R").id();
    let directory = base.join(format!("C-{}", std::process::id()));
    PeerStore::open(&directory)
        .unwrap()
        .add(*id(base, "B").id(), endpoint(base, "B").into())
        .unwrap();
    let resolver = Arc::new(Resolver::new(directory, Network::Loopback));
    let address = Address {
        id: host,
        key: "home".into(),
    };
    assert!(resolver
        .fetch(&address)
        .unwrap_err()
        .contains("Enable relay"));
    resolver.set_relay_enabled(true);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut browser = Browser::new();
    browser.navigate(&address.url(), &loader).unwrap();
    page_ready(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Relayed {
            destination: host,
            relay
        })
    );
    assert_eq!(browser.active().address, address.url());
    let PageState::Ready(page) = &browser.active().state else {
        panic!("{:?}", browser.active().state)
    };
    assert_eq!(page.document.metadata.title, "NAT homepage");
    let mut engine = TextEngine::new();
    let layout =
        NativeDocument::layout(&page.document, &page.style, 800.0, 1.0, &mut engine).unwrap();
    let mut pixmap = tiny_skia::Pixmap::new(800, 600).unwrap();
    layout.draw(&mut pixmap, 0.0, 0.0, 0.0, 600.0, &mut engine);
    assert!(pixmap.data().iter().any(|b| *b != 0));
    pixmap.save_png(base.join("homepage.png")).unwrap();
    // Relative connection endpoints never replace the stable identity. The
    // actual inert YLang link goes through Browser::follow, not direct fetching.
    fn links(nodes: &[ylang::Node]) -> Option<String> {
        for node in nodes {
            match &node.kind {
                ylang::NodeKind::Link { target, .. } => return Some(target.clone()),
                ylang::NodeKind::Section { children, .. } => {
                    if let Some(link) = links(children) {
                        return Some(link);
                    }
                }
                _ => {}
            }
        }
        None
    }
    let link = links(&page.document.nodes).unwrap();
    // Navigation may overlap the single serving slot renewal.
    browser.follow(&link, &loader).unwrap();
    page_ready(&mut browser, &loader);
    assert_eq!(browser.active().address, format!("yami://{host}/about"));
    assert_eq!(browser.active().title(), "NAT second page");
    assert!(browser.back(&loader));
    page_ready(&mut browser, &loader);
    assert_eq!(browser.active().title(), "NAT homepage");
    browser.reload(&loader);
    page_ready(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Relayed {
            destination: host,
            relay
        })
    );
    let config = yami_config::Config::parse(&format!("yaminet-config 1\nrole browser\nnetwork loopback\nstate {}\nrelay-fallback yes\nbootstrap {}@{}\n", base.join(format!("trial-C-{}",std::process::id())).display(),id(base,"B").id(),endpoint(base,"B"))).unwrap();
    let (report, success) = yami_browser::network_trial::run(
        &config,
        &format!("yami://{host}/home"),
        "cover",
        None,
        false,
        false,
    );
    assert!(success, "{report}");
    assert!(report
        .to_string()
        .contains("authenticated-relay-registration"));
    assert!(report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["name"] == "document-and-png" && step["observation"]["route"] == "relay"));
    fs::write(base.join("network-trial.json"), report.to_string()).unwrap();
    fs::write(
        base.join("C.done"),
        "rendered and navigated with verified destination and relay",
    )
    .unwrap();
}

#[test]
fn process_fixture() {
    let Ok(directory) = std::env::var("YAMINET_MILESTONE_FIXTURE") else {
        return;
    };
    let base = Path::new(&directory);
    let role = std::env::var("YAMINET_MILESTONE_ROLE").unwrap();
    let signal = Shutdown::default();
    stop_when_requested(base, &role, signal.clone());
    match role.as_str() {
        "R" | "S" => {
            let address = if base.join(format!("{role}.endpoint")).exists() {
                SocketAddr::V4(endpoint(base, &role))
            } else {
                "127.0.0.1:0".parse().unwrap()
            };
            let relay = Relay::bind_public(address, id(base, &role), Limits::default()).unwrap();
            fs::write(
                base.join(format!("{role}.endpoint")),
                relay.local_addr().unwrap().to_string(),
            )
            .unwrap();
            relay.run(signal.flag()).unwrap();
        }
        "B" | "D" | "N" => {
            let identity = id(base, &role);
            let selected = if base.join(format!("{role}.endpoint")).exists() {
                SocketAddr::V4(endpoint(base, &role))
            } else {
                "127.0.0.1:0".parse().unwrap()
            };
            let listener = TlsListener::bind(selected, identity.clone()).unwrap();
            listener.set_nonblocking(true).unwrap();
            let SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
                unreachable!()
            };
            let service = Service::open(
                base.join(format!("{role}.cache")),
                *identity.id(),
                Network::Loopback,
            )
            .unwrap();
            service
                .prepare_self(&identity, vec![endpoint], yami_dht::now().unwrap())
                .unwrap();
            fs::write(base.join(format!("{role}.endpoint")), endpoint.to_string()).unwrap();
            let site = Site::open(base.join("site")).unwrap();
            yami_node::serve_with_handler_until(
                &listener,
                4,
                &|c| yami_node::serve_connection_with_dht(c, &site, &service),
                &|_| {},
                &signal,
            )
            .unwrap();
        }
        "A" => {
            let identity = id(base, "A");
            let directory = base.join("A-peers");
            let mut peers = PeerStore::open(&directory).unwrap();
            for name in ["B", "D"] {
                if base.join(format!("{name}.endpoint")).exists()
                    && !peers.entries().iter().any(|p| p.id == *id(base, name).id())
                {
                    peers
                        .add(*id(base, name).id(), endpoint(base, name).into())
                        .unwrap();
                }
            }
            drop(peers);
            let endpoints_file = base.join("A.endpoints");
            let direct = if endpoints_file.exists() {
                yami_node::maintenance::read_endpoints(&endpoints_file, Network::Loopback).unwrap()
            } else {
                vec![]
            };
            let service =
                Service::open(directory.join("A.cache"), *identity.id(), Network::Loopback)
                    .unwrap();
            let engine = Maintainer::new(
                service.clone(),
                identity.clone(),
                direct,
                Config {
                    renew_secs: if endpoints_file.exists() { 43200 } else { 30 },
                    health_secs: 5,
                },
            )
            .unwrap();
            let active = ActiveRoutes::default();
            let mut routes = vec![RelayRoute {
                relay: *id(base, "R").id(),
                endpoint: endpoint(base, "R"),
            }];
            if base.join("S.endpoint").exists() {
                routes.push(RelayRoute {
                    relay: *id(base, "S").id(),
                    endpoint: endpoint(base, "S"),
                });
            }
            routes.sort();
            let _hosts = Hosts::start(
                routes,
                identity,
                Arc::new(Site::open(base.join("site")).unwrap()),
                service,
                signal.clone(),
                active.clone(),
            )
            .unwrap();
            let mut worker = Worker::start_with_routes(
                engine,
                directory,
                if endpoints_file.exists() {
                    Some(endpoints_file)
                } else {
                    None
                },
                signal.clone(),
                Some(active),
            )
            .unwrap();
            // No TcpListener or TlsListener is constructed anywhere in A.
            while !signal.is_stopped() {
                signal.wait(Duration::from_millis(100));
            }
            worker.stop();
        }
        "C" => client(base),
        "T" => resilience_client(base),
        _ => panic!("unknown fixture role"),
    }
    signal.stop();
}

#[test]
fn four_process_outbound_host_dht_browser_render_navigation_and_reconnect() {
    let scratch = Scratch::new("four-process");
    let base = &scratch.0;
    for role in ["A", "B", "R"] {
        NodeIdentity::create(base.join(role)).unwrap();
    }
    fs::create_dir_all(base.join("site")).unwrap();
    let host = *id(base, "A").id();
    fs::write(base.join("site/home.ylang"),format!("@page 0.1\n  title NAT homepage\n= Home behind NAT\n:: Links\n  > yami://{host}/about | About\n")).unwrap();
    fs::write(
        base.join("site/about.ylang"),
        "@page 0.1\n  title NAT second page\n= Reached by stable identity\n",
    )
    .unwrap();
    fs::create_dir_all(base.join("site/assets")).unwrap();
    fs::write(
        base.join("site/assets/cover.png"),
        yami_render::assets::encode_png(1, 1, &[20, 180, 70, 255]).unwrap(),
    )
    .unwrap();
    let mut relay = Process::spawn(base, "R");
    wait(|| base.join("R.endpoint").exists());
    let mut peer = Process::spawn(base, "B");
    wait(|| base.join("B.endpoint").exists());
    let mut host = Process::spawn(base, "A");
    wait(|| stored(base).is_some_and(|r| !r.relay_routes().is_empty()));
    let first = stored(base).unwrap();
    assert!(first.endpoints().is_empty());
    let mut client = Process::spawn(base, "C");
    client.success();
    assert!(base.join("C.done").exists());
    assert!(base.join("homepage.png").exists());
    fs::write(base.join("R.stop"), "").unwrap();
    relay.success();
    wait(|| {
        stored(base).is_some_and(|r| r.sequence() > first.sequence() && r.relay_routes().is_empty())
    });
    let withdrawn = stored(base).unwrap();
    // A stale cached advertisement cannot authenticate anything while R is down.
    let resolver = Resolver::new(base.join("failure-client"), Network::Loopback);
    resolver.set_relay_enabled(true);
    let cache = Service::open_lookup(resolver.state_path(), Network::Loopback).unwrap();
    cache.accept(first, yami_dht::now().unwrap()).unwrap();
    drop(cache);
    assert!(resolver
        .fetch(&Address {
            id: *id(base, "A").id(),
            key: "home".into()
        })
        .is_err());
    drop(resolver);
    fs::remove_file(base.join("R.stop")).unwrap();
    relay = Process::spawn(base, "R");
    wait(|| {
        stored(base)
            .is_some_and(|r| r.sequence() > withdrawn.sequence() && !r.relay_routes().is_empty())
    });
    client = Process::spawn(base, "C");
    client.success();
    // Graceful host stop removes the live relay registration; its older signed
    // advertisement remains only a claim until a higher withdrawal or expiry.
    fs::write(base.join("A.stop"), "").unwrap();
    host.success();
    let resolver = Resolver::new(base.join("offline-client"), Network::Loopback);
    resolver.set_relay_enabled(true);
    PeerStore::open(base.join("offline-client"))
        .unwrap()
        .add(*id(base, "B").id(), endpoint(base, "B").into())
        .unwrap();
    assert!(resolver
        .fetch(&Address {
            id: *id(base, "A").id(),
            key: "home".into()
        })
        .is_err());
    fs::write(base.join("R.stop"), "").unwrap();
    relay.success();
    fs::write(base.join("B.stop"), "").unwrap();
    peer.success();
}

#[test]
fn browser_direct_failure_relay_fallback_and_relay_pin_rejection() {
    let scratch = Scratch::new("fallback");
    let host = Arc::new(NodeIdentity::generate().unwrap());
    let relay_key = Arc::new(NodeIdentity::generate().unwrap());
    let relay = Arc::new(
        Relay::bind(
            "127.0.0.1:0".parse().unwrap(),
            relay_key.clone(),
            Limits::default(),
        )
        .unwrap(),
    );
    let SocketAddr::V4(endpoint) = relay.local_addr().unwrap() else {
        unreachable!()
    };
    let signal = Shutdown::default();
    let relay_signal = signal.clone();
    let relay_task = relay.clone();
    let task = thread::spawn(move || relay_task.run(relay_signal.flag()).unwrap());
    // Join every task even if an assertion fails.
    struct Guard(Shutdown, Option<thread::JoinHandle<()>>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.stop();
            self.1.take().unwrap().join().unwrap();
        }
    }
    let _guard = Guard(signal.clone(), Some(task));
    fs::create_dir_all(scratch.0.join("site")).unwrap();
    fs::write(
        scratch.0.join("site/home.ylang"),
        "@page 0.1\n  title Fallback success\n= Destination verified\n",
    )
    .unwrap();
    let service =
        Service::open(scratch.0.join("host.cache"), *host.id(), Network::Loopback).unwrap();
    let _hosts = Hosts::start(
        vec![RelayRoute {
            relay: *relay_key.id(),
            endpoint,
        }],
        host.clone(),
        Arc::new(Site::open(scratch.0.join("site")).unwrap()),
        service,
        signal,
        ActiveRoutes::default(),
    )
    .unwrap();
    wait(|| relay.statistics().registrations == 1);
    let at = yami_dht::now().unwrap();
    let address = Address {
        id: *host.id(),
        key: "home".into(),
    };
    let wrong_route = RelayRoute {
        relay: *NodeIdentity::generate().unwrap().id(),
        endpoint,
    };
    let wrong_record = PeerRecord::sign_routes(
        &host,
        vec![],
        vec![wrong_route],
        at + 60,
        1,
        Network::Loopback,
        at,
    )
    .unwrap();
    fn cached(base: &Path, record: PeerRecord) -> Arc<Resolver> {
        let resolver = Arc::new(Resolver::new(base.into(), Network::Loopback));
        resolver.set_relay_enabled(true);
        let cache = Service::open_lookup(resolver.state_path(), Network::Loopback).unwrap();
        cache.accept(record, yami_dht::now().unwrap()).unwrap();
        drop(cache);
        resolver
    }
    let wrong = cached(&scratch.0.join("wrong-relay"), wrong_record);
    let loader = Loader::with_resolver(Arc::new(|| {}), wrong).unwrap();
    let mut browser = Browser::new();
    browser.navigate(&address.url(), &loader).unwrap();
    page_ready(&mut browser, &loader);
    assert!(
        matches!(&browser.active().state,PageState::Error(e) if e.contains("identity mismatch"))
    );
    assert_eq!(browser.active().security(), None);
    // A failed direct connection must continue to the independently pinned relay.
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let SocketAddr::V4(dead_address) = dead.local_addr().unwrap() else {
        unreachable!()
    };
    drop(dead);
    let record = PeerRecord::sign_routes(
        &host,
        vec![dead_address],
        vec![RelayRoute {
            relay: *relay_key.id(),
            endpoint,
        }],
        at + 60,
        2,
        Network::Loopback,
        at,
    )
    .unwrap();
    let resolver = cached(&scratch.0.join("fallback-client"), record);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut browser = Browser::new();
    browser.navigate(&address.url(), &loader).unwrap();
    page_ready(&mut browser, &loader);
    assert_eq!(browser.active().title(), "Fallback success");
    assert_eq!(
        browser.active().security(),
        Some(&Security::Relayed {
            destination: *host.id(),
            relay: *relay_key.id()
        })
    );
}

#[test]
fn relay_outer_handshake_obeys_the_whole_attempt_deadline() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(600));
    });
    let route = yami_relay::Route {
        relay: *NodeIdentity::generate().unwrap().id(),
        destination: *NodeIdentity::generate().unwrap().id(),
        endpoint: address,
    };
    let start = Instant::now();
    assert!(route
        .connect_with_timeout(false, Duration::from_millis(150))
        .is_err());
    assert!(start.elapsed() < Duration::from_millis(500));
    task.join().unwrap();
}

fn stored_at(base: &Path, name: &str) -> Option<PeerRecord> {
    Service::read_snapshot(&base.join(format!("{name}.cache")), Network::Loopback)
        .ok()?
        .get(*id(base, "A").id(), yami_dht::now().unwrap())
}
fn resilience_client(base: &Path) {
    let directory = base.join(format!("T-{}", std::process::id()));
    let profile=format!("yaminet-config 1\nrole browser\nnetwork loopback\nstate {}\nrelay-fallback yes\nbootstrap {}@{}\nbootstrap {}@{}\n",directory.display(),id(base,"B").id(),endpoint(base,"B"),id(base,"D").id(),endpoint(base,"D"));
    let resolver =
        Arc::new(Resolver::from_config(&yami_config::Config::parse(&profile).unwrap()).unwrap());
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut browser = Browser::new();
    let direct = Address {
        id: *id(base, "N").id(),
        key: "home".into(),
    };
    let dir = base.join("N-client");
    // N publishes its signed direct record to the independent DHT peers.
    let mut node_records = Service::read_snapshot(&base.join("N.cache"), Network::Loopback)
        .unwrap()
        .records()
        .to_vec();
    let cache = Service::open_lookup(dir.join("direct.cache"), Network::Loopback).unwrap();
    let record = node_records.remove(0);
    let seeds = [
        yami_dht::Contact::new(*id(base, "B").id(), endpoint(base, "B"), Network::Loopback)
            .unwrap(),
        yami_dht::Contact::new(*id(base, "D").id(), endpoint(base, "D"), Network::Loopback)
            .unwrap(),
    ];
    assert!(yami_dht::publish(&cache, &record, &seeds).unwrap() >= 1);
    drop(cache);
    browser.navigate(&direct.url(), &loader).unwrap();
    page_ready(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(direct.id))
    );
    let host = *id(base, "A").id();
    let address = format!("yami://{host}/home");
    browser.navigate(&address, &loader).unwrap();
    page_ready(&mut browser, &loader);
    let Some(Security::Relayed { destination, relay }) = browser.active().security() else {
        panic!("{:?}", browser.active().state)
    };
    assert_eq!(*destination, host);
    if let Ok(failed) = fs::read_to_string(base.join("failed-relay.id")) {
        assert_ne!(relay.to_string(), failed.trim());
    }
    let selected = relay.to_string();
    // Two real loader workers compete for a single slot at each relay. Queued
    // tabs share owner discovery; all document sessions independently authenticate.
    for _ in 0..5 {
        browser.open_tab();
        browser.navigate(&address, &loader).unwrap();
    }
    wait(|| {
        browser.poll(&loader);
        browser
            .tabs
            .iter()
            .all(|tab| !matches!(tab.state, PageState::Loading))
    });
    for tab in &browser.tabs {
        assert!(
            matches!(tab.security(),Some(Security::Relayed{destination,..}) if *destination==host),
            "{:?}",
            tab.state
        );
        assert_eq!(tab.address, address);
    }
    fs::write(base.join("T.done"), selected).unwrap();
}

#[test]
fn two_relays_two_dht_peers_concurrent_tabs_endpoint_updates_and_restart() {
    let scratch = Scratch::new("resilience");
    let base = &scratch.0;
    for role in ["A", "B", "D", "N", "R", "S"] {
        NodeIdentity::create(base.join(role)).unwrap();
    }
    fs::create_dir_all(base.join("site")).unwrap();
    fs::write(
        base.join("site/home.ylang"),
        "@page 0.1\n  title Resilient site\n= A site through independent routes\n",
    )
    .unwrap();
    fn dead_endpoint() -> SocketAddrV4 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        endpoint
    }
    let old_direct = dead_endpoint();
    fs::write(base.join("A.endpoints"), old_direct.to_string()).unwrap();
    let mut r = Process::spawn(base, "R");
    wait(|| base.join("R.endpoint").exists());
    let mut s = Process::spawn(base, "S");
    wait(|| base.join("S.endpoint").exists());
    let mut b = Process::spawn(base, "B");
    wait(|| base.join("B.endpoint").exists());
    let mut d = Process::spawn(base, "D");
    wait(|| base.join("D.endpoint").exists());
    let mut n = Process::spawn(base, "N");
    wait(|| base.join("N.endpoint").exists());
    let mut a = Process::spawn(base, "A");
    wait(|| stored_at(base, "D").is_some_and(|r| r.relay_routes().len() == 2));
    let first = stored_at(base, "D").unwrap();
    assert_eq!(first.endpoints(), &[old_direct]);
    let mut client = Process::spawn(base, "T");
    client.success();
    let selected = fs::read_to_string(base.join("T.done")).unwrap();
    let preferred = if selected == id(base, "R").id().to_string() {
        "R"
    } else {
        "S"
    };
    fs::write(base.join("failed-relay.id"), &selected).unwrap();
    fs::write(base.join(format!("{preferred}.stop")), "").unwrap();
    if preferred == "R" {
        r.success();
    } else {
        s.success();
    }
    wait(|| stored_at(base, "D").is_some_and(|r| r.relay_routes().len() == 1));
    client = Process::spawn(base, "T");
    client.success();
    assert_ne!(fs::read_to_string(base.join("T.done")).unwrap(), selected);
    fs::write(base.join("B.stop"), "").unwrap();
    b.success();
    let updated_endpoint = dead_endpoint();
    assert_ne!(old_direct, updated_endpoint);
    fs::write(base.join("A.endpoints"), updated_endpoint.to_string()).unwrap();
    wait(|| stored_at(base, "D").is_some_and(|r| r.endpoints() == [updated_endpoint]));
    let updated = stored_at(base, "D").unwrap();
    assert!(updated.sequence() > first.sequence());
    client = Process::spawn(base, "T");
    client.success(); // Fresh cache resolves through D while B is offline.
    fs::remove_file(base.join("B.stop")).unwrap();
    b = Process::spawn(base, "B");
    wait(|| stored_at(base, "B").is_some_and(|r| r.sequence() >= updated.sequence())); // Health repair, not 12-hour renewal.
    fs::remove_file(base.join(format!("{preferred}.stop"))).unwrap();
    if preferred == "R" {
        r = Process::spawn(base, "R");
    } else {
        s = Process::spawn(base, "S");
    }
    fs::remove_file(base.join("failed-relay.id")).unwrap();
    wait(|| stored_at(base, "D").is_some_and(|r| r.relay_routes().len() == 2));
    let before_restart = stored_at(base, "D").unwrap().sequence();
    fs::write(base.join("A.stop"), "").unwrap();
    a.success();
    fs::remove_file(base.join("A.stop")).unwrap();
    a = Process::spawn(base, "A");
    wait(|| {
        stored_at(base, "D")
            .is_some_and(|r| r.sequence() > before_restart && r.relay_routes().len() == 2)
    });
    client = Process::spawn(base, "T");
    client.success();
    let publisher = Service::open_lookup(base.join("rollback.cache"), Network::Loopback).unwrap();
    let seed = yami_dht::Contact::new(*id(base, "D").id(), endpoint(base, "D"), Network::Loopback)
        .unwrap();
    assert_eq!(
        yami_dht::publish(&publisher, &first, &[seed]).unwrap(),
        0,
        "old valid signatures must not roll records back"
    );
    for (role, process) in [
        ("A", &mut a),
        ("B", &mut b),
        ("D", &mut d),
        ("N", &mut n),
        ("R", &mut r),
        ("S", &mut s),
    ] {
        fs::write(base.join(format!("{role}.stop")), "").unwrap();
        process.success();
    }
}
