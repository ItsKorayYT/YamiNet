//! YNP over TLS 1.3 on a private IPv4 LAN address of this machine.
//!
//! These tests bind a real private interface address, so client and node are
//! on one computer: they exercise the LAN code paths and address policy, not a
//! second physical machine or a firewall (see docs/lan-testing.md for that).
//! Set `YAMINET_TEST_LAN_ADDRESS` to choose the address; without a private
//! IPv4 address the tests that need one say so and pass without running.
use std::{
    fs,
    io::{self, BufRead, BufReader, Read},
    net::{IpAddr, SocketAddr, TcpStream, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::Arc,
    thread::{self, JoinHandle},
};
use yami_identity::NodeIdentity;
use yami_node::{serve_once, Reply, Store};
use ynp::{DocumentData, Frame, LocalListener, LoopbackTcp, Message, Protection, Transport};
use ynp_tls::{IdentityMismatch, Scope, TlsListener, TlsTransport};

/// A private IPv4 address of this machine, or `None` with a note on stderr.
fn lan_address(test: &str) -> Option<IpAddr> {
    if let Ok(text) = std::env::var("YAMINET_TEST_LAN_ADDRESS") {
        let ip: IpAddr = text.parse().expect("YAMINET_TEST_LAN_ADDRESS is an IP");
        assert!(ynp_tls::is_private_lan(ip), "{ip} is not private IPv4");
        return Some(ip);
    }
    // Connecting a UDP socket sends no packet; it only asks the routing table
    // which local address would reach a private destination.
    for probe in ["192.168.0.1:9", "10.0.0.1:9", "172.16.0.1:9"] {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if socket.connect(probe).is_ok() {
            if let Ok(local) = socket.local_addr() {
                if ynp_tls::is_private_lan(local.ip()) {
                    return Some(local.ip());
                }
            }
        }
    }
    eprintln!("{test}: SKIPPED, this machine has no private IPv4 address");
    None
}
fn identity() -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::generate().unwrap())
}
fn page(key: &str, title: &str) -> DocumentData {
    DocumentData {
        key: key.into(),
        source: format!("@page 0.1\n  title {title}\n= {title}\nServed over the LAN.\n"),
        style: include_str!("../../../examples/default.ysl").into(),
    }
}
fn store(documents: &[DocumentData]) -> Store {
    let mut store = Store::default();
    for document in documents {
        store.insert(document.clone()).unwrap();
    }
    store
}
/// Serve exactly `count` connections in the background and report each outcome.
fn serve(
    listener: TlsListener,
    documents: Store,
    count: usize,
) -> (SocketAddr, JoinHandle<Vec<io::Result<Reply>>>) {
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        (0..count)
            .map(|_| serve_once(&listener, &documents))
            .collect()
    });
    (address, server)
}

#[test]
fn encrypted_lan_connection_delivers_documents_from_the_pinned_node() {
    let Some(ip) = lan_address("encrypted_lan_connection") else {
        return;
    };
    let node = identity();
    let listener = TlsListener::bind_lan((ip, 0).into(), Arc::clone(&node)).unwrap();
    assert_eq!(listener.scope(), Scope::Lan);
    let home = page("home", "LAN home");
    let (address, server) = serve(listener, store(std::slice::from_ref(&home)), 3);
    assert_eq!(address.ip(), ip, "bound to the one chosen address");
    let transport = TlsTransport::new(*node.id()).unwrap();

    let connection = transport.connect(address).unwrap();
    assert_eq!(connection.protection(), Protection::Tls13PinnedServer);
    drop(connection);
    let fetched = ynp::fetch(&transport, address, "home").unwrap();
    assert_eq!(fetched, home);
    assert_eq!(
        ylang::parse(&fetched.source).unwrap().metadata.title,
        "LAN home"
    );
    let url = ynp_tls::format_url(node.id(), address, "missing");
    let (id, endpoint, key) = ynp_tls::parse_url(&url).unwrap();
    assert_eq!(
        ynp::fetch(&TlsTransport::new(id).unwrap(), endpoint, &key)
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    let outcomes = server.join().unwrap();
    assert!(outcomes[0].is_err(), "first connection sent no request");
    assert_eq!(
        *outcomes[1].as_ref().unwrap(),
        Reply::Document("home".into())
    );
    assert_eq!(
        *outcomes[2].as_ref().unwrap(),
        Reply::NotFound("missing".into())
    );
}

#[test]
fn a_lan_node_with_another_identity_is_refused_before_any_request() {
    let Some(ip) = lan_address("lan_wrong_identity") else {
        return;
    };
    let (expected, actual) = (identity(), identity());
    let listener = TlsListener::bind_lan((ip, 0).into(), Arc::clone(&actual)).unwrap();
    let (address, server) = serve(listener, store(&[page("home", "Home")]), 1);
    let error =
        ynp::fetch(&TlsTransport::new(*expected.id()).unwrap(), address, "home").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    let mismatch = error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<IdentityMismatch>())
        .unwrap();
    assert_eq!(mismatch.expected, *expected.id());
    assert_eq!(mismatch.found, Some(*actual.id()));
    // The handshake failed, so the node never saw a request.
    assert!(server.join().unwrap()[0].is_err());
}

