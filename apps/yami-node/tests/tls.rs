use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
    CertificateError, ClientConfig, ClientConnection, DigitallySignedStruct, SignatureScheme,
    StreamOwned,
};
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Output, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};
use yami_identity::NodeIdentity;
use yami_node::{serve_concurrently, serve_once, Reply, Store};
use ynp::{DocumentData, Frame, LocalListener, LoopbackTcp, Message, Protection, Transport};
use ynp_tls::{Credential, IdentityMismatch, TlsListener, TlsTransport};

const DAY: Duration = Duration::from_secs(24 * 60 * 60);

fn identity() -> Arc<NodeIdentity> {
    Arc::new(NodeIdentity::generate().unwrap())
}
fn welcome() -> DocumentData {
    DocumentData {
        key: "welcome".into(),
        source: include_str!("../../../examples/welcome.ylang").into(),
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
fn local() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
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
fn rustls_error(error: &io::Error) -> &rustls::Error {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
        .unwrap_or_else(|| panic!("not a TLS error: {error:?}"))
}
fn certificate_error(error: &io::Error) -> &CertificateError {
    match rustls_error(error) {
        rustls::Error::InvalidCertificate(error) => error,
        other => panic!("not a certificate error: {other:?}"),
    }
}
fn mismatch(error: &io::Error) -> &IdentityMismatch {
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<IdentityMismatch>())
        .unwrap_or_else(|| panic!("not an identity mismatch: {error:?}"))
}

#[test]
fn fetches_a_ylang_document_from_the_pinned_node() {
    let node = identity();
    let listener = TlsListener::bind(local(), Arc::clone(&node)).unwrap();
    assert_eq!(listener.node_id(), node.id());
    let (address, server) = serve(listener, store(&[welcome()]), 3);
    let transport = TlsTransport::new(*node.id()).unwrap();

    let connection = transport.connect(address).unwrap();
    assert_eq!(connection.protection(), Protection::Tls13PinnedServer);
    drop(connection);
    let document = ynp::fetch(&transport, address, "welcome").unwrap();
    assert_eq!(document, welcome());
    assert!(ylang::parse(&document.source).is_ok());
    assert_eq!(
        ynp::fetch(&transport, address, "missing")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    let outcomes = server.join().unwrap();
    // The first connection closed without a request.
    assert!(outcomes[0].is_err());
    assert_eq!(
        *outcomes[1].as_ref().unwrap(),
        Reply::Document("welcome".into())
    );
    assert_eq!(
        *outcomes[2].as_ref().unwrap(),
        Reply::NotFound("missing".into())
    );
}

#[test]
fn a_node_with_another_identity_is_refused() {
    let (trusted, impostor) = (identity(), identity());
    let listener = TlsListener::bind(local(), Arc::clone(&impostor)).unwrap();
    let (address, server) = serve(listener, store(&[welcome()]), 1);
    let error = ynp::fetch(
        &TlsTransport::new(*trusted.id()).unwrap(),
        address,
        "welcome",
    )
    .unwrap_err();
    let found = mismatch(&error);
    assert_eq!(found.expected, *trusted.id());
    assert_eq!(found.found, Some(*impostor.id()));
    assert!(error.to_string().contains(&impostor.id().to_string()));
    // The client aborted the handshake, so the impostor served nothing.
    assert!(server.join().unwrap()[0].is_err());
}

#[test]
fn expired_and_not_yet_valid_credentials_are_refused() {
    let node = identity();
    let now = SystemTime::now();
    let transport = TlsTransport::new(*node.id()).unwrap();
    for (not_before, not_after, expired) in [
        (now - 2 * DAY, now - DAY, true),
        (now + DAY, now + 2 * DAY, false),
    ] {
        let credential = Credential::issue(Arc::clone(&node), not_before, not_after).unwrap();
        let listener = TlsListener::bind_with_credential(local(), &credential).unwrap();
        let (address, server) = serve(listener, store(&[welcome()]), 1);
        let error = ynp::fetch(&transport, address, "welcome").unwrap_err();
        let reason = certificate_error(&error);
        if expired {
            assert!(
                matches!(
                    reason,
                    CertificateError::Expired | CertificateError::ExpiredContext { .. }
                ),
                "{reason:?}"
            );
        } else {
            assert!(
                matches!(
                    reason,
                    CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. }
                ),
                "{reason:?}"
            );
        }
        assert!(server.join().unwrap()[0].is_err());
    }
    assert!(Credential::issue(Arc::clone(&node), now, now).is_err());
}

#[test]
fn altered_certificates_are_refused() {
    let node = identity();
    let genuine = Credential::issue_now(Arc::clone(&node)).unwrap();
    let bytes = genuine.certificate().to_vec();
    let transport = TlsTransport::new(*node.id()).unwrap();
    let mut offsets: Vec<usize> = (0..bytes.len()).step_by(11).collect();
    offsets.push(bytes.len() - 1);
    for offset in offsets {
        let mut altered = bytes.clone();
        altered[offset] ^= 0x01;
        let credential = Credential::with_certificate(Arc::clone(&node), altered.into());
        let listener = TlsListener::bind_with_credential(local(), &credential).unwrap();
        let (address, server) = serve(listener, store(&[welcome()]), 1);
        let error = ynp::fetch(&transport, address, "welcome").unwrap_err();
        assert!(
            matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::PermissionDenied
            ),
            "byte {offset}: {error}"
        );
        assert!(server.join().unwrap()[0].is_err(), "byte {offset}");
    }
}

