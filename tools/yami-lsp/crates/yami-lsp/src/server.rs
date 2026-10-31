//! JSON-RPC lifecycle, method dispatch and LSP response shaping.

use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::completion::{self, CompletionItem};
use crate::definition;
use crate::diagnostics::{self, Diagnostic};
use crate::documents::{
    ChangeOutcome, DocumentStore, MAX_CHANGES_PER_NOTIFICATION, MAX_DOCUMENT_BYTES,
};
use crate::hover;
use crate::json::{self, Value};
use crate::site::{self, SiteRoot};
use crate::spec;
use crate::text::{Position, Range, TextChange};
use crate::transport::{self, ReadOutcome};

const MAX_BATCH_REQUESTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lifecycle {
    Uninitialized,
    Running,
    ShuttingDown,
}

#[derive(Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
        }
    }
}

pub struct Server {
    lifecycle: Lifecycle,
    documents: DocumentStore,
    configured_site_root: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    pending: Vec<Value>,
    should_exit: bool,
    exit_code: u8,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new() -> Self {
        Self {
            lifecycle: Lifecycle::Uninitialized,
            documents: DocumentStore::new(),
            configured_site_root: None,
            workspace_root: None,
            pending: Vec::new(),
            should_exit: false,
            exit_code: 1,
        }
    }

    /// Process one unframed JSON-RPC payload and return responses. Server-sent
    /// notifications (diagnostics) are queued separately; call
    /// [`Server::take_pending`] after each payload.
    pub fn handle_payload(&mut self, body: &str) -> Vec<Value> {
        match json::parse(body) {
            Ok(Value::Array(items)) => {
                if items.is_empty() {
                    return vec![error_response(Value::Null, -32600, "empty JSON-RPC batch")];
                }
                if items.len() > MAX_BATCH_REQUESTS {
                    return vec![error_response(
                        Value::Null,
                        -32600,
                        format!("batch exceeds {MAX_BATCH_REQUESTS} requests"),
                    )];
                }
                let mut responses = Vec::new();
                for item in items {
                    if let Some(response) = self.handle_value(item) {
                        responses.push(response);
                    }
                }
                if responses.is_empty() {
                    Vec::new()
                } else {
                    vec![Value::Array(responses)]
                }
            }
            Ok(value) => self.handle_value(value).into_iter().collect(),
            Err(error) => vec![error_response(
                Value::Null,
                -32700,
                format!("Parse error: {}", error.message),
            )],
        }
    }

