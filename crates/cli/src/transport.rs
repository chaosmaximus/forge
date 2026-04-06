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
    pub fn init_global(transport: Transport) {
        let _ = GLOBAL_TRANSPORT.set(transport);
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

/// Send a request over HTTP to the remote daemon.
pub async fn http_send(
    endpoint: &str,
    token: Option<&str>,
    request: &Request,
) -> Result<Response, String> {
    let url = format!("{}/api", endpoint.trim_end_matches('/'));

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
    if status.is_server_error() {
        return Err(format!("server error: HTTP {}", status.as_u16()));
    }

    resp.json::<Response>()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))
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
