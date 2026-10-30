//! Deterministic DHT record time with real loopback TLS, RPCs and YNP. These
//! tests model days of operation; TLS certificates/deadlines retain real time.
use std::{
    collections::HashSet,
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use yami_dht::{Clock, Contact, Network, RpcConnector, Service};
use yami_identity::{NodeId, NodeIdentity};
use yami_node::{
    maintenance::{
        self, Config, Failure, Maintainer, Worker, MAX_BACKOFF, MAX_MAINTENANCE_QUERIES,
    },
    shutdown::Shutdown,
    Site,
};
use ynp_tls::{TlsListener, TlsTransport};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("yami-maintenance-{name}-{}", std::process::id()));
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
struct TestClock(AtomicU64);
impl TestClock {
    fn new() -> Self {
        Self(AtomicU64::new(yami_dht::now().unwrap()))
    }
    fn set(&self, at: u64) {
        self.0.store(at, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn now(&self) -> io::Result<u64> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}
fn service(path: &Path, id: NodeId, clock: &Arc<TestClock>) -> Service {
    Service::open_with_clock(path, id, Network::Loopback, clock.clone()).unwrap()
}
fn site(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::write(
        path.join("home.ylang"),
        "@page 0.1\n  title Maintained site\n= Still here\n",
    )
    .unwrap();
}
struct Node {
    contact: Contact,
    service: Service,
    online: Arc<AtomicBool>,
    shutdown: Shutdown,
    task: Option<thread::JoinHandle<()>>,
}
impl Node {
    fn start(identity: Arc<NodeIdentity>, service: Service, site: &Path, ip: &str) -> Self {
        let listener =
            TlsListener::bind(format!("{ip}:0").parse().unwrap(), identity.clone()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let contact = Contact::new(*identity.id(), endpoint, Network::Loopback).unwrap();
        let shutdown = Shutdown::default();
        let signal = shutdown.clone();
        let handler = service.clone();
        let site = Site::open(site).unwrap();
        let online = Arc::new(AtomicBool::new(true));
        let connected = online.clone();
        let task = thread::spawn(move || {
            yami_node::serve_with_handler_until(
                &listener,
                2,
                &|connection| {
                    if !connected.load(Ordering::SeqCst) {
                        return Err(io::Error::other("simulated offline"));
                    }
                    yami_node::serve_connection_with_dht(connection, &site, &handler)
                },
                &|_| {},
                &signal,
            )
            .unwrap()
        });
        Self {
            contact,
            service,
            online,
            shutdown,
            task: Some(task),
        }
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        self.shutdown.stop();
        self.task.take().unwrap().join().unwrap();
    }
}
fn engine(node: &Node, identity: &Arc<NodeIdentity>, config: Config) -> Maintainer {
    Maintainer::new(
        node.service.clone(),
        identity.clone(),
        vec![node.contact.endpoint],
        config,
    )
    .unwrap()
}
fn fetch_from_b(
    root: &Path,
    name: &str,
    c: NodeId,
    a: NodeId,
    b: Contact,
    clock: &Arc<TestClock>,
    expected: u64,
) {
    let cold = service(&root.join(format!("cold-{name}.cache")), c, clock);
    let found = yami_dht::lookup(&cold, a, &[b]).unwrap().record.unwrap();
    assert_eq!(found.sequence(), expected);
    found
        .verify(Network::Loopback, clock.now().unwrap())
        .unwrap();
    let document = ynp::fetch(
        &TlsTransport::new(a).unwrap(),
        found.endpoints()[0].into(),
        "home",
    )
    .unwrap();
    assert!(document.source.contains("Still here"));
}

#[test]
fn three_nodes_renew_across_days_and_recover_after_bootstrap_outage_and_ip_change() {
    let scratch = Scratch::new("three");
    let clock = Arc::new(TestClock::new());
    let site_path = scratch.0.join("site");
    site(&site_path);
    let ids: Vec<_> = (0..3)
        .map(|i| Arc::new(NodeIdentity::create(scratch.0.join(format!("id-{i}"))).unwrap()))
        .collect();
    let a_state = service(&scratch.0.join("a.cache"), *ids[0].id(), &clock);
    let a = Node::start(ids[0].clone(), a_state.clone(), &site_path, "127.0.0.2");
    let b = Node::start(
        ids[1].clone(),
        service(&scratch.0.join("b.cache"), *ids[1].id(), &clock),
        &site_path,
        "127.0.0.1",
    );
    let c = Node::start(
        ids[2].clone(),
        service(&scratch.0.join("c.cache"), *ids[2].id(), &clock),
        &site_path,
        "127.0.0.1",
    );
    let mut ae = engine(&a, &ids[0], Config::default());
    let mut be = engine(&b, &ids[1], Config::default());
    let mut ce = engine(&c, &ids[2], Config::default());
    let stop = Shutdown::default();
    let original = ae.record().clone();
    assert_eq!(
        ae.tick(&[b.contact], &stop, 0, &yami_dht::PinnedTls)
            .unwrap()
            .published,
        1
    );
    assert_eq!(
        ce.tick(&[b.contact], &stop, 0, &yami_dht::PinnedTls)
            .unwrap()
            .published,
        1
    );
    fetch_from_b(
        &scratch.0,
        "initial",
        c.contact.id,
        a.contact.id,
        b.contact,
        &clock,
        original.sequence(),
    );
    let start = clock.now().unwrap();
    for period in 0..6 {
        let due = ae.next_publication();
        assert!(due < ae.record().expires(), "renew before expiry");
        clock.set(due);
        if period == 2 {
            b.online.store(false, Ordering::SeqCst);
        }
        if period == 4 {
            b.online.store(true, Ordering::SeqCst);
        }
        if b.online.load(Ordering::SeqCst) {
            be.tick(&[], &stop, period, &yami_dht::PinnedTls).unwrap();
        }
        ce.tick(&[b.contact], &stop, period, &yami_dht::PinnedTls)
            .unwrap();
        let previous = ae.record().sequence();
        let pass = ae
            .tick(&[b.contact], &stop, period, &yami_dht::PinnedTls)
            .unwrap();
        assert!(pass.queries <= MAX_MAINTENANCE_QUERIES);
        assert!(ae.record().sequence() > previous);
        assert!(
            pass.published > 0,
            "learned C keeps A discoverable without B"
        );
        let persisted =
            Service::read_snapshot(&scratch.0.join("a.cache"), Network::Loopback).unwrap();
        assert_eq!(persisted.get(a.contact.id, due).unwrap(), *ae.record());
        if b.online.load(Ordering::SeqCst) {
            fetch_from_b(
                &scratch.0,
                &format!("period-{period}"),
                c.contact.id,
                a.contact.id,
                b.contact,
                &clock,
                ae.record().sequence(),
            );
        } else {
            assert!(a
                .service
                .snapshot()
                .unwrap()
                .routing
                .contacts()
                .iter()
                .all(|p| p.id != b.contact.id));
        }
    }
    assert!(
        clock.now().unwrap() > start + yami_dht::MAX_RECORD_TTL,
        "simulate beyond original record lifetime"
    );
    assert!(
        b.service.accept(original, clock.now().unwrap()).is_err(),
        "expired records remain rejected"
    );
    assert!(ae.status(true).unwrap().recently_verified > 0);
    assert!(ae.status(true).unwrap().last_publication.is_some());

    // The operator changes A's actual listening IP and supplies that endpoint.
    // The same running scheduler/signing identity publishes a newer record.
    let old_contact = a.contact;
    drop(a);
    let a = Node::start(ids[0].clone(), a_state, &site_path, "127.0.0.3");
    ae.set_endpoints(vec![a.contact.endpoint]).unwrap();
    clock.set(clock.now().unwrap() + 5);
    assert!(
        ae.tick(&[b.contact], &stop, 99, &yami_dht::PinnedTls)
            .unwrap()
            .published
            > 0
    );
    assert_ne!(old_contact.endpoint.ip(), a.contact.endpoint.ip());
    fetch_from_b(
        &scratch.0,
        "moved",
        c.contact.id,
        a.contact.id,
        b.contact,
        &clock,
        ae.record().sequence(),
    );
    let current = ae.record().clone();
    drop(ae);
    drop(a);
    let state = service(&scratch.0.join("a.cache"), *ids[0].id(), &clock);
    let restarted = Maintainer::new(
        state,
        ids[0].clone(),
        vec![old_contact.endpoint],
        Config::default(),
    )
    .unwrap();
    assert!(
        restarted.record().sequence() > current.sequence(),
        "restart keeps signed sequence continuity"
    );
}

struct Refused {
    calls: AtomicUsize,
    contacts: Mutex<HashSet<Contact>>,
    stop: Option<Shutdown>,
}
impl RpcConnector for Refused {
    fn connect(&self, contact: Contact, budget: Duration) -> io::Result<Box<dyn ynp::Connection>> {
        assert!(budget > Duration::ZERO && budget <= yami_dht::RPC_TIMEOUT);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.contacts.lock().unwrap().insert(contact);
        if let Some(stop) = &self.stop {
            stop.stop();
        }
        Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "simulated outage",
        ))
    }
}
#[test]
fn outage_retries_are_bounded_back_off_rotate_and_obey_shutdown() {
    let scratch = Scratch::new("bounded");
    let clock = Arc::new(TestClock::new());
    let id = Arc::new(NodeIdentity::generate().unwrap());
    let state = service(&scratch.0.join("node.cache"), *id.id(), &clock);
    let mut engine = Maintainer::new(
        state.clone(),
        id.clone(),
        vec!["127.0.0.1:1".parse().unwrap()],
        Config {
            renew_secs: 30,
            health_secs: 5,
        },
    )
    .unwrap();
    let seeds: Vec<_> = (0..40)
        .map(|_| {
            Contact::new(
                *NodeIdentity::generate().unwrap().id(),
                "127.0.0.1:1".parse().unwrap(),
                Network::Loopback,
            )
            .unwrap()
        })
        .collect();
    let refused = Refused {
        calls: AtomicUsize::new(0),
        contacts: Mutex::new(HashSet::new()),
        stop: None,
    };
    let stop = Shutdown::default();
    let mut last = 0;
    for _ in 0..30 {
        let at = engine.next_due();
        clock.set(at);
        let pass = engine.tick(&seeds, &stop, 0, &refused).unwrap();
        assert!(pass.queries <= MAX_MAINTENANCE_QUERIES);
        assert!(engine.next_due() >= at + 5);
        assert!(engine.next_publication() <= at + MAX_BACKOFF);
        assert!(engine.record().sequence() >= last);
        last = engine.record().sequence();
        let calls = refused.calls.load(Ordering::SeqCst);
        for _ in 0..100 {
            assert_eq!(engine.tick(&seeds, &stop, 0, &refused).unwrap().queries, 0);
        }
        assert_eq!(
            refused.calls.load(Ordering::SeqCst),
            calls,
            "no busy retries at the same clock time"
        );
    }
    assert_eq!(
        refused.contacts.lock().unwrap().len(),
        seeds.len(),
        "round-robin probes all known peers"
    );
    assert_eq!(
        engine.status(true).unwrap().last_error,
        Some(Failure::Unreachable)
    );
    assert_eq!(engine.status(true).unwrap().recently_verified, 0);
    assert!(state.snapshot().unwrap().routing.contacts().is_empty());
    assert_eq!(state.snapshot().unwrap().records().len(), 1);
    let cancel = Shutdown::default();
    let connector = Refused {
        calls: AtomicUsize::new(0),
        contacts: Mutex::new(HashSet::new()),
        stop: Some(cancel.clone()),
    };
    clock.set(engine.next_due());
    engine.tick(&seeds, &cancel, 0, &connector).unwrap();
    assert_eq!(
        connector.calls.load(Ordering::SeqCst),
        1,
        "stop between individual RPCs"
    );
    assert_eq!(
        engine.tick(&seeds, &cancel, 0, &connector).unwrap().queries,
        0
    );
}

#[test]
fn failed_bootstrap_recovers_on_retry_and_clock_rollback_reserves_a_new_sequence() {
    let scratch = Scratch::new("retry");
    let clock = Arc::new(TestClock::new());
    let path = scratch.0.join("site");
    site(&path);
    let b_id = Arc::new(NodeIdentity::generate().unwrap());
    let b = Node::start(
        b_id.clone(),
        service(&scratch.0.join("b.cache"), *b_id.id(), &clock),
        &path,
        "127.0.0.1",
    );
    let _be = engine(&b, &b_id, Config::default());
    let a_id = Arc::new(NodeIdentity::generate().unwrap());
    let state = service(&scratch.0.join("a.cache"), *a_id.id(), &clock);
    let mut ae = Maintainer::new(
        state,
        a_id,
        vec!["127.0.0.1:1".parse().unwrap()],
        Config::default(),
    )
    .unwrap();
    let stop = Shutdown::default();
    b.online.store(false, Ordering::SeqCst);
    assert_eq!(
        ae.tick(&[b.contact], &stop, 8, &yami_dht::PinnedTls)
            .unwrap()
            .published,
        0
    );
    let previous = ae.record().sequence();
    clock.set(ae.next_publication());
    b.online.store(true, Ordering::SeqCst);
    assert_eq!(
        ae.tick(&[b.contact], &stop, 8, &yami_dht::PinnedTls)
            .unwrap()
            .published,
        1
    );
    assert!(ae.record().sequence() > previous);
    assert_eq!(ae.status(true).unwrap().last_error, None);
    let last = ae.record().sequence();
    clock.set(clock.now().unwrap() - 600);
    assert_eq!(
        ae.tick(&[b.contact], &stop, 8, &yami_dht::PinnedTls)
            .unwrap()
            .published,
        1
    );
    assert!(ae.record().sequence() > last);
}

#[test]
fn crash_staging_file_and_failed_writes_cannot_reuse_advertised_sequences() {
    let scratch = Scratch::new("disk");
    let clock = Arc::new(TestClock::new());
    let identity = NodeIdentity::generate().unwrap();
    let path = scratch.0.join("state.cache");
    let state = service(&path, *identity.id(), &clock);
    let record = state
        .prepare_self(
            &identity,
            vec!["127.0.0.1:1".parse().unwrap()],
            clock.now().unwrap(),
        )
        .unwrap();
    fs::write(
        path.with_extension("tmp"),
        b"simulated interrupted staging write",
    )
    .unwrap();
    assert!(state
        .prepare_self(
            &identity,
            vec!["127.0.0.1:1".parse().unwrap()],
            clock.now().unwrap()
        )
        .is_err());
    assert_eq!(
        state
            .snapshot()
            .unwrap()
            .get(*identity.id(), clock.now().unwrap())
            .unwrap(),
        record,
        "a failure before rename retains the last safely committed state"
    );
    assert!(state
        .prepare_self(
            &identity,
            vec!["127.0.0.1:1".parse().unwrap()],
            clock.now().unwrap()
        )
        .is_err());
    drop(state);
    // Leave another interrupted staging file for startup recovery.
    fs::write(path.with_extension("tmp"), b"interrupted").unwrap();
    let state = service(&path, *identity.id(), &clock);
    let next = state
        .prepare_self(
            &identity,
            vec!["127.0.0.1:1".parse().unwrap()],
            clock.now().unwrap(),
        )
        .unwrap();
    assert_eq!(next.sequence(), record.sequence() + 1);
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "bounded test wait");
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn worker_shutdown_interrupts_long_wait_and_releases_persistent_state() {
    let scratch = Scratch::new("worker");
    let clock = Arc::new(TestClock::new());
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let path = scratch.0.join("state.cache");
    let state = service(&path, *identity.id(), &clock);
    let engine = Maintainer::new(
        state,
        identity.clone(),
        vec!["127.0.0.1:1".parse().unwrap()],
        Config::default(),
    )
    .unwrap();
    let signal = Shutdown::default();
    let mut worker = Worker::start(engine, scratch.0.clone(), None, signal).unwrap();
    let status_path = maintenance::status_path(&scratch.0, Network::Loopback);
    wait_until(|| maintenance::read_status(&status_path, Network::Loopback).is_ok());
    let started = Instant::now();
    worker.stop();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "condvar wait must be cancellable"
    );
    let status = maintenance::read_status(&status_path, Network::Loopback).unwrap();
    assert!(!status.running);
    assert_eq!(status.last_error, Some(Failure::NoPeers));
    assert!(maintenance::read_status(&status_path, Network::Public).is_err());
    let state = service(&path, *identity.id(), &clock);
    assert_eq!(state.snapshot().unwrap().records().len(), 1);
}

