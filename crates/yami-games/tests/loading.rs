use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use yami_games::*;
static IDS: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yami-game-{}-{}",
            std::process::id(),
            IDS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn manifest(&self, body: &str) -> PathBuf {
        let path = self.0.join("test.yami-game.json");
        fs::write(&path, body).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn manifests_are_versioned_bounded_and_local_only() {
    let f = Fixture::new();
    fs::write(f.0.join("test.wasm"), b"module bytes").unwrap();
    for body in ["{}".to_owned(),"[1,2]".into()," ".repeat(4097),
        r#"{"version":2,"abi":1,"title":"X","description":"","module":"test.wasm"}"#.into(),
        r#"{"version":1,"abi":2,"title":"X","description":"","module":"test.wasm"}"#.into(),
        r#"{"version":1,"abi":1,"title":"X","description":"","module":"test.wasm","capabilities":["wasi"]}"#.into(),
    ] {assert!(load_local(&f.manifest(&body)).is_err());}
    for module in [
        "https://example.org/game.wasm",
        "ynps://node/game",
        "yami://id/game",
        "../escape.wasm",
        "/tmp/game.wasm",
        "C:\\game.wasm",
        "./test.wasm",
        "",
    ] {
        let body =
            serde_json::json!({"version":1,"abi":1,"title":"X","description":"","module":module})
                .to_string();
        assert!(load_local(&f.manifest(&body)).is_err(), "{module}");
    }
    let valid = r#"{"version":1,"abi":1,"title":"X","description":"Test","module":"test.wasm"}"#;
    assert_eq!(load_local(&f.manifest(valid)).unwrap().1, b"module bytes");
    fs::write(f.0.join("test.wasm"), vec![0; MAX_MODULE_BYTES + 1]).unwrap();
    assert!(load_local(&f.manifest(valid)).is_err());
}
#[cfg(unix)]
#[test]
fn module_symlinks_cannot_escape_package() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = Fixture::new();
    fs::write(outside.0.join("outside.wasm"), b"bytes").unwrap();
    symlink(outside.0.join("outside.wasm"), f.0.join("test.wasm")).unwrap();
    assert!(load_local(
        &f.manifest(r#"{"version":1,"abi":1,"title":"X","description":"","module":"test.wasm"}"#)
    )
    .is_err());
}
