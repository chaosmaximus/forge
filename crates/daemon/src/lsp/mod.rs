pub mod client;
pub mod detect;
pub mod symbols;

use client::LspClient;
use detect::LspServerConfig;
use std::collections::HashMap;

/// Manages persistent LSP client connections.
/// Keeps language servers alive between index cycles and auto-restarts dead ones.
pub struct LspManager {
    clients: HashMap<String, LspClient>,
    project_dir: String,
}

impl LspManager {
    pub fn new(project_dir: String) -> Self {
        LspManager {
            clients: HashMap::new(),
            project_dir,
        }
    }

    /// Get or create an LSP client for the given language server config.
    /// If the client doesn't exist or has crashed, spawns a new one.
    pub async fn get_client(
        &mut self,
        config: &LspServerConfig,
    ) -> Result<&mut LspClient, String> {
        let language = config.language.clone();

        // Check if existing client needs replacement
        let needs_spawn = if let Some(c) = self.clients.get_mut(&language) {
            if c.is_alive() {
                false
            } else {
                eprintln!("[lsp-manager] {} server died, restarting", language);
                self.clients.remove(&language);
                true
            }
        } else {
            true
        };

        if needs_spawn {
            let client = tokio::time::timeout(
                std::time::Duration::from_secs(60),
                LspClient::spawn(config, &self.project_dir),
            )
            .await
            .map_err(|_| format!("{} timed out during spawn/initialize", config.command))?
            .map_err(|e| format!("{} spawn failed: {}", config.command, e))?;
            self.clients.insert(language.clone(), client);
        }

        Ok(self.clients.get_mut(&language).unwrap())
    }

    /// Shut down all managed language servers.
    pub async fn shutdown_all(self) {
        for (language, client) in self.clients {
            if let Err(e) = client.shutdown().await {
                eprintln!("[lsp-manager] {} shutdown error: {}", language, e);
            }
        }
    }

    /// Get the project directory this manager is configured for.
    pub fn project_dir(&self) -> &str {
        &self.project_dir
    }
}
