use std::{fs, sync::Arc, thread};
use yami_identity::NodeIdentity;
use yami_node::Site;
use ynp::{GameResource, Incoming, Transport};
#[test]
fn games_require_tls_and_typed_safe_files_with_bounded_reads() {
    let root = std::env::temp_dir().join(format!("yami-game-node-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("games")).unwrap();
    fs::write(root.join("games/prism.json"), b"{}").unwrap();
    let module = include_bytes!("../../../games/breakout/breakout.wasm");
    fs::write(root.join("games/prism.wasm"), module).unwrap();
    let site = Site::open(&root).unwrap();
    for key in ["../prism", "prism.json", "NUL", "yn1-other@host"] {
        assert!(site.game(key, GameResource::Manifest).is_err());
    }
    fs::write(
        root.join("games/large.json"),
        vec![0; ynp::MAX_GAME_MANIFEST + 1],
    )
    .unwrap();
    assert!(site.game("large", GameResource::Manifest).is_err());
    fs::write(root.join("games/empty.wasm"), []).unwrap();
    assert!(site.game("empty", GameResource::Module).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("games/prism.wasm"), root.join("games/alias.wasm"))
            .unwrap();
        assert!(site.game("alias", GameResource::Module).is_err());
    }
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let id = *identity.id();
    let listener = ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let host = thread::spawn(move || {
        for _ in 0..3 {
            let mut stream = listener.accept().unwrap();
            yami_node::serve_connection(&mut *stream, &site).unwrap();
        }
    });
    for (key, kind, expected) in [
        ("prism", GameResource::Manifest, Some(b"{}".to_vec())),
        ("prism", GameResource::Module, Some(module.to_vec())),
        ("missing", GameResource::Module, None),
    ] {
        let mut stream = ynp_tls::TlsTransport::new(id)
            .unwrap()
            .connect(endpoint)
            .unwrap();
        let result = ynp::request_game(&mut *stream, key, kind);
        match expected {
            Some(bytes) => assert_eq!(result.unwrap(), bytes),
            None => assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound),
        }
    }
    host.join().unwrap();
    let site = Site::open(&root).unwrap();
    let listener = ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let host = thread::spawn(move || yami_node::serve_once(&listener, &site).is_err());
    let mut stream = std::net::TcpStream::connect(endpoint).unwrap();
    ynp::write_frame(
        &mut stream,
        &ynp::Frame {
            request_id: 3,
            message: ynp::Message::GetGame {
                key: "prism".into(),
                resource: GameResource::Module,
            },
        },
    )
    .unwrap();
    assert!(ynp::read_frame(&mut stream).is_err());
    assert!(host.join().unwrap());
    #[cfg(unix)]
    {
        fs::rename(root.join("games"), root.join("other")).unwrap();
        std::os::unix::fs::symlink(root.join("other"), root.join("games")).unwrap();
        assert!(Site::open(&root)
            .unwrap()
            .game("prism", GameResource::Module)
            .is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn site_check_and_build_preserve_only_validated_hosted_resources() {
    let root = std::env::temp_dir().join(format!("yami-game-author-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("site/games")).unwrap();
    let site = root.join("site");
    fs::write(
        site.join("home.ylang"),
        "@page 0.2\n  title Arcade\n> game:prism | Prism\n",
    )
    .unwrap();
    assert!(yami_node::authoring::check(&site).is_err());
    let bytes = include_bytes!("../../../games/breakout/breakout.wasm");
    let hash = yami_identity::sha256(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let json = format!(
        r#"{{"version":1,"abi":1,"title":"Prism","description":"Demo","module":{{"key":"module","bytes":{},"sha256":"{}"}}}}"#,
        bytes.len(),
        hash
    );
    fs::write(site.join("games/prism.json"), json).unwrap();
    fs::write(site.join("games/module.wasm"), bytes).unwrap();
    fs::write(site.join("games/unreferenced.wasm"), b"private").unwrap();
    fs::write(site.join("games/private.key"), b"private").unwrap();
    let report = yami_node::authoring::build(&site, root.join("built")).unwrap();
    assert_eq!(report.games, vec!["prism"]);
    assert_eq!(report.modules, vec!["module"]);
    assert_eq!(
        fs::read(root.join("built/games/module.wasm")).unwrap(),
        bytes
    );
    assert!(!root.join("built/games/unreferenced.wasm").exists());
    assert!(!root.join("built/games/private.key").exists());
    fs::write(site.join("games/module.wasm"), b"changed").unwrap();
    assert!(yami_node::authoring::check(&site).is_err());
    fs::remove_dir_all(root).unwrap();
}
