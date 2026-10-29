//! Reproducible three-process simulation. All endpoints are loopback; this
//! proves distributed lookup behavior, not actual internet deployment.
use std::{
    fs,
    io::{self, BufRead, BufReader, Write},
    net::{SocketAddrV4, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use yami_dht::{Network, PeerRecord, Service};
use yami_identity::{NodeId, NodeIdentity};
use yami_node::peers::PeerStore;

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("yaminet-dht-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
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
fn start(endpoint: SocketAddrV4, keys: &Path, config: &Path, publications: usize) -> Running {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args([
            "serve-dir",
            &endpoint.to_string(),
            "sites/alice",
            "--tls",
            "--dht",
            "--dht-local",
            "--identity",
        ])
        .arg(keys)
        .arg("--dht-dir")
        .arg(config)
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stderr.take().unwrap());
    let running = Running(child);
    let (send, receive) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        let (mut url, mut published) = (false, false);
        let mut log = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            url |= line.contains("Open in Yami Browser:");
            published |= line.contains(&format!(
                "DHT startup publication complete: {publications} authenticated bootstrap(s)"
            ));
            log.push_str(&line);
            line.clear();
            if url && published {
                let _ = send.send(Ok(()));
                break;
            }
        }
        if !url || !published {
            let _ = send.send(Err(log));
        }
        let _ = io::copy(&mut reader, &mut io::sink());
    });
    receive
        .recv_timeout(Duration::from_secs(20))
        .expect("bounded DHT startup")
        .unwrap();
    running
}
fn lookup(command: &str, id: NodeId, config: &Path, keys: &Path, fetch: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_yami-node"));
    cmd.args(["dht", command, &id.to_string()]);
    if fetch {
        cmd.arg("home");
    }
    cmd.arg("--local")
        .arg("--dir")
        .arg(config)
        .arg("--identity")
        .arg(keys)
        .output()
        .unwrap()
}
#[test]
fn three_nodes_publish_lookup_fetch_and_reject_wrong_tls_identity() {
    let scratch = Scratch::new("three");
    let reserves: Vec<_> = (0..3)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect();
    let endpoints: Vec<SocketAddrV4> = reserves
        .iter()
        .map(|l| match l.local_addr().unwrap() {
            std::net::SocketAddr::V4(a) => a,
            _ => unreachable!(),
        })
        .collect();
    drop(reserves);
    let keys: Vec<_> = (0..3)
        .map(|i| scratch.0.join(format!("identity-{i}")))
        .collect();
    let identities: Vec<_> = keys
        .iter()
        .map(|p| NodeIdentity::create(p).unwrap())
        .collect();
    let configs: Vec<_> = (0..3)
        .map(|i| scratch.0.join(format!("config-{i}")))
        .collect();
    // A and C know only ordinary bootstrap B. C never receives A's IP as input.
    for index in [0, 2] {
        PeerStore::open(&configs[index])
            .unwrap()
            .add(*identities[1].id(), endpoints[1].into())
            .unwrap();
    }
    let b_node = start(endpoints[1], &keys[1], &configs[1], 0);
    let a = start(endpoints[0], &keys[0], &configs[0], 1);
    let _c = start(endpoints[2], &keys[2], &configs[2], 1);
    let joined = Service::read_snapshot(
        &configs[2].join("dht-local-server.cache"),
        Network::Loopback,
    )
    .unwrap();
    assert!(
        joined
            .get(*identities[0].id(), yami_dht::now().unwrap())
            .is_some(),
        "C queries B during join and learns A's signed endpoint"
    );
    assert!(
        joined
            .get(*identities[1].id(), yami_dht::now().unwrap())
            .is_some(),
        "C can advertise its signed bootstrap contact to later peers"
    );
    let stored = Service::read_snapshot(
        &configs[1].join("dht-local-server.cache"),
        Network::Loopback,
    )
    .unwrap();
    let a_record = stored
        .get(*identities[0].id(), yami_dht::now().unwrap())
        .expect("B stores A's validated publication");
    a_record
        .verify(Network::Loopback, yami_dht::now().unwrap())
        .unwrap();
    assert_eq!(a_record.endpoints(), [endpoints[0]]);
    let output = lookup("lookup", *identities[0].id(), &configs[2], &keys[2], false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(&ynp_tls::format_url(
            identities[0].id(),
            endpoints[0].into(),
            "home"
        ))
    );
    let fetched = lookup("fetch", *identities[0].id(), &configs[2], &keys[2], true);
    assert!(
        fetched.status.success(),
        "{}",
        String::from_utf8_lossy(&fetched.stderr)
    );
    assert!(String::from_utf8_lossy(&fetched.stdout).contains("Alice"));
    assert!(String::from_utf8_lossy(&fetched.stderr).contains("Verified node"));
    for (from, target) in [(0, 2), (1, 0), (2, 1)] {
        let result = lookup(
            "lookup",
            *identities[target].id(),
            &configs[from],
            &keys[from],
            false,
        );
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    // Even a VALID signature can lie about endpoint ownership. The actual YNP
    // fetch must still pin its TLS connection to the claimed node's exact ID.
    let impostor = NodeIdentity::generate().unwrap();
    let at = yami_dht::now().unwrap();
    let claim = PeerRecord::sign(
        &impostor,
        vec![endpoints[0]],
        at + 60,
        1,
        Network::Loopback,
        at,
    )
    .unwrap();
    let client = Service::open(
        configs[2].join("dht-local-client.cache"),
        *identities[2].id(),
        Network::Loopback,
    )
    .unwrap();
    client.accept(claim, at).unwrap();
    drop(client);
    let refused = lookup("fetch", *impostor.id(), &configs[2], &keys[2], true);
    assert!(!refused.status.success());
    assert!(refused.stdout.is_empty());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("server is not the trusted node"));
    // Restarting reserves a larger sequence before re-publication. Old valid
    // records remain rejected by B even though their signatures still verify.
    drop(a);
    let _a = start(endpoints[0], &keys[0], &configs[0], 1);
    let newer = Service::read_snapshot(
        &configs[1].join("dht-local-server.cache"),
        Network::Loopback,
    )
    .unwrap();
    assert!(
        newer
            .get(*identities[0].id(), yami_dht::now().unwrap())
            .unwrap()
            .sequence()
            > a_record.sequence()
    );
    let client = Service::open(
        configs[2].join("dht-local-client.cache"),
        *identities[2].id(),
        Network::Loopback,
    )
    .unwrap();
    let b = yami_dht::Contact::new(*identities[1].id(), endpoints[1], Network::Loopback).unwrap();
    assert_eq!(
        yami_dht::publish(&client, &a_record, &[b]).unwrap(),
        0,
        "B rejects a signed rollback"
    );
    drop(client);
    // Bootstrap B is not a permanent authority: C can use the validated
    // location it learned to query/authenticate A directly after B goes away.
    drop(b_node);
    let without_bootstrap = lookup("fetch", *identities[0].id(), &configs[2], &keys[2], true);
    assert!(
        without_bootstrap.status.success(),
        "{}",
        String::from_utf8_lossy(&without_bootstrap.stderr)
    );
    assert!(String::from_utf8_lossy(&without_bootstrap.stdout).contains("Alice"));
    let newcomer = NodeIdentity::generate().unwrap();
    let client = Service::open(
        scratch.0.join("newcomer.cache"),
        *newcomer.id(),
        Network::Loopback,
    )
    .unwrap();
    let via_c =
        yami_dht::Contact::new(*identities[2].id(), endpoints[2], Network::Loopback).unwrap();
    assert!(
        yami_dht::lookup(&client, *identities[0].id(), &[via_c])
            .unwrap()
            .record
            .is_some(),
        "an ordinary joined peer can bootstrap a new lookup without B"
    );
}

