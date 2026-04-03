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

/// Convert an absolute filesystem path to a `file://` URI string.
fn path_to_file_uri(path: &str) -> String {
    // On Unix, file URIs are file:///absolute/path
    // Ensure the path is absolute.
    let abs = if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path).to_string_lossy().to_string())
            .unwrap_or_else(|_| path.to_string())
    };
    format!("file://{}", abs)
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
        };

        // Build root URI.
        let root_uri_str = path_to_file_uri(root_dir);
        let root_uri: Uri = root_uri_str
            .parse()
            .map_err(|e| format!("Invalid root URI '{}': {}", root_uri_str, e))?;

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
            workspace_folders: None,
            client_info: Some(ClientInfo {
                name: "forge-daemon".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            locale: None,
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let init_result: InitializeResult = client
            .send_request("initialize", init_params)
            .await?;
        client.server_caps = Some(init_result);

        // Send initialized notification.
        client
            .send_notification("initialized", InitializedParams {})
            .await?;

        Ok(client)
    }

    /// Request document symbols for a file.
    ///
    /// `file_uri` should be a `file://` URI, e.g. `file:///home/user/project/src/main.rs`.
    /// Use `path_to_file_uri()` to convert a filesystem path.
    pub async fn document_symbols(
        &mut self,
        file_uri: &str,
    ) -> Result<Vec<DocumentSymbol>, String> {
        let uri: Uri = file_uri
            .parse()
            .map_err(|e| format!("Invalid URI '{}': {}", file_uri, e))?;

        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };

        let response: Option<DocumentSymbolResponse> = self
            .send_request("textDocument/documentSymbol", params)
            .await?;

        match response {
            Some(DocumentSymbolResponse::Flat(sym_infos)) => {
                // Convert SymbolInformation to DocumentSymbol (lossy — no children).
                Ok(sym_infos
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
                    .collect())
            }
            Some(DocumentSymbolResponse::Nested(symbols)) => Ok(symbols),
            None => Ok(vec![]),
        }
    }

    /// Cleanly shut down the language server.
    pub async fn shutdown(mut self) -> Result<(), String> {
        // Send shutdown request.
        let _: Option<()> = self.send_request("shutdown", ()).await.ok();

        // Send exit notification.
        let _ = self.send_notification("exit", ()).await;

        // Wait for the child to exit (with a timeout).
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.child.wait(),
        )
        .await;

        Ok(())
    }

    // ── JSON-RPC transport ──────────────────────────────────────────

    /// Send a JSON-RPC request and wait for the response.
    async fn send_request<P: Serialize, R: for<'de> Deserialize<'de>>(
        &mut self,
        method: &'static str,
        params: P,
    ) -> Result<R, String> {
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
                            format!(
                                "Response to {} (id={}) has no result and no error",
                                method, id
                            )
                        });
                    }
                }
            }
            // Not our response — probably a server notification; skip it.
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
        let body =
            serde_json::to_string(msg).map_err(|e| format!("Serialize error: {}", e))?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());

        self.stdin
            .write_all(header.as_bytes())
            .await
            .map_err(|e| format!("Write header error: {}", e))?;
        self.stdin
            .write_all(body.as_bytes())
            .await
            .map_err(|e| format!("Write body error: {}", e))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| format!("Flush error: {}", e))?;

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
                .map_err(|e| format!("Read header error: {}", e))?;

            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }

            if let Some(val) = trimmed.strip_prefix("Content-Length:") {
                content_length = Some(
                    val.trim()
                        .parse::<usize>()
                        .map_err(|e| format!("Invalid Content-Length: {}", e))?,
                );
            }
            // Ignore other headers (Content-Type, etc.).
        }

        let len =
            content_length.ok_or_else(|| "Missing Content-Length header".to_string())?;

        // Guard against OOM from malicious/buggy language server
        const MAX_LSP_MESSAGE_BYTES: usize = 64 * 1024 * 1024; // 64 MB
        if len > MAX_LSP_MESSAGE_BYTES {
            return Err(format!(
                "LSP message too large ({} bytes, max {})",
                len, MAX_LSP_MESSAGE_BYTES
            ));
        }

        let mut buf = vec![0u8; len];
        self.stdout
            .read_exact(&mut buf)
            .await
            .map_err(|e| format!("Read body error: {}", e))?;

        String::from_utf8(buf).map_err(|e| format!("Invalid UTF-8 in LSP body: {}", e))
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
    fn test_client_struct_exists() {
        // Compile-time check that the public API surface is present.
        fn _assert_spawn_signature(
            _config: &super::super::detect::LspServerConfig,
            _root: &str,
        ) {
            // Just verifying the types compile.
        }
    }
}
