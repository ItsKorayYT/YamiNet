use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use yami_identity::{
    DocumentMetadata, Error, NodeId, NodeIdentity, Purpose, Signature, SignatureFile, SiteManifest,
    ID_FILE, KEY_FILE, NODE_ID_PREFIX,
};
use ynp::DocumentData;

/// A unique scratch directory, removed when dropped.
struct TempDir(PathBuf);
impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "yaminet-identity-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        Self(path)
    }
    fn key(&self) -> PathBuf {
        self.0.join(KEY_FILE)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn document(source: &str) -> DocumentData {
    DocumentData {
        key: "home".into(),
        source: source.into(),
        style: "@style 0.1\n".into(),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn rewrite_key(path: &Path, change: impl FnOnce(&mut Vec<u8>)) {
    let mut bytes = fs::read(path).unwrap();
    change(&mut bytes);
    fs::write(path, bytes).unwrap();
}

#[test]
fn identity_persists_across_restarts() {
    let dir = TempDir::new("persist");
    let created = NodeIdentity::create(&dir.0).unwrap();
    for _ in 0..3 {
        assert_eq!(NodeIdentity::load(&dir.0).unwrap().id(), created.id());
        let (loaded, new) = NodeIdentity::load_or_create(&dir.0).unwrap();
        assert!(!new);
        assert_eq!(loaded.id(), created.id());
    }
    assert_eq!(
        fs::read_to_string(dir.0.join(ID_FILE)).unwrap(),
        format!("{}\n", created.id())
    );
    // A loaded identity signs exactly as the original did.
    let metadata = DocumentMetadata::of(&document("= Home\n")).unwrap();
    let signature = NodeIdentity::load(&dir.0).unwrap().sign(&metadata);
    assert!(created.id().verify(&metadata, &signature).is_ok());
}

#[test]
fn load_or_create_creates_once() {
    let dir = TempDir::new("once");
    let (first, created) = NodeIdentity::load_or_create(&dir.0).unwrap();
    assert!(created);
    let (second, created) = NodeIdentity::load_or_create(&dir.0).unwrap();
    assert!(!created);
    assert_eq!(first.id(), second.id());
}

#[test]
fn create_never_replaces_an_identity() {
    let dir = TempDir::new("replace");
    let original = NodeIdentity::create(&dir.0).unwrap();
    let key = fs::read(dir.key()).unwrap();
    assert!(matches!(
        NodeIdentity::create(&dir.0),
        Err(Error::AlreadyExists(_))
    ));
    assert_eq!(fs::read(dir.key()).unwrap(), key);
    assert_eq!(NodeIdentity::load(&dir.0).unwrap().id(), original.id());
}

#[test]
fn lost_key_is_not_silently_replaced() {
    let dir = TempDir::new("lost");
    NodeIdentity::create(&dir.0).unwrap();
    fs::remove_file(dir.key()).unwrap();
    assert!(matches!(
        NodeIdentity::load_or_create(&dir.0),
        Err(Error::MissingKey(_))
    ));
    assert!(!dir.key().exists());
}

#[test]
fn different_nodes_have_different_ids() {
    let a = NodeIdentity::generate().unwrap();
    let b = NodeIdentity::generate().unwrap();
    assert_ne!(a.id(), b.id());
}

#[test]
fn node_ids_round_trip_and_reject_damage() {
    let id = *NodeIdentity::generate().unwrap().id();
    let text = id.to_string();
    assert!(text.starts_with(NODE_ID_PREFIX));
    assert_eq!(text.len(), NODE_ID_PREFIX.len() + 58);
    assert_eq!(text.parse::<NodeId>().unwrap(), id);
    assert_eq!(id.public_key_hex().len(), 64);

    let body = &text[NODE_ID_PREFIX.len()..];
    let mut damaged = vec![
        String::new(),
        body.to_owned(),
        text.to_uppercase(),
        format!("yn2-{body}"),
        format!("{text}a"),
        text[..text.len() - 1].to_owned(),
        format!(" {text}"),
    ];
    // Every single-character substitution must be caught by the checksum or
    // the canonical-encoding rules.
    for index in NODE_ID_PREFIX.len()..text.len() {
        let mut bytes = text.clone().into_bytes();
        bytes[index] = if bytes[index] == b'a' { b'b' } else { b'a' };
        damaged.push(String::from_utf8(bytes).unwrap());
    }
    for text in damaged {
        assert!(
            matches!(text.parse::<NodeId>(), Err(Error::InvalidNodeId(_))),
            "{text}"
        );
    }
}

#[test]
fn valid_document_signature_verifies() {
    let identity = NodeIdentity::generate().unwrap();
    let metadata = DocumentMetadata::of(&document("= Home\n")).unwrap();
    let signature = identity.sign(&metadata);
    assert!(identity.id().verify(&metadata, &signature).is_ok());
    // Ed25519 signing is deterministic.
    assert_eq!(identity.sign(&metadata), signature);
}

#[test]
fn modified_content_fails_verification() {
    let identity = NodeIdentity::generate().unwrap();
    let original = document("= Home\n");
    let signature = identity.sign(&DocumentMetadata::of(&original).unwrap());
    let mut changed_source = original.clone();
    changed_source.source.push('!');
    let mut changed_style = original.clone();
    changed_style.style.push(' ');
    let mut changed_key = original.clone();
    changed_key.key = "home2".into();
    for modified in [changed_source, changed_style, changed_key] {
        let metadata = DocumentMetadata::of(&modified).unwrap();
        assert!(matches!(
            identity.id().verify(&metadata, &signature),
            Err(Error::BadSignature)
        ));
    }
}

#[test]
fn modified_site_manifest_fails_verification() {
    let identity = NodeIdentity::generate().unwrap();
    let page = |key: &str, digest| (key.to_owned(), [digest; 32]);
    let manifest = SiteManifest::new([0; 32], [page("home", 1), page("about", 2)]).unwrap();
    let signature = identity.sign(&manifest);
    assert!(identity.id().verify(&manifest, &signature).is_ok());
    // Input order does not matter; the encoding is canonical.
    let reordered = SiteManifest::new([0; 32], [page("about", 2), page("home", 1)]).unwrap();
    assert!(identity.id().verify(&reordered, &signature).is_ok());
    for modified in [
        SiteManifest::new([0; 32], [page("home", 1)]).unwrap(),
        SiteManifest::new([0; 32], [page("home", 1), page("about", 3)]).unwrap(),
        SiteManifest::new([0; 32], [page("home", 1), page("about", 2), page("x", 4)]).unwrap(),
        SiteManifest::new([9; 32], [page("home", 1), page("about", 2)]).unwrap(),
    ] {
        assert!(identity.id().verify(&modified, &signature).is_err());
    }
}

#[test]
fn incorrect_key_fails_verification() {
    let signer = NodeIdentity::generate().unwrap();
    let other = NodeIdentity::generate().unwrap();
    let metadata = DocumentMetadata::of(&document("= Home\n")).unwrap();
    let signature = signer.sign(&metadata);
    assert!(matches!(
        other.id().verify(&metadata, &signature),
        Err(Error::BadSignature)
    ));
}

#[test]
fn tampered_signatures_fail() {
    let identity = NodeIdentity::generate().unwrap();
    let metadata = DocumentMetadata::of(&document("= Home\n")).unwrap();
    let text = identity.sign(&metadata).to_string();
    for index in 0..text.len() {
        let mut bytes = text.clone().into_bytes();
        bytes[index] = if bytes[index] == b'0' { b'1' } else { b'0' };
        let tampered: Signature = String::from_utf8(bytes).unwrap().parse().unwrap();
        assert!(identity.id().verify(&metadata, &tampered).is_err());
    }
    for bad in ["", "00", &text.to_uppercase(), &format!("{text}0")] {
        assert!(bad.parse::<Signature>().is_err());
    }
}

#[test]
fn signature_files_round_trip_and_check_signer_and_purpose() {
    let identity = NodeIdentity::generate().unwrap();
    let other = NodeIdentity::generate().unwrap();
    let metadata = DocumentMetadata::of(&document("= Home\n")).unwrap();
    let signed = SignatureFile::sign(&identity, &metadata);
    let text = signed.to_string();
    assert_eq!(SignatureFile::parse(&text).unwrap(), signed);
    // Line endings converted by Windows tools are tolerated.
    let crlf = SignatureFile::parse(&text.replace('\n', "\r\n")).unwrap();
    assert!(crlf.verify(&metadata, identity.id()).is_ok());

    assert!(matches!(
        signed.verify(&metadata, other.id()),
        Err(Error::Mismatch { .. })
    ));
    // Relabelling the purpose is caught before verification.
    let relabelled = text.replace(
        Purpose::DocumentMetadata.name(),
        Purpose::SiteManifest.name(),
    );
    let manifest = SiteManifest::new([0; 32], []).unwrap();
    assert!(matches!(
        SignatureFile::parse(&relabelled)
            .unwrap()
            .verify(&manifest, identity.id()),
        Err(Error::BadSignature)
    ));
    assert!(matches!(
        signed.verify(&manifest, identity.id()),
        Err(Error::WrongPurpose { .. })
    ));
    // Claiming another signer does not transfer the signature.
    let forged = text.replace(&identity.id().to_string(), &other.id().to_string());
    assert!(matches!(
        SignatureFile::parse(&forged)
            .unwrap()
            .verify(&metadata, other.id()),
        Err(Error::BadSignature)
    ));

    for malformed in [
        String::new(),
        text.trim_end().to_owned(),
        format!("{text}extra\n"),
        text.replace("yaminet-signature 1", "yaminet-signature 2"),
        text.replace("purpose ", "purpose  "),
        text.replace(Purpose::DocumentMetadata.name(), "yaminet.other.v1"),
        text.replace("signer", "author"),
        "x".repeat(SignatureFile::MAX_LEN + 1),
    ] {
        assert!(SignatureFile::parse(&malformed).is_err(), "{malformed:?}");
    }
}

#[test]
fn corrupted_key_files_are_rejected() {
    type Change = Box<dyn FnOnce(&mut Vec<u8>)>;
    let cases: Vec<(&str, Change)> = vec![
        ("empty", Box::new(|bytes| bytes.clear())),
        ("truncated", Box::new(|bytes| bytes.truncate(40))),
        ("extended", Box::new(|bytes| bytes.push(0))),
        ("magic", Box::new(|bytes| bytes[0] ^= 1)),
        ("version", Box::new(|bytes| bytes[4] = 2)),
        ("algorithm", Box::new(|bytes| bytes[5] = 9)),
        ("reserved", Box::new(|bytes| bytes[7] = 1)),
        ("secret", Box::new(|bytes| bytes[20] ^= 0x80)),
        ("public", Box::new(|bytes| bytes[60] ^= 0x80)),
    ];
    for (name, change) in cases {
        let dir = TempDir::new(name);
        NodeIdentity::create(&dir.0).unwrap();
        rewrite_key(&dir.key(), change);
        assert!(
            matches!(NodeIdentity::load(&dir.0), Err(Error::CorruptKey(_))),
            "{name}"
        );
        // A damaged identity is never replaced by a new one.
        assert!(NodeIdentity::load_or_create(&dir.0).is_err(), "{name}");
    }
}

#[test]
fn mismatched_identity_files_are_rejected() {
    let dir = TempDir::new("mismatch");
    let identity = NodeIdentity::create(&dir.0).unwrap();
    let other = NodeIdentity::generate().unwrap();
    fs::write(dir.0.join(ID_FILE), format!("{}\n", other.id())).unwrap();
    match NodeIdentity::load(&dir.0) {
        Err(Error::Mismatch { expected, found }) => {
            assert_eq!(expected, *other.id());
            assert_eq!(found, *identity.id());
        }
        result => panic!("expected mismatch, got {result:?}"),
    }
    fs::write(dir.0.join(ID_FILE), "not an id\n").unwrap();
    assert!(matches!(
        NodeIdentity::load(&dir.0),
        Err(Error::InvalidNodeId(_))
    ));
    // The ID file is derivable; without it the key alone is authoritative.
    fs::remove_file(dir.0.join(ID_FILE)).unwrap();
    assert_eq!(NodeIdentity::load(&dir.0).unwrap().id(), identity.id());
}

#[test]
fn secret_key_never_appears_in_public_output() {
    let dir = TempDir::new("secret");
    let identity = NodeIdentity::create(&dir.0).unwrap();
    let key_file = fs::read(dir.key()).unwrap();
    let secret = hex(&key_file[8..40]);
    let metadata = DocumentMetadata::of(&document("= Home\n")).unwrap();
    let outputs = [
        format!("{identity:?}"),
        format!("{:?}", NodeIdentity::load(&dir.0).unwrap()),
        identity.id().to_string(),
        identity.id().public_key_hex(),
        SignatureFile::sign(&identity, &metadata).to_string(),
        fs::read_to_string(dir.0.join(ID_FILE)).unwrap(),
    ];
    for output in outputs {
        assert!(!output.contains(&secret), "{output}");
    }
    // Errors about the key file describe it without quoting it.
    rewrite_key(&dir.key(), |bytes| bytes[63] ^= 1);
    let error = NodeIdentity::load(&dir.0).unwrap_err().to_string();
    assert!(!error.contains(&secret), "{error}");
}

#[cfg(unix)]
#[test]
fn key_files_are_owner_only_and_lax_permissions_are_refused() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new("permissions");
    NodeIdentity::create(&dir.0).unwrap();
    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir.key()), 0o600);
    assert_eq!(mode(&dir.0), 0o700);
    fs::set_permissions(dir.key(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        NodeIdentity::load(&dir.0),
        Err(Error::InsecurePermissions(_))
    ));
}

