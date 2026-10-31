//! End-to-end JSON-RPC tests against the real `yami-lsp` executable.

use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};

use yami_lsp::json::{self, Value};
use yami_lsp::transport::{self, ReadOutcome};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap()
}

fn object(entries: Vec<(&str, Value)>) -> Value {
    Value::object(entries)
}

fn request(id: i64, method: &str, params: Value) -> Value {
    object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("id", Value::number(id)),
        ("method", Value::string(method)),
        ("params", params),
    ])
}

fn notification(method: &str, params: Value) -> Value {
    object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("method", Value::string(method)),
        ("params", params),
    ])
}

fn position(line: u32, character: u32) -> Value {
    object(vec![
        ("line", Value::number(i64::from(line))),
        ("character", Value::number(i64::from(character))),
    ])
}

fn document_uri(uri: &str) -> Value {
    object(vec![("uri", Value::string(uri))])
}

fn did_open(uri: &str, language_id: &str, version: i64, text: &str) -> Value {
    notification(
        "textDocument/didOpen",
        object(vec![(
            "textDocument",
            object(vec![
                ("uri", Value::string(uri)),
                ("languageId", Value::string(language_id)),
                ("version", Value::number(version)),
                ("text", Value::string(text)),
            ]),
        )]),
    )
}

fn did_change_full(uri: &str, version: i64, text: &str) -> Value {
    notification(
        "textDocument/didChange",
        object(vec![
            (
                "textDocument",
                object(vec![
                    ("uri", Value::string(uri)),
                    ("version", Value::number(version)),
                ]),
            ),
            (
                "contentChanges",
                Value::array(vec![object(vec![("text", Value::string(text))])]),
            ),
        ]),
    )
}

fn did_change_range(uri: &str, version: i64, start: Value, end: Value, text: &str) -> Value {
    notification(
        "textDocument/didChange",
        object(vec![
            (
                "textDocument",
                object(vec![
                    ("uri", Value::string(uri)),
                    ("version", Value::number(version)),
                ]),
            ),
            (
                "contentChanges",
                Value::array(vec![object(vec![
                    ("range", object(vec![("start", start), ("end", end)])),
                    ("text", Value::string(text)),
                ])]),
            ),
        ]),
    )
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    reaped: bool,
}

impl Client {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_yami-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn yami-lsp");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        Self {
            child,
            stdin,
            stdout,
            reaped: false,
        }
    }

    fn send(&mut self, value: &Value) {
        transport::write_message(self.stdin.as_mut().expect("stdin open"), value)
            .expect("write LSP frame");
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(bytes).expect("write raw bytes");
        stdin.flush().expect("flush raw bytes");
    }

    fn recv(&mut self) -> Value {
        match transport::read_message(&mut self.stdout).expect("read LSP frame") {
            ReadOutcome::Message(body) => json::parse(&body).expect("valid JSON response"),
            other => panic!("expected a message, got {other:?}"),
        }
    }

    fn finish(&mut self) -> ExitStatus {
        self.stdin.take();
        let status = self.child.wait().expect("wait for yami-lsp");
        self.reaped = true;
        status
    }

    fn expect_eof(&mut self) {
        assert_eq!(
            transport::read_message(&mut self.stdout).unwrap(),
            ReadOutcome::Eof
        );
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stdin.take();
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn error_code(response: &Value) -> Option<i64> {
    response
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64)
}

fn response_error_code(response: &Value) -> i64 {
    error_code(response).unwrap_or_else(|| panic!("expected error: {response:?}"))
}

fn initialize(client: &mut Client, site_root: Option<&Path>) -> Value {
    let options = match site_root {
        Some(path) => object(vec![(
            "siteRoot",
            Value::string(path.to_string_lossy().to_string()),
        )]),
        None => Value::Null,
    };
    client.send(&request(
        1,
        "initialize",
        object(vec![
            ("processId", Value::Null),
            ("rootUri", Value::Null),
            ("initializationOptions", options),
            ("capabilities", object(vec![])),
        ]),
    ));
    let response = client.recv();
    client.send(&notification("initialized", object(vec![])));
    response
}

fn diagnostics_from(message: &Value) -> &[Value] {
    assert_eq!(
        message.get("method").and_then(Value::as_str),
        Some("textDocument/publishDiagnostics"),
        "expected diagnostics notification: {message:?}"
    );
    message
        .get("params")
        .and_then(|params| params.get("diagnostics"))
        .and_then(Value::as_array)
        .expect("diagnostics array")
}