    pub fn take_pending(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.pending)
    }

    pub fn should_exit(&self) -> bool {
        self.should_exit
    }

    pub fn exit_code(&self) -> u8 {
        if self.should_exit {
            self.exit_code
        } else if self.lifecycle == Lifecycle::ShuttingDown {
            0
        } else {
            1
        }
    }

    pub fn document_text(&self, uri: &str) -> Option<&str> {
        self.documents.get(uri).and_then(|document| document.text())
    }

    pub fn configured_site_root(&self) -> Option<&Path> {
        self.configured_site_root.as_deref()
    }

    fn handle_value(&mut self, value: Value) -> Option<Value> {
        let Value::Object(_) = value else {
            return Some(error_response(
                Value::Null,
                -32600,
                "JSON-RPC message must be an object",
            ));
        };

        let raw_id = value.get("id").cloned();
        let id = raw_id.clone().filter(is_valid_id);
        let envelope_error = if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            Some("`jsonrpc` must be exactly \"2.0\"")
        } else if raw_id.is_some() && id.is_none() {
            Some("`id` must be a string or number")
        } else if value.get("method").and_then(Value::as_str).is_none() {
            Some("JSON-RPC message is missing a string `method`")
        } else {
            None
        };
        if let Some(message) = envelope_error {
            return Some(error_response(id.unwrap_or(Value::Null), -32600, message));
        }

        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            // Valid JSON-RPC notification: never respond, even to bad params.
            self.handle_notification(&method, &params);
            return None;
        };
        if !matches!(params, Value::Null | Value::Object(_) | Value::Array(_)) {
            return Some(error_response(
                id,
                -32602,
                "`params` must be an object, array or null",
            ));
        }

        if method == "initialize" {
            if self.lifecycle != Lifecycle::Uninitialized {
                return Some(error_response(
                    id,
                    -32600,
                    "initialize may only be requested once",
                ));
            }
            let result = self.initialize(&params);
            return Some(result_response(id, result));
        }
        match self.lifecycle {
            Lifecycle::Uninitialized => {
                return Some(error_response(
                    id,
                    -32002,
                    "server is not initialized; send initialize first",
                ));
            }
            Lifecycle::ShuttingDown => {
                return Some(error_response(id, -32600, "server is shutting down"));
            }
            Lifecycle::Running => {}
        }

        let result = match method.as_str() {
            "shutdown" => {
                self.lifecycle = Lifecycle::ShuttingDown;
                Ok(Value::Null)
            }
            "textDocument/completion" => self.completion_response(&params),
            "textDocument/hover" => self.hover_response(&params),
            "textDocument/definition" => self.definition_response(&params),
            _ => Err(RpcError {
                code: -32601,
                message: format!("method not found: {method}"),
            }),
        };
        match result {
            Ok(result) => Some(result_response(id, result)),
            Err(error) => Some(error_response(id, error.code, error.message)),
        }
    }

    fn handle_notification(&mut self, method: &str, params: &Value) {
        if method == "exit" {
            self.should_exit = true;
            self.exit_code = if self.lifecycle == Lifecycle::ShuttingDown {
                0
            } else {
                1
            };
            return;
        }
        if self.lifecycle != Lifecycle::Running {
            // Before initialize and after shutdown, only `exit` is meaningful.
            return;
        }
        match method {
            "textDocument/didOpen" => self.did_open(params),
            "textDocument/didChange" => self.did_change(params),
            "textDocument/didClose" => self.did_close(params),
            "workspace/didChangeConfiguration" => self.did_change_configuration(params),
            "workspace/didChangeWorkspaceFolders" => self.did_change_workspace_folders(params),
            "initialized" | "textDocument/didSave" | "$/cancelRequest" | "$/setTrace" => {}
            _ => {}
        }
    }

    fn initialize(&mut self, params: &Value) -> Value {
        let root_uri = params.get("rootUri").and_then(Value::as_str);
        let root_path = params.get("rootPath").and_then(Value::as_str);
        let folder_uri = params
            .get("workspaceFolders")
            .and_then(Value::as_array)
            .and_then(|folders| folders.first())
            .and_then(|folder| folder.get("uri"))
            .and_then(Value::as_str);
        self.workspace_root = root_uri
            .and_then(site::file_uri_to_path)
            .or_else(|| root_path.map(PathBuf::from))
            .or_else(|| folder_uri.and_then(site::file_uri_to_path));

        if let Some(options) = params.get("initializationOptions") {
            let configured = options
                .get("siteRoot")
                .and_then(Value::as_str)
                .or_else(|| options.get("siteRootUri").and_then(Value::as_str));
            if let Some(value) = configured {
                if let Some(path) = site::path_from_setting(value, self.workspace_root.as_deref()) {
                    if let Some(root) = SiteRoot::open(&path) {
                        self.configured_site_root = Some(root.root().to_path_buf());
                    }
                }
            }
        }

        self.lifecycle = Lifecycle::Running;
        Value::object(vec![
            (
                "capabilities",
                Value::object(vec![
                    ("positionEncoding", Value::string("utf-16")),
                    (
                        "textDocumentSync",
                        Value::object(vec![
                            ("openClose", Value::Bool(true)),
                            ("change", Value::number(2)),
                        ]),
                    ),
                    (
                        "completionProvider",
                        Value::object(vec![
                            (
                                "triggerCharacters",
                                Value::Array(
                                    ["@", ":", ">", "!", "[", "/", "=", "-", "+", "#", " "]
                                        .into_iter()
                                        .map(Value::string)
                                        .collect(),
                                ),
                            ),
                            ("resolveProvider", Value::Bool(false)),
                        ]),
                    ),
                    ("hoverProvider", Value::Bool(true)),
                    ("definitionProvider", Value::Bool(true)),
                    (
                        "workspace",
                        Value::object(vec![(
                            "workspaceFolders",
                            Value::object(vec![
                                ("supported", Value::Bool(true)),
                                ("changeNotifications", Value::Bool(true)),
                            ]),
                        )]),
                    ),
                ]),
            ),
            (
                "serverInfo",
                Value::object(vec![
                    ("name", Value::string("yami-lsp")),
                    ("version", Value::string(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
        ])
    }

    fn did_open(&mut self, params: &Value) {
        let Some(text_document) = params.get("textDocument") else {
            return;
        };
        let Some(uri) = text_document.get("uri").and_then(Value::as_str) else {
            return;
        };
        let Some(text) = text_document.get("text").and_then(Value::as_str) else {
            return;
        };
        let language_id = text_document
            .get("languageId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version = text_document
            .get("version")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        match self
            .documents
            .open(uri, language_id, version, text.to_string())
        {
            Ok(()) => self.publish_diagnostics(uri),
            Err(error) => {
                let message = format!("yami-lsp: {error}");
                self.show_message(2, message);
            }
        }
    }

    fn did_change(&mut self, params: &Value) {
        let Some(uri) = params
            .get("textDocument")
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let version = params
            .get("textDocument")
            .and_then(|document| document.get("version"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if self.documents.get(uri).is_none() {
            return;
        }
        let Some(content_changes) = params.get("contentChanges").and_then(Value::as_array) else {
            self.publish_rejected(uri, "contentChanges must be an array");
            return;
        };
        if content_changes.len() > MAX_CHANGES_PER_NOTIFICATION {
            self.publish_rejected(uri, "too many changes in one notification");
            return;
        }
        let mut changes = Vec::with_capacity(content_changes.len());
        for change in content_changes {
            let Some(text) = change.get("text").and_then(Value::as_str) else {
                self.publish_rejected(uri, "content change is missing string `text`");
                return;
            };
            let range = match change.get("range") {
                None | Some(Value::Null) => None,
                Some(value) => match parse_range(value) {
                    Some(range) => Some(range),
                    None => {
                        self.publish_rejected(uri, "content change has an invalid range");
                        return;
                    }
                },
            };
            changes.push(TextChange {
                range,
                text: text.to_string(),
            });
        }
        match self.documents.change(uri, version, &changes) {
            Ok(ChangeOutcome::Applied) => self.publish_diagnostics(uri),
            Ok(ChangeOutcome::TooLarge) => {
                let version = self
                    .documents
                    .get(uri)
                    .map_or(0, |document| document.version());
                let diagnostic = diagnostics::too_large(MAX_DOCUMENT_BYTES);
                self.push_diagnostics(uri, version, &[diagnostic]);
            }
            Err(error) => {
                let message = error.to_string();
                self.publish_rejected(uri, &message);
            }
        }
    }

    fn did_close(&mut self, params: &Value) {
        let Some(uri) = params
            .get("textDocument")
            .and_then(|document| document.get("uri"))
            .and_then(Value::as_str)
        else {
            return;
        };
        self.documents.close(uri);
        self.pending.push(notification(
            "textDocument/publishDiagnostics",
            Value::object(vec![
                ("uri", Value::string(uri)),
                ("diagnostics", Value::Array(Vec::new())),
            ]),
        ));
    }

    fn did_change_configuration(&mut self, params: &Value) {
        let Some(settings) = params.get("settings") else {
            return;
        };
        let configured = settings
            .get("siteRoot")
            .or_else(|| settings.get("yami").and_then(|yami| yami.get("siteRoot")));
        match configured {
            Some(Value::Null) => self.configured_site_root = None,
            Some(Value::String(value)) => {
                if let Some(path) = site::path_from_setting(value, self.workspace_root.as_deref()) {
                    if let Some(root) = SiteRoot::open(&path) {
                        self.configured_site_root = Some(root.root().to_path_buf());
                    }
                }
            }
            Some(_) => self.show_message(
                2,
                "yami-lsp: siteRoot must be a path or file URI".to_string(),
            ),
            None => {}
        }
    }

    fn did_change_workspace_folders(&mut self, params: &Value) {
        let Some(event) = params.get("event") else {
            return;
        };
        if let Some(added) = event.get("added").and_then(Value::as_array) {
            if let Some(uri) = added
                .first()
                .and_then(|folder| folder.get("uri"))
                .and_then(Value::as_str)
            {
                if let Some(path) = site::file_uri_to_path(uri) {
                    self.workspace_root = Some(path);
                }
            }
        }
    }

    fn completion_response(&self, params: &Value) -> Result<Value, RpcError> {
        let uri = text_document_uri(params)?;
        let position = position_param(params)?;
        let Some(document) = self.documents.get(uri) else {
            return Ok(Value::Null);
        };
        let Some(text) = document.text_document() else {
            return Ok(Value::Null);
        };
        let Some(language) = document.language() else {
            return Ok(Value::Null);
        };
        let offset = text.offset_at(position);
        let version = spec::version_or_latest(language, text.text());
        let site = self.site_for(uri);
        let items = completion::complete(text, offset, language, version, site.as_ref());
        Ok(Value::object(vec![
            ("isIncomplete", Value::Bool(false)),
            (
                "items",
                Value::Array(
                    items
                        .iter()
                        .map(|item| completion_item(text, item))
                        .collect(),
                ),
            ),
        ]))
    }

    fn hover_response(&self, params: &Value) -> Result<Value, RpcError> {
        let uri = text_document_uri(params)?;
        let position = position_param(params)?;
        let Some(document) = self.documents.get(uri) else {
            return Ok(Value::Null);
        };
        let (Some(text), Some(language)) = (document.text_document(), document.language()) else {
            return Ok(Value::Null);
        };
        let version = spec::version_or_latest(language, text.text());
        let site = self.site_for(uri);
        let offset = text.offset_at(position);
        let Some(result) = hover::hover_at(text, offset, language, version, site.as_ref()) else {
            return Ok(Value::Null);
        };
        Ok(Value::object(vec![
            (
                "contents",
                Value::object(vec![
                    ("kind", Value::string("markdown")),
                    ("value", Value::string(result.markdown)),
                ]),
            ),
            (
                "range",
                range_json(text.range_between(result.start, result.end)),
            ),
        ]))
    }

    fn definition_response(&self, params: &Value) -> Result<Value, RpcError> {
        let uri = text_document_uri(params)?;
        let position = position_param(params)?;
        let Some(document) = self.documents.get(uri) else {
            return Ok(Value::Null);
        };
        let (Some(text), Some(language)) = (document.text_document(), document.language()) else {
            return Ok(Value::Null);
        };
        // Local page links only exist in an explicitly selected YLang 0.2
        // document. Do not resolve them in invalid or versionless buffers.
        let Some(version) = spec::detect_version(language, text.text()) else {
            return Ok(Value::Null);
        };
        let site = self.site_for(uri);
        let offset = text.offset_at(position);
        let Some(path) = definition::definition_at(text, offset, language, version, site.as_ref())
        else {
            return Ok(Value::Null);
        };
        let Some(target_uri) = site::path_to_file_uri(&path) else {
            return Ok(Value::Null);
        };
        Ok(Value::object(vec![
            ("uri", Value::string(target_uri)),
            (
                "range",
                range_json(Range::new(Position::new(0, 0), Position::new(0, 0))),
            ),
        ]))
    }

    fn site_for(&self, uri: &str) -> Option<SiteRoot> {
        if let Some(path) = &self.configured_site_root {
            if let Some(site) = SiteRoot::open(path) {
                return Some(site);
            }
        }
        if let Some(path) = &self.workspace_root {
            if let Some(site) = SiteRoot::open(path) {
                return Some(site);
            }
        }
        site::document_directory(uri).and_then(|path| SiteRoot::open(&path))
    }

    fn publish_diagnostics(&mut self, uri: &str) {
        let Some(document) = self.documents.get(uri) else {
            return;
        };
        let version = document.version();
        let diagnostics = match document.text_document() {
            Some(text) => diagnostics::compute(document.language(), text),
            None => vec![diagnostics::too_large(MAX_DOCUMENT_BYTES)],
        };
        self.push_diagnostics(uri, version, &diagnostics);
    }

    fn publish_rejected(&mut self, uri: &str, message: &str) {
        let version = self
            .documents
            .get(uri)
            .map_or(0, |document| document.version());
        let diagnostic = diagnostics::rejected_change(message);
        self.push_diagnostics(uri, version, &[diagnostic]);
    }

    fn push_diagnostics(&mut self, uri: &str, version: i64, diagnostics: &[Diagnostic]) {
        let values = diagnostics
            .iter()
            .map(diagnostic_value)
            .collect::<Vec<Value>>();
        self.pending.push(notification(
            "textDocument/publishDiagnostics",
            Value::object(vec![
                ("uri", Value::string(uri)),
                ("version", Value::number(version)),
                ("diagnostics", Value::Array(values)),
            ]),
        ));
    }

    fn show_message(&mut self, kind: i64, message: String) {
        self.pending.push(notification(
            "window/showMessage",
            Value::object(vec![
                ("type", Value::number(kind)),
                ("message", Value::string(message)),
            ]),
        ));
    }
}

/// Run the language server until `exit`, EOF or a transport error.
pub fn run_stdio() -> io::Result<u8> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());
    let mut server = Server::new();
    loop {
        match transport::read_message(&mut reader)? {
            ReadOutcome::Eof => break,
            ReadOutcome::Malformed(message) => {
                let response =
                    error_response(Value::Null, -32700, format!("Parse error: {message}"));
                transport::write_message(&mut writer, &response)?;
            }
            ReadOutcome::Message(body) => {
                let responses = server.handle_payload(&body);
                for notification in server.take_pending() {
                    transport::write_message(&mut writer, &notification)?;
                }
                for response in responses {
                    transport::write_message(&mut writer, &response)?;
                }
                if server.should_exit() {
                    break;
                }
            }
        }
    }
    writer.flush()?;
    Ok(server.exit_code())
}

fn is_valid_id(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Number(_))
}

fn notification(method: &str, params: Value) -> Value {
    Value::object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("method", Value::string(method)),
        ("params", params),
    ])
}

fn result_response(id: Value, result: Value) -> Value {
    Value::object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("id", id),
        ("result", result),
    ])
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    Value::object(vec![
        ("jsonrpc", Value::string("2.0")),
        ("id", id),
        (
            "error",
            Value::object(vec![
                ("code", Value::number(code)),
                ("message", Value::string(message.into())),
            ]),
        ),
    ])
}

