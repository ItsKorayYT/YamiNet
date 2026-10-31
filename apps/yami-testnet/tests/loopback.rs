//! Real loopback sockets, real pinned TLS, real signed DHT records and a real
//! relay. This is still a single-machine simulation; the report says so.
use std::{
    fs, io,
    net::{SocketAddrV4, TcpListener},
    path::{Path, PathBuf},
    process,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use yami_config::Config;
use yami_dht::{Contact, Network, PeerRecord, RelayRoute, Service};
use yami_identity::NodeIdentity;
use yami_node::{serve_connection, serve_connection_with_dht, shutdown::Shutdown, Site};
use yami_relay::{Limits, Relay, Route};
use yami_testnet::{
    config::Loaded,
    preflight::{self, Options, RouteMode, Target},
};
use ynp_tls::TlsListener;

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("yami-testnet-loopback-{name}-{}", process::id()));
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

struct NodeThread {
    shutdown: Shutdown,
    handle: Option<thread::JoinHandle<io::Result<()>>>,
}
impl NodeThread {
    fn serve(listener: ynp_tls::TlsListener, site: Site, dht: Option<Service>) -> Self {
        listener.set_nonblocking(true).unwrap();
        let shutdown = Shutdown::default();
        let signal = shutdown.clone();
        let handle = thread::spawn(move || {
            yami_node::serve_with_handler_until(
                &listener,
                2,
                &|connection| match &dht {
                    Some(service) => serve_connection_with_dht(connection, &site, service),
                    None => serve_connection(connection, &site),
                },
                &|_| {},
                &signal,
            )
        });
        Self {
            shutdown,
            handle: Some(handle),
        }
    }
}
impl Drop for NodeThread {
    fn drop(&mut self) {
        self.shutdown.stop();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct AtomicThread {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Drop for AtomicThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn site(root: &Path) -> Site {
    yami_node::authoring::new_site(root, "personal").unwrap();
    Site::open(root).unwrap()
}

fn endpoint(listener: &TlsListener) -> SocketAddrV4 {
    match listener.local_addr().unwrap() {
        std::net::SocketAddr::V4(endpoint) => endpoint,
        _ => unreachable!(),
    }
}

fn contact(id: &NodeIdentity, endpoint: SocketAddrV4) -> Contact {
    Contact::new(*id.id(), endpoint, Network::Loopback).unwrap()
}

fn publish(service: &Service, record: &PeerRecord, seed: Contact) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if yami_dht::publish(service, record, &[seed]).unwrap_or(0) > 0 {
            return;
        }
        assert!(Instant::now() < deadline, "DHT publication timed out");
        thread::sleep(Duration::from_millis(50));
    }
}

fn client_config(path: &Path, state: &Path, bootstrap: Contact, relays: bool) -> Loaded {
    fs::write(
        path,
        format!(
            "yaminet-config 1\nrole browser\nnetwork loopback\nstate {}\nbootstrap {}@{}\nrelay-fallback {}\n",
            state.display(),
            bootstrap.id,
            bootstrap.endpoint,
            if relays { "yes" } else { "no" }
        ),
    )
    .unwrap();
    Loaded::load(path, None).unwrap()
}

fn host_config(
    path: &Path,
    identity_dir: &Path,
    state: &Path,
    site: &Path,
    bootstrap: Contact,
    relay: Option<RelayRoute>,
) -> Loaded {
    let relay_line = relay
        .map(|route| format!("relay {}@{}\n", route.relay, route.endpoint))
        .unwrap_or_default();
    fs::write(
        path,
        format!(
            "yaminet-config 1\nrole node\nnetwork loopback\nidentity {}\nstate {}\nsite {}\n{}bootstrap {}@{}\nrenew-secs 60\nhealth-secs 30\n",
            identity_dir.display(),
            state.display(),
            site.display(),
            relay_line,
            bootstrap.id,
            bootstrap.endpoint,
        ),
    )
    .unwrap();
    Loaded::load(path, None).unwrap()
}

fn bootstrap_node(
    base: &Path,
    name: &str,
) -> (Arc<NodeIdentity>, NodeThread, SocketAddrV4, Service, Site) {
    let identity = Arc::new(NodeIdentity::create(base.join(format!("{name}-keys"))).unwrap());
    let listener =
        ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&identity)).unwrap();
    let endpoint = endpoint(&listener);
    let site = site(&base.join(format!("{name}-site")));
    let dht = Service::open(
        base.join(format!("{name}.cache")),
        *identity.id(),
        Network::Loopback,
    )
    .unwrap();
    dht.prepare_self(&identity, vec![endpoint], yami_dht::now().unwrap())
        .unwrap();
    let thread = NodeThread::serve(listener, site.clone(), Some(dht.clone()));
    (identity, thread, endpoint, dht, site)
}