fn diagnostic_start_character(message: &Value) -> u32 {
    let diagnostics = diagnostics_from(message);
    assert_eq!(diagnostics.len(), 1, "expected one diagnostic: {message:?}");
    diagnostics[0]
        .get("range")
        .and_then(|range| range.get("start"))
        .and_then(|start| start.get("character"))
        .and_then(Value::as_u32)
        .expect("diagnostic character")
}

fn completion_labels(response: &Value) -> Vec<&str> {
    response
        .get("result")
        .and_then(|result| result.get("items"))
        .and_then(Value::as_array)
        .expect("completion items")
        .iter()
        .map(|item| item.get("label").and_then(Value::as_str).unwrap_or(""))
        .collect()
}

#[test]
fn full_lsp_session_diagnostics_navigation_and_shutdown() {
    let site = repo_root().join("sites/blog");
    let mut client = Client::start();
    let init = initialize(&mut client, Some(&site));
    let capabilities = init
        .get("result")
        .and_then(|result| result.get("capabilities"))
        .expect("capabilities");
    assert_eq!(
        capabilities.get("positionEncoding").and_then(Value::as_str),
        Some("utf-16")
    );
    assert_eq!(
        capabilities
            .get("completionProvider")
            .and_then(|provider| provider.get("resolveProvider"))
            .and_then(Value::as_bool),
        Some(false)
    );

    let uri = "file:///site/home.ylang";
    let text = "@page 0.2\n  title Test\n> /about | About\n> /missing | Missing\n";
    client.send(&did_open(uri, "ylang", 1, text));
    let opened = client.recv();
    assert!(diagnostics_from(&opened).is_empty());
    assert_eq!(
        opened
            .get("params")
            .and_then(|params| params.get("version"))
            .and_then(Value::as_i64),
        Some(1)
    );

    client.send(&request(
        2,
        "textDocument/completion",
        object(vec![
            ("textDocument", document_uri(uri)),
            ("position", position(2, 3)),
        ]),
    ));
    let completion = client.recv();
    let labels = completion_labels(&completion);
    assert!(labels.contains(&"/about"), "{labels:?}");
    assert!(labels.contains(&"/home"), "{labels:?}");

    client.send(&request(
        3,
        "textDocument/hover",
        object(vec![
            ("textDocument", document_uri(uri)),
            ("position", position(2, 4)),
        ]),
    ));
    let hover = client.recv();
    assert!(hover
        .get("result")
        .and_then(|result| result.get("contents"))
        .and_then(|contents| contents.get("value"))
        .and_then(Value::as_str)
        .is_some_and(|value| value.contains("about.ylang")));

    client.send(&request(
        4,
        "textDocument/definition",
        object(vec![
            ("textDocument", document_uri(uri)),
            ("position", position(2, 4)),
        ]),
    ));
    let definition = client.recv();
    assert!(definition
        .get("result")
        .and_then(|result| result.get("uri"))
        .and_then(Value::as_str)
        .is_some_and(|uri| uri.ends_with("about.ylang")));

    // A missing page resolves to null without an error.
    client.send(&request(
        5,
        "textDocument/definition",
        object(vec![
            ("textDocument", document_uri(uri)),
            ("position", position(3, 4)),
        ]),
    ));
    let missing = client.recv();
    assert!(missing.get("result").is_some_and(Value::is_null));

    // An in-memory change updates diagnostics without any disk access.
    client.send(&did_change_full(
        uri,
        2,
        "@page 0.2\n  title Test\n> /../home | Bad\n",
    ));
    let broken = client.recv();
    assert!(!diagnostics_from(&broken).is_empty());
    assert_eq!(
        broken
            .get("params")
            .and_then(|params| params.get("version"))
            .and_then(Value::as_i64),
        Some(2)
    );
    // Path traversal is rejected even though the line is present in the
    // unsaved buffer.
    client.send(&request(
        6,
        "textDocument/definition",
        object(vec![
            ("textDocument", document_uri(uri)),
            ("position", position(2, 4)),
        ]),
    ));
    assert!(client.recv().get("result").is_some_and(Value::is_null));

    client.send(&did_change_full(
        uri,
        3,
        "@page 0.2\n  title Test\n> /about | About\n",
    ));
    assert!(diagnostics_from(&client.recv()).is_empty());

    client.send(&notification(
        "textDocument/didClose",
        object(vec![("textDocument", document_uri(uri))]),
    ));
    let closed = client.recv();
    assert!(diagnostics_from(&closed).is_empty());

    client.send(&request(7, "shutdown", Value::Null));
    let shutdown = client.recv();
    assert!(shutdown.get("result").is_some_and(Value::is_null));
    client.send(&notification("exit", Value::Null));
    assert!(client.finish().success());
    client.expect_eof();
}

