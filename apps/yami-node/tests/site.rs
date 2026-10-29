use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
    thread,
};
use yami_node::{serve_concurrently, Site};
use ynp::{Frame, LocalListener, LoopbackTcp, Message, Transport};

const PAGE: &str = "@page 0.1\n  title Home\n\n= Home\n";
const STYLE: &str = "@style 0.1\ntext\n  ink #ffffff\n";

/// A unique scratch directory, removed when dropped.
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yaminet-site-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn site(&self) -> PathBuf {
        let site = self.0.join("site");
        fs::create_dir_all(&site).unwrap();
        site
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Serve a site in the background for the rest of the test process.
fn serve(root: &Path, workers: usize) -> SocketAddr {
    let site = Site::open(root).unwrap();
    let listener = LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || serve_concurrently(&listener, &site, workers, &|_| {}));
    address
}

/// Send a Get with an arbitrary key, bypassing client-side key validation.
fn raw_get(address: SocketAddr, key: &[u8]) -> io::Result<Frame> {
    let mut connection = LoopbackTcp.connect(address)?;
    let mut bytes = b"YNP\0\x01\x01\0\0".to_vec();
    bytes.extend_from_slice(&(key.len() as u32 + 2).to_be_bytes());
    bytes.extend_from_slice(&7u32.to_be_bytes());
    bytes.extend_from_slice(&(key.len() as u16).to_be_bytes());
    bytes.extend_from_slice(key);
    connection.write_all(&bytes)?;
    ynp::read_frame(&mut *connection)
}

#[test]
fn serves_multiple_pages_with_shared_style() {
    let temp = TempDir::new("multi");
    let root = temp.site();
    for key in ["home", "about", "projects"] {
        fs::write(root.join(format!("{key}.ylang")), PAGE.replace("Home", key)).unwrap();
    }
    fs::write(root.join("default.ysl"), STYLE).unwrap();
    let address = serve(&root, 4);
    for key in ["home", "about", "projects"] {
        let doc = ynp::fetch(&LoopbackTcp, address, key).unwrap();
        assert_eq!(doc.key, key);
        assert!(doc.source.contains(&format!("title {key}")));
        assert_eq!(doc.style, STYLE);
    }
    assert_eq!(
        Site::open(&root).unwrap().pages().unwrap(),
        ["about", "home", "projects"]
    );
}