fn target(identity: &NodeIdentity, asset: bool) -> Target {
    Target::parse(
        &format!("yami://{}/home", identity.id()),
        asset.then_some("cover"),
    )
    .unwrap()
}

fn options(
    primary: Loaded,
    client: Option<Loaded>,
    host: Option<Loaded>,
    target: Target,
    route: RouteMode,
) -> Options {
    Options {
        primary,
        client,
        host,
        target,
        extra_bootstraps: Vec::new(),
        route,
        timeout: Duration::from_secs(10),
        check_listeners: false,
        include_endpoints: true,
    }
}

fn status(report: &yami_testnet::report::Report, id: &str) -> &'static str {
    report
        .checks()
        .iter()
        .find(|check| check.id == id)
        .map(|check| check.status.as_str())
        .unwrap_or("MISSING")
}

#[test]
fn direct_signed_lookup_document_and_png_pass_on_loopback() {
    let scratch = Scratch::new("direct");
    let (_bootstrap, _bootstrap_thread, bootstrap_endpoint, _bootstrap_dht, _bootstrap_site) =
        bootstrap_node(&scratch.0, "bootstrap");
    let bootstrap_identity = NodeIdentity::load(scratch.0.join("bootstrap-keys")).unwrap();
    let bootstrap = contact(&bootstrap_identity, bootstrap_endpoint);

    let host_identity = Arc::new(NodeIdentity::create(scratch.0.join("host-keys")).unwrap());
    let host_site = site(&scratch.0.join("host-site"));
    let host_listener =
        ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&host_identity))
            .unwrap();
    let host_endpoint = endpoint(&host_listener);
    let _host_thread = NodeThread::serve(host_listener, host_site, None);
    let host_dht = Service::open(
        scratch.0.join("host.cache"),
        *host_identity.id(),
        Network::Loopback,
    )
    .unwrap();
    let record = host_dht
        .prepare_self(
            &host_identity,
            vec![host_endpoint],
            yami_dht::now().unwrap(),
        )
        .unwrap();
    let publisher =
        Service::open_lookup(scratch.0.join("publish.cache"), Network::Loopback).unwrap();
    publish(&publisher, &record, bootstrap);

    let client = client_config(
        &scratch.0.join("client.conf"),
        &scratch.0.join("client-state"),
        bootstrap,
        false,
    );
    let report = preflight::run(&options(
        client,
        None,
        None,
        target(&host_identity, true),
        RouteMode::Auto,
    ));
    assert_eq!(report.outcome().as_str(), "PASS", "{}", report.to_text());
    assert_eq!(status(&report, "bootstrap-reachability"), "PASS");
    assert_eq!(status(&report, "signed-dht-lookup"), "PASS");
    assert_eq!(status(&report, "destination-tls"), "PASS");
    assert_eq!(status(&report, "document-retrieval"), "PASS");
    assert_eq!(status(&report, "png-retrieval"), "PASS");
    assert_eq!(status(&report, "route-selection"), "PASS");
    assert_eq!(
        report.to_json()["scope"]["independent_machines"],
        "NOT VERIFIED"
    );
}

