use std::{fs, process::Command};
use yami_identity::NodeIdentity;

#[test]
fn setup_is_offline_validated_reuses_keys_and_refuses_config_overwrite() {
    let dir = std::env::temp_dir().join(format!("yami-setup-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let site = dir.join("site");
    let keys = dir.join("private");
    let first = dir.join("first.conf");
    let prepared =
        yami_node::setup::prepare(&site, &keys, &first, Some("personal"), None, &[]).unwrap();
    assert_eq!(prepared.pages, 2);
    assert!(!prepared.config.public_listen);
    assert_eq!(
        prepared.config.listen.unwrap().ip().to_string(),
        "127.0.0.1"
    );
    let key_bytes = fs::read(keys.join("node.key")).unwrap();
    let second =
        yami_node::setup::prepare(&site, &keys, &dir.join("second.conf"), None, None, &[]).unwrap();
    assert_eq!(prepared.id, second.id);
    assert_eq!(key_bytes, fs::read(keys.join("node.key")).unwrap());
    assert!(yami_node::setup::prepare(&site, &keys, &first, None, None, &[]).is_err());
    let result = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(["diagnose", "--config"])
        .arg(&first)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout)
        .contains("No document or discovery requests were sent"));
    let bad = dir.join("invalid.conf");
    assert!(yami_node::setup::prepare(
        &site,
        &dir.join("unused-keys"),
        &bad,
        None,
        Some("wrong-pin"),
        &[]
    )
    .is_err());
    assert!(!bad.exists());
    assert!(!dir.join("unused-keys/node.key").exists());
    assert_eq!(*NodeIdentity::load(&keys).unwrap().id(), prepared.id);
    let existing = dir.join("existing-site");
    fs::create_dir(&existing).unwrap();
    fs::write(
        existing.join("start.ylang"),
        "@page 0.1\n  title Existing site\n= Start here\n",
    )
    .unwrap();
    let existing = yami_node::setup::prepare(
        &existing,
        &keys,
        &dir.join("existing.conf"),
        None,
        None,
        &[],
    )
    .unwrap();
    assert_eq!(existing.entry_key, "start");
    assert_eq!(existing.id, prepared.id);
    fs::remove_dir_all(dir).unwrap();
}
