// extraction/backend.rs — Backend choice enum + auto-detection

use crate::config::{resolve_api_key, ForgeConfig};

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
    ClaudeApi,
    OpenAi,
    Gemini,
    Ollama,
    None(String),
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Detect the best available extraction backend based on config.
pub async fn detect_backend(config: &ForgeConfig) -> BackendChoice {
    match config.extraction.backend.as_str() {
        "claude_api" | "anthropic" => {
            if resolve_api_key(&config.extraction.claude_api.api_key, "ANTHROPIC_API_KEY").is_some()
            {
                BackendChoice::ClaudeApi
            } else {
                BackendChoice::None("ANTHROPIC_API_KEY not set".to_string())
            }
        }
        "openai" => {
            if resolve_api_key(&config.extraction.openai.api_key, "OPENAI_API_KEY").is_some() {
                BackendChoice::OpenAi
            } else {
                BackendChoice::None("OPENAI_API_KEY not set".to_string())
            }
        }
        "gemini" | "google" => {
            if resolve_api_key(&config.extraction.gemini.api_key, "GEMINI_API_KEY").is_some() {
                BackendChoice::Gemini
            } else {
                BackendChoice::None("GEMINI_API_KEY not set".to_string())
            }
        }
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
                BackendChoice::None(format!("ollama not reachable at {endpoint}"))
            }
        }
        _ => {
            // Auto: try local first (free), then cloud providers with API keys, then CLI
            let endpoint = &config.extraction.ollama.endpoint;
            if is_ollama_available(endpoint).await {
                return BackendChoice::Ollama;
            }
            if resolve_api_key(&config.extraction.claude_api.api_key, "ANTHROPIC_API_KEY")
                .is_some()
            {
                return BackendChoice::ClaudeApi;
            }
            if resolve_api_key(&config.extraction.openai.api_key, "OPENAI_API_KEY").is_some() {
                return BackendChoice::OpenAi;
            }
            if resolve_api_key(&config.extraction.gemini.api_key, "GEMINI_API_KEY").is_some() {
                return BackendChoice::Gemini;
            }
            if is_claude_cli_available().await {
                return BackendChoice::ClaudeCli;
            }
            BackendChoice::None(
                "no extraction backend available (tried ollama, claude API, openai, gemini, claude CLI)".to_string(),
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
pub async fn is_ollama_available(endpoint: &str) -> bool {
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
            _ => {
                panic!("should never return non-Claude when backend is 'claude'");
            }
        }
    }

    #[tokio::test]
    async fn test_detect_backend_claude_api_with_key() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "claude_api".to_string();
        cfg.extraction.claude_api.api_key = "sk-ant-test-key".to_string();

        let result = detect_backend(&cfg).await;
        assert_eq!(result, BackendChoice::ClaudeApi);
    }

    #[tokio::test]
    async fn test_detect_backend_claude_api_without_key() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "claude_api".to_string();
        // No API key set, env var should also not be set for this test

        let result = detect_backend(&cfg).await;
        match result {
            BackendChoice::None(reason) => {
                assert!(reason.contains("ANTHROPIC_API_KEY"), "reason should mention ANTHROPIC_API_KEY: {reason}");
            }
            _ => panic!("should return None when API key is missing"),
        }
    }

    #[tokio::test]
    async fn test_detect_backend_openai_with_key() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "openai".to_string();
        cfg.extraction.openai.api_key = "sk-openai-test".to_string();

        let result = detect_backend(&cfg).await;
        assert_eq!(result, BackendChoice::OpenAi);
    }

    #[tokio::test]
    async fn test_detect_backend_openai_without_key() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "openai".to_string();

        let result = detect_backend(&cfg).await;
        match result {
            BackendChoice::None(reason) => {
                assert!(reason.contains("OPENAI_API_KEY"), "reason should mention OPENAI_API_KEY: {reason}");
            }
            _ => panic!("should return None when API key is missing"),
        }
    }

    #[tokio::test]
    async fn test_detect_backend_gemini_with_key() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "gemini".to_string();
        cfg.extraction.gemini.api_key = "gemini-test-key".to_string();

        let result = detect_backend(&cfg).await;
        assert_eq!(result, BackendChoice::Gemini);
    }

    #[tokio::test]
    async fn test_detect_backend_gemini_without_key() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "gemini".to_string();

        let result = detect_backend(&cfg).await;
        match result {
            BackendChoice::None(reason) => {
                assert!(reason.contains("GEMINI_API_KEY"), "reason should mention GEMINI_API_KEY: {reason}");
            }
            _ => panic!("should return None when API key is missing"),
        }
    }

    #[tokio::test]
    async fn test_detect_backend_anthropic_alias() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "anthropic".to_string();
        cfg.extraction.claude_api.api_key = "sk-ant-test".to_string();

        let result = detect_backend(&cfg).await;
        assert_eq!(result, BackendChoice::ClaudeApi);
    }

    #[tokio::test]
    async fn test_detect_backend_google_alias() {
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "google".to_string();
        cfg.extraction.gemini.api_key = "gemini-key".to_string();

        let result = detect_backend(&cfg).await;
        assert_eq!(result, BackendChoice::Gemini);
    }

    #[tokio::test]
    async fn test_detect_backend_auto_with_claude_api_key() {
        // Auto mode with no Ollama + Claude API key available
        // should pick ClaudeApi (Ollama may or may not be running,
        // so we just verify the key-based path works when explicitly set)
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "claude_api".to_string(); // Explicit for determinism
        cfg.extraction.claude_api.api_key = "sk-ant-test".to_string();

        let result = detect_backend(&cfg).await;
        assert_eq!(result, BackendChoice::ClaudeApi);
    }

    #[tokio::test]
    async fn test_select_backend_gemini() {
        // Explicit "gemini" backend with a valid API key should return Gemini variant
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "gemini".to_string();
        cfg.extraction.gemini.api_key = "test-gemini-api-key".to_string();

        let result = detect_backend(&cfg).await;
        assert_eq!(result, BackendChoice::Gemini);
    }

    #[tokio::test]
    async fn test_select_backend_ollama() {
        // Explicit "ollama" backend — ollama is not running in test env,
        // so detect_backend should return None with a reason mentioning the endpoint.
        let mut cfg = ForgeConfig::default();
        cfg.extraction.backend = "ollama".to_string();

        let result = detect_backend(&cfg).await;
        match result {
            BackendChoice::Ollama => {
                // Ollama happens to be running locally — acceptable
            }
            BackendChoice::None(reason) => {
                assert!(
                    reason.contains("ollama"),
                    "reason should mention ollama: {reason}"
                );
            }
            other => {
                panic!("expected Ollama or None, got {other:?}");
            }
        }
    }

    #[tokio::test]
    async fn test_select_backend_default() {
        // Default config has backend = "auto".
        // In auto mode, the function tries ollama, then cloud providers with keys,
        // then claude CLI. With no keys and no running services, it should return None.
        let cfg = ForgeConfig::default();
        assert_eq!(cfg.extraction.backend, "auto");

        let result = detect_backend(&cfg).await;
        // In CI/test: ollama likely not running, no API keys set, claude CLI
        // may or may not be on PATH. We accept any valid result from the auto path.
        match &result {
            BackendChoice::None(reason) => {
                assert!(
                    reason.contains("no extraction backend available"),
                    "auto-fallback reason should list tried backends: {reason}"
                );
            }
            BackendChoice::Ollama
            | BackendChoice::ClaudeApi
            | BackendChoice::OpenAi
            | BackendChoice::Gemini
            | BackendChoice::ClaudeCli => {
                // Some backend was detected in the environment — also valid
            }
        }
    }
}
