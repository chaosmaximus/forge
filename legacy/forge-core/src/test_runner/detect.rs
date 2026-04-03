//! Test framework detection from project files.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TestFramework {
    Pytest,
    Vitest,
    Jest,
    CargoTest,
    GoTest,
}

impl TestFramework {
    pub fn as_str(&self) -> &'static str {
        match self {
            TestFramework::Pytest => "pytest",
            TestFramework::Vitest => "vitest",
            TestFramework::Jest => "jest",
            TestFramework::CargoTest => "cargo_test",
            TestFramework::GoTest => "go_test",
        }
    }

    /// Human-friendly install hint when framework binary is missing.
    pub fn install_hint(&self) -> &'static str {
        match self {
            TestFramework::Pytest => "pip install pytest",
            TestFramework::Vitest => "npm install -D vitest",
            TestFramework::Jest => "npm install -D jest",
            TestFramework::CargoTest => "rustup update",
            TestFramework::GoTest => "https://go.dev/dl/",
        }
    }
}

/// Detect which test framework to use for a given project directory.
/// Returns `None` if no recognizable framework is found.
pub fn detect_framework(project_dir: &str) -> Option<TestFramework> {
    let dir = Path::new(project_dir);
    detect_in(dir)
}

fn detect_in(dir: &Path) -> Option<TestFramework> {
    // Pytest: conftest.py, pytest.ini, or pyproject.toml with [tool.pytest]
    if dir.join("conftest.py").exists() || dir.join("pytest.ini").exists() {
        return Some(TestFramework::Pytest);
    }
    if dir.join("pyproject.toml").exists() {
        if let Ok(content) = std::fs::read_to_string(dir.join("pyproject.toml")) {
            if content.contains("[tool.pytest") {
                return Some(TestFramework::Pytest);
            }
        }
    }

    // Vitest: vitest.config.* or package.json with "vitest"
    if has_glob_match(dir, "vitest.config") {
        return Some(TestFramework::Vitest);
    }
    if let Some(fw) = check_package_json(dir) {
        return Some(fw);
    }

    // Cargo test: Cargo.toml
    if dir.join("Cargo.toml").exists() {
        return Some(TestFramework::CargoTest);
    }

    // Go test: go.mod
    if dir.join("go.mod").exists() {
        return Some(TestFramework::GoTest);
    }

    // Pytest fallback: setup.py or pyproject.toml exists (Python project)
    if dir.join("setup.py").exists() || dir.join("setup.cfg").exists() {
        return Some(TestFramework::Pytest);
    }
    if dir.join("pyproject.toml").exists() {
        return Some(TestFramework::Pytest);
    }

    // Walk up (max 3 levels) to find project root
    let mut current = dir.to_path_buf();
    for _ in 0..3 {
        if let Some(parent) = current.parent() {
            if let Some(fw) = detect_in_flat(parent) {
                return Some(fw);
            }
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    None
}

/// Single-level detection without recursion (prevents infinite walk-up).
fn detect_in_flat(dir: &Path) -> Option<TestFramework> {
    if dir.join("conftest.py").exists() || dir.join("pytest.ini").exists() {
        return Some(TestFramework::Pytest);
    }
    if dir.join("pyproject.toml").exists() {
        if let Ok(content) = std::fs::read_to_string(dir.join("pyproject.toml")) {
            if content.contains("[tool.pytest") {
                return Some(TestFramework::Pytest);
            }
        }
    }
    if has_glob_match(dir, "vitest.config") {
        return Some(TestFramework::Vitest);
    }
    if let Some(fw) = check_package_json(dir) {
        return Some(fw);
    }
    if dir.join("Cargo.toml").exists() {
        return Some(TestFramework::CargoTest);
    }
    if dir.join("go.mod").exists() {
        return Some(TestFramework::GoTest);
    }
    None
}

/// Check if vitest.config.{ts,js,mts,mjs} exists.
fn has_glob_match(dir: &Path, prefix: &str) -> bool {
    for ext in &["ts", "js", "mts", "mjs"] {
        if dir.join(format!("{}.{}", prefix, ext)).exists() {
            return true;
        }
    }
    false
}

/// Check package.json for vitest or jest dependency/script references.
fn check_package_json(dir: &Path) -> Option<TestFramework> {
    let pkg = dir.join("package.json");
    if !pkg.exists() {
        return None;
    }
    if let Ok(content) = std::fs::read_to_string(&pkg) {
        // Check for vitest first (more specific)
        if content.contains("\"vitest\"") {
            return Some(TestFramework::Vitest);
        }
        if content.contains("\"jest\"") {
            return Some(TestFramework::Jest);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_pytest_conftest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("conftest.py"), "").unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::Pytest)
        );
    }

    #[test]
    fn test_detect_pytest_ini() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pytest.ini"), "[pytest]").unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::Pytest)
        );
    }

    #[test]
    fn test_detect_pytest_pyproject() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.pytest.ini_options]\nminversion = \"6.0\"",
        )
        .unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::Pytest)
        );
    }

    #[test]
    fn test_detect_vitest_config() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("vitest.config.ts"), "export default {}").unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::Vitest)
        );
    }

    #[test]
    fn test_detect_vitest_package_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"vitest":"^1.0"}}"#,
        )
        .unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::Vitest)
        );
    }

    #[test]
    fn test_detect_jest_package_json() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"jest":"^29"}}"#,
        )
        .unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::Jest)
        );
    }

    #[test]
    fn test_detect_cargo_test() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::CargoTest)
        );
    }

    #[test]
    fn test_detect_go_test() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/foo").unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::GoTest)
        );
    }

    #[test]
    fn test_detect_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_framework(dir.path().to_str().unwrap()), None);
    }

    #[test]
    fn test_vitest_over_jest() {
        // If both vitest and jest are in package.json, vitest wins
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"vitest":"^1.0","jest":"^29"}}"#,
        )
        .unwrap();
        assert_eq!(
            detect_framework(dir.path().to_str().unwrap()),
            Some(TestFramework::Vitest)
        );
    }

    #[test]
    fn test_framework_as_str() {
        assert_eq!(TestFramework::Pytest.as_str(), "pytest");
        assert_eq!(TestFramework::Vitest.as_str(), "vitest");
        assert_eq!(TestFramework::Jest.as_str(), "jest");
        assert_eq!(TestFramework::CargoTest.as_str(), "cargo_test");
        assert_eq!(TestFramework::GoTest.as_str(), "go_test");
    }
}
