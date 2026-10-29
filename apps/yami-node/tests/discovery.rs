//! Real IPv4 multicast on one machine, explicitly opt-in. This is not a
//! two-computer/firewall test. See docs/discovery-testing.md.
use std::{
    io::{self, BufRead, BufReader},
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use yami_identity::{NodeId, NodeIdentity};
use ynp_discovery::{Advertisement, Config};
use ynp_tls::TlsTransport;

struct Node(Child);
impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn start(
    ip: Ipv4Addr,
    keys: &std::path::Path,
    config: Config,
    discoverable: bool,
) -> (Node, NodeId, SocketAddr) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yami-node"));
    command
        .args(["serve-dir", &format!("{ip}:0"), "sites/alice", "--identity"])
        .arg(keys)
        .args(["--tls", "--lan"])
        .env("YAMINET_DISCOVERY_GROUP", config.group.to_string())
        .env("YAMINET_DISCOVERY_PORT", config.port.to_string())
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .stderr(Stdio::piped());
    if discoverable {
        command.arg("--discoverable");
    }
    let mut child = command.spawn().unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let node = Node(child);
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        while stderr.read_line(&mut line).unwrap_or(0) != 0 {
            if let Some(url) = line.trim().strip_prefix("Open in Yami Browser: ") {
                let _ = sender.send(url.to_owned());
                break;
            }
            line.clear();
        }
        let _ = io::copy(&mut stderr, &mut io::sink());
    });
    let url = receiver
        .recv_timeout(Duration::from_secs(15))
        .expect("node startup must finish");
    let (id, endpoint, _) = ynp_tls::parse_url(&url).unwrap();
    (node, id, endpoint)
}

#[test]
fn cli_discovery_requires_encrypted_explicit_lan_mode() {
    for flags in [vec![], vec!["--tls"], vec!["--lan"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .args(["serve-dir", "127.0.0.1:0", "sites/alice", "--discoverable"])
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("--discoverable needs --tls --lan")
        );
    }
}

#[test]
#[ignore = "uses selected private interface and multicast; set YAMINET_TEST_LAN_ADDRESS"]
fn two_discoverable_nodes_an_undiscoverable_node_and_authenticated_fetch() {
    let ip: Ipv4Addr = std::env::var("YAMINET_TEST_LAN_ADDRESS")
        .expect("select a local private IPv4 LAN address")
        .parse()
        .unwrap();
    let interface = ynp_discovery::interface(ip).unwrap();
    let reserve = UdpSocket::bind((ip, 0)).unwrap();
    let config = Config {
        port: reserve.local_addr().unwrap().port(),
        ..Config::default()
    };
    drop(reserve);
    let scratch =
        Scratch(std::env::temp_dir().join(format!("yaminet-discovery-{}", std::process::id())));
    std::fs::create_dir_all(&scratch.0).unwrap();
    let (one, id_one, endpoint_one) = start(ip, &scratch.0.join("one"), config, true);
    let (two, id_two, endpoint_two) = start(ip, &scratch.0.join("two"), config, true);
    let (_hidden, hidden_id, hidden_endpoint) = start(ip, &scratch.0.join("hidden"), config, false);
    let found = ynp_discovery::discover(&interface, config, Duration::from_secs(1)).unwrap();
    assert_eq!(found.len(), 2, "{found:?}");
    for (id, endpoint) in [(id_one, endpoint_one), (id_two, endpoint_two)] {
        let node = found
            .iter()
            .find(|n| n.id() == id)
            .expect("node is discovered");
        assert_eq!(SocketAddr::V4(node.endpoint()), endpoint);
        assert!(node.url().ends_with("/home"));
        let document =
            ynp::fetch(&TlsTransport::new(node.id()).unwrap(), endpoint, "home").unwrap();
        assert!(document.source.contains("Alice"));
    }
    assert!(!found.iter().any(|n| n.id() == hidden_id));
    assert!(
        ynp::fetch(
            &TlsTransport::new(hidden_id).unwrap(),
            hidden_endpoint,
            "home"
        )
        .is_ok(),
        "undiscoverable TLS node still works"
    );
    let forged_id = *NodeIdentity::generate().unwrap().id();
    let forged = Advertisement::new(forged_id, found[0].endpoint(), "home").unwrap();
    let error = ynp::fetch(
        &TlsTransport::new(forged.id()).unwrap(),
        forged.endpoint().into(),
        "home",
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error
        .get_ref()
        .unwrap()
        .downcast_ref::<ynp_tls::IdentityMismatch>()
        .is_some());
    drop((one, two));
    let started = Instant::now();
    let found = ynp_discovery::discover(&interface, config, Duration::from_millis(300)).unwrap();
    assert!(found.is_empty(), "undiscoverable node must not answer");
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < Duration::from_secs(1));
}
