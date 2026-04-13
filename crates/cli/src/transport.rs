//! HTTP transport for remote forge-daemon connections.
//!
//! When `--endpoint` is provided (or `FORGE_ENDPOINT` env var), the CLI
//! sends requests over HTTP instead of the local Unix socket.

use forge_core::protocol::{Request, Response};
use std::sync::OnceLock;

/// Global transport configuration, set once at startup from CLI flags / env vars.
static GLOBAL_TRANSPORT: OnceLock<Transport> = OnceLock::new();

/// Transport mode for daemon communication.
#[derive(Debug, Clone)]
pub enum Transport {
    /// Local Unix domain socket (default). Uses existing client module.
    Unix,
    /// Remote HTTP endpoint with optional JWT auth.
    Http {
        endpoint: String,
        token: Option<String>,
    },
}

impl Transport {
    /// Detect transport from CLI flags and env vars.
    ///
    /// Priority: CLI flag > env var > default (Unix).
    pub fn detect(endpoint: Option<&str>, token: Option<&str>) -> Self {
        if let Some(ep) = endpoint {
            return Transport::Http {
                endpoint: ep.to_string(),
                token: token.map(|t| t.to_string()),
            };
        }
        if let Ok(ep) = std::env::var("FORGE_ENDPOINT") {
            return Transport::Http {
                endpoint: ep,
                token: token
                    .map(|t| t.to_string())
                    .or_else(|| std::env::var("FORGE_TOKEN").ok()),
            };
        }
        Transport::Unix
    }

    /// Initialize the global transport. Must be called once at startup.
    /// Panics if called more than once (programming error).
    pub fn init_global(transport: Transport) {
        if GLOBAL_TRANSPORT.set(transport).is_err() {
            eprintln!("[cli] WARN: transport already initialized, ignoring re-init");
        }
    }

    /// Get the global transport. Returns `Unix` if not initialized.
    pub fn global() -> &'static Transport {
        GLOBAL_TRANSPORT.get_or_init(|| Transport::Unix)
    }

    /// Returns true if this transport targets a remote HTTP endpoint.
    #[allow(dead_code)]
    pub fn is_http(&self) -> bool {
        matches!(self, Transport::Http { .. })
    }
}

/// Returns true if the endpoint is a localhost address (exempt from HTTPS requirement).
fn is_localhost(endpoint: &str) -> bool {
    endpoint.starts_with("http://localhost")
        || endpoint.starts_with("http://127.0.0.1")
        || endpoint.starts_with("http://[::1]")
}

/// Validate endpoint URL and enforce HTTPS for non-localhost.
fn validate_endpoint(endpoint: &str) -> Result<String, String> {
    let trimmed = endpoint.trim_end_matches('/');
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(format!(
            "endpoint must start with https:// or http:// (got: {trimmed})"
        ));
    }
    if trimmed.starts_with("http://") && !is_localhost(trimmed) {
        return Err(format!(
            "endpoint must use HTTPS for non-localhost targets (got: {trimmed}). \
             Use https:// or connect to localhost for development."
        ));
    }
    Ok(format!("{trimmed}/api"))
}

/// Send a request over HTTP to the remote daemon.
pub async fn http_send(
    endpoint: &str,
    token: Option<&str>,
    request: &Request,
) -> Result<Response, String> {
    let url = validate_endpoint(endpoint)?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("HTTP client error: {e}"))?;

    let mut builder = client.post(&url).json(request);
    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {t}"));
    }

    let resp = builder
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("authentication failed: invalid or expired token".to_string());
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err("permission denied: insufficient role".to_string());
    }
    if !status.is_success() {
        return Err(format!("HTTP error: {}", status.as_u16()));
    }

    resp.json::<Response>()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
}

/// Subscribe to daemon events via streaming transport.
/// For Unix socket: sends Subscribe request, reads NDJSON lines from stream.
/// For HTTP: connects to GET /api/subscribe SSE endpoint.
pub async fn subscribe_stream(
    events: Option<Vec<String>>,
    session_id: Option<String>,
    team_id: Option<String>,
) -> Result<(), String> {
    let transport = Transport::global();

    match transport {
        Transport::Unix => unix_subscribe(events, session_id, team_id).await,
        Transport::Http { endpoint, token } => {
            http_subscribe(endpoint, token.as_deref(), events, session_id, team_id).await
        }
    }
}

