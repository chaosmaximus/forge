use forge_v2_core::protocol::{Request, Response};
use forge_v2_core::default_socket_path;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Maximum allowed response line length (1 MB).
const MAX_RESPONSE_LINE_BYTES: usize = 1_048_576;

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

    // Socket not available — start the daemon
    let daemon_path = find_daemon_binary();

    // C3: Spawn daemon as a detached background process
    // Use Stdio::null() for stderr to prevent the daemon from hanging on a broken pipe
    let child = tokio::process::Command::new(&daemon_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
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

    // Read response line
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .await
        .map_err(|e| format!("read error: {e}"))?;

    // I6: Check response line length before parsing
    if line.len() > MAX_RESPONSE_LINE_BYTES {
        return Err("response too large from daemon (>1MB)".to_string());
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("empty response from daemon".to_string());
    }

    serde_json::from_str(trimmed).map_err(|e| format!("response parse error: {e}"))
}
