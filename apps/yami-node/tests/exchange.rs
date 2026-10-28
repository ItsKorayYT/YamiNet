use std::{
    io::{self, Write},
    process::Command,
    thread,
};
use yami_node::{serve_once, Store};
use ynp::{DocumentData, Frame, LocalListener, LoopbackTcp, Message, Transport};

#[test]
fn two_sample_nodes_exchange_documents() {
    let output = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .arg("demo")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Node one received about from node two via YNP"));
    assert!(text.contains("Node two received welcome from node one via YNP"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not secure for sensitive use"));
}
#[test]
fn rejects_bad_client_then_serves_missing_and_existing_keys() {
    let listener = LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let mut store = Store::default();
    let doc = DocumentData {
        key: "a".into(),
        source: "a document".into(),
        style: "a style".into(),
    };
    store.insert(doc.clone()).unwrap();
    let server = thread::spawn(move || {
        assert!(serve_once(&listener, &store).is_err());
        assert!(serve_once(&listener, &store).is_err());
        serve_once(&listener, &store).unwrap();
        serve_once(&listener, &store).unwrap();
    });
    let mut bad = LoopbackTcp.connect(address).unwrap();
    bad.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
    drop(bad);
    let mut wrong_kind = LoopbackTcp.connect(address).unwrap();
    ynp::write_frame(
        &mut *wrong_kind,
        &Frame {
            request_id: 1,
            message: Message::NotFound { key: "a".into() },
        },
    )
    .unwrap();
    drop(wrong_kind);
    assert_eq!(
        ynp::fetch(&LoopbackTcp, address, "missing")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    assert_eq!(ynp::fetch(&LoopbackTcp, address, "a").unwrap(), doc);
    server.join().unwrap();
}