#[test]
fn a_failed_peer_backs_off_while_other_peers_remain_healthy() {
    struct PartialFailure {
        failed: Contact,
        attempts: AtomicUsize,
    }
    impl RpcConnector for PartialFailure {
        fn connect(
            &self,
            contact: Contact,
            budget: Duration,
        ) -> io::Result<Box<dyn ynp::Connection>> {
            if contact == self.failed {
                self.attempts.fetch_add(1, Ordering::SeqCst);
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "offline peer",
                ));
            }
            yami_dht::PinnedTls.connect(contact, budget)
        }
    }
    let scratch = Scratch::new("partial");
    let clock = Arc::new(TestClock::new());
    let path = scratch.0.join("site");
    site(&path);
    let b_id = Arc::new(NodeIdentity::generate().unwrap());
    let b = Node::start(
        b_id.clone(),
        service(&scratch.0.join("b.cache"), *b_id.id(), &clock),
        &path,
        "127.0.0.1",
    );
    let _be = engine(&b, &b_id, Config::default());
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let state = service(&scratch.0.join("a.cache"), *identity.id(), &clock);
    let mut engine = Maintainer::new(
        state,
        identity,
        vec!["127.0.0.1:1".parse().unwrap()],
        Config {
            renew_secs: 43200,
            health_secs: 5,
        },
    )
    .unwrap();
    let failed = Contact::new(
        *NodeIdentity::generate().unwrap().id(),
        "127.0.0.1:2".parse().unwrap(),
        Network::Loopback,
    )
    .unwrap();
    let connector = PartialFailure {
        failed,
        attempts: AtomicUsize::new(0),
    };
    let stop = Shutdown::default();
    for _ in 0..20 {
        clock.set(engine.next_due());
        assert!(
            engine
                .tick(&[b.contact, failed], &stop, 0, &connector)
                .unwrap()
                .verified
                > 0
        );
    }
    assert!(
        connector.attempts.load(Ordering::SeqCst) < 10,
        "failed peer must not be retried on every healthy pass"
    );
    // Endpoint files are bounded and policy-checked before a running update.
    let endpoints = scratch.0.join("operator-endpoints.txt");
    fs::write(&endpoints, "127.0.0.3:7443\n").unwrap();
    assert!(maintenance::read_endpoints(&endpoints, Network::Loopback).is_ok());
    assert!(maintenance::read_endpoints(&endpoints, Network::Public).is_err());
    fs::write(&endpoints, "127.0.0.1:1\n".repeat(5)).unwrap();
    assert!(maintenance::read_endpoints(&endpoints, Network::Loopback).is_err());
    fs::write(&endpoints, "x".repeat(257)).unwrap();
    assert!(maintenance::read_endpoints(&endpoints, Network::Loopback).is_err());
}

