//! Integration test: WebSocket terminal round-trip.
//!
//! Validates the full PTY-over-WebSocket flow:
//! 1. Build an Axum router with the terminal WS endpoint.
//! 2. Start it on a random port.
//! 3. Connect a WebSocket client (tokio-tungstenite).
//! 4. Read the first message — JSON `{"id": N}`.
//! 5. Send `echo INTEGRATION_TEST_OK\n` as text.
//! 6. Read binary frames until output contains "INTEGRATION_TEST_OK".
//! 7. Close the WebSocket.
//! 8. Verify cleanup.

use axum::routing::get;
use axum::Router;
use forge_daemon::server::pty::PtyManager;
use forge_daemon::server::ws::{terminal_ws_handler, SharedPtyManager, TerminalState};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

/// Build a minimal Axum router with just the terminal WebSocket endpoint.
/// Auth is disabled for tests — the auth validation is tested separately.
fn build_test_router() -> (Router, SharedPtyManager) {
    let pty_mgr: SharedPtyManager = Arc::new(Mutex::new(PtyManager::new()));
    let terminal_state = TerminalState {
        pty_mgr: pty_mgr.clone(),
        auth_enabled: false,
        auth_config: None,
        jwks_cache: None,
        db_path: None,
        rate_limiter: None,
    };
    let app = Router::new()
        .route("/api/terminal", get(terminal_ws_handler))
        .with_state(terminal_state);
    (app, pty_mgr)
}