#[test]
fn plaintext_ynp_is_refused_on_the_lan() {
    let Some(ip) = lan_address("lan_plaintext") else {
        return;
    };
    // No plaintext listener or client for a LAN address.
    let lan: SocketAddr = (ip, 0).into();
    assert!(LocalListener::bind(lan).is_err());
    assert!(LoopbackTcp.connect((ip, 7443).into()).is_err());
    assert!(ynp::parse_url(&format!("ynp://{ip}:7443/home")).is_err());
    // And a LAN TLS listener without explicit opt-in is refused too.
    assert!(TlsListener::bind(lan, identity()).is_err());

    // A raw plaintext YNP request to a LAN TLS node gets no document.
    let listener = TlsListener::bind_lan(lan, identity()).unwrap();
    let (address, server) = serve(listener, store(&[page("home", "Home")]), 1);
    let mut raw = TcpStream::connect(address).unwrap();
    ynp::write_frame(
        &mut raw,
        &Frame {
            request_id: 1,
            message: Message::Get { key: "home".into() },
        },
    )
    .unwrap();
    assert!(server.join().unwrap()[0].is_err());
    let mut reply = Vec::new();
    let _ = raw.read_to_end(&mut reply);
    assert!(ynp::read_frame(&mut reply.as_slice()).is_err());
    assert!(!String::from_utf8_lossy(&reply).contains("Served over the LAN"));
}

/// A unique scratch directory, removed when dropped.
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("yaminet-lan-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
/// Kills the node process even when an assertion fails.
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn node(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(args)
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap()
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn cli_refuses_lan_listening_without_tls_and_lan_and_any_wildcard() {
    let dir = TempDir::new("cli-refusals");
    let keys = dir.0.join("identity");
    let keys = keys.to_str().unwrap();
    for (args, expected) in [
        (
            vec!["serve-dir", "192.168.1.50:7443", "sites/alice"],
            "only loopback addresses are allowed without --lan",
        ),
        (
            vec![
                "serve-dir",
                "192.168.1.50:7443",
                "sites/alice",
                "--identity",
                keys,
                "--tls",
            ],
            "only loopback addresses are allowed without --lan",
        ),
        (
            vec!["serve-dir", "192.168.1.50:7443", "sites/alice", "--lan"],
            "--lan needs --tls",
        ),
        (
            vec![
                "serve-dir",
                "192.168.1.50:7443",
                "sites/alice",
                "--identity",
                keys,
                "--lan",
            ],
            "--lan needs --tls",
        ),
    ] {
        let output = node(&args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            text(&output.stderr).contains(expected),
            "{args:?}: {}",
            text(&output.stderr)
        );
    }
    for address in [
        "0.0.0.0:7443",
        "127.0.0.1:7443",
        "8.8.8.8:7443",
        "169.254.1.1:7443",
    ] {
        let output = node(&[
            "serve-dir",
            address,
            "sites/alice",
            "--identity",
            keys,
            "--tls",
            "--lan",
        ]);
        assert!(!output.status.success(), "{address}");
        assert!(
            text(&output.stderr).contains("--lan needs one specific private IPv4 address"),
            "{address}: {}",
            text(&output.stderr)
        );
    }
    // Refused before any key was created.
    assert!(!dir.0.exists());
}

#[test]
fn cli_serves_a_site_on_the_lan_and_prints_its_ynps_address() {
    let Some(ip) = lan_address("cli_lan") else {
        return;
    };
    let dir = TempDir::new("cli-serve");
    let keys = dir.0.join("identity");
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(["serve-dir", &format!("{ip}:0"), "sites/alice", "--identity"])
        .arg(&keys)
        .args(["--tls", "--lan"])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let _node = Running(child);
    let mut line = String::new();
    let url = loop {
        line.clear();
        assert!(stderr.read_line(&mut line).unwrap() > 0, "node exited");
        if let Some(url) = line.trim().strip_prefix("Open in Yami Browser: ") {
            break url.to_owned();
        }
    };
    thread::spawn(move || io::copy(&mut stderr, &mut io::sink()));
    let id = NodeIdentity::load(&keys).unwrap().id().to_string();
    assert!(url.starts_with(&format!("ynps://{id}@{ip}:")), "{url}");
    assert!(url.ends_with("/home"), "{url}");

    let fetched = node(&["fetch", &url]);
    assert!(fetched.status.success(), "{}", text(&fetched.stderr));
    assert!(text(&fetched.stdout).contains("Alice"));
    assert!(text(&fetched.stderr).contains(&format!("Verified node {id}")));

    let other = identity().id().to_string();
    let refused = node(&["fetch", &url.replace(&id, &other)]);
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert!(text(&refused.stderr).contains("server is not the trusted node"));

    let (_, address, _) = ynp_tls::parse_url(&url).unwrap();
    let plaintext = node(&["fetch", &address.to_string(), "home"]);
    assert!(!plaintext.status.success());
    assert!(plaintext.stdout.is_empty());
}