#[test]
fn a_copied_certificate_is_useless_without_the_node_key() {
    // A node certificate is public: every client receives it. Presenting it
    // without the matching secret key must fail the handshake signature.
    let (trusted, impostor) = (identity(), identity());
    let stolen = Credential::issue_now(Arc::clone(&trusted)).unwrap();
    let replay = Credential::with_certificate(impostor, stolen.certificate().clone());
    let listener = TlsListener::bind_with_credential(local(), &replay).unwrap();
    let (address, server) = serve(listener, store(&[welcome()]), 1);
    let error = ynp::fetch(
        &TlsTransport::new(*trusted.id()).unwrap(),
        address,
        "welcome",
    )
    .unwrap_err();
    assert!(
        matches!(certificate_error(&error), CertificateError::BadSignature),
        "{error}"
    );
    assert!(server.join().unwrap()[0].is_err());
}

/// Forwards one connection to `target`, flipping one bit of the server's
/// bytes at stream offset `flip_at`.
fn tampering_proxy(target: SocketAddr, flip_at: usize) -> SocketAddr {
    let listener = TcpListener::bind(local()).unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        let mut server = TcpStream::connect(target).unwrap();
        let (mut upstream, mut downstream) =
            (client.try_clone().unwrap(), server.try_clone().unwrap());
        thread::spawn(move || io::copy(&mut upstream, &mut downstream));
        let (mut buffer, mut offset) = ([0; 4096], 0);
        while let Ok(count @ 1..) = server.read(&mut buffer) {
            if (offset..offset + count).contains(&flip_at) {
                buffer[flip_at - offset] ^= 0x01;
            }
            if client.write_all(&buffer[..count]).is_err() {
                break;
            }
            offset += count;
        }
    });
    address
}

#[test]
fn tampering_in_transit_is_detected() {
    let node = identity();
    let large = DocumentData {
        key: "large".into(),
        source: "é".repeat(ynp::MAX_DOCUMENT / 2),
        style: "y".repeat(ynp::MAX_STYLE),
    };
    let transport = TlsTransport::new(*node.id()).unwrap();
    // Offset 100 is inside the handshake; 100000 inside the encrypted reply.
    for flip_at in [100, 100_000] {
        let listener = TlsListener::bind(local(), Arc::clone(&node)).unwrap();
        let (target, server) = serve(listener, store(std::slice::from_ref(&large)), 1);
        let error = ynp::fetch(&transport, tampering_proxy(target, flip_at), "large")
            .expect_err("a tampered connection must not deliver a document");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        let _ = server.join().unwrap();
    }
}

#[test]
fn maximum_documents_arrive_byte_for_byte() {
    let node = identity();
    let documents = [
        DocumentData {
            key: "k".repeat(ynp::MAX_KEY),
            source: "x".repeat(ynp::MAX_DOCUMENT),
            style: "y".repeat(ynp::MAX_STYLE),
        },
        DocumentData {
            key: "unicode".into(),
            source: "Unicode: café ✓ 𝄞\n".repeat(1000),
            style: String::new(),
        },
    ];
    let listener = TlsListener::bind(local(), Arc::clone(&node)).unwrap();
    let (address, server) = serve(listener, store(&documents), 2);
    let transport = TlsTransport::new(*node.id()).unwrap();
    for document in &documents {
        assert_eq!(
            ynp::fetch(&transport, address, &document.key).unwrap(),
            *document
        );
    }
    assert!(server.join().unwrap().iter().all(Result::is_ok));
}

