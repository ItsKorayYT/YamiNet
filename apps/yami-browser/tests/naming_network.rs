//! Actual TLS/YNP exchanges with honest and adversarial naming providers.
use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use yami_browser::native::Resolver;
use yami_dht::Network;
use yami_identity::NodeIdentity;
use yami_yns::{
    network::{self, Provider, Service},
    Bundle, Name, Record, RecordKind, Store,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "yns-net-{}",
            NodeIdentity::generate().unwrap().id()
        )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
type Handler = Arc<dyn Fn(&[u8]) -> io::Result<Vec<u8>> + Send + Sync>;
struct Documents {
    handler: Handler,
    calls: Arc<AtomicUsize>,
}
impl yami_node::Documents for Documents {
    fn lookup(&self, _: &str) -> io::Result<Option<ynp::DocumentData>> {
        Ok(None)
    }
    fn naming(&self, request: &[u8]) -> io::Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.handler)(request)
    }
}
struct Host {
    provider: Provider,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Host {
    fn start(handler: Handler) -> Self {
        let key = Arc::new(NodeIdentity::generate().unwrap());
        let listener =
            ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), key.clone()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let provider = Provider {
            id: key.id().to_string(),
            endpoint,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let docs = Documents {
            handler,
            calls: calls.clone(),
        };
        let task = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let _ = yami_node::serve_once(&listener, &docs);
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            provider,
            calls,
            stop,
            task: Some(task),
        }
    }
    fn mirror(path: &Path, peers: Vec<Provider>) -> Self {
        let service = Service::open(path, Network::Loopback, peers).unwrap();
        Self::start(Arc::new(move |bytes| {
            service
                .handle(bytes, yami_dht::now()?)
                .map_err(io::Error::other)
        }))
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.task.take().unwrap().join().unwrap();
    }
}
fn signed(owner: &NodeIdentity, sequence: u64, target: &NodeIdentity) -> Bundle {
    let at = yami_dht::now().unwrap();
    let ns = format!("{}.yns", owner.id());
    Bundle {
        name: format!("blog.{ns}"),
        records: vec![Record::sign(
            owner,
            &ns,
            "blog",
            sequence,
            at - 1,
            at + 300,
            RecordKind::Map(target.id().to_string()),
        )
        .unwrap()],
    }
}
fn reply(bundle: Option<Bundle>, peers: Vec<Provider>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"version":1,"message":{"op":"answer","answer":{"bundle":bundle,"peers":peers}}})).unwrap()
}
fn resolver(path: &Path, providers: Vec<Provider>) -> Resolver {
    let r = Resolver::new(path.into(), Network::Loopback);
    Store::open(r.naming_path())
        .unwrap()
        .configure_providers(providers, Network::Loopback)
        .unwrap();
    r
}
fn publish(host: &Host, bundle: Bundle) {
    network::publish(
        &host.provider,
        Network::Loopback,
        bundle,
        Instant::now() + network::RPC_BUDGET,
    )
    .unwrap();
}
#[test]
fn provider_graph_publication_mirroring_and_persistent_offline_cache() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let target = NodeIdentity::generate().unwrap();
    let b = signed(&owner, 1, &target);
    let n = Name::parse(&b.name).unwrap();
    let mirror = Host::mirror(&tmp.0.join("mirror"), vec![]);
    let seed = Host::mirror(&tmp.0.join("seed"), vec![mirror.provider.clone()]);
    publish(&mirror, b.clone());
    let r = resolver(&tmp.0.join("browser"), vec![seed.provider.clone()]);
    assert_eq!(r.resolve_name(&n).unwrap(), *target.id());
    assert!(seed.calls.load(Ordering::SeqCst) > 0);
    assert!(mirror.calls.load(Ordering::SeqCst) > 1);
    let answer = network::query(
        &mirror.provider,
        Network::Loopback,
        &n,
        Instant::now() + network::RPC_BUDGET,
    )
    .unwrap();
    assert_eq!(answer.bundle, Some(b));
    // Any independent publisher can copy unchanged signed bytes to a new mirror.
    network::publish(
        &seed.provider,
        Network::Loopback,
        answer.bundle.unwrap(),
        Instant::now() + network::RPC_BUDGET,
    )
    .unwrap();
    let seed_pin = seed.provider.clone();
    drop(mirror);
    let fresh = resolver(&tmp.0.join("other-browser"), vec![seed_pin]);
    assert_eq!(fresh.resolve_name(&n).unwrap(), *target.id());
    drop(seed);
    drop(r);
    let restarted = Resolver::new(tmp.0.join("browser"), Network::Loopback);
    assert_eq!(restarted.resolve_name(&n).unwrap(), *target.id());
    let mut cache = Store::open(restarted.naming_path()).unwrap();
    cache
        .configure_providers(vec![], Network::Loopback)
        .unwrap();
    assert_eq!(
        cache.resolve(&n, yami_dht::now().unwrap()).unwrap(),
        *target.id()
    );
}
#[test]
fn malicious_publisher_forgery_wrong_identity_missing_records_and_offline_peer_are_tolerated() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let target = NodeIdentity::generate().unwrap();
    let b = signed(&owner, 1, &target);
    let n = Name::parse(&b.name).unwrap();
    let good = Host::mirror(&tmp.0.join("good"), vec![]);
    publish(&good, b.clone());
    let attacker = NodeIdentity::generate().unwrap();
    let at = yami_dht::now().unwrap();
    let unauthorized = Bundle {
        name: b.name.clone(),
        records: vec![Record::sign(
            &attacker,
            &format!("{}.yns", owner.id()),
            "blog",
            2,
            at - 1,
            at + 300,
            RecordKind::Map(attacker.id().to_string()),
        )
        .unwrap()],
    };
    assert!(network::publish(
        &good.provider,
        Network::Loopback,
        unauthorized,
        Instant::now() + network::RPC_BUDGET
    )
    .is_err());

    let mut bad: serde_json::Value = serde_json::from_slice(&reply(Some(b), vec![])).unwrap();
    bad["message"]["answer"]["bundle"]["records"][0]["signature"] =
        serde_json::json!("00".repeat(64));
    let bytes = serde_json::to_vec(&bad).unwrap();
    let forged = Host::start(Arc::new(move |_| Ok(bytes.clone())));
    let wrong = Host::mirror(&tmp.0.join("wrong"), vec![]);
    let mut wrong_pin = wrong.provider.clone();
    wrong_pin.id = NodeIdentity::generate().unwrap().id().to_string();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
        unreachable!()
    };
    drop(listener);
    let offline = Provider {
        id: owner.id().to_string(),
        endpoint,
    };
    let r = resolver(
        &tmp.0.join("browser"),
        vec![
            offline,
            wrong_pin,
            forged.provider.clone(),
            good.provider.clone(),
        ],
    );
    assert_eq!(r.resolve_name(&n).unwrap(), *target.id());
    assert_eq!(wrong.calls.load(Ordering::SeqCst), 0);
    let missing = Name::parse(&format!("missing.{}.yns", owner.id())).unwrap();
    assert!(r.resolve_name(&missing).is_err());
    let unknown = Resolver::new(tmp.0.join("no-bootstrap"), Network::Loopback);
    assert!(unknown
        .resolve_name(&n)
        .unwrap_err()
        .contains("No YNS providers configured"));
    let mut s = Store::open(unknown.naming_path()).unwrap();
    s.set_alias("blog", *target.id(), None).unwrap();
    drop(s);
    assert_eq!(
        unknown
            .resolve_name(&Name::parse("@blog").unwrap())
            .unwrap(),
        *target.id()
    );
}
#[test]
fn safe_refresh_blocks_identity_changes_rollback_and_equivocation_across_restart() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let first = NodeIdentity::generate().unwrap();
    let second = NodeIdentity::generate().unwrap();
    let old = signed(&owner, 1, &first);
    let n = Name::parse(&old.name).unwrap();
    let body = Arc::new(Mutex::new(reply(Some(old.clone()), vec![])));
    let source = body.clone();
    let host = Host::start(Arc::new(move |_| Ok(source.lock().unwrap().clone())));
    let r = resolver(&tmp.0.join("browser"), vec![host.provider.clone()]);
    assert_eq!(r.resolve_name(&n).unwrap(), *first.id());
    *body.lock().unwrap() = reply(Some(signed(&owner, 2, &second)), vec![]);
    Store::open(r.naming_path())
        .unwrap()
        .mark_refreshed(&n, yami_dht::now().unwrap() - network::REFRESH_SECONDS)
        .unwrap();
    assert!(r
        .resolve_name(&n)
        .unwrap_err()
        .contains("identity change blocked"));
    assert!(r
        .resolve_name_until(&n, Instant::now() + network::DISCOVERY_BUDGET, true)
        .unwrap_err()
        .contains("identity change blocked"));
    *body.lock().unwrap() = reply(Some(old), vec![]);
    assert!(r
        .resolve_name_until(&n, Instant::now() + network::DISCOVERY_BUDGET, true)
        .unwrap_err()
        .contains("identity change blocked"));
    let conflict = signed(&owner, 2, &first);
    *body.lock().unwrap() = reply(Some(conflict), vec![]);
    assert!(r
        .resolve_name_until(&n, Instant::now() + network::DISCOVERY_BUDGET, true)
        .unwrap_err()
        .contains("conflicting sequence"));
    drop(r);
    let r = Resolver::new(tmp.0.join("browser"), Network::Loopback);
    assert!(r
        .resolve_name(&n)
        .unwrap_err()
        .contains("conflicting sequence"));
    *body.lock().unwrap() = reply(Some(signed(&owner, 3, &first)), vec![]);
    assert_eq!(r.resolve_name(&n).unwrap(), *first.id());
}
#[test]
fn stale_records_are_not_accepted_and_aliases_never_leave_the_profile() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let ns = format!("{}.yns", owner.id());
    let n = Name::parse(&format!("blog.{ns}")).unwrap();
    let at = yami_dht::now().unwrap();
    let expired = Bundle {
        name: n.to_string(),
        records: vec![Record::sign(
            &owner,
            &ns,
            "blog",
            1,
            at - 100,
            at - 1,
            RecordKind::Map(owner.id().to_string()),
        )
        .unwrap()],
    };
    let bytes = reply(Some(expired), vec![]);
    let host = Host::start(Arc::new(move |_| Ok(bytes.clone())));
    let r = resolver(&tmp.0.join("browser"), vec![host.provider.clone()]);
    assert!(r.resolve_name(&n).is_err());
    assert!(Store::open(r.naming_path())
        .unwrap()
        .export_bundle(&n, at)
        .unwrap()
        .is_none());
    let mut s = Store::open(r.naming_path()).unwrap();
    s.set_alias("private", *owner.id(), None).unwrap();
    drop(s);
    let before = host.calls.load(Ordering::SeqCst);
    assert_eq!(
        r.resolve_name(&Name::parse("@private").unwrap()).unwrap(),
        *owner.id()
    );
    assert_eq!(host.calls.load(Ordering::SeqCst), before);
}
#[test]
fn referrals_cycles_and_total_discovery_work_are_bounded() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let n = Name::parse(&format!("blog.{}.yns", owner.id())).unwrap();
    let host = Host::mirror(&tmp.0.join("mirror"), vec![]);
    let bytes = reply(None, vec![host.provider.clone(); network::MAX_PEERS + 1]);
    let malicious = Host::start(Arc::new(move |_| Ok(bytes.clone())));
    let result = network::discover(
        &[malicious.provider.clone(), host.provider.clone()],
        Network::Loopback,
        &n,
        &[],
        yami_dht::now().unwrap(),
        Instant::now() + network::DISCOVERY_BUDGET,
    )
    .unwrap();
    assert_eq!(result.contacted, 2);
    assert_eq!(result.failures, 1);
    let cycle = reply(None, vec![host.provider.clone(); network::MAX_PEERS]);
    let peer = Host::start(Arc::new(move |_| Ok(cycle.clone())));
    let result = network::discover(
        &[peer.provider.clone(), host.provider.clone()],
        Network::Loopback,
        &n,
        &[],
        yami_dht::now().unwrap(),
        Instant::now() + network::DISCOVERY_BUDGET,
    )
    .unwrap();
    assert_eq!(result.contacted, 2);
    assert!(network::discover(
        &vec![host.provider.clone(); network::MAX_CONTACTS + 1],
        Network::Loopback,
        &n,
        &[],
        yami_dht::now().unwrap(),
        Instant::now() + network::DISCOVERY_BUDGET
    )
    .is_err());
    let result = network::discover(
        std::slice::from_ref(&host.provider),
        Network::Loopback,
        &n,
        &[],
        yami_dht::now().unwrap(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(result.contacted, 0);
}

#[test]
fn official_trust_remains_local_and_separate_from_mirror_tls_identity() {
    let tmp = Temp::new();
    let registry = NodeIdentity::generate().unwrap();
    let ordinary = NodeIdentity::generate().unwrap();
    let target = NodeIdentity::generate().unwrap();
    let at = yami_dht::now().unwrap();
    let service = Service::open(tmp.0.join("mirror"), Network::Loopback, vec![]).unwrap();
    service.configure_registry(&[*registry.id()]).unwrap();
    let host = Host::start(Arc::new(move |b| {
        service
            .handle(b, yami_dht::now()?)
            .map_err(io::Error::other)
    }));
    let name = Name::parse("welcome.yami").unwrap();
    let official = Bundle {
        name: name.to_string(),
        records: vec![Record::sign(
            &registry,
            "yami",
            "welcome",
            1,
            at - 1,
            at + 300,
            RecordKind::Map(target.id().to_string()),
        )
        .unwrap()],
    };
    publish(&host, official);
    let scoped = signed(&ordinary, 1, &target);
    publish(&host, scoped.clone());
    let r = resolver(&tmp.0.join("browser"), vec![host.provider.clone()]);
    let before = host.calls.load(Ordering::SeqCst);
    assert!(r
        .resolve_name(&name)
        .unwrap_err()
        .contains("trusted .yami registry key"));
    assert_eq!(host.calls.load(Ordering::SeqCst), before);
    Store::open(r.naming_path())
        .unwrap()
        .configure_registry(&[*registry.id()])
        .unwrap();
    assert_eq!(r.resolve_name(&name).unwrap(), *target.id());
    Store::open(r.naming_path())
        .unwrap()
        .configure_registry(&[])
        .unwrap();
    assert_eq!(
        r.resolve_name(&Name::parse(&scoped.name).unwrap()).unwrap(),
        *target.id()
    );
}
#[test]
fn expired_cached_records_cannot_be_used_when_providers_are_offline() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let at = yami_dht::now().unwrap();
    let namespace = format!("{}.yns", owner.id());
    let n = Name::parse(&format!("blog.{namespace}")).unwrap();
    let host = Host::mirror(&tmp.0.join("mirror"), vec![]);
    let provider = host.provider.clone();
    drop(host);
    let r = resolver(&tmp.0.join("browser"), vec![provider]);
    let mut cache = Store::open(r.naming_path()).unwrap();
    let record = Record::sign(
        &owner,
        &namespace,
        "blog",
        1,
        at - 100,
        at - 1,
        RecordKind::Map(owner.id().to_string()),
    )
    .unwrap();
    cache.import(record, at - 10).unwrap();
    cache.resolve(&n, at - 10).unwrap();
    cache.mark_refreshed(&n, at).unwrap();
    drop(cache);
    assert!(r.resolve_name(&n).unwrap_err().contains("expired"));
}