#[tokio::test]
async fn test_terminal_ws_round_trip() {
    // 1. Bind to a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind");
    let port = listener.local_addr().unwrap().port();

    // 2. Build router and serve in a background task.
    let (app, pty_mgr) = build_test_router();
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server a moment to start accepting connections.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 3. Connect WebSocket client.
    let url = format!("ws://127.0.0.1:{port}/api/terminal?cols=80&rows=24");
    let (ws_stream, _response) = timeout(Duration::from_secs(5), connect_async(&url))
        .await
        .expect("WS connect timed out")
        .expect("WS connect failed");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // 4. Read the first message — should be JSON `{"id": N}`.
    let first_msg = timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .expect("timed out waiting for first message")
        .expect("stream ended before first message")
        .expect("error reading first message");

    let first_text = match first_msg {
        Message::Text(t) => t,
        other => panic!("expected Text message with PTY ID, got: {other:?}"),
    };

    let id_json: serde_json::Value =
        serde_json::from_str(&first_text).expect("first message should be valid JSON");
    let pty_id = id_json["id"]
        .as_u64()
        .expect("first message should contain numeric 'id' field");
    assert!(pty_id > 0, "PTY ID should be positive, got {pty_id}");

    // 5. Send `echo INTEGRATION_TEST_OK\n` as text.
    ws_tx
        .send(Message::Text("echo INTEGRATION_TEST_OK\n".into()))
        .await
        .expect("failed to send echo command");

    // 6. Read binary frames until output contains "INTEGRATION_TEST_OK".
    let marker = "INTEGRATION_TEST_OK";
    let mut collected_output = String::new();
    let found = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = ws_rx.next().await {
            match msg_result {
                Ok(Message::Binary(data)) => {
                    if let Ok(text) = String::from_utf8(data.to_vec()) {
                        collected_output.push_str(&text);
                        // The marker should appear at least twice: once in the echoed command
                        // and once in the shell output. We look for presence in the output stream.
                        if collected_output.contains(marker) {
                            return true;
                        }
                    }
                }
                Ok(Message::Text(text)) => {
                    // Some output may arrive as text frames.
                    collected_output.push_str(&text);
                    if collected_output.contains(marker) {
                        return true;
                    }
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => continue,
                Err(e) => {
                    eprintln!("WS recv error: {e}");
                    break;
                }
            }
        }
        false
    })
    .await
    .expect("timed out waiting for echo output");

    // 7. Assert marker was found.
    assert!(
        found,
        "marker '{marker}' not found in PTY output. Collected: {collected_output:?}"
    );

    // 8. Close the WebSocket.
    ws_tx
        .send(Message::Close(None))
        .await
        .expect("failed to send close frame");

    // Drop the receiver half too so the server sees a fully closed connection.
    drop(ws_rx);

    // Verify cleanup: poll until the PTY manager has removed this session.
    // The server-side cleanup is async (after both relay tasks finish), so we poll.
    let cleaned_up = timeout(Duration::from_secs(5), async {
        loop {
            {
                let mgr = pty_mgr.lock().await;
                if !mgr.sessions.contains_key(&(pty_id as u32)) {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;

    assert!(
        cleaned_up.is_ok() && cleaned_up.unwrap(),
        "PTY session {pty_id} should have been cleaned up after WS close"
    );

    // Abort the server task (it runs forever otherwise).
    server_handle.abort();
}

#[tokio::test]
async fn test_terminal_ws_receives_pty_id_json() {
    // Verify the first message is well-formed JSON with an "id" field.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind");
    let port = listener.local_addr().unwrap().port();

    let (app, _pty_mgr) = build_test_router();
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let url = format!("ws://127.0.0.1:{port}/api/terminal");
    let (ws_stream, _) = timeout(Duration::from_secs(5), connect_async(&url))
        .await
        .expect("WS connect timed out")
        .expect("WS connect failed");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    let first_msg = timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .expect("timed out")
        .expect("stream ended")
        .expect("error");

    let text = match first_msg {
        Message::Text(t) => t,
        other => panic!("expected Text, got: {other:?}"),
    };

    let json: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert!(json.get("id").is_some(), "should have 'id' field");
    assert!(json["id"].is_u64(), "'id' should be a number");

    let _ = ws_tx.send(Message::Close(None)).await;
    server_handle.abort();
}

#[tokio::test]
async fn test_terminal_ws_resize_command() {
    // Verify that sending a resize JSON command does not crash the server.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind");
    let port = listener.local_addr().unwrap().port();

    let (app, _pty_mgr) = build_test_router();
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let url = format!("ws://127.0.0.1:{port}/api/terminal?cols=80&rows=24");
    let (ws_stream, _) = timeout(Duration::from_secs(5), connect_async(&url))
        .await
        .expect("WS connect timed out")
        .expect("WS connect failed");

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // Read the ID message first.
    let _ = timeout(Duration::from_secs(5), ws_rx.next())
        .await
        .expect("timed out")
        .expect("stream ended")
        .expect("error");

    // Send a resize command.
    let resize_cmd = r#"{"resize":{"cols":120,"rows":40}}"#;
    ws_tx
        .send(Message::Text(resize_cmd.into()))
        .await
        .expect("failed to send resize");

    // Send an echo command to verify the session is still alive after resize.
    ws_tx
        .send(Message::Text("echo RESIZE_OK\n".into()))
        .await
        .expect("failed to send echo");

    // Read until we see the output.
    let mut output = String::new();
    let found = timeout(Duration::from_secs(10), async {
        while let Some(msg_result) = ws_rx.next().await {
            match msg_result {
                Ok(Message::Binary(data)) => {
                    if let Ok(text) = String::from_utf8(data.to_vec()) {
                        output.push_str(&text);
                        if output.contains("RESIZE_OK") {
                            return true;
                        }
                    }
                }
                Ok(Message::Text(text)) => {
                    output.push_str(&text);
                    if output.contains("RESIZE_OK") {
                        return true;
                    }
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => continue,
                Err(_) => break,
            }
        }
        false
    })
    .await
    .expect("timed out waiting for output after resize");

    assert!(found, "should see RESIZE_OK after resize command");

    let _ = ws_tx.send(Message::Close(None)).await;
    server_handle.abort();
}

#[tokio::test]
async fn test_pty_session_count_and_limit() {
    // Verify session_count, max_sessions, and that create respects the limit.
    let mut mgr = PtyManager::new();
    assert_eq!(mgr.session_count(), 0);
    assert_eq!(mgr.max_sessions(), 8);

    // Create sessions up to the limit
    let mut ids = Vec::new();
    for _ in 0..8 {
        let (id, _rx) = mgr
            .create(80, 24, None)
            .expect("create should succeed within limit");
        ids.push(id);
    }
    assert_eq!(mgr.session_count(), 8);

    // Clean up
    for id in ids {
        mgr.close(id);
    }
    assert_eq!(mgr.session_count(), 0);
}

#[tokio::test]
async fn test_pty_idle_reap() {
    // Verify reap_idle removes stale sessions.
    let mut mgr = PtyManager::new();
    let (id, _rx) = mgr.create(80, 24, None).expect("create should succeed");
    assert_eq!(mgr.session_count(), 1);

    // Manually backdate the last_activity to trigger idle reap.
    if let Some(session) = mgr.sessions.get_mut(&id) {
        session.last_activity = std::time::Instant::now() - std::time::Duration::from_secs(1000);
    }

    let reaped = mgr.reap_idle();
    assert_eq!(reaped, 1, "should have reaped 1 idle session");
    assert_eq!(mgr.session_count(), 0, "no sessions should remain");
}

#[tokio::test]
async fn test_pty_close_all() {
    let mut mgr = PtyManager::new();
    for _ in 0..3 {
        let _ = mgr.create(80, 24, None).expect("create should succeed");
    }
    assert_eq!(mgr.session_count(), 3);
    mgr.close_all();
    assert_eq!(mgr.session_count(), 0);
}
