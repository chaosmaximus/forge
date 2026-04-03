//! Shared path functions for Forge daemon and CLI.
//! Centralizes all path derivation logic to avoid duplication.

/// Returns the path to the Forge data directory (~/.forge).
pub fn forge_dir() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{home}/.forge")
}

/// Returns the default Unix domain socket path.
pub fn default_socket_path() -> String {
    format!("{}/forge.sock", forge_dir())
}

/// Returns the default SQLite database path.
pub fn default_db_path() -> String {
    format!("{}/forge.db", forge_dir())
}

/// Returns the default PID file path.
pub fn default_pid_path() -> String {
    format!("{}/forge.pid", forge_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_forge_dir_uses_home() {
        // This test relies on HOME being set in CI/dev environments
        let dir = forge_dir();
        assert!(dir.ends_with("/.forge"), "forge_dir should end with /.forge, got: {dir}");
    }

    #[test]
    fn test_paths_are_consistent() {
        let dir = forge_dir();
        assert!(default_socket_path().starts_with(&dir));
        assert!(default_db_path().starts_with(&dir));
        assert!(default_pid_path().starts_with(&dir));
    }
}
