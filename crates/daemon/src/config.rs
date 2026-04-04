// config.rs — ~/.forge/config.toml parser

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ForgeConfig {
    pub extraction: ExtractionConfig,
    pub embedding: EmbeddingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractionConfig {
    pub backend: String, // "auto", "ollama", "claude", "claude_api", "openai", "gemini"
    pub claude: ClaudeCliConfig,
    pub claude_api: ClaudeApiConfig,
    pub openai: OpenAiConfig,
    pub gemini: GeminiConfig,
    pub ollama: OllamaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeCliConfig {
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeApiConfig {
    pub api_key: String, // or ANTHROPIC_API_KEY env var
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenAiConfig {
    pub api_key: String, // or OPENAI_API_KEY env var
    pub model: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeminiConfig {
    pub api_key: String, // or GEMINI_API_KEY env var
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub model: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    pub model: String,
    pub dimensions: usize,
}

// ---------------------------------------------------------------------------
// Default impls
// ---------------------------------------------------------------------------

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            backend: "auto".to_string(),
            claude: ClaudeCliConfig::default(),
            claude_api: ClaudeApiConfig::default(),
            openai: OpenAiConfig::default(),
            gemini: GeminiConfig::default(),
            ollama: OllamaConfig::default(),
        }
    }
}

impl Default for ClaudeCliConfig {
    fn default() -> Self {
        Self {
            model: "haiku".to_string(),
        }
    }
}

impl Default for ClaudeApiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "claude-haiku-4-5-20251001".to_string(),
        }
    }
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            endpoint: "https://api.openai.com/v1".to_string(),
        }
    }
}

impl Default for GeminiConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: "gemini-2.0-flash".to_string(),
        }
    }
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self {
            model: "gemma3:1b".to_string(),
            endpoint: "http://localhost:11434".to_string(),
        }
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: "nomic-embed-text".to_string(),
            dimensions: 768,
        }
    }
}

impl ForgeConfig {
    /// Validate that config fields are sensible.
    pub fn validate(&self) -> Result<(), String> {
        if self.embedding.dimensions == 0 {
            return Err("embedding.dimensions must be > 0".into());
        }
        if self.extraction.claude.model.trim().is_empty() {
            return Err("extraction.claude.model must not be empty".into());
        }
        if self.extraction.ollama.model.trim().is_empty() {
            return Err("extraction.ollama.model must not be empty".into());
        }
        if self.extraction.ollama.endpoint.trim().is_empty() {
            return Err("extraction.ollama.endpoint must not be empty".into());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Loaders
// ---------------------------------------------------------------------------

/// Load config from `~/.forge/config.toml`.
/// Returns defaults if the file doesn't exist or can't be parsed.
pub fn load_config() -> ForgeConfig {
    let dir = forge_core::forge_dir();
    let path = format!("{dir}/config.toml");
    load_config_from(&path)
}

/// Load config from an arbitrary path.
/// Returns defaults if the file doesn't exist or can't be parsed.
pub fn load_config_from(path: &str) -> ForgeConfig {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let config: ForgeConfig = match toml::from_str(&contents) {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("forge: warning: failed to parse {path}: {e}");
                    return ForgeConfig::default();
                }
            };
            if let Err(e) = config.validate() {
                eprintln!("[config] validation error: {e}, using defaults");
                return ForgeConfig::default();
            }
            config
        }
        Err(_) => ForgeConfig::default(),
    }
}

// ---------------------------------------------------------------------------
// API key resolution
// ---------------------------------------------------------------------------

/// Resolve API key: config value > environment variable > None.
/// SECURITY: never log the returned key value.
pub fn resolve_api_key(config_value: &str, env_var: &str) -> Option<String> {
    if !config_value.is_empty() {
        return Some(config_value.to_string());
    }
    std::env::var(env_var).ok().filter(|k| !k.is_empty())
}

// ---------------------------------------------------------------------------
// Config update (persist changes to disk)
// ---------------------------------------------------------------------------

/// Update a config value by dotted key and persist to ~/.forge/config.toml.
pub fn update_config(key: &str, value: &str) -> Result<(), String> {
    let dir = forge_core::forge_dir();
    let path = format!("{dir}/config.toml");
    update_config_at(&path, key, value)
}

