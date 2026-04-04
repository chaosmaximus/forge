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
    pub backend: String,
    pub claude: ClaudeCliConfig,
    pub ollama: OllamaConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaudeCliConfig {
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

/// Update a config value by dotted key and persist to ~/.forge/config.toml.
/// Supports 2-level (e.g., "extraction.backend") and 3-level (e.g., "extraction.ollama.model") keys.
pub fn update_config(key: &str, value: &str) -> Result<(), String> {
    let dir = forge_core::forge_dir();
    let path = format!("{dir}/config.toml");
    update_config_at(&path, key, value)
}

/// Update a config value at an arbitrary path (for testing).
pub fn update_config_at(path: &str, key: &str, value: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let parts: Vec<&str> = key.split('.').collect();

    // Simple TOML manipulation without toml_edit dependency
    // Parse existing config, update the value, serialize back
    let mut config: ForgeConfig = toml::from_str(&content).unwrap_or_default();

    match parts.as_slice() {
        ["extraction", "backend"] => config.extraction.backend = value.to_string(),
        ["extraction", "claude", "model"] => config.extraction.claude.model = value.to_string(),
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
}
