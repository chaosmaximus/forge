// extraction/backend.rs — Backend choice enum + auto-detection

use crate::config::ForgeConfig;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of an extraction attempt.
pub enum ExtractionResult {
    /// Successfully extracted memories.
    Success(Vec<super::prompt::ExtractedMemory>),
    /// The chosen backend is not available (with reason).
    Unavailable(String),
    /// An error occurred during extraction.
    Error(String),
}

/// Which extraction backend to use.
#[derive(Debug, PartialEq)]
pub enum BackendChoice {
    ClaudeCli,
    Ollama,
    None(String),
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect the best available extraction backend based on config.
pub async fn detect_backend(config: &ForgeConfig) -> BackendChoice {
    match config.extraction.backend.as_str() {
        "claude" => {
            if is_claude_cli_available().await {
                BackendChoice::ClaudeCli
            } else {
                BackendChoice::None("claude CLI not found on PATH".to_string())
            }
        }
        "ollama" => {
            let endpoint = &config.extraction.ollama.endpoint;
            if is_ollama_available(endpoint).await {
                BackendChoice::Ollama
            } else {
                BackendChoice::None(format!(
                    "ollama not reachable at {endpoint}"
                ))
            }
        }
        _ => {
            // Try Claude first, then Ollama, then None
            if is_claude_cli_available().await {
                return BackendChoice::ClaudeCli;
            }
            let endpoint = &config.extraction.ollama.endpoint;
            if is_ollama_available(endpoint).await {
                return BackendChoice::Ollama;
            }
            BackendChoice::None(
                "no extraction backend available (tried claude CLI, ollama)".to_string(),
            )
        }
    }
}

/// Check if `claude` CLI is available on PATH.
/// Wrapped in a 5-second timeout to prevent hangs on unresponsive binaries.
async fn is_claude_cli_available() -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::process::Command::new("claude")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Check if Ollama is reachable at the given endpoint.
async fn is_ollama_available(endpoint: &str) -> bool {
    let url = format!("{endpoint}/api/tags");
    reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ForgeConfig;

    #[tokio::test]
    async fn test_detect_backend_explicit_claude() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "claude".to_string();

        let result = detect_backend(&cfg).await;

        // In CI/test environments, claude CLI may or may not be on PATH.
        // We just verify the result is one of the two valid options.
        match &result {
            BackendChoice::ClaudeCli => {
                // Claude is installed — correct detection
            }
            BackendChoice::None(reason) => {
                assert!(
                    reason.contains("claude"),
                    "reason should mention claude: {reason}"
                );
            }
            BackendChoice::Ollama => {
                panic!("should never return Ollama when backend is 'claude'");
            }
        }
    }
}
