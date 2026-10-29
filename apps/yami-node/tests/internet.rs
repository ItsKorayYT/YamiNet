//! Public-listener semantics on isolated local addresses. These do not cross
//! an internet connection, NAT or provider firewall (docs/internet-testing.md).
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
use yami_identity::NodeIdentity;
use yami_node::{
    peers::{PeerStore, Status},
    serve_once, Documents, Store,
};
use ynp::{DocumentData, Protection, Transport};
use ynp_tls::{Scope, TlsListener, TlsTransport};

fn identity() -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::generate().unwrap())
}
fn store() -> Store {
    let mut store = Store::default();
    store
        .insert(DocumentData {
            key: "home".into(),
            source: "@page 0.1\n  title Remote home\n= Remote home\n".into(),
            style: String::new(),
        })
        .unwrap();
    store
}
fn server(
    ip: &str,
    count: usize,
) -> (
    SocketAddr,
    yami_identity::NodeId,
    thread::JoinHandle<Vec<io::Result<yami_node::Reply>>>,
) {
    let identity = identity();
    let id = *identity.id();
    let listener = TlsListener::bind_public(format!("{ip}:0").parse().unwrap(), identity).unwrap();
    assert_eq!(listener.scope(), Scope::Internet);
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || {
        let store = store();
        (0..count).map(|_| serve_once(&listener, &store)).collect()
    });
    (address, id, task)
}
#[test]
fn two_different_ip_nodes_exchange_over_pinned_tls_and_refuse_mismatch() {
    let (one, id_one, first) = server("127.0.0.1", 2);
    let (two, id_two, second) = server("127.0.0.2", 2);
    assert_ne!(one.ip(), two.ip());
    let fetched = ynp::fetch(&TlsTransport::new(id_two).unwrap(), two, "home").unwrap();
    assert_eq!(
        fetched,
        ynp::fetch(&TlsTransport::new(id_one).unwrap(), one, "home").unwrap()
    );
    let error = ynp::fetch(&TlsTransport::new(id_one).unwrap(), two, "home").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let mismatch = error
        .get_ref()
        .unwrap()
        .downcast_ref::<ynp_tls::IdentityMismatch>()
        .unwrap();
    assert_eq!(mismatch.expected, id_one);
    assert_eq!(mismatch.found, Some(id_two));
    let connection = TlsTransport::new(id_one).unwrap().connect(one).unwrap();
    assert_eq!(connection.protection(), Protection::Tls13PinnedServer);
    drop(connection);
    let results = first.join().unwrap();
    assert!(results[0].is_ok());
    assert!(results[1].is_err());
    let results = second.join().unwrap();
    assert!(results[0].is_ok());
    assert!(results[1].is_err());
}

