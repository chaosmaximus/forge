//! LSP client — spawns a language server and communicates via JSON-RPC over stdio.
//!
//! Uses raw JSON-RPC with LSP content-length framing rather than `async-lsp`,
//! because the simpler approach is easier to debug and has fewer moving parts.
//! The protocol is straightforward:
//!   - Write: `Content-Length: N\r\n\r\n{json}`
//!   - Read:  parse `Content-Length` header, then read exactly N bytes of JSON

use lsp_types::{
    ClientCapabilities, ClientInfo, DocumentSymbol, DocumentSymbolClientCapabilities,
    DocumentSymbolParams, DocumentSymbolResponse, GeneralClientCapabilities, InitializeParams,
    InitializeResult, InitializedParams, TextDocumentClientCapabilities, TextDocumentIdentifier,
    Uri, WorkDoneProgressParams,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::detect::LspServerConfig;

/// A diagnostic captured from a language server's `textDocument/publishDiagnostics` notification.
#[derive(Debug, Clone)]
pub struct LspDiagnostic {
    pub file_uri: String,
    pub severity: String, // "error", "warning", "information", "hint"
    pub message: String,
    pub line: u32,
    pub column: u32,
    pub source: String,
}

/// Convert an absolute filesystem path to a properly percent-encoded `file://` URI.
///
/// Handles spaces, non-ASCII characters, and other reserved URI characters.
/// Follows RFC 8089 (file URI scheme): each path segment is percent-encoded.
pub fn path_to_file_uri(path: &str) -> String {
    let abs = if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path).to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string())
    };

    // Percent-encode each path segment, preserving '/' as separator
    let encoded: String = abs
        .split('/')
        .map(|segment| {
            segment
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~'
                    {
                        // Unreserved characters (RFC 3986 §2.3) — pass through
                        String::from(b as char)
                    } else {
                        // Percent-encode everything else
                        format!("%{b:02X}")
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/");

    format!("file://{encoded}")
}

/// A minimal JSON-RPC LSP client that communicates over stdin/stdout.
pub struct LspClient {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    /// Server capabilities returned from initialize.
    #[allow(dead_code)]
    server_caps: Option<InitializeResult>,
    /// Captured diagnostics from `textDocument/publishDiagnostics` notifications.
    pub diagnostics_buffer: Vec<LspDiagnostic>,
}

/// JSON-RPC request envelope.
#[derive(Serialize)]
struct JsonRpcRequest<P: Serialize> {
    jsonrpc: &'static str,
    id: i64,
    method: &'static str,
    params: P,
}

/// JSON-RPC notification envelope (no id).
#[derive(Serialize)]
struct JsonRpcNotification<P: Serialize> {
    jsonrpc: &'static str,
    method: &'static str,
    params: P,
}

/// JSON-RPC notification received from the server (for deserialization).
#[derive(Deserialize)]
struct ServerNotification {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    method: Option<String>,
    params: Option<serde_json::Value>,
}

/// JSON-RPC response envelope.
#[derive(Deserialize)]
struct JsonRpcResponse<R> {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: Option<serde_json::Value>,
    result: Option<R>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LSP error {}: {}", self.code, self.message)
    }
}

impl LspClient {
    /// Spawn a language server process, perform the initialize handshake,
    /// and return a ready-to-use client.
    pub async fn spawn(config: &LspServerConfig, root_dir: &str) -> Result<Self, String> {
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .current_dir(root_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("Failed to spawn {}: {}", config.command, e))?;

        let stdin = child.stdin.take().ok_or("No stdin on child process")?;
        let stdout = child.stdout.take().ok_or("No stdout on child process")?;

        let mut client = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout),
            next_id: 1,
            server_caps: None,
            diagnostics_buffer: Vec::new(),
        };

        // Build root URI.
        let root_uri_str = path_to_file_uri(root_dir);
        let root_uri: Uri = root_uri_str
            .parse()
            .map_err(|e| format!("Invalid root URI '{root_uri_str}': {e}"))?;

        #[allow(deprecated)]
        let init_params = InitializeParams {
            process_id: Some(std::process::id()),
            root_uri: Some(root_uri),
            root_path: Some(root_dir.to_string()),
            capabilities: ClientCapabilities {
                general: Some(GeneralClientCapabilities {
                    ..Default::default()
                }),
                text_document: Some(TextDocumentClientCapabilities {
                    document_symbol: Some(DocumentSymbolClientCapabilities {
                        hierarchical_document_symbol_support: Some(true),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            initialization_options: None,
            trace: None,
            workspace_folders: Some(vec![lsp_types::WorkspaceFolder {
                uri: root_uri_str.parse().unwrap(),
                name: std::path::Path::new(root_dir)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "workspace".into()),
            }]),
            client_info: Some(ClientInfo {
                name: "forge-daemon".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            locale: None,
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let init_result: InitializeResult = client.send_request("initialize", init_params).await?;
        client.server_caps = Some(init_result);

        // Send initialized notification.
        client
            .send_notification("initialized", InitializedParams {})
            .await?;

        // Give the language server time to start indexing the workspace.
        // rust-analyzer in particular needs to build its project model
        // before textDocument/references can return meaningful results.
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        Ok(client)
    }

    /// Check if the server supports document symbols.
    pub fn supports_document_symbols(&self) -> bool {
        self.server_caps
            .as_ref()
            .and_then(|c| c.capabilities.document_symbol_provider.as_ref())
            .is_some()
    }

    /// Check if the server supports textDocument/references.
    pub fn supports_references(&self) -> bool {
        self.server_caps
            .as_ref()
            .and_then(|c| c.capabilities.references_provider.as_ref())
            .is_some()
    }

    /// Check whether the language server process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Notify the server that a file has been opened.
    /// Most servers REQUIRE didOpen before responding to symbol requests.
    pub async fn did_open(
        &mut self,
        file_uri: &str,
        language_id: &str,
        content: &str,
    ) -> Result<(), String> {
        let uri: Uri = file_uri.parse().map_err(|e| format!("Invalid URI: {e}"))?;
        let params = lsp_types::DidOpenTextDocumentParams {
            text_document: lsp_types::TextDocumentItem {
                uri,
                language_id: language_id.to_string(),
                version: 1,
                text: content.to_string(),
            },
        };
        self.send_notification("textDocument/didOpen", params).await
    }

    /// Notify the server that a file has been closed.
    pub async fn did_close(&mut self, file_uri: &str) -> Result<(), String> {
        let uri: Uri = file_uri.parse().map_err(|e| format!("Invalid URI: {e}"))?;
        let params = lsp_types::DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri },
        };
        self.send_notification("textDocument/didClose", params)
            .await
    }

    /// Notify the server that a file's content has changed.
    pub async fn did_change(
        &mut self,
        file_uri: &str,
        content: &str,
        version: i32,
    ) -> Result<(), String> {
        let uri: Uri = file_uri.parse().map_err(|e| format!("Invalid URI: {e}"))?;
        let params = lsp_types::DidChangeTextDocumentParams {
            text_document: lsp_types::VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content.to_string(),
            }],
        };
        self.send_notification("textDocument/didChange", params)
            .await
    }

    /// Drain all captured diagnostics from the buffer.
    pub fn drain_diagnostics(&mut self) -> Vec<LspDiagnostic> {
        std::mem::take(&mut self.diagnostics_buffer)
    }

    /// Parse a `textDocument/publishDiagnostics` notification and store results.
    fn capture_diagnostics(&mut self, params: serde_json::Value) {
        parse_publish_diagnostics(params, &mut self.diagnostics_buffer);
    }

    /// Request document symbols for a file.
    ///
    /// The file should already be opened via `did_open()`. Use `path_to_file_uri()`
    /// to convert a filesystem path.
    pub async fn document_symbols(
        &mut self,
        file_uri: &str,
    ) -> Result<Vec<DocumentSymbol>, String> {
        let uri: Uri = file_uri
            .parse()
            .map_err(|e| format!("Invalid URI '{file_uri}': {e}"))?;

        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let response: Option<DocumentSymbolResponse> = self
            .send_request("textDocument/documentSymbol", params)
            .await?;

        match response {
            Some(DocumentSymbolResponse::Flat(sym_infos)) => Ok(sym_infos
                .into_iter()
                .map(|si| {
                    #[allow(deprecated)]
                    DocumentSymbol {
                        name: si.name,
                        detail: None,
                        kind: si.kind,
                        tags: si.tags,
                        deprecated: None,
                        range: si.location.range,
                        selection_range: si.location.range,
                        children: None,
                    }
                })
                .collect()),
            Some(DocumentSymbolResponse::Nested(symbols)) => Ok(symbols),
            None => Ok(vec![]),
        }
    }

    /// Request references for a symbol at the given position.
    ///
    /// Returns all locations where the symbol is referenced (excluding its declaration).
    pub async fn references(
        &mut self,
        file_uri: &str,
        line: u32,
        character: u32,
    ) -> Result<Vec<lsp_types::Location>, String> {
        let uri: Uri = file_uri.parse().map_err(|e| format!("Invalid URI: {e}"))?;
        let params = lsp_types::ReferenceParams {
            text_document_position: lsp_types::TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: lsp_types::Position { line, character },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: lsp_types::ReferenceContext {
                include_declaration: false,
            },
        };
        let response: Option<Vec<lsp_types::Location>> =
            self.send_request("textDocument/references", params).await?;
        Ok(response.unwrap_or_default())
    }

    /// Cleanly shut down the language server.
    pub async fn shutdown(mut self) -> Result<(), String> {
        // Send shutdown request.
        let _: Option<()> = self.send_request("shutdown", ()).await.ok();

        // Send exit notification.
        let _ = self.send_notification("exit", ()).await;

        // Wait for the child to exit (with a timeout).
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), self.child.wait()).await;

        Ok(())
    }

    // ── JSON-RPC transport ──────────────────────────────────────────

    /// Send a JSON-RPC request and wait for the response.
    async fn send_request<P: Serialize, R: for<'de> Deserialize<'de>>(
        &mut self,
        method: &'static str,
        params: P,
    ) -> Result<R, String> {
        // Check if the server is still alive before writing
        if !self.is_alive() {
            return Err("Language server process has exited".to_string());
        }

        let id = self.next_id;
        self.next_id += 1;

        let request = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };

        self.write_message(&request).await?;

        // Read responses until we get one matching our id.
        // (Language servers may send notifications/requests interleaved.)
        const MAX_SKIPPED: usize = 1000;
        let mut skipped = 0usize;
        loop {
            let body = self.read_message().await?;

            // Check if this is our response (has matching id).
            if let Ok(resp) = serde_json::from_str::<JsonRpcResponse<R>>(&body) {
                if let Some(ref resp_id) = resp.id {
                    let matches = match resp_id {
                        serde_json::Value::Number(n) => n.as_i64() == Some(id),
                        _ => false,
                    };
                    if matches {
                        if let Some(error) = resp.error {
                            return Err(error.to_string());
                        }
                        return resp.result.ok_or_else(|| {
                            format!("Response to {method} (id={id}) has no result and no error")
                        });
                    }
                }
            }
            // Not our response — check if it's a publishDiagnostics notification.
            if let Ok(notif) = serde_json::from_str::<ServerNotification>(&body) {
                if notif.method.as_deref() == Some("textDocument/publishDiagnostics") {
                    if let Some(params) = notif.params {
                        self.capture_diagnostics(params);
                    }
                }
            }
            skipped += 1;
            if skipped > MAX_SKIPPED {
                return Err(format!(
                    "Exceeded {MAX_SKIPPED} non-response messages waiting for id {id}"
                ));
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    async fn send_notification<P: Serialize>(
        &mut self,
        method: &'static str,
        params: P,
    ) -> Result<(), String> {
        let notification = JsonRpcNotification {
            jsonrpc: "2.0",
            method,
            params,
        };
        self.write_message(&notification).await
    }

    /// Write an LSP message with Content-Length framing.
    async fn write_message<T: Serialize>(&mut self, msg: &T) -> Result<(), String> {
        let body = serde_json::to_string(msg).map_err(|e| format!("Serialize error: {e}"))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        self.stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| format!("Write header error: {e}"))?;
        self.stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| format!("Write body error: {e}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("Flush error: {e}"))?;

        Ok(())
    }

    /// Read one LSP message: parse Content-Length header, then read that many bytes.
    async fn read_message(&mut self) -> Result<String, String> {
        let mut content_length: Option<usize> = None;

        // Read headers until empty line.
        loop {
            let mut line = String::new();
            self.stdout
                .read_line(&mut line)
                .await
                .map_err(|e| format!("Read header error: {e}"))?;

            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }

            if let Some(val) = trimmed.strip_prefix("Content-Length:") {
                content_length = Some(
                    val.trim()
                        .parse::<usize>()
                        .map_err(|e| format!("Invalid Content-Length: {e}"))?,
                );
            }
            // Ignore other headers (Content-Type, etc.).
        }

        let len = content_length.ok_or_else(|| "Missing Content-Length header".to_string())?;

        // Guard against OOM from malicious/buggy language server
        const MAX_LSP_MESSAGE_BYTES: usize = 64 * 1024 * 1024; // 64 MB
        if len > MAX_LSP_MESSAGE_BYTES {
            return Err(format!(
                "LSP message too large ({len} bytes, max {MAX_LSP_MESSAGE_BYTES})"
            ));
        }

        let mut buf = vec![0u8; len];
        self.stdout
            .read_exact(&mut buf)
            .await
            .map_err(|e| format!("Read body error: {e}"))?;

        String::from_utf8(buf).map_err(|e| format!("Invalid UTF-8 in LSP body: {e}"))
    }
}

/// Parse a `textDocument/publishDiagnostics` notification params into LspDiagnostic entries.
fn parse_publish_diagnostics(params: serde_json::Value, buffer: &mut Vec<LspDiagnostic>) {
    let uri = params
        .get("uri")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let diagnostics = match params.get("diagnostics").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return,
    };

    for diag in diagnostics {
        let severity_num = diag.get("severity").and_then(|v| v.as_u64()).unwrap_or(4); // default to hint
        let severity = match severity_num {
            1 => "error",
            2 => "warning",
            3 => "information",
            _ => "hint",
        }
        .to_string();

        let message = diag
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let source = diag
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let range = diag.get("range").and_then(|r| r.get("start"));
        let line = range
            .and_then(|s| s.get("line"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let column = range
            .and_then(|s| s.get("character"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;

        // Bound buffer to prevent memory exhaustion from noisy LSP servers (Codex fix)
        const MAX_BUFFER_SIZE: usize = 500;
        if buffer.len() < MAX_BUFFER_SIZE {
            buffer.push(LspDiagnostic {
                file_uri: uri.clone(),
                severity,
                message,
                line,
                column,
                source,
            });
        }
    }
}

/// Convert an absolute filesystem path to a `file://` URI suitable for LSP.
pub fn file_uri(path: &str) -> String {
    path_to_file_uri(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_file_uri_absolute() {
        let uri = path_to_file_uri("/home/user/project/src/main.rs");
        assert_eq!(uri, "file:///home/user/project/src/main.rs");
    }

    #[test]
    fn test_path_to_file_uri_directory() {
        let uri = path_to_file_uri("/tmp/my-project");
        assert_eq!(uri, "file:///tmp/my-project");
    }

    #[test]
    fn test_file_uri_parses_as_lsp_uri() {
        let uri_str = path_to_file_uri("/home/user/project");
        let parsed: Result<Uri, _> = uri_str.parse();
        assert!(parsed.is_ok(), "file URI should parse as lsp_types::Uri");
    }

    #[test]
    fn test_path_to_file_uri_with_spaces() {
        let uri = path_to_file_uri("/Users/me/My Project/src/lib.rs");
        assert_eq!(uri, "file:///Users/me/My%20Project/src/lib.rs");
        // Must still parse as valid URI
        let parsed: Result<Uri, _> = uri.parse();
        assert!(parsed.is_ok(), "URI with encoded spaces should parse");
    }

    #[test]
    fn test_path_to_file_uri_with_special_chars() {
        let uri = path_to_file_uri("/home/user/café/naïve.rs");
        assert!(uri.starts_with("file:///home/user/"));
        assert!(!uri.contains("café"), "non-ASCII should be percent-encoded");
        // Verify it parses
        let parsed: Result<Uri, _> = uri.parse();
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_path_to_file_uri_preserves_normal_paths() {
        // Normal paths without special chars should be unchanged
        let uri = path_to_file_uri("/mnt/colab-disk/DurgaSaiK/forge/src/main.rs");
        assert_eq!(uri, "file:///mnt/colab-disk/DurgaSaiK/forge/src/main.rs");
    }

    #[test]
    fn test_client_struct_exists() {
        // Compile-time check that the public API surface is present.
        fn _assert_spawn_signature(_config: &super::super::detect::LspServerConfig, _root: &str) {
            // Just verifying the types compile.
        }
    }

    #[test]
    fn test_capture_diagnostics_parses_notification() {
        // Simulate a publishDiagnostics params payload
        let params = serde_json::json!({
            "uri": "file:///tmp/test.py",
            "diagnostics": [
                {
                    "range": {
                        "start": {"line": 5, "character": 10},
                        "end": {"line": 5, "character": 15}
                    },
                    "severity": 1,
                    "message": "undefined variable 'foo'",
                    "source": "pyright"
                },
                {
                    "range": {
                        "start": {"line": 12, "character": 0},
                        "end": {"line": 12, "character": 20}
                    },
                    "severity": 2,
                    "message": "unused import",
                    "source": "pyright"
                }
            ]
        });

        // We can't construct a full LspClient without spawning a process,
        // so test capture_diagnostics by creating a minimal buffer scenario.
        // We'll test the parsing logic via a standalone helper.
        let mut buffer: Vec<LspDiagnostic> = Vec::new();
        parse_publish_diagnostics(params, &mut buffer);

        assert_eq!(buffer.len(), 2);
        assert_eq!(buffer[0].file_uri, "file:///tmp/test.py");
        assert_eq!(buffer[0].severity, "error");
        assert_eq!(buffer[0].message, "undefined variable 'foo'");
        assert_eq!(buffer[0].line, 5);
        assert_eq!(buffer[0].column, 10);
        assert_eq!(buffer[0].source, "pyright");

        assert_eq!(buffer[1].severity, "warning");
        assert_eq!(buffer[1].message, "unused import");
        assert_eq!(buffer[1].line, 12);
    }

    #[test]
    fn test_capture_diagnostics_empty_array() {
        let params = serde_json::json!({
            "uri": "file:///tmp/clean.py",
            "diagnostics": []
        });
        let mut buffer: Vec<LspDiagnostic> = Vec::new();
        parse_publish_diagnostics(params, &mut buffer);
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn test_capture_diagnostics_missing_fields() {
        // Missing severity, source, range — should use defaults
        let params = serde_json::json!({
            "uri": "file:///tmp/bad.py",
            "diagnostics": [
                {
                    "message": "something wrong"
                }
            ]
        });
        let mut buffer: Vec<LspDiagnostic> = Vec::new();
        parse_publish_diagnostics(params, &mut buffer);
        assert_eq!(buffer.len(), 1);
        assert_eq!(buffer[0].severity, "hint"); // default for unknown severity
        assert_eq!(buffer[0].source, "unknown");
        assert_eq!(buffer[0].line, 0);
        assert_eq!(buffer[0].column, 0);
    }

    #[test]
    fn test_lsp_diagnostic_struct() {
        let d = LspDiagnostic {
            file_uri: "file:///tmp/test.rs".into(),
            severity: "error".into(),
            message: "test".into(),
            line: 1,
            column: 2,
            source: "rust-analyzer".into(),
        };
        assert_eq!(d.line, 1);
        assert_eq!(d.column, 2);
    }
}
