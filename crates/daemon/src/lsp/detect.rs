use std::path::Path;

/// Configuration for a language server binary.
#[derive(Debug, Clone)]
pub struct LspServerConfig {
    pub language: String,
    pub command: String,
    pub args: Vec<String>,
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
        });
    }

    // TypeScript/JavaScript: tsconfig.json or package.json + typescript-language-server
    if (dir.join("tsconfig.json").exists() || dir.join("package.json").exists())
        && command_exists("typescript-language-server")
    {
        servers.push(LspServerConfig {
            language: "typescript".into(),
            command: "typescript-language-server".into(),
            args: vec!["--stdio".into()],
        });
    }

    // Go: go.mod + gopls
    if dir.join("go.mod").exists() && command_exists("gopls") {
        servers.push(LspServerConfig {
            language: "go".into(),
            command: "gopls".into(),
            args: vec!["serve".into()],
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