#[test]
fn direct_failure_then_enabled_relay_fallback_passes() {
    let scratch = Scratch::new("relay");
    let (_bootstrap, _bootstrap_thread, bootstrap_endpoint, _bootstrap_dht, _bootstrap_site) =
        bootstrap_node(&scratch.0, "relay-bootstrap");
    let bootstrap_identity = NodeIdentity::load(scratch.0.join("relay-bootstrap-keys")).unwrap();
    let bootstrap = contact(&bootstrap_identity, bootstrap_endpoint);

    let relay_identity = Arc::new(NodeIdentity::create(scratch.0.join("relay-keys")).unwrap());
    let relay = Arc::new(
        Relay::bind(
            "127.0.0.1:0".parse().unwrap(),
            Arc::clone(&relay_identity),
            Limits::default(),
        )
        .unwrap(),
    );
    let relay_endpoint = match relay.local_addr().unwrap() {
        std::net::SocketAddr::V4(endpoint) => endpoint,
        _ => unreachable!(),
    };
    let relay_stop = Arc::new(AtomicBool::new(false));
    let relay_thread = {
        let relay = Arc::clone(&relay);
        let stop = Arc::clone(&relay_stop);
        thread::spawn(move || relay.run(&stop).unwrap())
    };
    let _relay_guard = AtomicThread {
        stop: Arc::clone(&relay_stop),
        handle: Some(relay_thread),
    };

    let host_identity = Arc::new(NodeIdentity::create(scratch.0.join("relay-host-keys")).unwrap());
    let host_site = site(&scratch.0.join("relay-host-site"));
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_endpoint = match dead.local_addr().unwrap() {
        std::net::SocketAddr::V4(endpoint) => endpoint,
        _ => unreachable!(),
    };
    drop(dead);
    let host_dht = Service::open(
        scratch.0.join("relay-host.cache"),
        *host_identity.id(),
        Network::Loopback,
    )
    .unwrap();
    let record = host_dht
        .prepare_self_routes(
            &host_identity,
            vec![dead_endpoint],
            vec![RelayRoute {
                relay: *relay_identity.id(),
                endpoint: relay_endpoint,
            }],
            yami_dht::now().unwrap(),
        )
        .unwrap();
    let publisher =
        Service::open_lookup(scratch.0.join("relay-publish.cache"), Network::Loopback).unwrap();
    publish(&publisher, &record, bootstrap);

    let host_stop = Arc::new(AtomicBool::new(false));
    let host_thread = {
        let route = Route {
            relay: *relay_identity.id(),
            endpoint: relay_endpoint.into(),
            destination: *host_identity.id(),
        };
        let identity = Arc::clone(&host_identity);
        let stop = Arc::clone(&host_stop);
        let site = host_site.clone();
        thread::spawn(move || {
            yami_relay::run_host(
                route,
                identity,
                &stop,
                &|connection| serve_connection(connection, &site).map(|_| ()),
                &|_| {},
            )
            .unwrap()
        })
    };
    let _host_guard = AtomicThread {
        stop: Arc::clone(&host_stop),
        handle: Some(host_thread),
    };
    thread::sleep(Duration::from_millis(400));

    let host = host_config(
        &scratch.0.join("relay-host.conf"),
        &scratch.0.join("relay-host-keys"),
        &scratch.0.join("relay-host-state"),
        &scratch.0.join("relay-host-site"),
        bootstrap,
        Some(RelayRoute {
            relay: *relay_identity.id(),
            endpoint: relay_endpoint,
        }),
    );
    let client = client_config(
        &scratch.0.join("relay-client.conf"),
        &scratch.0.join("relay-client-state"),
        bootstrap,
        true,
    );
    let report = preflight::run(&options(
        host,
        Some(client),
        None,
        target(&host_identity, true),
        RouteMode::Auto,
    ));
    assert_eq!(report.outcome().as_str(), "PASS", "{}", report.to_text());
    assert_eq!(status(&report, "relay-fallback"), "PASS");
    assert_eq!(status(&report, "relay-identity"), "PASS");
    assert_eq!(status(&report, "destination-tls"), "PASS");
    assert_eq!(status(&report, "document-retrieval"), "PASS");
    assert_eq!(status(&report, "png-retrieval"), "PASS");
    assert_eq!(status(&report, "local-host-profile"), "PASS");
    let json = report.to_json();
    let attempts = json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == "destination-tls")
        .unwrap()["data"]["attempts"]
        .as_array()
        .unwrap();
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt["path"] == "direct" && attempt["result"] != "authenticated"),
        "the direct candidate should be attempted and fail: {attempts:?}"
    );
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt["path"] == "relay" && attempt["result"] == "authenticated"),
        "the relay path should authenticate: {attempts:?}"
    );

    // A host profile with explicit relay pins must also be testable without a
    // separate browser profile. The relay may be between registrations, so
    // retry the bounded run rather than pretending a busy slot is a failure.
    let host_config_path = scratch.0.join("relay-host.conf");
    let mut host_only_passed = false;
    for _ in 0..10 {
        let host_only = Loaded::load(&host_config_path, None).unwrap();
        let report = preflight::run(&options(
            host_only,
            None,
            None,
            target(&host_identity, true),
            RouteMode::Auto,
        ));
        if report.outcome().as_str() == "PASS" {
            assert_eq!(status(&report, "relay-fallback"), "PASS");
            host_only_passed = true;
            break;
        }
        thread::sleep(Duration::from_millis(500));
    }
    assert!(
        host_only_passed,
        "host profile relay preflight did not recover"
    );
}

