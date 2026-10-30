//! Exercise the actual outbound-only node CLI, persistent state and clean stop.
#[cfg(unix)]
#[test]
fn host_relay_cli_publishes_fetches_stops_and_restarts_with_new_sequence() {
    use std::{
        fs,
        net::SocketAddr,
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };
    use yami_dht::{Network, Service};
    use yami_identity::NodeIdentity;
    use yami_node::{peers::PeerStore, shutdown::Shutdown};
    use yami_relay::{Limits, Relay, Route};
    struct Scratch(PathBuf);
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
    fn wait(mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !ready() {
            assert!(Instant::now() < deadline, "CLI condition timed out");
            thread::sleep(Duration::from_millis(50));
        }
    }
    let base = std::env::temp_dir().join(format!("yami-host-relay-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let _scratch = Scratch(base.clone());
    let host = NodeIdentity::create(base.join("host-keys")).unwrap();
    let peer = Arc::new(NodeIdentity::create(base.join("peer-keys")).unwrap());
    let relay_key = Arc::new(NodeIdentity::generate().unwrap());
    let relay = Arc::new(
        Relay::bind(
            "127.0.0.1:0".parse().unwrap(),
            relay_key.clone(),
            Limits::default(),
        )
        .unwrap(),
    );
    let signal = Shutdown::default();
    let relay_signal = signal.clone();
    let runner = relay.clone();
    let relay_task = thread::spawn(move || runner.run(relay_signal.flag()).unwrap());
    struct Tasks(Shutdown, Option<thread::JoinHandle<()>>);
    impl Drop for Tasks {
        fn drop(&mut self) {
            self.0.stop();
            self.1.take().unwrap().join().unwrap();
        }
    }
    let _tasks = Tasks(signal, Some(relay_task));
    fs::create_dir_all(base.join("site")).unwrap();
    fs::write(
        base.join("site/home.ylang"),
        "@page 0.1\n  title CLI relay host\n= No inbound listener\n",
    )
    .unwrap();
    // Bind B in-process through the same TLS dispatcher. A is the executable.
    let listener =
        ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), peer.clone()).unwrap();
    let SocketAddr::V4(peer_endpoint) = listener.local_addr().unwrap() else {
        unreachable!()
    };
    listener.set_nonblocking(true).unwrap();
    let dht = Service::open(base.join("peer.cache"), *peer.id(), Network::Loopback).unwrap();
    dht.prepare_self(&peer, vec![peer_endpoint], yami_dht::now().unwrap())
        .unwrap();
    let site = yami_node::Site::open(base.join("site")).unwrap();
    let peer_stop = Shutdown::default();
    let peer_signal = peer_stop.clone();
    let peer_service = dht.clone();
    let peer_task = thread::spawn(move || {
        yami_node::serve_with_handler_until(
            &listener,
            2,
            &|c| yami_node::serve_connection_with_dht(c, &site, &peer_service),
            &|_| {},
            &peer_signal,
        )
        .unwrap()
    });
    let _peer_task = Tasks(peer_stop, Some(peer_task));
    PeerStore::open(base.join("host-peers"))
        .unwrap()
        .add(*peer.id(), peer_endpoint.into())
        .unwrap();
    let relay_endpoint = relay.local_addr().unwrap();
    let pin = format!("{}@{relay_endpoint}", relay_key.id());
    let spawn = || {
        Process(
            Command::new(env!("CARGO_BIN_EXE_yami-node"))
                .arg("host-relay")
                .arg(base.join("site"))
                .arg("--identity")
                .arg(base.join("host-keys"))
                .args(["--relay", &pin, "--dht-local", "--dht-dir"])
                .arg(base.join("host-peers"))
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut previous = 0;
    for _ in 0..2 {
        let mut child = spawn();
        wait(|| {
            dht.snapshot()
                .unwrap()
                .get(*host.id(), yami_dht::now().unwrap())
                .is_some_and(|r| r.sequence() > previous && !r.relay_routes().is_empty())
        });
        let record = dht
            .snapshot()
            .unwrap()
            .get(*host.id(), yami_dht::now().unwrap())
            .unwrap();
        assert!(record.endpoints().is_empty());
        previous = record.sequence();
        assert_eq!(record.relay_routes()[0].relay, *relay_key.id());
        let route = Route {
            relay: *relay_key.id(),
            endpoint: relay_endpoint,
            destination: *host.id(),
        };
        assert!(route
            .fetch("home")
            .unwrap()
            .source
            .contains("No inbound listener"));
        assert!(Command::new("kill")
            .args(["-TERM", &child.0.id().to_string()])
            .status()
            .unwrap()
            .success());
        wait(|| child.0.try_wait().unwrap().is_some());
        assert!(child.0.wait().unwrap().success());
        wait(|| relay.statistics().registrations == 0 && relay.statistics().tunnels == 0);
    }
    for flags in [
        vec![],
        vec!["--relay", "invalid"],
        vec!["--relay", &pin],
        vec!["--relay", &pin, "--dht-local", "--dht-renew-secs", "0"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .arg("host-relay")
            .arg(base.join("site"))
            .arg("--identity")
            .arg(base.join("host-keys"))
            .args(flags)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}