#[test]
fn malformed_jsonrpc_never_breaks_the_server() {
    let mut client = Client::start();

    let malformed = "{not json";
    client.send_raw(format!("Content-Length: {}\r\n\r\n{}", malformed.len(), malformed).as_bytes());
    assert_eq!(response_error_code(&client.recv()), -32700);

    client.send_raw(b"Content-Length: nope\r\n\r\n");
    assert_eq!(response_error_code(&client.recv()), -32700);

    client.send_raw(b"Content-Length: 1\r\n\r\n\xff");
    assert_eq!(response_error_code(&client.recv()), -32700);

    let init = initialize(&mut client, None);
    assert!(init.get("result").is_some());

    client.send(&object(vec![
        ("jsonrpc", Value::string("1.0")),
        ("id", Value::number(8)),
        ("method", Value::string("textDocument/hover")),
        ("params", Value::Null),
    ]));
    let wrong_version = client.recv();
    assert_eq!(response_error_code(&wrong_version), -32600);
    assert_eq!(wrong_version.get("id").and_then(Value::as_i64), Some(8));

    client.send(&request(9, "does/not/exist", Value::Null));
    assert_eq!(response_error_code(&client.recv()), -32601);

    client.send(&object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("id", Value::Bool(true)),
        ("method", Value::string("textDocument/hover")),
        ("params", Value::Null),
    ]));
    assert_eq!(response_error_code(&client.recv()), -32600);

    client.send(&request(10, "shutdown", Value::Null));
    assert!(client.recv().get("result").is_some());
    client.send(&notification("exit", Value::Null));
    assert!(client.finish().success());
}

#[test]
fn utf16_columns_survive_incremental_unsaved_edits() {
    let mut client = Client::start();
    initialize(&mut client, None);
    let uri = "untitled:unicode";
    client.send(&did_open(
        uri,
        "ylang",
        1,
        "@page 0.2\n  title T\n😀 [bad: x]\n",
    ));
    assert_eq!(diagnostic_start_character(&client.recv()), 3);

    // Insert one more emoji at UTF-16 position (2, 0); the error must move two
    // UTF-16 units to the right.
    client.send(&did_change_range(
        uri,
        2,
        position(2, 0),
        position(2, 0),
        "😀",
    ));
    assert_eq!(diagnostic_start_character(&client.recv()), 5);

    client.send(&did_change_full(
        uri,
        3,
        "@page 0.2\n  title T\nA valid [strong: span]\n",
    ));
    assert!(diagnostics_from(&client.recv()).is_empty());

    client.send(&request(2, "shutdown", Value::Null));
    client.recv();
    client.send(&notification("exit", Value::Null));
    assert!(client.finish().success());
}

#[test]
fn version_specific_ysl_features_and_properties() {
    let mut client = Client::start();
    initialize(&mut client, None);
    let uri = "file:///site/default.ysl";
    let text = "@style 0.1\ncanvas\n  width 76\n";
    client.send(&did_open(uri, "ysl", 1, text));
    assert!(diagnostics_from(&client.recv()).is_empty());

    client.send(&request(
        2,
        "textDocument/completion",
        object(vec![
            ("textDocument", document_uri(uri)),
            ("position", position(2, 2)),
        ]),
    ));
    let completion = client.recv();
    let labels = completion_labels(&completion);
    assert!(labels.contains(&"width"));
    assert!(labels.contains(&"padding"));
    assert!(!labels.contains(&"size"));

    client.send(&request(
        3,
        "textDocument/hover",
        object(vec![
            ("textDocument", document_uri(uri)),
            ("position", position(2, 3)),
        ]),
    ));
    let hover = client.recv();
    assert!(hover
        .get("result")
        .and_then(|result| result.get("contents"))
        .and_then(|contents| contents.get("value"))
        .and_then(Value::as_str)
        .is_some_and(|value| value.contains("20..160")));

    // 900 is outside the 0.1 canvas range.
    client.send(&did_change_full(
        uri,
        2,
        "@style 0.1\ncanvas\n  width 900\n",
    ));
    assert!(!diagnostics_from(&client.recv()).is_empty());
    // The same value is valid in 0.2.
    client.send(&did_change_full(
        uri,
        3,
        "@style 0.2\ncanvas\n  width 900\n",
    ));
    assert!(diagnostics_from(&client.recv()).is_empty());

    client.send(&request(4, "shutdown", Value::Null));
    client.recv();
    client.send(&notification("exit", Value::Null));
    assert!(client.finish().success());
}
