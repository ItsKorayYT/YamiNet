use std::{process::Command, thread};
use yami_node::{serve_once, Store};
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