/// Update a config value at an arbitrary path (for testing).
pub fn update_config_at(path: &str, key: &str, value: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let mut config: ForgeConfig = toml::from_str(&content).unwrap_or_default();

    match key.split('.').collect::<Vec<_>>().as_slice() {
        ["extraction", "backend"] => config.extraction.backend = value.to_string(),
        ["extraction", "claude", "model"] => config.extraction.claude.model = value.to_string(),
        ["extraction", "claude_api", "api_key"] => config.extraction.claude_api.api_key = value.to_string(),
        ["extraction", "claude_api", "model"] => config.extraction.claude_api.model = value.to_string(),
        ["extraction", "openai", "api_key"] => config.extraction.openai.api_key = value.to_string(),
        ["extraction", "openai", "model"] => config.extraction.openai.model = value.to_string(),
        ["extraction", "openai", "endpoint"] => config.extraction.openai.endpoint = value.to_string(),
        ["extraction", "gemini", "api_key"] => config.extraction.gemini.api_key = value.to_string(),
        ["extraction", "gemini", "model"] => config.extraction.gemini.model = value.to_string(),
        ["extraction", "ollama", "model"] => config.extraction.ollama.model = value.to_string(),
        ["extraction", "ollama", "endpoint"] => config.extraction.ollama.endpoint = value.to_string(),
        ["embedding", "model"] => config.embedding.model = value.to_string(),
        ["embedding", "dimensions"] => {
            config.embedding.dimensions = value.parse().map_err(|e| format!("invalid dimensions: {e}"))?;
        }
        _ => return Err(format!("unknown config key: {key}")),
    }

    let toml_str = toml::to_string_pretty(&config).map_err(|e| format!("serialize error: {e}"))?;
    std::fs::write(path, toml_str).map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = ForgeConfig::default();

        // Extraction defaults
        assert_eq!(cfg.extraction.backend, "auto");
        assert_eq!(cfg.extraction.claude.model, "haiku");
        assert_eq!(cfg.extraction.ollama.model, "gemma3:1b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://localhost:11434");

        // Embedding defaults
        assert_eq!(cfg.embedding.model, "nomic-embed-text");
        assert_eq!(cfg.embedding.dimensions, 768);
    }

    #[test]
    fn test_parse_config_toml() {
        let toml_str = r#"
[extraction]
backend = "claude"

[extraction.claude]
model = "sonnet"

[extraction.ollama]
model = "llama3:70b"
endpoint = "http://gpu-server:11434"

[embedding]
model = "mxbai-embed-large"
dimensions = 1024
"#;

        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();

        assert_eq!(cfg.extraction.backend, "claude");
        assert_eq!(cfg.extraction.claude.model, "sonnet");
        assert_eq!(cfg.extraction.ollama.model, "llama3:70b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://gpu-server:11434");
        assert_eq!(cfg.embedding.model, "mxbai-embed-large");
        assert_eq!(cfg.embedding.dimensions, 1024);
    }

    #[test]
    fn test_partial_config() {
        let toml_str = r#"
[extraction]
backend = "ollama"
"#;

        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();

        // Overridden field
        assert_eq!(cfg.extraction.backend, "ollama");

        // All other fields should be defaults
        assert_eq!(cfg.extraction.claude.model, "haiku");
        assert_eq!(cfg.extraction.ollama.model, "gemma3:1b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://localhost:11434");
        assert_eq!(cfg.embedding.model, "nomic-embed-text");
        assert_eq!(cfg.embedding.dimensions, 768);
    }

    #[test]
    fn test_validate_zero_dimensions() {
        let mut config = ForgeConfig::default();
        config.embedding.dimensions = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_model() {
        let mut config = ForgeConfig::default();
        config.extraction.claude.model = "".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_default_passes() {
        let config = ForgeConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_missing_file_returns_defaults() {
        let cfg = load_config_from("/nonexistent/path/config.toml");

        assert_eq!(cfg.extraction.backend, "auto");
        assert_eq!(cfg.extraction.claude.model, "haiku");
        assert_eq!(cfg.extraction.ollama.model, "gemma3:1b");
        assert_eq!(cfg.extraction.ollama.endpoint, "http://localhost:11434");
        assert_eq!(cfg.embedding.model, "nomic-embed-text");
        assert_eq!(cfg.embedding.dimensions, 768);
    }

    #[test]
    fn test_new_provider_defaults() {
        let cfg = ForgeConfig::default();

        // Claude API defaults
        assert!(cfg.extraction.claude_api.api_key.is_empty());
        assert_eq!(cfg.extraction.claude_api.model, "claude-haiku-4-5-20251001");

        // OpenAI defaults
        assert!(cfg.extraction.openai.api_key.is_empty());
        assert_eq!(cfg.extraction.openai.model, "gpt-4o-mini");
        assert_eq!(cfg.extraction.openai.endpoint, "https://api.openai.com/v1");

        // Gemini defaults
        assert!(cfg.extraction.gemini.api_key.is_empty());
        assert_eq!(cfg.extraction.gemini.model, "gemini-2.0-flash");
    }

    #[test]
    fn test_parse_config_with_new_providers() {
        let toml_str = r#"
[extraction]
backend = "claude_api"

[extraction.claude_api]
api_key = "sk-ant-test"
model = "claude-sonnet-4-20250514"

[extraction.openai]
api_key = "sk-openai-test"
model = "gpt-4o"
endpoint = "https://custom.openai.com/v1"

[extraction.gemini]
api_key = "gemini-test-key"
model = "gemini-1.5-pro"
"#;

        let cfg: ForgeConfig = toml::from_str(toml_str).unwrap();

        assert_eq!(cfg.extraction.backend, "claude_api");
        assert_eq!(cfg.extraction.claude_api.api_key, "sk-ant-test");
        assert_eq!(cfg.extraction.claude_api.model, "claude-sonnet-4-20250514");
        assert_eq!(cfg.extraction.openai.api_key, "sk-openai-test");
        assert_eq!(cfg.extraction.openai.model, "gpt-4o");
        assert_eq!(cfg.extraction.openai.endpoint, "https://custom.openai.com/v1");
        assert_eq!(cfg.extraction.gemini.api_key, "gemini-test-key");
        assert_eq!(cfg.extraction.gemini.model, "gemini-1.5-pro");
    }

    #[test]
    fn test_resolve_api_key_from_config() {
        // Config value takes priority over env var
        let result = resolve_api_key("sk-from-config", "NONEXISTENT_VAR_12345");
        assert_eq!(result, Some("sk-from-config".to_string()));
    }

    #[test]
    fn test_resolve_api_key_from_env() {
        // Set a temporary env var
        std::env::set_var("FORGE_TEST_API_KEY_12345", "sk-from-env");
        let result = resolve_api_key("", "FORGE_TEST_API_KEY_12345");
        assert_eq!(result, Some("sk-from-env".to_string()));
        std::env::remove_var("FORGE_TEST_API_KEY_12345");
    }

    #[test]
    fn test_resolve_api_key_none() {
        // Neither config nor env var set
        let result = resolve_api_key("", "NONEXISTENT_VAR_67890");
        assert_eq!(result, None);
    }

    #[test]
    fn test_resolve_api_key_empty_env() {
        // Empty env var should return None
        std::env::set_var("FORGE_TEST_EMPTY_KEY", "");
        let result = resolve_api_key("", "FORGE_TEST_EMPTY_KEY");
        assert_eq!(result, None);
        std::env::remove_var("FORGE_TEST_EMPTY_KEY");
    }

    #[test]
    fn test_config_reload_from_disk() {
        // Write initial config to temp file
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let path_str = path.to_str().unwrap();

        let initial_toml = r#"
[extraction]
backend = "ollama"

[extraction.ollama]
model = "gemma3:1b"
"#;
        std::fs::write(&path, initial_toml).unwrap();

        // Load initial config
        let cfg1 = load_config_from(path_str);
        assert_eq!(cfg1.extraction.backend, "ollama");
        assert_eq!(cfg1.extraction.ollama.model, "gemma3:1b");

        // Change config on disk (simulates `forge-next config set`)
        let updated_toml = r#"
[extraction]
backend = "claude_api"

[extraction.ollama]
model = "llama3:70b"
"#;
        std::fs::write(&path, updated_toml).unwrap();

        // Reload — should see new values without restart
        let cfg2 = load_config_from(path_str);
        assert_eq!(cfg2.extraction.backend, "claude_api", "backend should reflect disk change");
        assert_eq!(cfg2.extraction.ollama.model, "llama3:70b", "model should reflect disk change");
    }
}