#[test]
fn dht_cli_reuses_manual_peer_configuration_and_requires_tls_opt_in() {
    let scratch = Scratch::new("cli");
    let identity = NodeIdentity::generate().unwrap();
    let add = node(&[
        "dht",
        "bootstrap",
        &identity.id().to_string(),
        "127.0.0.1:7443",
        "--local",
        "--dir",
        scratch.0.to_str().unwrap(),
    ]);
    assert!(add.status.success());
    let peers = node(&["peer", "list", "--dir", scratch.0.to_str().unwrap()]);
    assert!(
        String::from_utf8_lossy(&peers.stdout).contains(&identity.id().to_string()),
        "no duplicate bootstrap store"
    );
    let denied = node(&[
        "dht",
        "bootstrap",
        &NodeIdentity::generate().unwrap().id().to_string(),
        "127.0.0.1:7443",
        "--dir",
        scratch.0.to_str().unwrap(),
    ]);
    assert!(
        !denied.status.success(),
        "public lookup cannot probe private or loopback endpoints"
    );
    for flags in [
        vec!["--dht"],
        vec!["--dht-local"],
        vec!["--tls", "--dht"],
        vec!["--dht", "--tls", "--dht-local", "--public"],
    ] {
        let mut args = vec!["serve-dir", "127.0.0.1:0", "sites/alice"];
        args.extend(flags);
        assert!(!node(&args).status.success());
    }
}

#[test]
fn malformed_plaintext_clients_do_not_turn_into_dht_queries() {
    // A plain YNP listener never dispatches the DHT protocol, even if the peer
    // sends a correctly sized frame with its magic prefix.
    let listener = ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let server =
        thread::spawn(move || yami_node::serve_once(&listener, &yami_node::Store::default()));
    let mut stream = TcpStream::connect(endpoint).unwrap();
    let mut bytes = [0; yami_dht::MAX_FRAME];
    bytes[..4].copy_from_slice(&yami_dht::MAGIC);
    let _ = stream.write_all(&bytes);
    drop(stream);
    assert!(server.join().unwrap().is_err());
    // The DHT handler itself also rejects an explicitly plaintext transport,
    // even when a local library caller attaches it to the protocol dispatcher.
    use ynp::Incoming;
    let scratch = Scratch::new("plaintext");
    let owner = NodeIdentity::generate().unwrap();
    let dht = Service::open(
        scratch.0.join("dht-local-server.cache"),
        *owner.id(),
        Network::Loopback,
    )
    .unwrap();
    let listener = ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let handler = dht.clone();
    let server = thread::spawn(move || {
        let mut connection = listener.accept().unwrap();
        yami_node::serve_connection_with_dht(
            &mut *connection,
            &yami_node::Store::default(),
            &handler,
        )
    });
    let mut stream = TcpStream::connect(endpoint).unwrap();
    let _ = stream.write_all(&bytes);
    drop(stream);
    assert!(server.join().unwrap().is_err());
    assert!(dht.snapshot().unwrap().records().is_empty());
}
