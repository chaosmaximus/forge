use std::path::Path;

/// Configuration for a language server binary.
#[derive(Debug, Clone)]
pub struct LspServerConfig {
    pub language: String,
    pub command: String,
    pub args: Vec<String>,
    /// When present, the LSP server is initialized with this as its rootUri
    /// instead of the project root. Used when marker files (e.g. tsconfig.json)
    /// live in a subdirectory.
    pub root_dir: Option<String>,
}

/// Check if a command exists on PATH.
fn command_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Detect available language servers for a project directory.
///
/// Inspects marker files (Cargo.toml, pyproject.toml, tsconfig.json, etc.)
/// and checks whether the corresponding language server binary is on PATH.
pub fn detect_language_servers(project_dir: &str) -> Vec<LspServerConfig> {
    let dir = Path::new(project_dir);
    let mut servers = Vec::new();

    // Rust: Cargo.toml + rust-analyzer
    if dir.join("Cargo.toml").exists() && command_exists("rust-analyzer") {
        servers.push(LspServerConfig {
            language: "rust".into(),
            command: "rust-analyzer".into(),
            args: vec![],
            root_dir: None,
        });
    }

    // Python: pyproject.toml or requirements.txt or setup.py + pyright
    if (dir.join("pyproject.toml").exists()
        || dir.join("requirements.txt").exists()
        || dir.join("setup.py").exists())
        && command_exists("pyright-langserver")
    {
        servers.push(LspServerConfig {
            language: "python".into(),
            command: "pyright-langserver".into(),
            args: vec!["--stdio".into()],
            root_dir: None,
        });
    }

    // TypeScript/JavaScript: tsconfig.json or package.json + typescript-language-server
    // Check project root first, then scan immediate subdirectories (1 level deep)
    if command_exists("typescript-language-server") {
        let ts_markers = &["tsconfig.json", "package.json"];
        if ts_markers.iter().any(|m| dir.join(m).exists()) {
            // Found at project root
            servers.push(LspServerConfig {
                language: "typescript".into(),
                command: "typescript-language-server".into(),
                args: vec!["--stdio".into()],
                root_dir: None,
            });
        } else if let Ok(entries) = std::fs::read_dir(dir) {
            // Scan immediate subdirectories for TS marker files
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let dir_name = entry.file_name();
                    let name_str = dir_name.to_string_lossy();
                    // Skip hidden dirs and known non-source dirs
                    if name_str.starts_with('.')
                        || name_str == "node_modules"
                        || name_str == "target"
                        || name_str == "dist"
                        || name_str == "build"
                    {
                        continue;
                    }
                    if ts_markers.iter().any(|m| path.join(m).exists()) {
                        servers.push(LspServerConfig {
                            language: "typescript".into(),
                            command: "typescript-language-server".into(),
                            args: vec!["--stdio".into()],
                            root_dir: Some(path.to_string_lossy().to_string()),
                        });
                        break; // only first match
                    }
                }
            }
        }
    }

    // Go: go.mod + gopls
    if dir.join("go.mod").exists() && command_exists("gopls") {
        servers.push(LspServerConfig {
            language: "go".into(),
            command: "gopls".into(),
            args: vec!["serve".into()],
            root_dir: None,
        });
    }

    servers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_on_this_repo() {
        // This repo has Cargo.toml at the workspace root.
        // If rust-analyzer is installed, we should detect it.
        let servers = detect_language_servers(env!("CARGO_MANIFEST_DIR"));
        // The daemon crate dir has no Cargo.toml of its own at the workspace
        // root, but it *does* have one in the crate directory. Check that:
        // - If rust-analyzer is present, we get a rust entry.
        // - If not, we still don't crash.
        if command_exists("rust-analyzer") {
            assert!(
                servers.iter().any(|s| s.language == "rust"),
                "Expected rust server when rust-analyzer is on PATH"
            );
        }
        // Should never panic regardless of what's installed.
    }

    #[test]
    fn test_detect_empty_dir() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let servers = detect_language_servers(tmp.path().to_str().unwrap());
        assert!(servers.is_empty(), "Empty dir should yield no servers");
    }

    #[test]
    fn test_detect_typescript_in_subdirectory() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let sub = tmp.path().join("app");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("tsconfig.json"), "{}").unwrap();

        let servers = detect_language_servers(tmp.path().to_str().unwrap());

        if command_exists("typescript-language-server") {
            let ts = servers.iter().find(|s| s.language == "typescript");
            assert!(ts.is_some(), "Should detect TS server from subdirectory");
            let ts = ts.unwrap();
            assert!(
                ts.root_dir.is_some(),
                "root_dir should be set to the subdirectory"
            );
            let root = ts.root_dir.as_ref().unwrap();
            assert!(
                root.ends_with("app"),
                "root_dir should point to app/ subdir, got: {}",
                root
            );
        }
        // Should not crash even without typescript-language-server installed
    }

    #[test]
    fn test_detect_typescript_at_root_no_root_dir() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        std::fs::write(tmp.path().join("package.json"), "{}").unwrap();

        let servers = detect_language_servers(tmp.path().to_str().unwrap());

        if command_exists("typescript-language-server") {
            let ts = servers.iter().find(|s| s.language == "typescript");
            assert!(ts.is_some(), "Should detect TS server at root");
            assert!(
                ts.unwrap().root_dir.is_none(),
                "root_dir should be None when marker is at project root"
            );
        }
    }

    #[test]
    fn test_detect_skips_hidden_and_nodemodules_subdirs() {
        let tmp = tempfile::tempdir().expect("create temp dir");
        // Put tsconfig.json only in hidden dir and node_modules — should NOT detect
        let hidden = tmp.path().join(".hidden_app");
        std::fs::create_dir(&hidden).unwrap();
        std::fs::write(hidden.join("tsconfig.json"), "{}").unwrap();

        let nm = tmp.path().join("node_modules");
        std::fs::create_dir(&nm).unwrap();
        std::fs::write(nm.join("package.json"), "{}").unwrap();

        let servers = detect_language_servers(tmp.path().to_str().unwrap());
        let ts = servers.iter().find(|s| s.language == "typescript");
        assert!(
            ts.is_none(),
            "Should NOT detect TS in hidden dirs or node_modules"
        );
    }

    #[test]
    fn test_command_exists_positive() {
        // "ls" should exist on any UNIX system.
        assert!(command_exists("ls"), "ls must be on PATH");
    }

    #[test]
    fn test_command_exists_negative() {
        assert!(
            !command_exists("nonexistent_binary_xyz_9999"),
            "Fake binary must not be found"
        );
    }
}
