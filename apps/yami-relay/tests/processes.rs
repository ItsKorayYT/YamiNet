//! Real relay, outbound-only site host, and client executables on loopback.
//! This simulates NAT's connection direction; it is not internet/NAT validation.
use std::{
    fs,
    io::{self, BufRead, BufReader},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use yami_identity::NodeIdentity;
use yami_relay::Route;

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("yami-relay-processes-{}", std::process::id()));
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
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn relay(keys: &Path, endpoint: SocketAddr) -> (Process, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-relay"))
        .args(["serve", &endpoint.to_string(), "--identity"])
        .arg(keys)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stderr.take().unwrap());
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if line.starts_with("Relay listening on ") {
                let address: SocketAddr = line.split_whitespace().nth(3).unwrap().parse().unwrap();
                sender.send(address).unwrap();
                break;
            }
            line.clear();
        }
        let _ = io::copy(&mut reader, &mut io::sink());
    });
    let running = Process(child);
    let address = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("relay startup");
    (running, address)
}
fn host(keys: &Path, site: &Path, route: Route) -> (Process, mpsc::Receiver<()>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-relay"))
        .args([
            "host",
            &route.relay.to_string(),
            &route.endpoint.to_string(),
        ])
        .arg(site)
        .arg("--identity")
        .arg(keys)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stderr.take().unwrap());
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if line.starts_with("Registered outbound host ") {
                let _ = sender.try_send(());
            }
            line.clear();
        }
    });
    let running = Process(child);
    receiver
        .recv_timeout(Duration::from_secs(8))
        .expect("outbound registration");
    (running, receiver)
}
fn fetch(route: Route, key: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yami-relay"))
        .args([
            "fetch",
            &route.relay.to_string(),
            &route.endpoint.to_string(),
            &route.destination.to_string(),
            key,
        ])
        .output()
        .unwrap()
}
#[test]
fn three_processes_fetch_reconnect_and_close_when_host_disconnects() {
    // Run a deliberately dishonest host in a separate invocation of this test
    // executable. No production CLI option can change its TLS signing identity.
    if let Ok(directory) = std::env::var("YAMINET_RELAY_SUBSTITUTE_FIXTURE") {
        let identity = NodeIdentity::load(Path::new(&directory).join("host")).unwrap();
        let wrong = std::sync::Arc::new(
            NodeIdentity::load(Path::new(&directory).join("substitute")).unwrap(),
        );
        let relay_id = NodeIdentity::load(Path::new(&directory).join("relay")).unwrap();
        let endpoint: SocketAddr = std::env::var("YAMINET_RELAY_FIXTURE_ENDPOINT")
            .unwrap()
            .parse()
            .unwrap();
        let route = Route {
            relay: *relay_id.id(),
            endpoint,
            destination: *identity.id(),
        };
        let until = Instant::now() + Duration::from_secs(3);
        let registration = loop {
            match yami_relay::Registration::connect(route, &identity) {
                Ok(registration) => break registration,
                Err(_) => {
                    assert!(Instant::now() < until);
                    thread::sleep(Duration::from_millis(50));
                }
            }
        };
        eprintln!("Substituting host registered");
        let acceptor = ynp_tls::TlsAcceptor::new(wrong).unwrap();
        assert!(
            registration
                .accept(&acceptor, &std::sync::atomic::AtomicBool::new(false))
                .is_err(),
            "client must reject the substituted certificate before requesting YNP"
        );
        return;
    }
    let scratch = Scratch::new();
    let relay_keys = scratch.0.join("relay");
    let host_keys = scratch.0.join("host");
    let relay_id = NodeIdentity::create(&relay_keys).unwrap();
    let host_id = NodeIdentity::create(&host_keys).unwrap();
    let site = scratch.0.join("site");
    fs::create_dir_all(&site).unwrap();
    // Multiple inner/outer TLS records exercise bounded forwarding buffers.
    let source = format!(
        "@page 0.1\n  title Reverse relay site\n= Secret document canary\n{}",
        format!("{}\n", "opaque payload ".repeat(200)).repeat(60)
    );
    fs::write(site.join("home.ylang"), &source).unwrap();
    let (relay_process, endpoint) = relay(&relay_keys, "127.0.0.1:0".parse().unwrap());
    let route = Route {
        relay: *relay_id.id(),
        endpoint,
        destination: *host_id.id(),
    };
    let (host_process, registered) = host(&host_keys, &site, route);
    let output = fetch(route, "home");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), source);
    let document = ylang::parse(&source).unwrap();
    assert_eq!(document.metadata.title, "Reverse relay site");
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!(
        "Verified destination {} over end-to-end TLS 1.3",
        host_id.id()
    )));
    registered
        .recv_timeout(Duration::from_secs(5))
        .expect("host renews its outbound slot after serving");
    // A wrong relay pin fails before any tunnel or YNP request can be trusted.
    let wrong = NodeIdentity::generate().unwrap();
    let output = fetch(
        Route {
            relay: *wrong.id(),
            ..route
        },
        "home",
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not the trusted node"));
    let output = fetch(
        Route {
            destination: *wrong.id(),
            ..route
        },
        "home",
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no live registration"));
    // The host survives relay loss and reacquires a registration without restart.
    drop(relay_process);
    let (_relay_process, returned) = relay(&relay_keys, endpoint);
    assert_eq!(returned, endpoint);
    registered
        .recv_timeout(Duration::from_secs(8))
        .expect("host reconnects after relay restart");
    let output = fetch(route, "home");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    registered.recv_timeout(Duration::from_secs(5)).unwrap();
    // Pair a tunnel without starting inner TLS; disconnecting its outbound host
    // must close the client tunnel rather than leave a hanging registration.
    let mut tunnel = route.open_tunnel().unwrap();
    tunnel.read_timeout(Duration::from_secs(3)).unwrap();
    let started = Instant::now();
    drop(host_process);
    let result = tunnel.read(&mut [0; 1]);
    assert!(matches!(result, Ok(0) | Err(_)));
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(tunnel);

    let substitute = NodeIdentity::create(scratch.0.join("substitute")).unwrap();
    let mut fixture = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "three_processes_fetch_reconnect_and_close_when_host_disconnects",
            "--nocapture",
        ])
        .env("YAMINET_RELAY_SUBSTITUTE_FIXTURE", &scratch.0)
        .env("YAMINET_RELAY_FIXTURE_ENDPOINT", endpoint.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(fixture.stderr.take().unwrap());
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if line.starts_with("Substituting host registered") {
                sender.send(()).unwrap();
                break;
            }
            line.clear();
        }
        let _ = io::copy(&mut reader, &mut io::sink());
    });
    let mut fixture = Process(fixture);
    receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("substituting host ready");
    let output = fetch(route, "home");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("not the trusted node"), "{error}");
    assert!(error.contains(&host_id.id().to_string()), "{error}");
    assert!(error.contains(&substitute.id().to_string()), "{error}");
    assert!(fixture.0.wait().unwrap().success());
}