#[test]
fn never_falls_back_to_plaintext() {
    let node = identity();
    // A TLS client meeting a legacy plaintext node.
    let legacy = LocalListener::bind(local()).unwrap();
    let address = legacy.local_addr().unwrap();
    let server = thread::spawn(move || serve_once(&legacy, &store(&[welcome()])));
    let transport = TlsTransport::new(*node.id()).unwrap();
    assert!(ynp::fetch(&transport, address, "welcome").is_err());
    assert!(server.join().unwrap().is_err());

    // A plaintext client meeting a TLS node gets no document either.
    let listener = TlsListener::bind(local(), Arc::clone(&node)).unwrap();
    let (address, server) = serve(listener, store(&[welcome()]), 2);
    assert!(ynp::fetch(&LoopbackTcp, address, "welcome").is_err());
    let mut raw = TcpStream::connect(address).unwrap();
    raw.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
    let _ = raw.read(&mut [0; 64]);
    assert!(server.join().unwrap().iter().all(Result::is_err));

    // Neither side accepts public or wildcard addresses (LAN: see lan.rs).
    for address in ["192.0.2.1:7301", "0.0.0.0:0", "[::]:0"] {
        let address: SocketAddr = address.parse().unwrap();
        assert!(transport.connect(address).is_err(), "{address}");
        assert!(
            TlsListener::bind(address, Arc::clone(&node)).is_err(),
            "{address}"
        );
    }
}

/// Trusts any server and offers the given ALPN and signature schemes, to
/// exercise the node's own handshake checks.
#[derive(Debug)]
struct AnyServer(Vec<SignatureScheme>);
impl ServerCertVerifier for AnyServer {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.clone()
    }
}
fn custom_handshake(
    address: SocketAddr,
    alpn: &[&[u8]],
    schemes: &[SignatureScheme],
) -> io::Result<()> {
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyServer(schemes.to_vec())))
            .with_no_client_auth();
    config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    let tls = ClientConnection::new(Arc::new(config), "node".try_into().unwrap()).unwrap();
    let mut stream = StreamOwned::new(tls, TcpStream::connect(address)?);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock)?;
    }
    let get = Frame {
        request_id: 1,
        message: Message::Get {
            key: "welcome".into(),
        },
    };
    ynp::write_frame(&mut stream, &get)?;
    ynp::read_frame(&mut stream).map(drop)
}

#[test]
fn handshakes_without_ynp_alpn_or_ed25519_fail() {
    let node = identity();
    let listener = TlsListener::bind(local(), Arc::clone(&node)).unwrap();
    let (address, server) = serve(listener, store(&[welcome()]), 4);
    let ed25519 = [SignatureScheme::ED25519];
    assert!(custom_handshake(address, &[b"http/1.1"], &ed25519).is_err());
    assert!(custom_handshake(address, &[], &ed25519).is_err());
    assert!(custom_handshake(
        address,
        &[ynp_tls::ALPN],
        &[SignatureScheme::ECDSA_NISTP256_SHA256]
    )
    .is_err());
    // The same client succeeds once it offers both, so the failures above
    // are due to the node's checks.
    custom_handshake(address, &[ynp_tls::ALPN], &ed25519).unwrap();
    let outcomes = server.join().unwrap();
    assert!(outcomes[..3].iter().all(Result::is_err));
    assert!(outcomes[3].is_ok());
}

/// A unique scratch directory, removed when dropped.
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yaminet-tls-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn reconnects_and_keeps_trusting_a_restarted_node() {
    let dir = TempDir::new("restart");
    let node = Arc::new(NodeIdentity::create(&dir.0).unwrap());
    let transport = TlsTransport::new(*node.id()).unwrap();

    // Many sequential and concurrent connections to one running node.
    let listener = TlsListener::bind(local(), node).unwrap();
    let address = listener.local_addr().unwrap();
    let documents = store(&[welcome()]);
    thread::spawn(move || serve_concurrently(&listener, &documents, 4, &|_| {}));
    for _ in 0..5 {
        assert_eq!(
            ynp::fetch(&transport, address, "welcome").unwrap(),
            welcome()
        );
    }
    let clients: Vec<_> = (0..8)
        .map(|_| {
            let transport = transport.clone();
            thread::spawn(move || ynp::fetch(&transport, address, "welcome"))
        })
        .collect();
    for client in clients {
        assert_eq!(client.join().unwrap().unwrap(), welcome());
    }

    // The node restarts with its persistent identity on another port.
    let reloaded = Arc::new(NodeIdentity::load(&dir.0).unwrap());
    let (address, server) = serve(
        TlsListener::bind(local(), reloaded).unwrap(),
        store(&[welcome()]),
        1,
    );
    assert_eq!(
        ynp::fetch(&transport, address, "welcome").unwrap(),
        welcome()
    );
    server.join().unwrap()[0].as_ref().unwrap();

    // A node that lost its key and made a new one is a different node.
    let replaced = TempDir::new("replaced");
    let other = Arc::new(NodeIdentity::create(&replaced.0).unwrap());
    let (address, _server) = serve(
        TlsListener::bind(local(), other).unwrap(),
        store(&[welcome()]),
        1,
    );
    mismatch(&ynp::fetch(&transport, address, "welcome").unwrap_err());
}

