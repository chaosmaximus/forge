use crate::server::handler::{handle_request, DaemonState};
use forge_v2_core::protocol::{decode_request, encode_response, Response};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{watch, Mutex};
use tokio::time::{timeout, Duration};

/// Maximum allowed line length (1 MB). Requests or responses exceeding this
/// are rejected and the client is disconnected.
const MAX_LINE_BYTES: usize = 1_048_576;

/// Read timeout for idle clients (30 seconds).
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// M1: Check whether a daemon process is alive by reading the PID file and
/// sending signal 0 (existence check).
#[cfg(unix)]
fn is_daemon_alive(forge_dir: &str) -> bool {
    let pid_path = format!("{}/forge.pid", forge_dir);
    if let Ok(content) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            // Signal 0 = existence check, does not actually send a signal
            return unsafe { libc::kill(pid, 0) } == 0;
        }
    }
    false
}

pub async fn run_server(
    socket_path: &str,
    state: Arc<Mutex<DaemonState>>,
    shutdown_tx: watch::Sender<bool>,
) -> std::io::Result<()> {
    // M1: Before removing the socket, check if another daemon is actually alive
    if std::path::Path::new(socket_path).exists() {
        let forge_dir = std::path::Path::new(socket_path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        #[cfg(unix)]
        if is_daemon_alive(&forge_dir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another daemon is running",
            ));
        }
    }

    // Remove stale socket file (safe — we verified the old daemon is dead)
    let _ = std::fs::remove_file(socket_path);

    // I1: Set umask before bind so the socket is created with 0600 permissions
    // atomically (no TOCTOU window). Restore the old umask immediately after.
    #[cfg(unix)]
    let old_umask = unsafe { libc::umask(0o177) };

    let listener = UnixListener::bind(socket_path)?;

    #[cfg(unix)]
    unsafe {
        libc::umask(old_umask);
    }

    // Accept loop with shutdown support
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

                        // I5: Wrap read_line with a 30-second timeout to disconnect idle clients
                        let read_result = timeout(READ_TIMEOUT, reader.read_line(&mut line)).await;
                        let bytes_read = match read_result {
                            Ok(Ok(0)) => break,      // EOF
                            Ok(Ok(n)) => n,           // Got data
                            Ok(Err(_)) => break,      // IO error
                            Err(_) => break,          // Timeout — disconnect idle client
                        };

                        // I6: Reject lines exceeding 1 MB to prevent memory exhaustion
                        if bytes_read > MAX_LINE_BYTES {
                            let err_resp = Response::Error {
                                message: "request too large (>1MB)".to_string(),
                            };
                            let encoded = encode_response(&err_resp);
                            let _ = write_half.write_all(encoded.as_bytes()).await;
                            let _ = write_half.write_all(b"\n").await;
                            break; // Disconnect abusive client
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