fn position_json(position: Position) -> Value {
    Value::object(vec![
        ("line", Value::number(i64::from(position.line))),
        ("character", Value::number(i64::from(position.character))),
    ])
}

fn range_json(range: Range) -> Value {
    Value::object(vec![
        ("start", position_json(range.start)),
        ("end", position_json(range.end)),
    ])
}

fn diagnostic_value(diagnostic: &Diagnostic) -> Value {
    Value::object(vec![
        ("range", range_json(diagnostic.range)),
        ("severity", Value::number(1)),
        ("source", Value::string(diagnostic.source)),
        ("code", Value::string(diagnostic.code)),
        ("message", Value::string(diagnostic.message.clone())),
    ])
}

fn completion_item(text: &crate::text::TextDocument, item: &CompletionItem) -> Value {
    let range = text.range_between(item.edit_start, item.edit_end);
    Value::object(vec![
        ("label", Value::string(item.label.clone())),
        ("kind", Value::number(i64::from(item.kind.lsp()))),
        ("detail", Value::string(item.detail.clone())),
        (
            "documentation",
            Value::object(vec![
                ("kind", Value::string("markdown")),
                ("value", Value::string(item.documentation.clone())),
            ]),
        ),
        (
            "textEdit",
            Value::object(vec![
                ("range", range_json(range)),
                ("newText", Value::string(item.new_text.clone())),
            ]),
        ),
    ])
}

