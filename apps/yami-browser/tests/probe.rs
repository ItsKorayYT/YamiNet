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
