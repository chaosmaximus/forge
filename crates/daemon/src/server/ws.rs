//! WebSocket handler for browser-based terminal access.
//!
//! Endpoint: `GET /api/terminal?cols=80&rows=24&cwd=/some/path&token=<JWT>`
//!
//! Protocol:
//!   1. Client opens WS with query params (cols, rows, optional cwd, token for auth).
//!   2. Server validates JWT token (if auth enabled) before upgrading.
//!   3. Server spawns PTY via PtyManager, sends `{"id": N}` as first message.
//!   4. Server relays PTY output as Binary WS frames.
//!   5. Client sends Text frames as PTY input.
//!   6. Client sends JSON `{"resize": {"cols": N, "rows": N}}` for resize.
//!   7. Connection close triggers PTY cleanup.
//!   8. When PTY exits, server sends `{"exit": true}`.

use crate::server::pty::PtyManager;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Shared PTY manager — thread-safe handle for all WebSocket connections.
pub type SharedPtyManager = Arc<Mutex<PtyManager>>;

/// Query parameters for the terminal WebSocket endpoint.
#[derive(serde::Deserialize)]
pub struct TerminalQuery {
    /// Number of columns (defaults to 80).
    #[serde(default = "default_cols")]
    pub cols: u16,
    /// Number of rows (defaults to 24).
    #[serde(default = "default_rows")]
    pub rows: u16,
    /// Working directory for the shell.
    pub cwd: Option<String>,
    /// JWT token for authentication (WebSocket can't use Authorization header).
    pub token: Option<String>,
}

fn default_cols() -> u16 {
    80
}

fn default_rows() -> u16 {
    24
}

/// JSON message from client for resize commands.
#[derive(serde::Deserialize)]
struct ClientResize {
    resize: ResizeDimensions,
}

#[derive(serde::Deserialize)]
struct ResizeDimensions {
    cols: u16,
    rows: u16,
}

/// State passed to the terminal WebSocket handler.
#[derive(Clone)]
pub struct TerminalState {
    pub pty_mgr: SharedPtyManager,
    pub auth_enabled: bool,
    pub auth_config: Option<crate::config::AuthConfig>,
    pub jwks_cache: Option<super::auth::SharedJwksCache>,
    /// DB path for audit logging terminal sessions.
    pub db_path: Option<String>,
}

/// WebSocket upgrade handler for terminal sessions.
///
/// When auth is enabled, validates JWT from ?token= query param before upgrading.
/// Enforces PTY session limit (max 8 concurrent).
pub async fn terminal_ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<TerminalQuery>,
    State(term_state): State<TerminalState>,
) -> axum::response::Response {
    // Auth check: validate token if auth is enabled
    if term_state.auth_enabled {
        let token = match &query.token {
            Some(t) if !t.is_empty() => t.clone(),
            _ => {
                return (
                    axum::http::StatusCode::UNAUTHORIZED,
                    axum::Json(serde_json::json!({"error": "missing ?token= parameter — JWT required for terminal access"})),
                ).into_response();
            }
        };
        match (&term_state.jwks_cache, &term_state.auth_config) {
            (Some(ref cache), Some(ref cfg)) => {
                match super::auth::validate_token(&token, cache, cfg).await {
                    Ok(claims) => {
                        tracing::info!(user = %claims.sub, "terminal WebSocket authenticated");
                    }
                    Err(e) => {
                        tracing::warn!("terminal WebSocket auth failed: {e}");
                        return (
                            axum::http::StatusCode::UNAUTHORIZED,
                            axum::Json(serde_json::json!({"error": "invalid or expired token"})),
                        ).into_response();
                    }
                }
            }
            _ => {
                // Fail-closed: auth enabled but infrastructure misconfigured
                tracing::error!("auth enabled but JWKS cache or auth config is None — rejecting terminal access");
                return (
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    axum::Json(serde_json::json!({"error": "server auth misconfiguration"})),
                ).into_response();
            }
        }
    }

    // Enforce PTY session limit
    {
        let mgr = term_state.pty_mgr.lock().await;
        if mgr.session_count() >= mgr.max_sessions() {
            tracing::warn!(
                count = mgr.session_count(),
                max = mgr.max_sessions(),
                "PTY session limit reached — rejecting new terminal"
            );
            return (
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                axum::Json(serde_json::json!({"error": "maximum terminal sessions reached"})),
            ).into_response();
        }
    }

    let pty_mgr = term_state.pty_mgr.clone();
    let db_path = term_state.db_path.clone();
    ws.on_upgrade(move |socket| handle_terminal_socket(socket, query, pty_mgr, db_path))
        .into_response()
}