#[test]
fn relay_only_record_without_explicit_fallback_fails_closed() {
    let scratch = Scratch::new("no-fallback");
    let (_bootstrap, _bootstrap_thread, bootstrap_endpoint, _bootstrap_dht, _bootstrap_site) =
        bootstrap_node(&scratch.0, "nf-bootstrap");
    let bootstrap_identity = NodeIdentity::load(scratch.0.join("nf-bootstrap-keys")).unwrap();
    let bootstrap = contact(&bootstrap_identity, bootstrap_endpoint);

    let host_identity = NodeIdentity::create(scratch.0.join("nf-host-keys")).unwrap();
    let host_site = site(&scratch.0.join("nf-host-site"));
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_endpoint = match dead.local_addr().unwrap() {
        std::net::SocketAddr::V4(endpoint) => endpoint,
        _ => unreachable!(),
    };
    drop(dead);
    let relay_identity = NodeIdentity::create(scratch.0.join("nf-relay-keys")).unwrap();
    let relay_endpoint: SocketAddrV4 = "127.0.0.1:1".parse().unwrap();
    let host_dht = Service::open(
        scratch.0.join("nf-host.cache"),
        *host_identity.id(),
        Network::Loopback,
    )
    .unwrap();
    let record = host_dht
        .prepare_self_routes(
            &host_identity,
            vec![dead_endpoint],
            vec![RelayRoute {
                relay: *relay_identity.id(),
                endpoint: relay_endpoint,
            }],
            yami_dht::now().unwrap(),
        )
        .unwrap();
    let publisher =
        Service::open_lookup(scratch.0.join("nf-publish.cache"), Network::Loopback).unwrap();
    publish(&publisher, &record, bootstrap);

    let host = host_config(
        &scratch.0.join("nf-host.conf"),
        &scratch.0.join("nf-host-keys"),
        &scratch.0.join("nf-host-state"),
        &scratch.0.join("nf-host-site"),
        bootstrap,
        Some(RelayRoute {
            relay: *relay_identity.id(),
            endpoint: relay_endpoint,
        }),
    );
    let client = client_config(
        &scratch.0.join("nf-client.conf"),
        &scratch.0.join("nf-client-state"),
        bootstrap,
        false,
    );
    let report = preflight::run(&options(
        host,
        Some(client),
        None,
        target(&host_identity, false),
        RouteMode::Auto,
    ));
    assert_eq!(report.outcome().as_str(), "FAIL", "{}", report.to_text());
    assert_eq!(status(&report, "destination-tls"), "FAIL");
    assert_eq!(status(&report, "document-retrieval"), "FAIL");
    assert!(!report.to_text().contains("plaintext: enabled"));
    let _ = host_site;
}