async fn unix_subscribe(
    events: Option<Vec<String>>,
    session_id: Option<String>,
    team_id: Option<String>,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let stream = crate::client::connect()
        .await
        .map_err(|e| format!("connect failed: {e}"))?;

    let req = Request::Subscribe {
        events,
        session_id,
        team_id,
    };
    let json = serde_json::to_string(&req).map_err(|e| format!("serialize: {e}"))?;

    let (read_half, mut write_half) = tokio::io::split(stream);

    write_half
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;
    write_half
        .write_all(b"\n")
        .await
        .map_err(|e| format!("write newline: {e}"))?;

    let reader = BufReader::new(read_half);
    let mut lines = reader.lines();

    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        println!("{line}");
                    }
                    Ok(None) => break, // EOF
                    Err(e) => {
                        eprintln!("read error: {e}");
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nDisconnecting...");
                break;
            }
        }
    }

    Ok(())
}

async fn http_subscribe(
    endpoint: &str,
    token: Option<&str>,
    events: Option<Vec<String>>,
    session_id: Option<String>,
    team_id: Option<String>,
) -> Result<(), String> {
    use futures_util::StreamExt;

    let mut url = format!("{}/api/subscribe", endpoint.trim_end_matches('/'));
    let mut params = Vec::new();
    if let Some(ref evts) = events {
        params.push(format!("events={}", evts.join(",")));
    }
    if let Some(ref sid) = session_id {
        params.push(format!("session_id={sid}"));
    }
    if let Some(ref tid) = team_id {
        params.push(format!("team_id={tid}"));
    }
    if let Some(t) = token {
        params.push(format!("token={t}"));
    }
    if !params.is_empty() {
        url = format!("{}?{}", url, params.join("&"));
    }

    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .header("Accept", "text/event-stream")
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!(
            "HTTP {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }

    let mut stream = resp.bytes_stream();

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        // SSE format: "data: {...}\n\n"
                        for line in text.lines() {
                            if let Some(data) = line.strip_prefix("data: ") {
                                println!("{data}");
                            }
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("stream error: {e}");
                        break;
                    }
                    None => break, // Stream ended
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nDisconnecting...");
                break;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env-var-dependent tests to avoid races
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_detect_defaults_to_unix() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("FORGE_ENDPOINT");
        std::env::remove_var("FORGE_TOKEN");
        let t = Transport::detect(None, None);
        assert!(matches!(t, Transport::Unix));
    }

    #[test]
    fn test_detect_from_flag() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("FORGE_ENDPOINT");
        let t = Transport::detect(Some("https://forge.example.com"), Some("my-token"));
        match t {
            Transport::Http { endpoint, token } => {
                assert_eq!(endpoint, "https://forge.example.com");
                assert_eq!(token, Some("my-token".to_string()));
            }
            _ => panic!("expected Http"),
        }
    }

    #[test]
    fn test_detect_from_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FORGE_ENDPOINT", "https://env.example.com");
        std::env::set_var("FORGE_TOKEN", "env-token");
        let t = Transport::detect(None, None);
        match t {
            Transport::Http { endpoint, token } => {
                assert_eq!(endpoint, "https://env.example.com");
                assert_eq!(token, Some("env-token".to_string()));
            }
            _ => panic!("expected Http"),
        }
        std::env::remove_var("FORGE_ENDPOINT");
        std::env::remove_var("FORGE_TOKEN");
    }

    #[test]
    fn test_flag_overrides_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("FORGE_ENDPOINT", "https://env.example.com");
        let t = Transport::detect(Some("https://flag.example.com"), None);
        match t {
            Transport::Http { endpoint, .. } => {
                assert_eq!(endpoint, "https://flag.example.com");
            }
            _ => panic!("expected Http"),
        }
        std::env::remove_var("FORGE_ENDPOINT");
    }

    #[test]
    fn test_validate_endpoint_https_required() {
        assert!(validate_endpoint("https://forge.company.com").is_ok());
        assert!(validate_endpoint("http://localhost:8420").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:8420").is_ok());
        assert!(validate_endpoint("http://remote.server.com").is_err());
        assert!(validate_endpoint("ftp://bad.com").is_err());
    }

    #[test]
    fn test_is_http() {
        let unix = Transport::Unix;
        assert!(!unix.is_http());

        let http = Transport::Http {
            endpoint: "https://example.com".to_string(),
            token: None,
        };
        assert!(http.is_http());
    }
}
