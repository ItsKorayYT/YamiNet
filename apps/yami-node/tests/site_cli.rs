//! Exercises the shipped CLI and real authenticated loopback preview process.
use std::{
    fs,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
use ynp::Transport;
struct Preview(Child);
impl Drop for Preview {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn cli_creates_checks_builds_and_previews_without_exporting_identity() {
    let root = std::env::temp_dir().join(format!("yami-site-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    let site = root.join("site");
    let built = root.join("built");
    let identity = root.join("identity");
    let run = |args: &[&std::ffi::OsStr]| {
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    run(&[
        "site".as_ref(),
        "new".as_ref(),
        site.as_os_str(),
        "portfolio".as_ref(),
    ]);
    run(&["site".as_ref(), "check".as_ref(), site.as_os_str()]);
    run(&["site".as_ref(), "info".as_ref(), site.as_os_str()]);
    fs::write(site.join("private.key"), "exclude me").unwrap();
    run(&[
        "site".as_ref(),
        "build".as_ref(),
        site.as_os_str(),
        built.as_os_str(),
    ]);
    assert!(!built.join("private.key").exists());
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .arg("site")
        .arg("preview")
        .arg(&built)
        .arg("--identity")
        .arg(&identity)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut preview = Preview(child);
    let (send, receive) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let line = line.unwrap();
            if let Some(url) = line.trim().strip_prefix("ynps://") {
                let _ = send.send(format!("ynps://{url}"));
            }
        }
    });
    let mut home = String::new();
    for _ in 0..2 {
        let url = receive.recv_timeout(Duration::from_secs(5)).unwrap();
        if url.ends_with("/home") {
            home = url;
        }
    }
    assert!(!home.is_empty());
    let (id, endpoint, key) = ynp_tls::parse_url(&home).unwrap();
    assert!(endpoint.ip().is_loopback());
    let doc = ynp::fetch(&ynp_tls::TlsTransport::new(id).unwrap(), endpoint, &key).unwrap();
    assert!(doc.source.starts_with("@page 0.2"));
    let mut connection = ynp_tls::TlsTransport::new(id)
        .unwrap()
        .connect(endpoint)
        .unwrap();
    let image = ynp::request_asset(&mut *connection, "cover", ynp::MAX_ASSET_BYTES).unwrap();
    assert_eq!(
        yami_render::assets::decode_png(&image.bytes, yami_render::assets::MAX_DECODED)
            .unwrap()
            .width,
        240
    );
    drop(connection);
    assert_eq!(
        *yami_identity::NodeIdentity::load(&identity).unwrap().id(),
        id
    );
    preview.0.kill().unwrap();
    preview.0.wait().unwrap();
    reader.join().unwrap();
    // The same explicit identity survives the next preview/startup.
    assert_eq!(
        *yami_identity::NodeIdentity::load(&identity).unwrap().id(),
        id
    );
    fs::remove_dir_all(root).unwrap();
}