fn text_document_uri(params: &Value) -> Result<&str, RpcError> {
    params
        .get("textDocument")
        .and_then(|document| document.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("missing textDocument.uri"))
}

fn position_param(params: &Value) -> Result<Position, RpcError> {
    let position = params
        .get("position")
        .ok_or_else(|| RpcError::invalid_params("missing position"))?;
    let line = position
        .get("line")
        .and_then(Value::as_u32)
        .ok_or_else(|| RpcError::invalid_params("position.line must be a nonnegative integer"))?;
    let character = position
        .get("character")
        .and_then(Value::as_u32)
        .ok_or_else(|| {
            RpcError::invalid_params("position.character must be a nonnegative integer")
        })?;
    Ok(Position::new(line, character))
}

fn parse_range(value: &Value) -> Option<Range> {
    let start = parse_position(value.get("start")?)?;
    let end = parse_position(value.get("end")?)?;
    Some(Range::new(start, end))
}

fn parse_position(value: &Value) -> Option<Position> {
    Some(Position::new(
        value.get("line")?.as_u32()?,
        value.get("character")?.as_u32()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(server: &mut Server, id: i64, method: &str, params: Value) -> Value {
        let request = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("id", Value::number(id)),
            ("method", Value::string(method)),
            ("params", params),
        ]);
        let responses = server.handle_payload(&request.to_json_string());
        assert_eq!(responses.len(), 1);
        responses.into_iter().next().unwrap()
    }

    fn init(server: &mut Server, site_root: Option<&Path>) -> Value {
        let options = match site_root {
            Some(path) => Value::object(vec![("siteRoot", Value::string(path.to_string_lossy()))]),
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

    #[test]
    fn initialize_then_shutdown_and_exit_is_graceful() {
        let mut server = Server::new();
        let response = init(&mut server, None);
        assert_eq!(
            response
                .get("result")
                .and_then(|result| result.get("capabilities"))
                .and_then(|capabilities| capabilities.get("positionEncoding"))
                .and_then(Value::as_str),
            Some("utf-16")
        );
        assert!(!server.should_exit());
        let response = call(&mut server, 2, "shutdown", Value::Null);
        assert!(response.get("result").is_some_and(Value::is_null));
        let exit = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("exit")),
        ]);
        assert!(server.handle_payload(&exit.to_json_string()).is_empty());
        assert!(server.should_exit());
        assert_eq!(server.exit_code(), 0);
    }

    #[test]
    fn exit_without_shutdown_reports_failure() {
        let mut server = Server::new();
        init(&mut server, None);
        let exit = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("exit")),
        ]);
        assert!(server.handle_payload(&exit.to_json_string()).is_empty());
        assert!(server.should_exit());
        assert_eq!(server.exit_code(), 1);
    }

    #[test]
    fn requests_are_rejected_before_initialize_and_after_shutdown() {
        let mut server = Server::new();
        let response = call(&mut server, 1, "textDocument/hover", Value::Null);
        assert_eq!(
            response
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64),
            Some(-32002)
        );
        let mut server = Server::new();
        init(&mut server, None);
        call(&mut server, 2, "shutdown", Value::Null);
        let response = call(&mut server, 3, "textDocument/hover", Value::Null);
        assert_eq!(
            response
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64),
            Some(-32600)
        );
    }

    #[test]
    fn malformed_json_and_unknown_methods_do_not_break_the_session() {
        let mut server = Server::new();
        let responses = server.handle_payload("{not json");
        assert_eq!(
            responses[0]
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64),
            Some(-32700)
        );
        init(&mut server, None);
        let response = call(&mut server, 2, "unknown/method", Value::Null);
        assert_eq!(
            response
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64),
            Some(-32601)
        );
        let response = call(
            &mut server,
            3,
            "textDocument/hover",
            Value::object(vec![(
                "textDocument",
                Value::object(vec![("uri", Value::string("untitled:1"))]),
            )]),
        );
        assert_eq!(
            response
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_i64),
            Some(-32602)
        );
    }

    #[test]
    fn did_open_publishes_diagnostics_and_did_change_updates_them() {
        let mut server = Server::new();
        init(&mut server, None);
        let open = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("textDocument/didOpen")),
            (
                "params",
                Value::object(vec![(
                    "textDocument",
                    Value::object(vec![
                        ("uri", Value::string("untitled:1")),
                        ("languageId", Value::string("ylang")),
                        ("version", Value::number(1)),
                        (
                            "text",
                            Value::string("@page 0.2\n  title T\n😀😀 [bad: x]\n"),
                        ),
                    ]),
                )]),
            ),
        ]);
        assert!(server.handle_payload(&open.to_json_string()).is_empty());
        let pending = server.take_pending();
        assert_eq!(pending.len(), 1);
        let diagnostics = pending[0]
            .get("params")
            .and_then(|params| params.get("diagnostics"))
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0]
                .get("range")
                .and_then(|range| range.get("start"))
                .and_then(|start| start.get("character"))
                .and_then(Value::as_u32),
            Some(5)
        );

        let change = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("textDocument/didChange")),
            (
                "params",
                Value::object(vec![
                    (
                        "textDocument",
                        Value::object(vec![
                            ("uri", Value::string("untitled:1")),
                            ("version", Value::number(2)),
                        ]),
                    ),
                    (
                        "contentChanges",
                        Value::array(vec![Value::object(vec![(
                            "text",
                            Value::string("@page 0.2\n  title Fixed\n= valid\n"),
                        )])]),
                    ),
                ]),
            ),
        ]);
        server.handle_payload(&change.to_json_string());
        let pending = server.take_pending();
        let diagnostics = pending[0]
            .get("params")
            .and_then(|params| params.get("diagnostics"))
            .and_then(Value::as_array)
            .unwrap();
        assert!(diagnostics.is_empty());
        assert!(server
            .document_text("untitled:1")
            .unwrap()
            .contains("Fixed"));
    }

    #[test]
    fn completion_edits_use_utf16_columns_after_emoji() {
        let mut server = Server::new();
        init(&mut server, None);
        let open = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("textDocument/didOpen")),
            (
                "params",
                Value::object(vec![(
                    "textDocument",
                    Value::object(vec![
                        ("uri", Value::string("untitled:emoji")),
                        ("languageId", Value::string("ylang")),
                        ("version", Value::number(1)),
                        ("text", Value::string("@page 0.2\n  title T\n😀 [str\n")),
                    ]),
                )]),
            ),
        ]);
        server.handle_payload(&open.to_json_string());
        server.take_pending();
        let response = call(
            &mut server,
            2,
            "textDocument/completion",
            Value::object(vec![
                (
                    "textDocument",
                    Value::object(vec![("uri", Value::string("untitled:emoji"))]),
                ),
                (
                    "position",
                    Value::object(vec![
                        ("line", Value::number(2)),
                        ("character", Value::number(7)),
                    ]),
                ),
            ]),
        );
        let item = response
            .get("result")
            .and_then(|result| result.get("items"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .expect("inline completion");
        assert_eq!(item.get("label").and_then(Value::as_str), Some("strong"));
        let edit = item.get("textEdit").expect("text edit");
        assert_eq!(
            edit.get("newText").and_then(Value::as_str),
            Some("[strong: ")
        );
        let range = edit.get("range").expect("edit range");
        assert_eq!(
            range
                .get("start")
                .and_then(|start| start.get("character"))
                .and_then(Value::as_u32),
            Some(3)
        );
        assert_eq!(
            range
                .get("end")
                .and_then(|end| end.get("character"))
                .and_then(Value::as_u32),
            Some(7)
        );
    }

    #[test]
    fn definition_resolves_local_pages_and_never_remote_targets() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        let site = root.join("sites/blog");
        let mut server = Server::new();
        init(&mut server, Some(&site));
        let open = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("textDocument/didOpen")),
            (
                "params",
                Value::object(vec![(
                    "textDocument",
                    Value::object(vec![
                        ("uri", Value::string("file:///site/home.ylang")),
                        ("languageId", Value::string("ylang")),
                        ("version", Value::number(1)),
                        (
                            "text",
                            Value::string(
                                "@page 0.2\n  title T\n> /about | About\n> /missing | Missing\n",
                            ),
                        ),
                    ]),
                )]),
            ),
        ]);
        server.handle_payload(&open.to_json_string());
        server.take_pending();

        let position = |line: u32| {
            Value::object(vec![
                ("line", Value::number(i64::from(line))),
                ("character", Value::number(4)),
            ])
        };
        let response = call(
            &mut server,
            2,
            "textDocument/definition",
            Value::object(vec![
                (
                    "textDocument",
                    Value::object(vec![("uri", Value::string("file:///site/home.ylang"))]),
                ),
                ("position", position(2)),
            ]),
        );
        assert!(response
            .get("result")
            .and_then(|result| result.get("uri"))
            .and_then(Value::as_str)
            .is_some_and(|uri| uri.ends_with("about.ylang")));
        let response = call(
            &mut server,
            3,
            "textDocument/definition",
            Value::object(vec![
                (
                    "textDocument",
                    Value::object(vec![("uri", Value::string("file:///site/home.ylang"))]),
                ),
                ("position", position(3)),
            ]),
        );
        assert!(response.get("result").is_some_and(Value::is_null));

        // A local link in a buffer without a valid `@page 0.2` header must not
        // resolve, even though the target file exists.
        let invalid = Value::object(vec![
            ("jsonrpc", Value::string("2.0")),
            ("method", Value::string("textDocument/didOpen")),
            (
                "params",
                Value::object(vec![(
                    "textDocument",
                    Value::object(vec![
                        ("uri", Value::string("untitled:invalid")),
                        ("languageId", Value::string("ylang")),
                        ("version", Value::number(1)),
                        (
                            "text",
                            Value::string("@page 0.3\n  title T\n> /about | About\n"),
                        ),
                    ]),
                )]),
            ),
        ]);
        server.handle_payload(&invalid.to_json_string());
        server.take_pending();
        let response = call(
            &mut server,
            4,
            "textDocument/definition",
            Value::object(vec![
                (
                    "textDocument",
                    Value::object(vec![("uri", Value::string("untitled:invalid"))]),
                ),
                ("position", position(2)),
            ]),
        );
        assert!(response.get("result").is_some_and(Value::is_null));
    }
}
