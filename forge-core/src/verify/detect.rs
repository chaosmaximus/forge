//! Language detection from file extensions and project config files.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Language {
    Python,
    TypeScript,
    JavaScript,
    Rust,
    Go,
    Unknown,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::JavaScript => "javascript",
            Language::Rust => "rust",
            Language::Go => "go",
            Language::Unknown => "unknown",
        }
    }
}

/// Detect language from a single file's extension.
/// If ambiguous (e.g. no extension), check parent directory for config files.
pub fn detect_language(file_path: &str) -> Language {
    let path = Path::new(file_path);

    // 1. Check file extension
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        match ext {
            "py" | "pyi" | "pyw" => return Language::Python,
            "ts" | "tsx" | "mts" | "cts" => return Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => return Language::JavaScript,
            "rs" => return Language::Rust,
            "go" => return Language::Go,
            _ => {}
        }
    }

    // 2. If no recognizable extension, check parent directory for config files
    if let Some(parent) = path.parent() {
        return detect_project_language_in(parent);
    }

    Language::Unknown
}

/// Detect the primary language of a project directory by checking config files.
pub fn detect_project_language(project_dir: &str) -> Language {
    let dir = Path::new(project_dir);
    detect_project_language_in(dir)
}

fn detect_project_language_in(dir: &Path) -> Language {
    // Check config files in priority order
    let checks: &[(&str, Language)] = &[
        ("Cargo.toml", Language::Rust),
        ("pyproject.toml", Language::Python),
        ("setup.py", Language::Python),
        ("setup.cfg", Language::Python),
        ("tsconfig.json", Language::TypeScript),
        ("package.json", Language::JavaScript), // Could be TS too; tsconfig.json wins above
        ("go.mod", Language::Go),
    ];

    for (config_file, lang) in checks {
        if dir.join(config_file).exists() {
            // Special case: package.json could mean TypeScript if tsconfig also exists
            if *lang == Language::JavaScript && dir.join("tsconfig.json").exists() {
                return Language::TypeScript;
            }
            return *lang;
        }
    }

    // Walk up to parent directories (max 3 levels)
    let mut current = dir.to_path_buf();
    for _ in 0..3 {
        if let Some(parent) = current.parent() {
            for (config_file, lang) in checks {
                if parent.join(config_file).exists() {
                    if *lang == Language::JavaScript && parent.join("tsconfig.json").exists() {
                        return Language::TypeScript;
                    }
                    return *lang;
                }
            }
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    Language::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_python() {
        assert_eq!(detect_language("src/main.py"), Language::Python);
        assert_eq!(detect_language("types.pyi"), Language::Python);
    }

    #[test]
    fn test_detect_typescript() {
        assert_eq!(detect_language("app.ts"), Language::TypeScript);
        assert_eq!(detect_language("Component.tsx"), Language::TypeScript);
    }

    #[test]
    fn test_detect_javascript() {
        assert_eq!(detect_language("index.js"), Language::JavaScript);
        assert_eq!(detect_language("lib.mjs"), Language::JavaScript);
    }

    #[test]
    fn test_detect_rust() {
        assert_eq!(detect_language("main.rs"), Language::Rust);
    }

    #[test]
    fn test_detect_go() {
        assert_eq!(detect_language("main.go"), Language::Go);
    }

    #[test]
    fn test_detect_unknown() {
        // Use a temp dir with no config files so parent-dir detection doesn't trigger
        let dir = tempfile::tempdir().unwrap();

        let csv_file = dir.path().join("data.csv");
        std::fs::write(&csv_file, "a,b,c").unwrap();
        assert_eq!(detect_language(csv_file.to_str().unwrap()), Language::Unknown);

        let makefile = dir.path().join("Makefile");
        std::fs::write(&makefile, "all:\n\techo hi").unwrap();
        assert_eq!(detect_language(makefile.to_str().unwrap()), Language::Unknown);
    }

    #[test]
    fn test_detect_project_language_with_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.ruff]").unwrap();
        assert_eq!(
            detect_project_language(dir.path().to_str().unwrap()),
            Language::Python
        );
    }

    #[test]
    fn test_detect_project_language_ts_over_js() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        assert_eq!(
            detect_project_language(dir.path().to_str().unwrap()),
            Language::TypeScript
        );
    }

    #[test]
    fn test_detect_project_language_cargo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(
            detect_project_language(dir.path().to_str().unwrap()),
            Language::Rust
        );
    }

    #[test]
    fn test_detect_project_language_go() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example").unwrap();
        assert_eq!(
            detect_project_language(dir.path().to_str().unwrap()),
            Language::Go
        );
    }

    #[test]
    fn test_language_as_str() {
        assert_eq!(Language::Python.as_str(), "python");
        assert_eq!(Language::TypeScript.as_str(), "typescript");
        assert_eq!(Language::Rust.as_str(), "rust");
        assert_eq!(Language::Go.as_str(), "go");
        assert_eq!(Language::Unknown.as_str(), "unknown");
    }
}
