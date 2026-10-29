use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};
use yami_identity::{Error, NodeIdentity, KEY_FILE};
use yami_node::{sign_site, verify_site, Site, SIGNATURE_FILE};

const PAGE: &str = "@page 0.1\n  title Home\n\n= Home\n";
const STYLE: &str = "@style 0.1\ntext\n  ink #ffffff\n";

/// A unique scratch directory with a two-page site, removed when dropped.
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yaminet-node-identity-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        let dir = Self(path);
        fs::create_dir_all(dir.site()).unwrap();
        fs::write(dir.site().join("home.ylang"), PAGE).unwrap();
        fs::write(dir.site().join("about.ylang"), PAGE).unwrap();
        fs::write(dir.site().join("default.ysl"), STYLE).unwrap();
        dir
    }
    fn site(&self) -> PathBuf {
        self.0.join("site")
    }
    fn keys(&self) -> PathBuf {
        self.0.join("identity")
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn node(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(args)
        .output()
        .unwrap()
}
fn path(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn signed_site_verifies_until_it_changes() {
    let dir = TempDir::new("changes");
    let identity = NodeIdentity::create(dir.keys()).unwrap();
    let site = Site::open(dir.site()).unwrap();
    assert!(verify_site(&site, identity.id()).is_err(), "unsigned site");
    sign_site(&site, &identity).unwrap();
    assert!(verify_site(&site, identity.id()).is_ok());

    type Edit = (&'static str, fn(&Path));
    let edits: [Edit; 4] = [
        ("edited page", |site| {
            fs::write(site.join("home.ylang"), format!("{PAGE}More\n")).unwrap()
        }),
        ("added page", |site| {
            fs::write(site.join("extra.ylang"), PAGE).unwrap()
        }),
        ("removed page", |site| {
            fs::remove_file(site.join("about.ylang")).unwrap()
        }),
        ("edited style", |site| {
            fs::write(site.join("default.ysl"), "@style 0.1\n").unwrap()
        }),
    ];
    for (name, edit) in edits {
        let dir = TempDir::new("edit");
        let site = Site::open(dir.site()).unwrap();
        sign_site(&site, &identity).unwrap();
        edit(&dir.site());
        assert!(
            matches!(verify_site(&site, identity.id()), Err(Error::BadSignature)),
            "{name}"
        );
        // Signing again after the change makes it valid again.
        sign_site(&site, &identity).unwrap();
        assert!(verify_site(&site, identity.id()).is_ok(), "{name}");
    }
}

#[test]
fn site_signed_by_another_node_is_a_mismatch() {
    let dir = TempDir::new("other");
    let site = Site::open(dir.site()).unwrap();
    let signer = NodeIdentity::generate().unwrap();
    let expected = NodeIdentity::generate().unwrap();
    sign_site(&site, &signer).unwrap();
    assert!(matches!(
        verify_site(&site, expected.id()),
        Err(Error::Mismatch { .. })
    ));
}

#[test]
fn site_signature_is_never_served() {
    let dir = TempDir::new("served");
    let site = Site::open(dir.site()).unwrap();
    sign_site(&site, &NodeIdentity::generate().unwrap()).unwrap();
    assert_eq!(site.pages().unwrap(), ["about", "home"]);
    assert!(site.lookup("site").unwrap().is_none());
}

#[test]
fn cli_identity_is_stable_and_never_prints_the_secret() {
    let dir = TempDir::new("cli");
    let keys = path(&dir.keys()).to_owned();
    let created = node(&["identity", "new", "--dir", &keys]);
    assert!(created.status.success(), "{created:?}");
    let created = String::from_utf8(created.stdout).unwrap();
    let id = created
        .lines()
        .find_map(|line| line.strip_prefix("Node ID"))
        .unwrap()
        .trim()
        .to_owned();
    assert!(id.starts_with("yn1-"));

    let secret: String = fs::read(dir.keys().join(KEY_FILE)).unwrap()[8..40]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut outputs = vec![created];
    for _ in 0..2 {
        let shown = node(&["identity", "show", "--dir", &keys]);
        assert!(shown.status.success());
        let stdout = String::from_utf8(shown.stdout).unwrap();
        assert!(stdout.contains(&id), "identity changed: {stdout}");
        outputs.push(stdout);
        outputs.push(String::from_utf8(shown.stderr).unwrap());
    }
    let inspected = node(&["identity", "inspect", &id]);
    assert!(inspected.status.success());
    outputs.push(String::from_utf8(inspected.stdout).unwrap());
    for output in outputs {
        assert!(!output.contains(&secret), "{output}");
    }

    // Creating again must fail and leave the identity untouched.
    assert!(!node(&["identity", "new", "--dir", &keys]).status.success());
    assert_eq!(NodeIdentity::load(dir.keys()).unwrap().id().to_string(), id);
}

#[test]
fn cli_signs_and_verifies_sites() {
    let dir = TempDir::new("cli-site");
    let (keys, site) = (path(&dir.keys()).to_owned(), path(&dir.site()).to_owned());
    assert!(node(&["identity", "new", "--dir", &keys]).status.success());
    let id = NodeIdentity::load(dir.keys()).unwrap().id().to_string();
    let other = NodeIdentity::generate().unwrap().id().to_string();

    assert!(!node(&["identity", "verify-site", &site, &id])
        .status
        .success());
    let signed = node(&["identity", "sign-site", &site, "--dir", &keys]);
    assert!(signed.status.success(), "{signed:?}");
    assert!(dir.site().join(SIGNATURE_FILE).is_file());
    let verified = node(&["identity", "verify-site", &site, &id]);
    assert!(verified.status.success(), "{verified:?}");

    let wrong = node(&["identity", "verify-site", &site, &other]);
    assert!(!wrong.status.success());
    assert!(String::from_utf8(wrong.stderr)
        .unwrap()
        .contains("mismatch"));

    fs::write(dir.site().join("home.ylang"), format!("{PAGE}Changed\n")).unwrap();
    let modified = node(&["identity", "verify-site", &site, &id]);
    assert!(!modified.status.success());
    assert!(String::from_utf8(modified.stderr)
        .unwrap()
        .contains("not valid"));
}

#[test]
fn cli_rejects_bad_identity_input() {
    let dir = TempDir::new("cli-bad");
    let keys = path(&dir.keys()).to_owned();
    for args in [
        vec!["identity"],
        vec!["identity", "inspect", "yn1-notanid"],
        vec!["identity", "show", "--dir", &keys],
        vec!["identity", "verify-site", path(&dir.site()), "yn1-x"],
        vec!["identity", "new", "--dir"],
    ] {
        assert!(!node(&args).status.success(), "{args:?}");
    }
    // A corrupted key is reported, not replaced.
    NodeIdentity::create(dir.keys()).unwrap();
    fs::write(dir.keys().join(KEY_FILE), b"YNSK").unwrap();
    let output = node(&["identity", "show", "--dir", &keys]);
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("corrupt"));
    assert_eq!(fs::read(dir.keys().join(KEY_FILE)).unwrap(), b"YNSK");
}