#[test]
fn oversized_public_id_cannot_hide_trailing_content() {
    let dir = std::env::temp_dir().join(format!("yami-id-oversize-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let identity = NodeIdentity::create(&dir).unwrap();
    std::fs::write(
        dir.join("node.id"),
        format!("{}{}secret", identity.id(), "\n".repeat(200)),
    )
    .unwrap();
    assert!(NodeIdentity::load(&dir).is_err());
    let _ = std::fs::remove_dir_all(dir);
}
#[cfg(unix)]
#[test]
fn identity_rejects_linked_key_paths_and_never_formats_key_bytes() {
    let dir = std::env::temp_dir().join(format!("yami-id-linked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    NodeIdentity::create(&dir).unwrap();
    std::fs::rename(dir.join("node.key"), dir.join("private.key")).unwrap();
    std::os::unix::fs::symlink(dir.join("private.key"), dir.join("node.key")).unwrap();
    assert!(NodeIdentity::load(&dir)
        .unwrap_err()
        .to_string()
        .contains("non-symlink"));
    std::fs::remove_file(dir.join("node.key")).unwrap();
    std::fs::hard_link(dir.join("private.key"), dir.join("node.key")).unwrap();
    assert!(NodeIdentity::load(&dir)
        .unwrap_err()
        .to_string()
        .contains("hard link"));
    let _ = std::fs::remove_dir_all(dir);
}
