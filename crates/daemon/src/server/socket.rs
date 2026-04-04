use crate::events::EventSender;
use crate::server::handler::{handle_request, DaemonState};
use crate::server::writer::{is_read_only, WriteCommand};
use forge_core::protocol::{decode_request, encode_response, Request, Response};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::{timeout, Duration};

/// Maximum allowed line length (1 MB). Requests or responses exceeding this
/// are rejected and the client is disconnected.
const MAX_LINE_BYTES: usize = 1_048_576;

/// Read timeout for idle clients (30 seconds).
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Read a newline-terminated line from a BufReader with a size cap.
///
/// Uses `fill_buf` + `consume` to avoid unbounded allocation: the internal
/// buffer (8 KB by default) is the maximum that can be read in one syscall,
/// and we check accumulated size after each chunk. Returns `Ok(0)` on EOF.
async fn read_line_limited<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut String,
    max_bytes: usize,
) -> std::io::Result<usize> {
    let mut total = 0usize;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            // EOF
            return Ok(total);
        }

        let newline_pos = available.iter().position(|&b| b == b'\n');
        let to_consume = match newline_pos {
            Some(pos) => pos + 1, // include the newline
            None => available.len(),
        };

        // Check before pushing to prevent the allocation itself from being huge
        if total + to_consume > max_bytes {
            // Consume the bytes so the stream isn't stuck, then report overflow
            reader.consume(to_consume);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line exceeds maximum allowed length",
            ));
        }

        buf.push_str(&String::from_utf8_lossy(&available[..to_consume]));
        total += to_consume;
        reader.consume(to_consume);

        if newline_pos.is_some() {
            // Complete line
            return Ok(total);
        }
    }
}

/// M1: Check whether a daemon process is alive by reading the PID file and
/// sending signal 0 (existence check).
///
/// Uses the canonical PID path (`~/.forge/forge.pid` or `$FORGE_DIR/forge.pid`)
/// rather than deriving from the socket path's parent directory, which would
/// break when `FORGE_SOCKET` points to a custom location like `/tmp/forge.sock`.
#[cfg(unix)]
pub fn is_daemon_alive() -> bool {
    let forge_dir = forge_core::forge_dir();
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
    db_path: String,
    events: EventSender,
    hlc: Arc<crate::sync::Hlc>,
    started_at: Instant,
    write_tx: mpsc::Sender<WriteCommand>,
    shutdown_tx: watch::Sender<bool>,
) -> std::io::Result<()> {
    // M1: Before removing the socket, check if another daemon is actually alive
    if std::path::Path::new(socket_path).exists() {
        #[cfg(unix)]
        if is_daemon_alive() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another daemon is running",
            ));
        }

        // FAIL-LOUD: log stale socket detection so operators can see it
        eprintln!("[daemon] stale socket detected at {}, cleaning up", socket_path);
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
                let write_tx = write_tx.clone();
                let shutdown_tx_clone = shutdown_tx.clone();
                let db_path = db_path.clone();
                let events = events.clone();
                let hlc = Arc::clone(&hlc);

                tokio::spawn(async move {
                    // Open a per-connection read-only SQLite connection.
                    // This allows read requests to be served without ANY mutex.
                    let mut reader_state = match DaemonState::new_reader(
                        &db_path, events.clone(), hlc, started_at,
                    ) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("[socket] ERROR: failed to open read-only connection: {e}");
                            return;
                        }
                    };

                    let (read_half, mut write_half) = tokio::io::split(stream);
                    let mut reader = BufReader::new(read_half);
                    let mut line = String::new();

                    loop {
                        line.clear();

                        // NEW-1: Use read_line_limited (fill_buf/consume) to cap allocation
                        // BEFORE memory is committed, preventing OOM from huge payloads.
                        // Also wrapped with a timeout (I5) to disconnect idle clients.
                        let read_result = timeout(
                            READ_TIMEOUT,
                            read_line_limited(&mut reader, &mut line, MAX_LINE_BYTES),
                        )
                        .await;
                        match read_result {
                            Ok(Ok(0)) => break,       // EOF
                            Ok(Ok(_n)) => {}           // Got data — continue below
                            Ok(Err(_)) => {
                                // IO error or line too long — send error and disconnect
                                let err_resp = Response::Error {
                                    message: "request too large or read error".to_string(),
                                };
                                let encoded = encode_response(&err_resp);
                                let _ = write_half.write_all(encoded.as_bytes()).await;
                                let _ = write_half.write_all(b"\n").await;
                                break;
                            }
                            Err(_) => break,           // Timeout — disconnect idle client
                        };

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

                        // If it's a Subscribe request, enter streaming mode
                        if let Request::Subscribe { events: ref filter } = request {
                            let mut rx = events.subscribe();
                            let filter = filter.clone();
                            let mut sub_shutdown_rx = shutdown_tx_clone.subscribe();

                            // Stream events until client disconnects or shutdown
                            loop {
                                tokio::select! {
                                    result = rx.recv() => {
                                        match result {
                                            Ok(event) => {
                                                // Apply filter
                                                if let Some(ref types) = filter {
                                                    if !types.is_empty() && !types.contains(&event.event) {
                                                        continue;
                                                    }
                                                }
                                                let line = serde_json::to_string(&event).unwrap_or_default();
                                                if write_half.write_all(line.as_bytes()).await.is_err() { break; }
                                                if write_half.write_all(b"\n").await.is_err() { break; }
                                            }
                                            Err(broadcast::error::RecvError::Closed) => break,
                                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                                        }
                                    }
                                    _ = sub_shutdown_rx.changed() => break,
                                }
                            }
                            break; // Exit the connection loop after streaming ends
                        }

                        // Check for shutdown before processing
                        let is_shutdown = matches!(request, Request::Shutdown);

                        // Route: read-only requests use the per-connection read-only SQLite
                        // connection (no mutex, no contention). Write requests are sent to
                        // the writer actor via mpsc channel.
                        let response = if is_read_only(&request) {
                            handle_request(&mut reader_state, request)
                        } else {
                            // Send write request to writer actor
                            let (reply_tx, reply_rx) = oneshot::channel();
                            match write_tx.send(WriteCommand::Raw { request, reply: reply_tx }).await {
                                Ok(()) => {
                                    match reply_rx.await {
                                        Ok(resp) => resp,
                                        Err(_) => Response::Error {
                                            message: "writer actor closed unexpectedly".to_string(),
                                        },
                                    }
                                }
                                Err(_) => Response::Error {
                                    message: "daemon writer unavailable".to_string(),
                                },
                            }
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
