//! Compatibility tests over the repository's real YamiNet sample documents.

use std::path::{Path, PathBuf};

use yami_lsp::json::Value;
use yami_lsp::server::Server;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap()
}

fn collect_samples(directory: &Path, samples: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            collect_samples(&path, samples);
        } else if file_type.is_file()
            && matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("ylang" | "ysl")
            )
        {
            samples.push(path);
        }
    }
}

fn call(server: &mut Server, id: i64, method: &str, params: Value) -> Value {
    let request = Value::object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("id", Value::number(id)),
        ("method", Value::string(method)),
        ("params", params),
    ]);
    let responses = server.handle_payload(&request.to_json_string());
    assert_eq!(responses.len(), 1, "{method} should produce one response");
    responses.into_iter().next().unwrap()
}

fn initialize(server: &mut Server, site_root: Option<&Path>) -> Value {
    let options = match site_root {
        Some(path) => Value::object(vec![(
            "siteRoot",
            Value::string(path.to_string_lossy().to_string()),
        )]),
        None => Value::Null,
    };
    call(
        server,
        1,
        "initialize",
        Value::object(vec![
            ("processId", Value::Null),
            ("rootUri", Value::Null),
            ("initializationOptions", options),
            ("capabilities", Value::object(vec![])),
        ]),
    )
}

fn open(server: &mut Server, uri: &str, language_id: &str, text: &str) -> Vec<Value> {
    let notification = Value::object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("method", Value::string("textDocument/didOpen")),
        (
            "params",
            Value::object(vec![(
                "textDocument",
                Value::object(vec![
                    ("uri", Value::string(uri)),
                    ("languageId", Value::string(language_id)),
                    ("version", Value::number(1)),
                    ("text", Value::string(text)),
                ]),
            )]),
        ),
    ]);
    assert!(server
        .handle_payload(&notification.to_json_string())
        .is_empty());
    server.take_pending()
}

fn diagnostics(message: &Value) -> &[Value] {
    message
        .get("params")
        .and_then(|params| params.get("diagnostics"))
        .and_then(Value::as_array)
        .expect("publishDiagnostics payload")
}

fn uri_for(path: &Path) -> String {
    yami_lsp::site::path_to_file_uri(path).expect("sample path is valid UTF-8")
}

#[test]
fn all_repository_samples_open_cleanly_and_answer_requests() {
    let root = repo_root();
    let mut samples = Vec::new();
    collect_samples(&root.join("sites"), &mut samples);
    collect_samples(&root.join("examples"), &mut samples);
    samples.sort();
    assert!(
        samples.len() >= 20,
        "expected the repository's real sample corpus, found {}",
        samples.len()
    );

    let mut server = Server::new();
    initialize(&mut server, Some(&root.join("sites/blog")));
    for (index, path) in samples.iter().enumerate() {
        let text = std::fs::read_to_string(path).unwrap();
        let language_id = if path.extension().and_then(|value| value.to_str()) == Some("ysl") {
            "ysl"
        } else {
            "ylang"
        };
        let uri = uri_for(path);
        let pending = open(&mut server, &uri, language_id, &text);
        assert_eq!(
            pending.len(),
            1,
            "one diagnostics notification for {path:?}"
        );
        assert_eq!(
            pending[0].get("method").and_then(Value::as_str),
            Some("textDocument/publishDiagnostics")
        );
        assert!(
            diagnostics(&pending[0]).is_empty(),
            "{path:?} should parse cleanly"
        );

        // Completion and hover must always answer with valid JSON-RPC results,
        // even for files whose header starts immediately.
        let completion = call(
            &mut server,
            100 + index as i64,
            "textDocument/completion",
            Value::object(vec![
                (
                    "textDocument",
                    Value::object(vec![("uri", Value::string(uri.clone()))]),
                ),
                (
                    "position",
                    Value::object(vec![
                        ("line", Value::number(0)),
                        ("character", Value::number(0)),
                    ]),
                ),
            ]),
        );
        let items = completion
            .get("result")
            .and_then(|result| result.get("items"))
            .and_then(Value::as_array)
            .expect("completion list");
        assert!(!items.is_empty(), "header completion for {path:?}");

        let hover = call(
            &mut server,
            1000 + index as i64,
            "textDocument/hover",
            Value::object(vec![
                (
                    "textDocument",
                    Value::object(vec![("uri", Value::string(uri.clone()))]),
                ),
                (
                    "position",
                    Value::object(vec![
                        ("line", Value::number(0)),
                        ("character", Value::number(2)),
                    ]),
                ),
            ]),
        );
        assert!(hover.get("result").is_some(), "hover for {path:?}");

        let close = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("textDocument/didClose")),
            (
                "params",
                Value::object(vec![(
                    "textDocument",
                    Value::object(vec![("uri", Value::string(uri))]),
                )]),
            ),
        ]);
        server.handle_payload(&close.to_json_string());
        let pending = server.take_pending();
        assert_eq!(pending.len(), 1);
        assert!(diagnostics(&pending[0]).is_empty());
    }
}

#[test]
fn real_blog_navigation_resolves_local_pages_and_rejects_escapes() {
    let root = repo_root();
    let site = root.join("sites/blog");
    let home = site.join("home.ylang");
    let text = std::fs::read_to_string(&home).unwrap();
    assert!(text.contains("> /about | Read more"));
    let uri = uri_for(&home);

    let mut server = Server::new();
    initialize(&mut server, Some(&site));
    let pending = open(&mut server, &uri, "ylang", &text);
    assert!(diagnostics(&pending[0]).is_empty());

    // Line 17 (1-based) of sites/blog/home.ylang is `  > /about | Read more`.
    let definition = call(
        &mut server,
        2,
        "textDocument/definition",
        Value::object(vec![
            (
                "textDocument",
                Value::object(vec![("uri", Value::string(uri.clone()))]),
            ),
            (
                "position",
                Value::object(vec![
                    ("line", Value::number(16)),
                    ("character", Value::number(6)),
                ]),
            ),
        ]),
    );
    assert!(definition
        .get("result")
        .and_then(|result| result.get("uri"))
        .and_then(Value::as_str)
        .is_some_and(|uri| uri.ends_with("about.ylang")));

    // A hostile local key is never resolved.
    let hostile = "@page 0.2\n  title T\n> /../about | Escape\n";
    let pending = open(&mut server, "untitled:hostile", "ylang", hostile);
    // The parser itself rejects `/../about` as a path traversal link, and the
    // definition handler independently refuses to resolve it.
    assert_eq!(diagnostics(&pending[0]).len(), 1);
    let definition = call(
        &mut server,
        3,
        "textDocument/definition",
        Value::object(vec![
            (
                "textDocument",
                Value::object(vec![("uri", Value::string("untitled:hostile"))]),
            ),
            (
                "position",
                Value::object(vec![
                    ("line", Value::number(2)),
                    ("character", Value::number(5)),
                ]),
            ),
        ]),
    );
    assert!(definition.get("result").is_some_and(Value::is_null));
}
