use std::{fs, sync::Arc};
use yami_browser::{native::Resolver, validation::probe};
use yami_identity::NodeIdentity;
#[test]
fn explicit_probe_checks_document_png_and_rejects_wrong_tls_identity() {
    let dir = std::env::temp_dir().join(format!("yami-probe-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let site_dir = dir.join("site");
    yami_node::authoring::new_site(&site_dir, "personal").unwrap();
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let id = *identity.id();
    let listener = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let site = yami_node::Site::open(&site_dir).unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..3 {
            let _ = yami_node::serve_once(&listener, &site);
        }
    });
    let resolver = Resolver::new(dir.join("peers"), yami_dht::Network::Loopback);
    let report = probe(
        &ynp_tls::format_url(&id, endpoint, "home"),
        Some("cover"),
        &resolver,
    )
    .unwrap();
    assert!(report.contains("document PASS"));
    assert!(report.contains("png PASS"));
    assert!(report.contains("scope loopback simulation"));
    assert!(report.contains("independent_networks NOT VERIFIED"));
    let wrong = *NodeIdentity::generate().unwrap().id();
    assert!(probe(
        &ynp_tls::format_url(&wrong, endpoint, "home"),
        None,
        &resolver
    )
    .is_err());
    assert!(probe("ynp://127.0.0.1:1/home", None, &resolver).is_err());
    server.join().unwrap();
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_trial_configuration_returns_redacted_structured_failure() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_yami-browser"))
        .args([
            "--network-trial",
            "yami://not-a-valid-id/home",
            "--config",
            "missing-private-path.conf",
            "--asset",
            "cover",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["result"], "FAIL");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("missing-private-path"));
}

#[test]
fn trial_games_are_opt_in_and_unadvertised_games_never_reach_the_transport() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use yami_browser::network_trial::{self, Checks};
    struct Tracked {
        site: yami_node::Site,
        games: Arc<AtomicUsize>,
    }
    impl yami_node::Documents for Tracked {
        fn lookup(&self, key: &str) -> std::io::Result<Option<ynp::DocumentData>> {
            self.site.lookup(key)
        }
        fn asset(&self, key: &str) -> std::io::Result<Option<Vec<u8>>> {
            self.site.asset(key)
        }
        fn game(&self, _: &str, _: ynp::GameResource) -> std::io::Result<Option<Vec<u8>>> {
            self.games.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }
    }
    let dir = std::env::temp_dir().join(format!("yami-trial-consent-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    yami_node::authoring::new_site(dir.join("site"), "personal").unwrap();
    let key = Arc::new(NodeIdentity::generate().unwrap());
    let id = *key.id();
    let listener = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), key.clone()).unwrap();
    listener.set_nonblocking(true).unwrap();
    let std::net::SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
        unreachable!()
    };
    let service =
        yami_dht::Service::open(dir.join("host.cache"), id, yami_dht::Network::Loopback).unwrap();
    service
        .prepare_self(&key, vec![endpoint], yami_dht::now().unwrap())
        .unwrap();
    let games = Arc::new(AtomicUsize::new(0));
    let documents = Tracked {
        site: yami_node::Site::open(dir.join("site")).unwrap(),
        games: games.clone(),
    };
    let shutdown = yami_node::shutdown::Shutdown::default();
    let stopping = shutdown.clone();
    let server = std::thread::spawn(move || {
        yami_node::serve_with_handler_until(
            &listener,
            2,
            &|c| yami_node::serve_connection_with_dht(c, &documents, &service),
            &|_| {},
            &stopping,
        )
        .unwrap()
    });
    let config = yami_config::Config::parse(&format!(
        "yaminet-config 1\nrole browser\nstate {}\nbootstrap {id}@{endpoint}\n",
        dir.join("client").display()
    ))
    .unwrap();
    let url = format!("yami://{id}/home");
    let (report, passed) = network_trial::run(&config, &url, "cover", None, false, false);
    assert!(passed, "{report}");
    assert!(report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["name"] == "hosted-game" && step["status"] == "NOT RUN"));
    assert_eq!(games.load(Ordering::SeqCst), 0);
    let (report, passed) = network_trial::run_with_checks(
        &config,
        &url,
        "cover",
        None,
        false,
        false,
        Checks {
            game: Some("unadvertised"),
            ..Checks::default()
        },
    );
    assert!(!passed);
    assert!(report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step["name"] == "hosted-game" && step["status"] == "FAIL"));
    assert_eq!(games.load(Ordering::SeqCst), 0);
    shutdown.stop();
    server.join().unwrap();
    fs::remove_dir_all(dir).unwrap();
}
