//! Real browser loader -> YNS -> existing DHT cache -> destination-pinned TLS.
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
    model::{Browser, Loader, Location, PageState, Security},
    native::Resolver,
};
use yami_dht::{Network, PeerRecord, Service};
use yami_identity::{NodeId, NodeIdentity};
use yami_yns::{Name, Record, RecordKind, Store};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "browser-yns-{}",
            NodeIdentity::generate().unwrap().id()
        )))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Documents(Arc<AtomicUsize>);
impl yami_node::Documents for Documents {
    fn lookup(&self, key: &str) -> io::Result<Option<ynp::DocumentData>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Some(ynp::DocumentData {
            key: key.into(),
            source: "@page 0.1\n  title Named page\n= Named page\n".into(),
            style: String::new(),
        }))
    }
}
struct Host {
    key: Arc<NodeIdentity>,
    endpoint: SocketAddr,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Host {
    fn start() -> Self {
        let key = Arc::new(NodeIdentity::generate().unwrap());
        let listener =
            ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), key.clone()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let docs = Documents(requests.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let task = thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let _ = yami_node::serve_once(&listener, &docs);
                thread::sleep(Duration::from_millis(2));
            }
        });
        Self {
            key,
            endpoint,
            requests,
            stop,
            task: Some(task),
        }
    }
    fn id(&self) -> NodeId {
        *self.key.id()
    }
    fn cache(&self, resolver: &Resolver, claimed: &NodeIdentity) {
        let SocketAddr::V4(endpoint) = self.endpoint else {
            unreachable!()
        };
        let now = yami_dht::now().unwrap();
        let record = PeerRecord::sign(
            claimed,
            vec![endpoint],
            now + 120,
            1,
            Network::Loopback,
            now,
        )
        .unwrap();
        Service::open_lookup(resolver.state_path(), Network::Loopback)
            .unwrap()
            .accept(record, now)
            .unwrap();
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.task.take().unwrap().join().unwrap();
    }
}
fn settle(b: &mut Browser, l: &Loader) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while matches!(b.active().state, PageState::Loading) {
        b.poll(l);
        assert!(Instant::now() < deadline, "navigation did not complete");
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn aliases_scoped_names_and_direct_addresses_share_pinned_tls() {
    let tmp = Temp::new();
    let host = Host::start();
    let resolver = Arc::new(Resolver::new(tmp.0.clone(), Network::Loopback));
    host.cache(&resolver, &host.key);
    let owner = NodeIdentity::generate().unwrap();
    let ns = format!("{}.yns", owner.id());
    let mut s = Store::open(resolver.naming_path()).unwrap();
    s.set_alias("blog", host.id(), None).unwrap();
    let now = yami_dht::now().unwrap();
    s.import(
        Record::sign(
            &owner,
            &ns,
            "blog",
            1,
            now,
            now + 120,
            RecordKind::Map(host.id().to_string()),
        )
        .unwrap(),
        now,
    )
    .unwrap();
    drop(s);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver.clone()).unwrap();
    let mut b = Browser::new();
    for url in [
        "yami://@blog/home".into(),
        format!("yami://blog.{ns}/home"),
        format!("yami://{}/home", host.id()),
    ] {
        b.navigate(&url, &loader).unwrap();
        settle(&mut b, &loader);
        assert_eq!(b.active().title(), "Named page");
        assert_eq!(b.active().security(), Some(&Security::Verified(host.id())));
        assert_eq!(
            b.active().resolve_link("/next"),
            format!("yami://{}/next", host.id())
        );
        assert!(b.follow("ynp://127.0.0.1:7999/home", &loader).is_err());
    }
    assert_eq!(host.requests.load(Ordering::SeqCst), 3);
    // A later signed destination change blocks before network traffic.
    let mut s = Store::open(resolver.naming_path()).unwrap();
    s.import(
        Record::sign(
            &owner,
            &ns,
            "blog",
            2,
            now,
            now + 120,
            RecordKind::Map(NodeIdentity::generate().unwrap().id().to_string()),
        )
        .unwrap(),
        now,
    )
    .unwrap();
    drop(s);
    b.navigate(&format!("yami://blog.{ns}/home"), &loader)
        .unwrap();
    settle(&mut b, &loader);
    assert!(
        matches!(&b.active().state,PageState::Error(e) if e.contains("identity change blocked"))
    );
    assert_eq!(host.requests.load(Ordering::SeqCst), 3);
    b.navigate(&format!("yami://{}/home", host.id()), &loader)
        .unwrap();
    settle(&mut b, &loader);
    assert_eq!(b.active().security(), Some(&Security::Verified(host.id())));
}
#[test]
fn naming_failures_and_corrupt_state_do_not_break_identity_browsing() {
    let tmp = Temp::new();
    let host = Host::start();
    let resolver = Arc::new(Resolver::new(tmp.0.clone(), Network::Loopback));
    host.cache(&resolver, &host.key);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver.clone()).unwrap();
    let mut b = Browser::new();
    for url in ["yami://@missing/home", "yami://welcome.yami/home"] {
        b.navigate(url, &loader).unwrap();
        settle(&mut b, &loader);
        assert!(matches!(b.active().state, PageState::Error(_)));
    }
    fs::write(resolver.naming_path().join("yns.json"), "broken").unwrap();
    b.navigate("yami://@missing/home", &loader).unwrap();
    settle(&mut b, &loader);
    assert!(matches!(b.active().state, PageState::Error(_)));
    b.navigate(&format!("yami://{}/home", host.id()), &loader)
        .unwrap();
    settle(&mut b, &loader);
    assert_eq!(b.active().security(), Some(&Security::Verified(host.id())));
    assert_eq!(host.requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        resolver
            .resolve_address(&format!("yami://{}/home", host.id()))
            .unwrap()
            .id,
        host.id()
    );
}
#[test]
fn a_name_never_relaxes_tls_destination_authentication() {
    let tmp = Temp::new();
    let wrong_host = Host::start();
    let intended = NodeIdentity::generate().unwrap();
    let resolver = Arc::new(Resolver::new(tmp.0.clone(), Network::Loopback));
    wrong_host.cache(&resolver, &intended);
    let mut s = Store::open(resolver.naming_path()).unwrap();
    s.set_alias("blog", *intended.id(), None).unwrap();
    drop(s);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut b = Browser::new();
    b.navigate("yami://@blog/home", &loader).unwrap();
    settle(&mut b, &loader);
    assert!(matches!(&b.active().state,PageState::Error(e) if e.contains("identity mismatch")));
    assert_eq!(wrong_host.requests.load(Ordering::SeqCst), 0);
    assert!(b.active().security().is_none());
}
#[test]
fn native_management_commands_persist_aliases_and_create_namespaces() {
    let tmp = Temp::new();
    let resolver = Resolver::new(tmp.0.join("state"), Network::Loopback);
    let owner_dir = tmp.0.join("owner").display().to_string();
    let command = |args: &[&str]| {
        yami_browser::naming::command(
            &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            &resolver,
        )
    };
    let created = command(&["namespace-create", &owner_dir]).unwrap();
    let owner = NodeIdentity::load(&owner_dir).unwrap();
    assert!(created.contains(&format!("{}.yns", owner.id())));
    assert!(command(&["namespace-create", &owner_dir]).is_err());
    let target = NodeIdentity::generate().unwrap().id().to_string();
    command(&["alias-set", "blog", &target]).unwrap();
    assert_eq!(command(&["resolve", "@blog"]).unwrap(), target);
    let ns = format!("{}.yns", owner.id());
    let now = yami_dht::now().unwrap();
    let wire = command(&[
        "record-sign",
        &owner_dir,
        &ns,
        "blog",
        "1",
        &now.to_string(),
        &(now + 120).to_string(),
        "map",
        &target,
    ])
    .unwrap();
    let record_path = tmp.0.join("record.json");
    fs::write(&record_path, wire).unwrap();
    command(&["import", &record_path.display().to_string()]).unwrap();
    assert_eq!(
        command(&["resolve", &format!("blog.{ns}")]).unwrap(),
        target
    );
    assert!(matches!(
        Location::parse("yami://@blog/home").unwrap(),
        Location::Named(_)
    ));
    assert!(Name::parse("@blog").is_ok());
}
