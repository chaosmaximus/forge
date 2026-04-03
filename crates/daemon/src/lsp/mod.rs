pub mod client;
pub mod detect;
pub mod symbols;

use client::LspClient;
use detect::LspServerConfig;
use std::collections::HashMap;

/// Manages persistent LSP client connections.
/// Keeps language servers alive between index cycles and auto-restarts dead ones.
pub struct LspManager {
    clients: HashMap<String, LspClient>, // language -> client
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
    /// Returns a mutable reference to the client.
    /// If the client doesn't exist or has crashed, spawns a new one.
    pub async fn get_client(
        &mut self,
        config: &LspServerConfig,
    ) -> Result<&mut LspClient, String> {
        let language = config.language.clone();

        // Check if we need to (re)spawn: missing or dead
        let alive = self
            .clients
            .get_mut(&language)
            .map(|c| c.is_alive())
            .unwrap_or(false);

        if !alive {
            // Remove dead client if present (drop triggers kill_on_drop)
            if self.clients.remove(&language).is_some() {
                eprintln!("[lsp-manager] {} server died, restarting", language);
            }
            let client = LspClient::spawn(config, &self.project_dir).await?;
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