struct Counted {
    lookups: Arc<AtomicUsize>,
    store: Store,
}
impl Documents for Counted {
    fn lookup(&self, key: &str) -> io::Result<Option<DocumentData>> {
        self.lookups.fetch_add(1, Ordering::Relaxed);
        self.store.lookup(key)
    }
}
#[test]
fn public_node_bounds_slow_and_malformed_clients_and_recovers() {
    let identity = identity();
    let id = *identity.id();
    let listener = TlsListener::bind_public("127.0.0.1:0".parse().unwrap(), identity).unwrap();
    let address = listener.local_addr().unwrap();
    let lookups = Arc::new(AtomicUsize::new(0));
    let documents = Counted {
        lookups: Arc::clone(&lookups),
        store: store(),
    };
    let task = thread::spawn(move || {
        (0..4)
            .map(|_| serve_once(&listener, &documents))
            .collect::<Vec<_>>()
    });
    // Plaintext YNP never gets a document, even on an explicitly public node.
    let mut raw = TcpStream::connect(address).unwrap();
    raw.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
    raw.write_all(b"YNP\x01\x01\0\0\0\0\0\0\0").unwrap();
    let mut response = Vec::new();
    let _ = raw.read_to_end(&mut response);
    assert!(!String::from_utf8_lossy(&response).contains("Remote home"));
    drop(raw);
    // A TLS-authenticated client still has to send a bounded valid YNP frame.
    let mut encrypted = TlsTransport::new(id).unwrap().connect(address).unwrap();
    encrypted.write_all(&[0; 12]).unwrap();
    assert!(ynp::read_frame(&mut encrypted).is_err());
    drop(encrypted);
    let started = Instant::now();
    let mut idle = TcpStream::connect(address).unwrap();
    idle.set_read_timeout(Some(Duration::from_secs(7))).unwrap();
    let _ = idle.read(&mut [0; 1]);
    drop(idle);
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "idle handshake is bounded"
    );
    assert_eq!(lookups.load(Ordering::Relaxed), 0);
    assert!(ynp::fetch(&TlsTransport::new(id).unwrap(), address, "home").is_ok());
    let results = task.join().unwrap();
    assert!(results[..3].iter().all(Result::is_err));
    assert!(results[3].is_ok());
    assert_eq!(lookups.load(Ordering::Relaxed), 1);
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("ynp-internet-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn peers_persist_success_unavailability_reconnection_and_mismatch() {
    let dir = Scratch::new("peers");
    let original = identity();
    let id = *original.id();
    let listener =
        TlsListener::bind_public("127.0.0.1:0".parse().unwrap(), Arc::clone(&original)).unwrap();
    let address = listener.local_addr().unwrap();
    let task = thread::spawn(move || serve_once(&listener, &store()));
    let mut peers = PeerStore::open(&dir.0).unwrap();
    peers.add(id, address).unwrap();
    peers.connect(id).unwrap();
    assert!(
        task.join().unwrap().is_err(),
        "handshake-only client sends no YNP request"
    );
    let success = peers.entries()[0].last_success.unwrap();
    assert_eq!(peers.entries()[0].status, Status::Authenticated);
    assert!(peers.connect(id).is_err());
    assert_eq!(peers.entries()[0].status, Status::Failed);
    assert_eq!(peers.entries()[0].last_success, Some(success));
    drop(peers);
    let mut peers = PeerStore::open(&dir.0).unwrap();
    assert_eq!(peers.entries()[0].last_success, Some(success));
    // A different key at the old address must not inherit the saved history.
    let listener = TlsListener::bind_public(address, identity()).unwrap();
    let task = thread::spawn(move || serve_once(&listener, &store()));
    assert!(peers.connect(id).is_err());
    assert!(task.join().unwrap().is_err());
    assert_eq!(peers.entries()[0].id, id);
    assert_eq!(peers.entries()[0].last_success, Some(success));
    // Reconnect the original identity on the same configured endpoint.
    let listener = TlsListener::bind_public(address, original).unwrap();
    let task = thread::spawn(move || serve_once(&listener, &store()));
    peers.connect(id).unwrap();
    assert!(
        task.join().unwrap().is_err(),
        "handshake-only client sends no YNP request"
    );
    assert_eq!(peers.entries()[0].status, Status::Authenticated);
    assert_eq!(peers.entries().len(), 1);
}
fn cli(args: &[&str], dir: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(args)
        .arg("--dir")
        .arg(dir)
        .output()
        .unwrap()
}
#[test]
fn peer_cli_manually_adds_lists_authenticates_and_removes() {
    let dir = Scratch::new("cli");
    let (address, id, task) = server("127.0.0.1", 1);
    let id = id.to_string();
    let address = address.to_string();
    assert!(cli(&["peer", "add", &id, &address], &dir.0)
        .status
        .success());
    let listed = cli(&["peer", "list"], &dir.0);
    let text = String::from_utf8(listed.stdout).unwrap();
    assert!(text.contains("unverified"));
    assert!(text.contains(&address));
    assert!(cli(&["peer", "connect", &id], &dir.0).status.success());
    task.join().unwrap();
    let text = String::from_utf8(cli(&["peer", "list"], &dir.0).stdout).unwrap();
    assert!(text.contains("authenticated"));
    assert!(cli(&["peer", "remove", &id], &dir.0).status.success());
    assert!(!String::from_utf8(cli(&["peer", "list"], &dir.0).stdout)
        .unwrap()
        .contains(&id));
}
#[test]
fn public_listening_requires_tls_and_separate_opt_in() {
    for flags in [
        vec!["--public"],
        vec!["--public", "--tls", "--lan"],
        vec!["--public", "--tls", "--discoverable"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .args(["serve-dir", "127.0.0.1:0", "sites/alice"])
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
    for address in ["0.0.0.0:0", "[::]:0", "169.254.1.1:0", "198.51.100.1:0"] {
        let dir = Scratch::new("refused");
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .args([
                "serve-dir",
                address,
                "sites/alice",
                "--tls",
                "--public",
                "--identity",
            ])
            .arg(dir.0.join("keys"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!dir.0.join("keys").exists());
    }
    assert!(ynp::LoopbackTcp
        .connect("8.8.8.8:7443".parse().unwrap())
        .is_err());
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn cli_serves_an_encrypted_site_with_public_mode_explicit() {
    let dir = Scratch::new("serve-public");
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args([
            "serve-dir",
            "127.0.0.2:0",
            "sites/alice",
            "--tls",
            "--public",
            "--identity",
        ])
        .arg(dir.0.join("identity"))
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stderr.take().unwrap());
    let _running = Running(child);
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        let mut public_warning = false;
        while reader.read_line(&mut line).unwrap_or(0) != 0 {
            public_warning |= line.contains("PUBLIC MODE:");
            if let Some(url) = line.trim().strip_prefix("Open in Yami Browser: ") {
                let _ = sender.send((url.to_string(), public_warning));
                break;
            }
            line.clear();
        }
        let _ = io::copy(&mut reader, &mut io::sink());
    });
    let (url, warning) = receiver
        .recv_timeout(Duration::from_secs(15))
        .expect("public CLI startup is bounded");
    assert!(warning);
    assert!(url.contains("@127.0.0.2:"));
    let response = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(["fetch", &url])
        .output()
        .unwrap();
    assert!(
        response.status.success(),
        "{}",
        String::from_utf8_lossy(&response.stderr)
    );
    assert!(String::from_utf8_lossy(&response.stderr).contains("Verified node"));
    assert!(String::from_utf8_lossy(&response.stdout).contains("Alice"));
}

#[test]
#[ignore = "requires an explicitly owned remote public ynps:// endpoint in YAMINET_TEST_REMOTE_URL"]
fn owned_remote_node_authenticates_and_serves_a_document() {
    let url = std::env::var("YAMINET_TEST_REMOTE_URL").expect("set an owned remote test node URL");
    let (id, endpoint, key) = ynp_tls::parse_url(&url).unwrap();
    assert!(matches!(endpoint.ip(), std::net::IpAddr::V4(ip) if ynp_tls::is_public_ipv4(ip)));
    let transport = TlsTransport::new(id).unwrap();
    assert_eq!(
        transport.connect(endpoint).unwrap().protection(),
        Protection::Tls13PinnedServer
    );
    let document = ynp::fetch(&transport, endpoint, &key).unwrap();
    ylang::parse(&document.source).unwrap();
    let wrong = *identity().id();
    assert!(ynp::fetch(&TlsTransport::new(wrong).unwrap(), endpoint, &key).is_err());
}
