use std::{process::Command, thread};
use yami_node::{serve_concurrently, serve_once, Site, Store};
use ynp::{DocumentData, LocalListener};

#[test]
fn fetched_document_is_parsed_styled_and_rasterized() {
    let listener = LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let url = format!("ynp://{}/welcome", listener.local_addr().unwrap());
    let mut store = Store::default();
    store
        .insert(DocumentData {
            key: "welcome".into(),
            source: include_str!("../../../examples/welcome.ylang").into(),
            style: include_str!("../../../examples/default.ysl").into(),
        })
        .unwrap();
    let server = thread::spawn(move || serve_once(&listener, &store));
    let image = std::env::temp_dir().join(format!("yaminet-browser-{}.ppm", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_yami-browser"))
        .arg(url)
        .arg("--ppm")
        .arg(&image)
        .output()
        .unwrap();
    server.join().unwrap().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Welcome to YamiNet"));
    assert!(text.contains("Privacy goals"));
    assert!(text.contains("ynp://127.0.0.1:7302/about"));
    let bytes = std::fs::read(&image).unwrap();
    assert!(bytes.starts_with(b"P6\n912 "));
    assert!(bytes.len() > 1000);
    std::fs::remove_file(image).unwrap();
}
#[test]
fn rejects_non_ynp_sources_and_reports_parse_location() {
    let output = Command::new(env!("CARGO_BIN_EXE_yami-browser"))
        .arg("http://127.0.0.1/document")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only YNP"));
    let source = std::env::temp_dir().join(format!("yaminet-invalid-{}.ylang", std::process::id()));
    std::fs::write(&source, "@page 0.1\n  title Test\n=\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yami-browser"))
        .arg(&source)
        .output()
        .unwrap();
    std::fs::remove_file(source).unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("3:1: heading"));
}
#[test]
fn browser_opens_pages_from_a_directory_site_node() {
    let listener = LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let site = Site::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sites/alice")).unwrap();
    thread::spawn(move || serve_concurrently(&listener, &site, 2, &|_| {}));
    for (key, title) in [("home", "Alice's Home"), ("projects", "Alice's Projects")] {
        let output = Command::new(env!("CARGO_BIN_EXE_yami-browser"))
            .arg(format!("ynp://{address}/{key}"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(title), "{text}");
        assert!(text.contains("ynp://127.0.0.1:7302/home"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_yami-browser"))
        .arg(format!("ynp://{address}/missing"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not found"));
}
#[test]
fn opens_ynps_addresses_only_from_the_named_node() {
    use std::sync::Arc;
    let identity = Arc::new(yami_identity::NodeIdentity::generate().unwrap());
    let id = *identity.id();
    let listener = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
    let address = listener.local_addr().unwrap();
    let site = Site::open(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sites/alice")).unwrap();
    thread::spawn(move || serve_concurrently(&listener, &site, 2, &|_| {}));
    let open = |url: String| {
        Command::new(env!("CARGO_BIN_EXE_yami-browser"))
            .arg(url)
            .output()
            .unwrap()
    };
    let output = open(ynp_tls::format_url(&id, address, "projects"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Alice's Projects"));
    assert!(String::from_utf8_lossy(&output.stderr).contains(&format!("Verified node {id}")));

    let other = *yami_identity::NodeIdentity::generate().unwrap().id();
    let output = open(ynp_tls::format_url(&other, address, "projects"));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Authentication failed"), "{stderr}");
    assert!(!stderr.contains("Verified node"), "{stderr}");
}
