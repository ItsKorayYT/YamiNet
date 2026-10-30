use std::{fs, path::PathBuf, sync::Arc, thread};
use yami_identity::NodeIdentity;
use yami_node::{Reply, Site};
use ynp::{Incoming, Transport};
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("yami-assets-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("assets")).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn directory_assets_use_tls_v2_and_reject_paths_aliases_and_plaintext() {
    let scratch = Scratch::new();
    let bytes = yami_render::assets::encode_png(2, 1, &[20, 40, 60, 255].repeat(2)).unwrap();
    fs::write(scratch.0.join("assets/cover.png"), &bytes).unwrap();
    let site = Site::open(&scratch.0).unwrap();
    assert_eq!(site.assets().unwrap(), vec!["cover"]);
    for key in ["../cover", "cover.png", "NUL", "com1", "a/b"] {
        assert!(site.asset(key).is_err());
    }
    fs::write(scratch.0.join("assets/corrupt.png"), b"not png").unwrap();
    assert!(site.asset("corrupt").is_err());
    let id = Arc::new(NodeIdentity::generate().unwrap());
    let identity = *id.id();
    let listener = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), id).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut results = Vec::new();
        for _ in 0..2 {
            let mut connection = listener.accept().unwrap();
            results.push(yami_node::serve_connection(&mut *connection, &site).unwrap());
        }
        results
    });
    let mut connection = ynp_tls::TlsTransport::new(identity)
        .unwrap()
        .connect(endpoint)
        .unwrap();
    assert_eq!(
        ynp::request_asset(&mut *connection, "cover", ynp::MAX_ASSET_BYTES)
            .unwrap()
            .bytes,
        bytes
    );
    drop(connection);
    let mut connection = ynp_tls::TlsTransport::new(identity)
        .unwrap()
        .connect(endpoint)
        .unwrap();
    assert_eq!(
        ynp::request_asset(&mut *connection, "missing", ynp::MAX_ASSET_BYTES)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    drop(connection);
    assert_eq!(
        server.join().unwrap(),
        vec![
            Reply::Asset("cover".into()),
            Reply::AssetNotFound("missing".into())
        ]
    );
    let site = Site::open(&scratch.0).unwrap();
    let listener = ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut connection = listener.accept().unwrap();
        yami_node::serve_connection(&mut *connection, &site).is_err()
    });
    let mut raw = std::net::TcpStream::connect(address).unwrap();
    ynp::write_frame(
        &mut raw,
        &ynp::Frame {
            request_id: 2,
            message: ynp::Message::GetAsset {
                key: "cover".into(),
            },
        },
    )
    .unwrap();
    assert!(ynp::read_frame(&mut raw).is_err());
    assert!(server.join().unwrap());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            scratch.0.join("assets/cover.png"),
            scratch.0.join("assets/alias.png"),
        )
        .unwrap();
        assert!(Site::open(&scratch.0).unwrap().asset("alias").is_err());
    }
}