#[test]
fn ynps_urls_carry_the_trusted_node_id() {
    let node = identity();
    let address: SocketAddr = "127.0.0.1:7443".parse().unwrap();
    let url = ynp_tls::format_url(node.id(), address, "home");
    assert_eq!(url, format!("ynps://{}@127.0.0.1:7443/home", node.id()));
    assert_eq!(
        ynp_tls::parse_url(&url).unwrap(),
        (*node.id(), address, "home".into())
    );
    let id = node.id().to_string();
    for bad in [
        "ynp://127.0.0.1:7443/home".to_owned(),
        "ynps://127.0.0.1:7443/home".to_owned(),
        format!("ynps://{}@127.0.0.1:7443/home", id.to_uppercase()),
        format!("ynps://{}x@127.0.0.1:7443/home", id),
        format!("ynps://{id}@192.0.2.1:7443/home"),
        format!("ynps://{id}@localhost:7443/home"),
        format!("ynps://{id}@127.0.0.1:0/home"),
        format!("ynps://{id}@127.0.0.1:7443/../home"),
        format!("ynps://{id}@{id}@127.0.0.1:7443/home"),
    ] {
        assert!(ynp_tls::parse_url(&bad).is_err(), "{bad}");
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
fn cli_tls_demo_runs_two_nodes_with_persistent_identities() {
    let dir = TempDir::new("cli-demo");
    let (one, two) = (dir.0.join("one"), dir.0.join("two"));
    let args = ["tls-demo", one.to_str().unwrap(), two.to_str().unwrap()];
    let ids = |output: &Output| -> Vec<String> {
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert!(text(&output.stderr).contains(ynp_tls::WARNING));
        let stdout = text(&output.stdout);
        assert!(stdout.contains("Node one received about from ynps://"));
        assert!(stdout.contains("Node two received welcome from ynps://"));
        assert!(stdout.contains("Refused node two"), "{stdout}");
        assert!(stdout.contains("server is not the trusted node"));
        stdout
            .lines()
            .filter_map(|line| line.strip_prefix("Node "))
            .filter_map(|line| line.split_whitespace().nth(1))
            .filter(|id| id.starts_with("yn1-"))
            .map(str::to_owned)
            .collect()
    };
    let first = ids(&node(&args));
    assert_eq!(first.len(), 2);
    assert_ne!(first[0], first[1]);
    assert_eq!(ids(&node(&args)), first, "identities persist across runs");
}

/// Kills the node process even when an assertion fails.
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn cli_serves_a_site_over_tls_and_fetches_with_a_pinned_id() {
    let dir = TempDir::new("cli-serve");
    let keys = dir.0.join("identity");
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(["serve-dir", "127.0.0.1:0", "sites/alice", "--identity"])
        .arg(&keys)
        .arg("--tls")
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let _node = Running(child);
    let mut line = String::new();
    let (address, id) = loop {
        line.clear();
        assert!(stderr.read_line(&mut line).unwrap() > 0, "node exited");
        if let Some((_, rest)) = line.split_once(" via YNP over TLS 1.3 on ") {
            let (address, id) = rest.trim().split_once(" as ").unwrap();
            break (address.to_owned(), id.to_owned());
        }
    };
    thread::spawn(move || io::copy(&mut stderr, &mut io::sink()));
    assert_eq!(id, NodeIdentity::load(&keys).unwrap().id().to_string());

    let fetched = node(&["fetch", &format!("ynps://{id}@{address}/home")]);
    assert!(fetched.status.success(), "{}", text(&fetched.stderr));
    assert!(text(&fetched.stdout).contains("@page 0.1"));
    assert!(text(&fetched.stderr).contains(&format!("Verified node {id}")));

    let other = identity().id().to_string();
    let refused = node(&["fetch", &format!("ynps://{other}@{address}/home")]);
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert!(text(&refused.stderr).contains("server is not the trusted node"));

    // The legacy plaintext client gets nothing from a TLS node.
    let plaintext = node(&["fetch", &address, "home"]);
    assert!(!plaintext.status.success());
    assert!(plaintext.stdout.is_empty());
}

#[test]
fn cli_requires_an_identity_for_tls_and_a_node_id_in_ynps_urls() {
    let output = node(&["serve-dir", "127.0.0.1:0", "sites/alice", "--tls"]);
    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("--tls needs --identity"));
    let output = node(&["fetch", "ynps://127.0.0.1:7301/home"]);
    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("missing trusted node ID"));
    let dir = TempDir::new("cli-public");
    let keys = dir.0.join("identity");
    let output = node(&[
        "serve-dir",
        "0.0.0.0:0",
        "sites/alice",
        "--identity",
        keys.to_str().unwrap(),
        "--tls",
    ]);
    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("only loopback addresses are allowed"));
}