#[test]
fn edits_new_pages_and_deletions_apply_without_restart() {
    let temp = TempDir::new("update");
    let root = temp.site();
    fs::write(root.join("home.ylang"), PAGE).unwrap();
    let address = serve(&root, 2);
    let first = ynp::fetch(&LoopbackTcp, address, "home").unwrap();
    assert_eq!(first.source, PAGE);
    assert_eq!(first.style, "", "missing stylesheet is served as empty");

    let edited = PAGE.replace("= Home", "= Home, edited");
    fs::write(root.join("home.ylang"), &edited).unwrap();
    fs::write(root.join("default.ysl"), STYLE).unwrap();
    let second = ynp::fetch(&LoopbackTcp, address, "home").unwrap();
    assert_eq!(second.source, edited);
    assert_eq!(second.style, STYLE);

    fs::write(root.join("later.ylang"), PAGE).unwrap();
    assert!(ynp::fetch(&LoopbackTcp, address, "later").is_ok());
    fs::remove_file(root.join("later.ylang")).unwrap();
    assert_eq!(
        ynp::fetch(&LoopbackTcp, address, "later")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    // A leading byte-order mark from a Windows editor is not page content.
    fs::write(root.join("home.ylang"), format!("\u{feff}{PAGE}")).unwrap();
    assert_eq!(
        ynp::fetch(&LoopbackTcp, address, "home").unwrap().source,
        PAGE
    );
}

#[test]
fn missing_pages_receive_a_ynp_not_found_frame() {
    let temp = TempDir::new("missing");
    let root = temp.site();
    fs::write(root.join("home.ylang"), PAGE).unwrap();
    let address = serve(&root, 2);
    let reply = raw_get(address, b"absent").unwrap();
    assert_eq!(reply.request_id, 7);
    assert_eq!(
        reply.message,
        Message::NotFound {
            key: "absent".into()
        }
    );
    assert_eq!(
        ynp::fetch(&LoopbackTcp, address, "absent")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    // Device names and the stylesheet are never pages.
    for key in ["con", "NUL", "com1", "default"] {
        assert!(
            matches!(
                raw_get(address, key.as_bytes()).unwrap().message,
                Message::NotFound { .. }
            ),
            "{key}"
        );
    }
}

#[test]
fn invalid_identifiers_and_escapes_are_rejected() {
    let temp = TempDir::new("escape");
    let root = temp.site();
    fs::write(root.join("home.ylang"), PAGE).unwrap();
    fs::write(temp.0.join("secret.ylang"), "outside the site").unwrap();
    fs::create_dir(root.join("folder.ylang")).unwrap();
    fs::write(root.join("big.ylang"), "x".repeat(ynp::MAX_DOCUMENT + 1)).unwrap();
    fs::write(root.join("binary.ylang"), [0xff, 0xfe, 0x00]).unwrap();
    let address = serve(&root, 2);
    for key in [
        &b"../secret"[..],
        b"..",
        b"",
        b"site/home",
        b"site\\home",
        b"C:home",
        b"home.ylang",
        b"%2e%2e",
        b"home\0",
        b"h\xc3\xa9",
    ] {
        assert!(raw_get(address, key).is_err(), "{key:?} was answered");
    }
    // Present but unservable files close the connection without content.
    // `HOME` must not alias home.ylang on case-insensitive file systems.
    for key in ["folder", "big", "binary", "HOME"] {
        assert!(ynp::fetch(&LoopbackTcp, address, key).is_err(), "{key}");
    }
    let site = Site::open(&root).unwrap();
    assert!(site.lookup("../secret").is_err());
    assert_eq!(site.pages().unwrap(), ["big", "binary", "home"]);
    assert!(ynp::parse_url("ynp://127.0.0.1:7301/../secret").is_err());
    assert!(ynp::parse_url("ynp://127.0.0.1:7301/a/b").is_err());
    // The node keeps serving valid requests afterwards.
    assert!(ynp::fetch(&LoopbackTcp, address, "home").is_ok());
}

#[test]
fn symlinks_cannot_escape_or_alias_pages() {
    let temp = TempDir::new("symlink");
    let root = temp.site();
    fs::write(root.join("home.ylang"), PAGE).unwrap();
    let outside = temp.0.join("secret.ylang");
    fs::write(&outside, "outside the site").unwrap();
    #[cfg(unix)]
    let link = |from: &Path, to: &Path| std::os::unix::fs::symlink(from, to);
    #[cfg(windows)]
    let link = |from: &Path, to: &Path| std::os::windows::fs::symlink_file(from, to);
    if let Err(error) = link(&outside, &root.join("escape.ylang")) {
        // Windows needs Developer Mode or elevation to create symlinks.
        eprintln!("skipping symlink test: {error}");
        return;
    }
    link(&root.join("home.ylang"), &root.join("alias.ylang")).unwrap();
    let address = serve(&root, 2);
    for key in ["escape", "alias"] {
        assert!(ynp::fetch(&LoopbackTcp, address, key).is_err(), "{key}");
    }
    assert!(ynp::fetch(&LoopbackTcp, address, "home").is_ok());
}

#[test]
fn concurrent_requests_and_slow_clients_are_bounded() {
    let temp = TempDir::new("concurrent");
    let root = temp.site();
    fs::write(root.join("home.ylang"), PAGE).unwrap();
    let address = serve(&root, 3);
    // Idle clients occupy two of three workers without blocking the third.
    let idle: Vec<_> = (0..2)
        .map(|_| LoopbackTcp.connect(address).unwrap())
        .collect();
    let clients: Vec<_> = (0..24)
        .map(|_| thread::spawn(move || ynp::fetch(&LoopbackTcp, address, "home")))
        .collect();
    for client in clients {
        assert_eq!(client.join().unwrap().unwrap().source, PAGE);
    }
    drop(idle);
    assert!(Site::open(root.join("home.ylang")).is_err());
    let listener = LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let site = Site::open(&root).unwrap();
    for workers in [0, yami_node::MAX_WORKERS + 1] {
        assert!(serve_concurrently(&listener, &site, workers, &|_| {}).is_err());
    }
}

/// Kills the node process even when an assertion fails.
struct Node(Child);
impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn start_node(root: &str) -> (Node, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(["serve-dir", "127.0.0.1:0", root])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let node = Node(child);
    let mut line = String::new();
    let address = loop {
        line.clear();
        assert!(stderr.read_line(&mut line).unwrap() > 0, "node exited");
        if let Some((_, rest)) = line.split_once(" via YNP on ") {
            break rest.split_whitespace().next().unwrap().parse().unwrap();
        }
    };
    // Keep draining logs so the node never blocks on a full pipe.
    thread::spawn(move || io::copy(&mut stderr, &mut io::sink()));
    (node, address)
}

#[test]
fn cli_serves_both_sample_sites() {
    let (_alice, alice) = start_node("sites/alice");
    let (_bob, bob) = start_node("sites/bob");
    for key in ["home", "about", "projects", "welcome"] {
        let doc = ynp::fetch(&LoopbackTcp, alice, key).unwrap();
        assert!(doc.style.starts_with("@style 0.1"), "{key}");
    }
    for key in ["home", "garden", "notes"] {
        assert!(ynp::fetch(&LoopbackTcp, bob, key).is_ok(), "{key}");
    }
    assert_eq!(
        ynp::fetch(&LoopbackTcp, bob, "projects")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
}

#[test]
fn cli_refuses_remote_addresses_and_missing_sites() {
    for args in [
        ["serve-dir", "0.0.0.0:7301", "sites/alice"],
        ["serve-dir", "192.0.2.1:7301", "sites/alice"],
        ["serve-dir", "127.0.0.1:0", "sites/does-not-exist"],
        ["serve-dir", "127.0.0.1:0", "sites/alice/home.ylang"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
            .args(args)
            .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .output()
            .unwrap();
        assert!(!output.status.success(), "{args:?}");
    }
}

/// Every link in the samples targets a page that exists on the node that a
/// reader would run for that port (7301 = alice, 7302 = bob).
#[test]
fn sample_site_links_resolve() {
    let sites = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../sites"));
    let mut cross_node = 0;
    for (site, port) in [("alice", 7301), ("bob", 7302)] {
        let site_root = sites.join(site);
        let pages = Site::open(&site_root).unwrap().pages().unwrap();
        assert!(pages.len() >= 3, "{site} needs several pages");
        for page in &pages {
            let mut source = String::new();
            fs::File::open(site_root.join(format!("{page}.ylang")))
                .unwrap()
                .read_to_string(&mut source)
                .unwrap();
            for line in source.lines() {
                let Some(link) = line.trim_start().strip_prefix("> ") else {
                    continue;
                };
                let target = link.split(" | ").next().unwrap();
                let (address, key) = ynp::parse_url(target).unwrap();
                let target_site = match address.port() {
                    7301 => "alice",
                    7302 => "bob",
                    other => panic!("{site}/{page} links to unknown port {other}"),
                };
                if address.port() != port {
                    cross_node += 1;
                }
                assert!(
                    sites
                        .join(target_site)
                        .join(format!("{key}.ylang"))
                        .is_file(),
                    "{site}/{page} links to missing {target}"
                );
            }
        }
    }
    assert!(cross_node >= 4);
}