#[cfg(unix)]
use std::process::{Child, Command, Stdio};
#[cfg(unix)]
struct Process(Child);
#[cfg(unix)]
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[cfg(unix)]
#[test]
fn cli_opt_in_status_sigterm_and_restart_are_clean() {
    let scratch = Scratch::new("cli");
    let path = scratch.0.join("site");
    site(&path);
    let keys = scratch.0.join("identity");
    let identity = NodeIdentity::create(&keys).unwrap();
    let config = scratch.0.join("config");
    let run = || {
        Process(
            Command::new(env!("CARGO_BIN_EXE_yami-node"))
                .args(["serve-dir", "127.0.0.1:0"])
                .arg(&path)
                .args([
                    "--tls",
                    "--dht",
                    "--dht-local",
                    "--dht-maintain",
                    "--dht-renew-secs",
                    "30",
                    "--dht-health-secs",
                    "5",
                    "--identity",
                ])
                .arg(&keys)
                .arg("--dht-dir")
                .arg(&config)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut last = 0;
    for _ in 0..2 {
        let mut child = run();
        let status_path = maintenance::status_path(&config, Network::Loopback);
        wait_until(|| {
            maintenance::read_status(&status_path, Network::Loopback).is_ok_and(|s| s.running)
        });
        let state =
            Service::read_snapshot(&config.join("dht-local-server.cache"), Network::Loopback)
                .unwrap();
        let record = state.get(*identity.id(), yami_dht::now().unwrap()).unwrap();
        assert!(record.sequence() > last);
        last = record.sequence();
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .args(["dht", "status", "--local", "--dir"])
            .arg(&config)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        for label in [
            "Known peers:",
            "Recently verified peers:",
            "Last successful publication:",
            "Local record expires in:",
            "Last maintenance error:",
        ] {
            assert!(text.contains(label), "{text}");
        }
        assert!(!text.contains("node.key"));
        assert!(Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()
            .unwrap()
            .success());
        wait_until(|| child.0.try_wait().unwrap().is_some());
        assert!(child.0.wait().unwrap().success());
        assert!(
            !maintenance::read_status(&status_path, Network::Loopback)
                .unwrap()
                .running
        );
    }
    for flags in [
        vec!["--dht-maintain"],
        vec!["--dht-renew-secs", "30"],
        vec!["--dht-health-secs", "5"],
        vec!["--dht", "--tls", "--dht-maintain", "--dht-renew-secs", "0"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .args(["serve-dir", "127.0.0.1:0"])
            .arg(&path)
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}

#[test]
fn relay_only_publication_withdrawal_renewal_and_restart_keep_high_water() {
    let scratch = Scratch::new("relay-lifecycle");
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let clock = Arc::new(TestClock::new());
    let at = clock.now().unwrap();
    let path = scratch.0.join("host.cache");
    let service = service(&path, *identity.id(), &clock);
    let mut engine = Maintainer::new(
        service.clone(),
        identity.clone(),
        vec![],
        Config {
            renew_secs: 30,
            health_secs: 5,
        },
    )
    .unwrap();
    let route = yami_dht::RelayRoute {
        relay: *NodeIdentity::generate().unwrap().id(),
        endpoint: "127.0.0.1:7444".parse().unwrap(),
    };
    engine.set_routes(vec![route]).unwrap();
    let signal = Shutdown::default();
    engine.tick(&[], &signal, 1, &yami_dht::PinnedTls).unwrap();
    let first = engine.record().clone();
    assert!(first.endpoints().is_empty());
    assert_eq!(first.relay_routes(), &[route]);
    clock.set(at + 60);
    engine.tick(&[], &signal, 1, &yami_dht::PinnedTls).unwrap();
    assert!(engine.record().sequence() > first.sequence());
    engine.set_routes(vec![]).unwrap();
    clock.set(at + 120);
    engine.tick(&[], &signal, 1, &yami_dht::PinnedTls).unwrap();
    let sequence = engine.record().sequence();
    assert!(engine.record().relay_routes().is_empty());
    drop(engine);
    drop(service);
    let restored =
        Service::open_with_clock(&path, *identity.id(), Network::Loopback, clock).unwrap();
    let engine = Maintainer::new(restored, identity, vec![], Config::default()).unwrap();
    assert!(engine.record().sequence() > sequence);
    assert!(engine.record().relay_routes().is_empty());
}
