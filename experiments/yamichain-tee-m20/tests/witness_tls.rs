//! Real TLS sockets; server authentication never replaces signed witness requests.
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::{
    fs,
    io::{BufRead, BufReader},
    net::TcpStream,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use yamichain_tee_m20::{
    quorum::{Committee, Member},
    witness::*,
    witness_tls::*,
    *,
};

struct Node {
    child: Child,
}
impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn tls(directory: &std::path::Path) -> (ServerTls, ClientTls) {
    use std::os::unix::fs::PermissionsExt;
    let key = directory.join("key.pem");
    let cert = directory.join("cert.pem");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-keyout"
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .output()
        .unwrap()
        .status
        .success());
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    (
        ServerTls {
            certificate_file: cert.clone(),
            key_file: key,
        },
        ClientTls {
            certificate_pem: fs::read(cert).unwrap(),
            server_name: "localhost".into(),
        },
    )
}
fn policy() -> (WitnessPolicy, SigningKey, SigningKey) {
    let members: [SigningKey; 4] = std::array::from_fn(|_| SigningKey::generate(&mut OsRng));
    let authorities: [SigningKey; 4] = std::array::from_fn(|_| SigningKey::generate(&mut OsRng));
    let observer = SigningKey::generate(&mut OsRng);
    let committee = Committee {
        chain: 23,
        members: std::array::from_fn(|i| Member {
            signing: members[i].verifying_key().to_bytes(),
            transport: [i as u8 + 101; 32],
        }),
    };
    (
        WitnessPolicy {
            committee,
            genesis: Head {
                chain: 23,
                epoch: 0,
                commitment: [9; 32],
            },
            authorities: std::array::from_fn(|i| authorities[i].verifying_key().to_bytes()),
            observer: observer.verifying_key().to_bytes(),
            native_scope: None,
        },
        members[0].clone(),
        authorities[0].clone(),
    )
}
#[test]
fn witness_tls_pins_identity_rejects_plaintext_and_bounds_slow_connections() {
    let dir = tempfile::tempdir().unwrap();
    let (server_tls, client_tls) = tls(dir.path());
    let (policy, member, authority) = policy();
    let config = ServiceConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        database: dir.path().join("witness.sqlite"),
        policy: policy.clone(),
        index: 0,
        signing_secret: authority.to_bytes(),
        tls: Some(server_tls),
        test_crash_after_prepare_commit: false,
    };
    let path = dir.path().join("config.json");
    fs::write(&path, bytes(&config)).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_witness-node"))
        .arg(path)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let address = line.trim().parse().unwrap();
    let node = Node { child };
    let legacy = Client::new(address, policy.clone(), 0, member.clone()).unwrap();
    let client = legacy.clone().with_tls(Some(client_tls.clone())).unwrap();
    let slow = TcpStream::connect(address).unwrap();
    let started = Instant::now();
    client
        .call(
            0,
            Operation::Initialize {
                snapshot: vec![1; 64],
            },
        )
        .unwrap();
    assert_eq!(client.get().unwrap().head, policy.genesis);
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(slow);
    assert!(legacy.get().is_err());
    let mut wrong_name = client_tls.clone();
    wrong_name.server_name = "invalid.example".into();
    assert!(legacy
        .clone()
        .with_tls(Some(wrong_name))
        .unwrap()
        .get()
        .is_err());
    let other = tempfile::tempdir().unwrap();
    let (_, other_ca) = tls(other.path());
    assert!(legacy
        .clone()
        .with_tls(Some(other_ca))
        .unwrap()
        .get()
        .is_err());
    let stranger = Client::new(address, policy.clone(), 0, SigningKey::generate(&mut OsRng))
        .unwrap()
        .with_tls(Some(client_tls.clone()))
        .unwrap();
    assert!(stranger.get().is_err());
    let mut connections = Vec::new();
    for _ in 0..32 {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_millis(200)) {
            connections.push(stream);
        }
    }
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string(format!("/proc/{}/status", node.child.id())).unwrap();
        let threads: usize = status
            .lines()
            .find_map(|line| line.strip_prefix("Threads:"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            threads, 9,
            "one acceptor plus exactly eight bounded workers"
        );
    }
    drop(connections);
    let deadline = Instant::now() + Duration::from_secs(7);
    loop {
        if client.get().is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "witness did not recover admission capacity"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // A numeric remote endpoint never attempts unauthenticated plaintext.
    let remote = Client::new("192.0.2.1:23456".parse().unwrap(), policy, 0, member).unwrap();
    assert!(remote.get().is_err());
}
#[test]
fn non_loopback_listener_and_exposed_tls_key_fail_before_opening_authority() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (server_tls, _) = tls(dir.path());
    let (policy, _, authority) = policy();
    let mut config = ServiceConfig {
        listen: "0.0.0.0:23456".parse().unwrap(),
        database: dir.path().join("witness.sqlite"),
        policy,
        index: 0,
        signing_secret: authority.to_bytes(),
        tls: None,
        test_crash_after_prepare_commit: false,
    };
    assert!(Service::open(config.clone()).is_err());
    assert!(!config.database.exists());
    fs::set_permissions(&server_tls.key_file, fs::Permissions::from_mode(0o644)).unwrap();
    config.tls = Some(server_tls);
    assert!(Service::open(config.clone()).is_err());
    assert!(!config.database.exists());
}
