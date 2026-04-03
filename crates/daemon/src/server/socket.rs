use crate::server::handler::{handle_request, DaemonState};
use forge_v2_core::protocol::{decode_request, encode_response, Response};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{watch, Mutex};

pub async fn run_server(
    socket_path: &str,
    state: Arc<Mutex<DaemonState>>,
    shutdown_tx: watch::Sender<bool>,
) -> std::io::Result<()> {
    // 1. Remove stale socket file
    let _ = std::fs::remove_file(socket_path);

    // 2. Bind UnixListener
    let listener = UnixListener::bind(socket_path)?;

    // 3. Set permissions 0600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, perms)?;
    }

    // 4. Accept loop with shutdown support
    let mut shutdown_rx = shutdown_tx.subscribe();
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (stream, _addr) = result?;
                let state_clone: Arc<Mutex<DaemonState>> = Arc::clone(&state);
                let shutdown_tx_clone = shutdown_tx.clone();

                tokio::spawn(async move {
                    let (read_half, mut write_half) = tokio::io::split(stream);
                    let mut reader = BufReader::new(read_half);
                    let mut line = String::new();

                    loop {
                        line.clear();
                        let bytes_read = match reader.read_line(&mut line).await {
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        if bytes_read == 0 {
                            // EOF
                            break;
                        }

                        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                        if trimmed.is_empty() {
                            continue;
                        }

                        // Decode request
                        let request = match decode_request(trimmed) {
                            Ok(req) => req,
                            Err(e) => {
                                let err_resp = Response::Error {
                                    message: format!("parse error: {e}"),
                                };
                                let encoded = encode_response(&err_resp);
                                let _ = write_half.write_all(encoded.as_bytes()).await;
                                let _ = write_half.write_all(b"\n").await;
                                continue;
                            }
                        };

                        // Check for shutdown before acquiring lock so we can respond then exit
                        let is_shutdown = matches!(request, forge_v2_core::protocol::Request::Shutdown);

                        // Handle request
                        let response = {
                            let mut locked = state_clone.lock().await;
                            handle_request(&mut locked, request)
                        };

                        // Write response + newline
                        let encoded = encode_response(&response);
                        let _ = write_half.write_all(encoded.as_bytes()).await;
                        let _ = write_half.write_all(b"\n").await;

                        // If shutdown, signal the server to stop
                        if is_shutdown {
                            let _ = shutdown_tx_clone.send(true);
                            break;
                        }
                    }
                });
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[daemon] shutdown signal received");
                break;
            }
        }
    }

    Ok(())
}
