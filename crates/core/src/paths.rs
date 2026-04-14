//! Shared path functions for Forge daemon and CLI.
//! Centralizes all path derivation logic to avoid duplication.

/// Returns the path to the Forge data directory.
///
/// Resolution order:
/// 1. `FORGE_DIR` env var — explicit override, takes precedence over everything
///    else. Used by tests, benchmarks, and isolated runs to redirect the daemon
///    away from the user's real `~/.forge` state. An empty value (e.g., from
///    `export FORGE_DIR=` with no RHS) is treated as unset to avoid returning
///    root-relative paths.
/// 2. `$HOME/.forge` — the default for production runs.
/// 3. `/tmp/.forge` — fallback if neither env var is set (unusual).
///
/// All downstream helpers (`default_socket_path`, `default_db_path`,
/// `default_pid_path`) flow through this function, so a single `FORGE_DIR`
/// override isolates all daemon state paths atomically.
pub fn forge_dir() -> String {
    if let Ok(dir) = std::env::var("FORGE_DIR") {
        if !dir.is_empty() {
            return dir;
        }
    }
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
    use std::sync::{Mutex, OnceLock};

    // Tests that read or write env vars must hold this lock to prevent races
    // across Rust's parallel test runner. Env vars are process-global; without
    // the lock, two tests could see each other's set_var calls.
    // Recovers from poison — safe because EnvGuard::drop restores env state
    // on panic, so the lock protects a consistent state even after a failure.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    // RAII guard that restores an env var to its pre-test value on drop —
    // panic-safe, so a failing assertion doesn't leak state into later tests.
    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, val);
            Self { key, prev }
        }

        fn unset(key: &'static str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.prev.take() {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn test_forge_dir_uses_home_when_forge_dir_unset() {
        let _lock = env_lock();
        let _g1 = EnvGuard::unset("FORGE_DIR");
        let _g2 = EnvGuard::set("HOME", "/tmp/test-home-for-forge");
        let dir = forge_dir();
        assert_eq!(
            dir, "/tmp/test-home-for-forge/.forge",
            "forge_dir should return $HOME/.forge when FORGE_DIR is unset"
        );
    }

    #[test]
    fn test_forge_dir_respects_forge_dir_env_var() {
        let _lock = env_lock();
        let _g = EnvGuard::set("FORGE_DIR", "/tmp/forge-test-override-01KP6J23");
        let dir = forge_dir();
        assert_eq!(
            dir, "/tmp/forge-test-override-01KP6J23",
            "forge_dir should return FORGE_DIR value when set"
        );
    }

    #[test]
    fn test_forge_dir_env_var_takes_precedence_over_home() {
        let _lock = env_lock();
        let _g1 = EnvGuard::set("HOME", "/home/should-be-ignored");
        let _g2 = EnvGuard::set("FORGE_DIR", "/tmp/explicit-forge-dir");
        let dir = forge_dir();
        assert_eq!(
            dir, "/tmp/explicit-forge-dir",
            "FORGE_DIR should win over HOME-derived path"
        );
    }

    #[test]
    fn test_empty_forge_dir_falls_back_to_home() {
        // `export FORGE_DIR=` produces Ok("") from std::env::var — an empty
        // FORGE_DIR must NOT be treated as a valid path. Falling through to
        // the HOME branch is the safe behavior.
        let _lock = env_lock();
        let _g1 = EnvGuard::set("FORGE_DIR", "");
        let _g2 = EnvGuard::set("HOME", "/tmp/test-empty-forge-dir-home");
        let dir = forge_dir();
        assert_eq!(
            dir, "/tmp/test-empty-forge-dir-home/.forge",
            "empty FORGE_DIR should fall back to $HOME/.forge"
        );
    }

    #[test]
    fn test_paths_are_consistent() {
        let _lock = env_lock();
        let dir = forge_dir();
        assert!(default_socket_path().starts_with(&dir));
        assert!(default_db_path().starts_with(&dir));
        assert!(default_pid_path().starts_with(&dir));
    }
}