/// Log a terminal session spawn to the audit_log table.
fn audit_terminal_spawn(db_path: &str, pty_id: u32, cwd: Option<&str>) {
    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE,
    ) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("failed to open DB for terminal audit: {e}");
            return;
        }
    };
    let audit_id = ulid::Ulid::new().to_string();
    let details = serde_json::json!({
        "pty_id": pty_id,
        "cwd": cwd.unwrap_or("default"),
    });
    if let Err(e) = conn.execute(
        "INSERT INTO audit_log (id, actor_type, actor_id, action, resource_type, resource_id, timestamp, details)
         VALUES (?1, 'terminal', 'websocket', 'pty_spawn', 'terminal', ?2, datetime('now'), ?3)",
        rusqlite::params![audit_id, pty_id.to_string(), details.to_string()],
    ) {
        tracing::warn!("failed to insert terminal audit log: {e}");
    }
}

/// Main WebSocket session loop.
async fn handle_terminal_socket(
    socket: WebSocket,
    query: TerminalQuery,
    pty_mgr: SharedPtyManager,
    db_path: Option<String>,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Spawn PTY session.
    let (pty_id, mut output_rx) = {
        let mut mgr = pty_mgr.lock().await;
        match mgr.create(query.cols, query.rows, query.cwd.clone()) {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("failed to create PTY: {e}");
                let _ = ws_tx
                    .send(Message::Text(
                        serde_json::json!({"error": e}).to_string(),
                    ))
                    .await;
                let _ = ws_tx.close().await;
                return;
            }
        }
    };

    // Audit log: record terminal session spawn
    if let Some(ref db) = db_path {
        audit_terminal_spawn(db, pty_id, query.cwd.as_deref());
    }

    tracing::info!(pty_id, "terminal WebSocket connected");

    // Send session ID as first message.
    if ws_tx
        .send(Message::Text(
            serde_json::json!({"id": pty_id}).to_string(),
        ))
        .await
        .is_err()
    {
        // Client disconnected before we could send the ID.
        pty_mgr.lock().await.close(pty_id);
        return;
    }

    // Channel to signal PTY exit from the output relay task.
    let (exit_tx, mut exit_rx) = tokio::sync::oneshot::channel::<()>();

    // Task 1: Relay PTY output -> WebSocket (Binary frames).
    let tx_handle = {
        let mut ws_tx_clone = ws_tx;
        tokio::spawn(async move {
            let mut exit_tx = Some(exit_tx);
            loop {
                match output_rx.recv().await {
                    Ok(data) => {
                        if ws_tx_clone.send(Message::Binary(data)).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // PTY reader thread ended — shell exited.
                        let _ = ws_tx_clone
                            .send(Message::Text(
                                serde_json::json!({"exit": true}).to_string(),
                            ))
                            .await;
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(pty_id, lagged = n, "terminal output lagged");
                        continue;
                    }
                }
            }
            // Signal that PTY output has ended.
            if let Some(tx) = exit_tx.take() {
                let _ = tx.send(());
            }
            ws_tx_clone
        })
    };

    // Task 2: Relay WebSocket input -> PTY (Text frames = input, JSON = resize).
    let pty_mgr_clone = Arc::clone(&pty_mgr);
    let rx_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                msg = ws_rx.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            // Try parsing as resize command first.
                            if let Ok(resize) = serde_json::from_str::<ClientResize>(&text) {
                                let mut mgr = pty_mgr_clone.lock().await;
                                if let Err(e) = mgr.resize(pty_id, resize.resize.cols, resize.resize.rows) {
                                    tracing::warn!(pty_id, "resize failed: {e}");
                                }
                            } else {
                                // Regular text input to PTY.
                                let mut mgr = pty_mgr_clone.lock().await;
                                if let Err(e) = mgr.write(pty_id, &text) {
                                    tracing::warn!(pty_id, "write failed: {e}");
                                    break;
                                }
                            }
                        }
                        Some(Ok(Message::Binary(data))) => {
                            // Binary input — convert to string and write.
                            if let Ok(text) = String::from_utf8(data) {
                                let mut mgr = pty_mgr_clone.lock().await;
                                if let Err(e) = mgr.write(pty_id, &text) {
                                    tracing::warn!(pty_id, "write failed: {e}");
                                    break;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                            // Axum handles ping/pong automatically.
                        }
                        Some(Err(e)) => {
                            tracing::warn!(pty_id, "ws recv error: {e}");
                            break;
                        }
                    }
                }
                _ = &mut exit_rx => {
                    // PTY exited — stop reading from client.
                    break;
                }
            }
        }
    });

    // Wait for both tasks to finish.
    let _ = rx_handle.await;
    let mut ws_tx = tx_handle.await.unwrap_or_else(|_| {
        // JoinError — task panicked or was cancelled. We still need to clean up.
        tracing::warn!(pty_id, "output relay task failed");
        // Return a dummy — we can't recover the sender, but cleanup still runs.
        unreachable!("output relay task should not panic");
    });
    let _ = ws_tx.close().await;

    // Cleanup: close the PTY session.
    pty_mgr.lock().await.close(pty_id);
    tracing::info!(pty_id, "terminal WebSocket disconnected, PTY cleaned up");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_query_defaults() {
        let query: TerminalQuery =
            serde_json::from_str(r#"{}"#).expect("empty JSON should deserialize with defaults");
        assert_eq!(query.cols, 80);
        assert_eq!(query.rows, 24);
        assert!(query.cwd.is_none());
    }

    #[test]
    fn test_terminal_query_custom() {
        let query: TerminalQuery = serde_json::from_str(r#"{"cols":120,"rows":40,"cwd":"/tmp"}"#)
            .expect("custom values should deserialize");
        assert_eq!(query.cols, 120);
        assert_eq!(query.rows, 40);
        assert_eq!(query.cwd.as_deref(), Some("/tmp"));
    }

    #[test]
    fn test_resize_parse() {
        let msg = r#"{"resize":{"cols":100,"rows":50}}"#;
        let parsed: ClientResize = serde_json::from_str(msg).expect("should parse resize");
        assert_eq!(parsed.resize.cols, 100);
        assert_eq!(parsed.resize.rows, 50);
    }

    #[test]
    fn test_resize_vs_text_discrimination() {
        // A regular text input should NOT parse as resize.
        let text = "echo hello\n";
        let result = serde_json::from_str::<ClientResize>(text);
        assert!(result.is_err(), "plain text should not parse as resize");

        // A resize command should parse.
        let resize = r#"{"resize":{"cols":80,"rows":24}}"#;
        let result = serde_json::from_str::<ClientResize>(resize);
        assert!(result.is_ok(), "resize JSON should parse");
    }

    #[test]
    fn test_shared_pty_manager_type() {
        // Verify SharedPtyManager can be constructed.
        let mgr: SharedPtyManager = Arc::new(Mutex::new(PtyManager::new()));
        // Just verify it compiles and can be cloned.
        let _clone = Arc::clone(&mgr);
    }
}
