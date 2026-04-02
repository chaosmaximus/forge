use forge_v2_core::protocol::{Request, Response};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// Returns the default socket path: ~/.forge/forge.sock
pub fn default_socket_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{home}/.forge/forge.sock")
}

/// Connect to the daemon socket, auto-starting the daemon if needed.
///
/// 1. Try connecting to the socket.
/// 2. If it fails, spawn the `forge-daemon` binary (found next to the current exe).
/// 3. Poll every 100ms for up to 3 seconds for the socket to appear.
/// 4. Connect.
pub async fn connect() -> Result<UnixStream, String> {
    let socket_path = std::env::var("FORGE_SOCKET").unwrap_or_else(|_| default_socket_path());

    // Try connecting directly first
    if let Ok(stream) = UnixStream::connect(&socket_path).await {
        return Ok(stream);
    }

    // Socket not available — start the daemon
    let current_exe = std::env::current_exe().map_err(|e| format!("cannot find current exe: {e}"))?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| "cannot determine exe directory".to_string())?;
    let daemon_path = exe_dir.join("forge-daemon");

    if !daemon_path.exists() {
        return Err(format!(
            "forge-daemon not found at {}. Build it with: cargo build -p forge-daemon",
            daemon_path.display()
        ));
    }

    // Spawn daemon as a detached background process
    let _child = tokio::process::Command::new(&daemon_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to start forge-daemon: {e}"))?;

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

/// Send a request to the daemon and return the response.
///
/// Opens a connection, writes the request as JSON + newline, reads one response line, parses it.
pub async fn send(request: &Request) -> Result<Response, String> {
    let stream = connect().await?;
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

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("empty response from daemon".to_string());
    }

    serde_json::from_str(trimmed).map_err(|e| format!("response parse error: {e}"))
}