#[test]
fn wrong_destination_identity_is_refused_without_a_document() {
    let scratch = Scratch::new("wrong-id");
    let (_bootstrap, _bootstrap_thread, bootstrap_endpoint, _bootstrap_dht, _bootstrap_site) =
        bootstrap_node(&scratch.0, "wrong-bootstrap");
    let bootstrap_identity = NodeIdentity::load(scratch.0.join("wrong-bootstrap-keys")).unwrap();
    let bootstrap = contact(&bootstrap_identity, bootstrap_endpoint);

    let claimed = NodeIdentity::create(scratch.0.join("claimed-keys")).unwrap();
    let impostor = Arc::new(NodeIdentity::create(scratch.0.join("impostor-keys")).unwrap());
    let impostor_listener =
        ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), Arc::clone(&impostor)).unwrap();
    let impostor_endpoint = endpoint(&impostor_listener);
    let impostor_site = site(&scratch.0.join("impostor-site"));
    let _impostor_thread = NodeThread::serve(impostor_listener, impostor_site, None);
    let at = yami_dht::now().unwrap();
    let claim = PeerRecord::sign(
        &claimed,
        vec![impostor_endpoint],
        at + 60,
        1,
        Network::Loopback,
        at,
    )
    .unwrap();
    let publisher =
        Service::open_lookup(scratch.0.join("wrong-publish.cache"), Network::Loopback).unwrap();
    publish(&publisher, &claim, bootstrap);

    let client = client_config(
        &scratch.0.join("wrong-client.conf"),
        &scratch.0.join("wrong-client-state"),
        bootstrap,
        false,
    );
    let report = preflight::run(&options(
        client,
        None,
        None,
        target(&claimed, false),
        RouteMode::Direct,
    ));
    assert_eq!(report.outcome().as_str(), "FAIL", "{}", report.to_text());
    assert_eq!(status(&report, "destination-tls"), "FAIL");
    assert_eq!(status(&report, "document-retrieval"), "FAIL");
    assert!(report.to_text().contains("identity mismatch"));
    assert!(!report.to_text().contains("PASS - "));
}

#[test]
fn unreachable_bootstrap_is_reported_without_claiming_success() {
    let scratch = Scratch::new("unreachable");
    let reserve = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = match reserve.local_addr().unwrap() {
        std::net::SocketAddr::V4(endpoint) => endpoint,
        _ => unreachable!(),
    };
    drop(reserve);
    let identity = NodeIdentity::create(scratch.0.join("dead-keys")).unwrap();
    let contact = contact(&identity, endpoint);
    let client = client_config(
        &scratch.0.join("dead-client.conf"),
        &scratch.0.join("dead-state"),
        contact,
        false,
    );
    let target = target(&identity, false);
    let mut options = options(client, None, None, target, RouteMode::Auto);
    options.timeout = Duration::from_secs(5);
    let report = preflight::run(&options);
    assert_eq!(report.outcome().as_str(), "FAIL");
    assert_eq!(status(&report, "bootstrap-reachability"), "FAIL");
    assert_eq!(status(&report, "signed-dht-lookup"), "FAIL");
    assert_eq!(status(&report, "destination-tls"), "NOT RUN");
}

// A tiny configuration parse guard: the generated client config above must be
// readable by the same parser used in production.
#[test]
fn generated_client_config_is_a_valid_browser_profile() {
    let scratch = Scratch::new("config");
    let identity = NodeIdentity::create(scratch.0.join("keys")).unwrap();
    let contact = contact(&identity, "127.0.0.1:17444".parse().unwrap());
    let client = client_config(
        &scratch.0.join("client.conf"),
        &scratch.0.join("state"),
        contact,
        true,
    );
    assert_eq!(client.config.role, yami_config::Role::Browser);
    let parsed = Config::load(scratch.0.join("client.conf")).unwrap();
    assert!(parsed.relay_fallback);
}
