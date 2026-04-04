use forge_core::protocol::{Request, Response};
use forge_core::default_socket_path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

/// Maximum allowed response line length (1 MB).
const MAX_RESPONSE_LINE_BYTES: usize = 1_048_576;

/// NEW-7: Read timeout for daemon responses (30 seconds).
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

/// NEW-1: Read a newline-terminated line with a size cap using fill_buf/consume.
///
/// Prevents unbounded allocation: the BufReader's internal buffer (8 KB) is
/// the most that can be read per syscall, and we check accumulated size after
/// each chunk. Returns `Ok(0)` on EOF.
async fn read_line_limited<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut String,
    max_bytes: usize,
) -> std::io::Result<usize> {
    let mut total = 0usize;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(total);
        }

        let newline_pos = available.iter().position(|&b| b == b'\n');
        let to_consume = match newline_pos {
            Some(pos) => pos + 1,
            None => available.len(),
        };

        if total + to_consume > max_bytes {
            reader.consume(to_consume);
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "response line exceeds maximum allowed length",
            ));
        }

        buf.push_str(&String::from_utf8_lossy(&available[..to_consume]));
        total += to_consume;
        reader.consume(to_consume);

        if newline_pos.is_some() {
            return Ok(total);
        }
    }
}

/// M3: Find the daemon binary — try sibling directory first, then fall back to PATH.
fn find_daemon_binary() -> String {
    // 1. Check sibling directory (development / local install)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("forge-daemon");
            if sibling.exists() {
                return sibling.to_string_lossy().to_string();
            }
        }
    }
    // 2. Fall back to PATH
    "forge-daemon".to_string()
}

/// Connect to the daemon socket without auto-starting the daemon.
/// Returns an error if the daemon is not running.
pub async fn connect_no_autostart() -> Result<UnixStream, String> {
    let socket_path = std::env::var("FORGE_SOCKET").unwrap_or_else(|_| default_socket_path());
    UnixStream::connect(&socket_path)
        .await
        .map_err(|e| format!("daemon not running: {e}"))
}

/// Connect to the daemon socket, auto-starting the daemon if needed.
///
/// 1. Try connecting to the socket.
/// 2. If it fails, spawn the `forge-daemon` binary (sibling dir or PATH).
/// 3. Poll every 100ms for up to 3 seconds for the socket to appear.
/// 4. Connect.
pub async fn connect() -> Result<UnixStream, String> {
    let socket_path = std::env::var("FORGE_SOCKET").unwrap_or_else(|_| default_socket_path());

    // Try connecting directly first
    if let Ok(stream) = UnixStream::connect(&socket_path).await {
        return Ok(stream);
    }

    // Socket not available — check for stale socket and clean up before starting daemon
    if std::path::Path::new(&socket_path).exists() {
        eprintln!("[cli] WARN: stale socket detected at {} — removing before daemon start", socket_path);
        let _ = std::fs::remove_file(&socket_path);
    }

    // Start the daemon
    let daemon_path = find_daemon_binary();

    // Spawn daemon as a detached background process.
    // Pass through FORGE_PROJECT and FORGE_DB if set in the CLI environment.
    let mut cmd = tokio::process::Command::new(&daemon_path);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // Forward relevant env vars to daemon
    if let Ok(v) = std::env::var("FORGE_PROJECT") {
        cmd.env("FORGE_PROJECT", v);
    }
    if let Ok(v) = std::env::var("FORGE_DB") {
        cmd.env("FORGE_DB", v);
    }
    if let Ok(v) = std::env::var("FORGE_SOCKET") {
        cmd.env("FORGE_SOCKET", v);
    }

    let child = cmd.spawn()
        .map_err(|e| format!("failed to start forge-daemon at '{}': {e}", daemon_path))?;

    // Explicitly drop the child handle so the CLI doesn't hold a reference
    drop(child);

    // Poll for socket availability (up to 3 seconds, every 100ms)
    let max_attempts = 30;
    for _ in 0..max_attempts {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if let Ok(stream) = UnixStream::connect(&socket_path).await {
            return Ok(stream);
        }
    }

    // Daemon started but socket never appeared — clean up stale socket if present
    if std::path::Path::new(&socket_path).exists() {
        eprintln!("[cli] WARN: daemon started but socket not connectable — cleaning stale socket at {}", socket_path);
        let _ = std::fs::remove_file(&socket_path);
    }

    Err(format!(
        "forge-daemon started but socket not available after 3s at {socket_path}"
    ))
}

/// Send a request to the daemon (with auto-start) and return the response.
///
/// Opens a connection, writes the request as JSON + newline, reads one response line, parses it.
pub async fn send(request: &Request) -> Result<Response, String> {
    send_on_stream(connect().await?, request).await
}

/// Send a request to the daemon without auto-starting it.
/// Returns an error if the daemon is not running.
pub async fn send_no_autostart(request: &Request) -> Result<Response, String> {
    send_on_stream(connect_no_autostart().await?, request).await
}

/// Internal: send a request on an already-connected stream and read the response.
async fn send_on_stream(stream: UnixStream, request: &Request) -> Result<Response, String> {
    let (read_half, mut write_half) = tokio::io::split(stream);

    // Serialize and send request
    let json = serde_json::to_string(request).map_err(|e| format!("serialize error: {e}"))?;
    write_half
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("write error: {e}"))?;
    write_half
        .write_all(b"\n")
        .await
        .map_err(|e| format!("write newline error: {e}"))?;

    // NEW-1 + NEW-7: Read response with size cap (fill_buf/consume) and 30s timeout
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    let read_result = timeout(
        CLIENT_TIMEOUT,
        read_line_limited(&mut reader, &mut line, MAX_RESPONSE_LINE_BYTES),
    )
    .await;
    match read_result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => return Err(format!("read error: {e}")),
        Err(_) => return Err("daemon response timed out (30s)".to_string()),
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("empty response from daemon".to_string());
    }

    serde_json::from_str(trimmed).map_err(|e| format!("response parse error: {e}"))
}
