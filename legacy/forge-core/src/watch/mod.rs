//! `forge watch [path]` — continuous file monitoring with debounced verification.
//!
//! Watches a project directory for changes to source files, debounces rapid saves
//! (500ms window), then runs the verify pipeline on each changed file. Results are
//! written atomically to `$STATE/diagnostics.json` for HUD and context injection.

use crate::hud_state;
use crate::verify::detect::{detect_language, Language};
use crate::verify::unified::CheckResult;
use notify::{Event, EventKind, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

const DEBOUNCE_MS: u64 = 500;
const WATCH_EXTENSIONS: &[&str] = &["py", "ts", "tsx", "js", "jsx", "rs", "go"];

/// Directories to ignore (never recurse into).
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    ".venv",
    "venv",
    ".mypy_cache",
    ".ruff_cache",
    "dist",
    "build",
    ".next",
];

/// A single diagnostic emitted by the watch pipeline.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Diagnostic {
    pub file: String,
    pub check: String,
    pub severity: String, // "error", "warning", "info"
    pub tool: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub issues: usize,
}

/// Entry point: block forever watching `path` for source file changes.
pub fn run(path: &str, state_dir: &str) {
    eprintln!("forge watch: monitoring {} for changes...", path);
    eprintln!("  extensions: {}", WATCH_EXTENSIONS.join(", "));
    eprintln!("  debounce:   {}ms", DEBOUNCE_MS);
    eprintln!("  state:      {}/diagnostics.json", state_dir);
    eprintln!("Press Ctrl+C to stop.\n");

    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })
    .expect("Failed to create file watcher");

    watcher
        .watch(Path::new(path), RecursiveMode::Recursive)
        .expect("Failed to watch directory");

    let mut pending: HashSet<String> = HashSet::new();
    let mut last_event = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                // Only care about create/modify events
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) => {}
                    _ => continue,
                }
                for p in event.paths {
                    // Skip ignored directories
                    let path_str = p.to_string_lossy();
                    if IGNORED_DIRS
                        .iter()
                        .any(|dir| path_str.contains(&format!("/{}/", dir)))
                    {
                        continue;
                    }
                    // Filter by extension
                    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                        if WATCH_EXTENSIONS.contains(&ext) {
                            pending.insert(p.to_string_lossy().to_string());
                            last_event = Instant::now();
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Check if debounce period has elapsed with pending changes
                if !pending.is_empty()
                    && last_event.elapsed() > Duration::from_millis(DEBOUNCE_MS)
                {
                    process_changes(&pending, state_dir);
                    pending.clear();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Run verification checks on a batch of changed files.
fn process_changes(files: &HashSet<String>, state_dir: &str) {
    let start = Instant::now();
    eprintln!(
        "forge watch: {} file(s) changed, running checks...",
        files.len()
    );

    let mut all_diagnostics: Vec<Diagnostic> = Vec::new();

    for file_path in files {
        // Skip files that no longer exist (deleted between event and processing)
        if !Path::new(file_path).exists() {
            continue;
        }

        let language = detect_language(file_path);

        // 1. Syntax check (tree-sitter) — fast, in-process
        let syntax = check_syntax_for_watch(file_path, &language);
        if syntax.status == "fail" {
            all_diagnostics.push(Diagnostic {
                file: file_path.clone(),
                check: syntax.name.clone(),
                severity: "error".into(),
                tool: syntax.tool.clone(),
                detail: syntax.detail.clone(),
                issues: syntax.issues,
            });
        }

        // 2. Security scan (secret detection) — fast, in-process
        let security = check_security_for_watch(file_path);
        if security.status == "fail" {
            all_diagnostics.push(Diagnostic {
                file: file_path.clone(),
                check: security.name.clone(),
                severity: "error".into(),
                tool: security.tool.clone(),
                detail: security.detail.clone(),
                issues: security.issues,
            });
        }

        // 3. Cross-file breakage — fast if caches exist
        let cross = check_cross_file_for_watch(file_path, &language, state_dir);
        if cross.status == "warn" || cross.status == "fail" {
            all_diagnostics.push(Diagnostic {
                file: file_path.clone(),
                check: cross.name.clone(),
                severity: if cross.status == "fail" {
                    "error"
                } else {
                    "warning"
                }
                .into(),
                tool: cross.tool.clone(),
                detail: cross.detail.clone(),
                issues: cross.issues,
            });
        }
    }

    // 4. Spawn type checker in background (non-blocking)
    spawn_type_checker(files);

    // Write diagnostics to state file atomically
    write_diagnostics(state_dir, &all_diagnostics);

    // Update HUD with diagnostic summary
    update_hud_diagnostics(state_dir, &all_diagnostics);

    let errors = all_diagnostics
        .iter()
        .filter(|d| d.severity == "error")
        .count();
    let warns = all_diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count();
    let elapsed = start.elapsed().as_millis();

    if errors > 0 || warns > 0 {
        eprintln!(
            "forge watch: {} error(s), {} warning(s) [{}ms]",
            errors, warns, elapsed
        );
        // Print details for errors
        for d in &all_diagnostics {
            if d.severity == "error" {
                let detail = d.detail.as_deref().unwrap_or("");
                eprintln!("  {} {}: {} {}", "\u{2717}", d.check, d.file, detail);
            }
        }
    } else {
        eprintln!("forge watch: all clear \u{2713} [{}ms]", elapsed);
    }
    eprintln!();
}

// ---------------------------------------------------------------------------
// Verification checks (reusing logic from verify::unified)
// ---------------------------------------------------------------------------

fn check_syntax_for_watch(path: &str, language: &Language) -> CheckResult {
    use tree_sitter::Parser;

    let ts_lang: Option<tree_sitter::Language> = match language {
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::JavaScript => Some(tree_sitter_javascript::LANGUAGE.into()),
        _ => None,
    };

    let ts_language = match ts_lang {
        Some(lang) => lang,
        None => {
            return CheckResult {
                name: "syntax".into(),
                status: "skip".into(),
                tool: "tree-sitter".into(),
                duration_ms: 0,
                issues: 0,
                detail: None,
            }
        }
    };

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return CheckResult {
                name: "syntax".into(),
                status: "skip".into(),
                tool: "tree-sitter".into(),
                duration_ms: 0,
                issues: 0,
                detail: Some("cannot read file".into()),
            }
        }
    };

    let start = Instant::now();
    let mut parser = Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return CheckResult {
            name: "syntax".into(),
            status: "skip".into(),
            tool: "tree-sitter".into(),
            duration_ms: 0,
            issues: 0,
            detail: None,
        };
    }

    let tree = match parser.parse(&content, None) {
        Some(t) => t,
        None => {
            return CheckResult {
                name: "syntax".into(),
                status: "fail".into(),
                tool: "tree-sitter".into(),
                duration_ms: start.elapsed().as_millis(),
                issues: 1,
                detail: Some("parse failed".into()),
            }
        }
    };

    let has_errors = tree.root_node().has_error();
    CheckResult {
        name: "syntax".into(),
        status: if has_errors { "fail" } else { "pass" }.into(),
        tool: "tree-sitter".into(),
        duration_ms: start.elapsed().as_millis(),
        issues: if has_errors { 1 } else { 0 },
        detail: if has_errors {
            Some("syntax errors detected".into())
        } else {
            None
        },
    }
}

fn check_security_for_watch(path: &str) -> CheckResult {
    use crate::scan::rules::RULES;

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return CheckResult {
                name: "security".into(),
                status: "skip".into(),
                tool: "forge-scan".into(),
                duration_ms: 0,
                issues: 0,
                detail: None,
            }
        }
    };

    let start = Instant::now();
    let mut issue_count = 0;
    for line in content.lines() {
        for rule in RULES.iter() {
            if rule.regex.is_match(line) {
                issue_count += 1;
                break;
            }
        }
    }

    CheckResult {
        name: "security".into(),
        status: if issue_count > 0 { "fail" } else { "pass" }.into(),
        tool: "forge-scan".into(),
        duration_ms: start.elapsed().as_millis(),
        issues: issue_count,
        detail: if issue_count > 0 {
            Some(format!("{} secret(s) detected", issue_count))
        } else {
            None
        },
    }
}

fn check_cross_file_for_watch(
    path: &str,
    language: &Language,
    state_dir: &str,
) -> CheckResult {
    // Only supported for Python/TS/JS
    match language {
        Language::Python | Language::TypeScript | Language::JavaScript => {}
        _ => {
            return CheckResult {
                name: "cross_file".into(),
                status: "skip".into(),
                tool: "forge".into(),
                duration_ms: 0,
                issues: 0,
                detail: None,
            }
        }
    }

    // Need signature cache to exist
    let cache_path = Path::new(state_dir)
        .join("index")
        .join("signatures.json");
    if !cache_path.exists() {
        return CheckResult {
            name: "cross_file".into(),
            status: "skip".into(),
            tool: "forge".into(),
            duration_ms: 0,
            issues: 0,
            detail: None,
        };
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return CheckResult {
                name: "cross_file".into(),
                status: "skip".into(),
                tool: "forge".into(),
                duration_ms: 0,
                issues: 0,
                detail: None,
            }
        }
    };

    let start = Instant::now();
    let lang_str = language.as_str();
    let breakages =
        crate::verify::cross_file::check_file(path, &content, lang_str, state_dir);
    let duration_ms = start.elapsed().as_millis();
    let issue_count = breakages.len();

    CheckResult {
        name: "cross_file".into(),
        status: if issue_count > 0 { "warn" } else { "pass" }.into(),
        tool: "forge".into(),
        duration_ms,
        issues: issue_count,
        detail: if issue_count > 0 {
            Some(format!("{} signature change(s)", issue_count))
        } else {
            None
        },
    }
}

// ---------------------------------------------------------------------------
// Type checker (background spawn — non-blocking)
// ---------------------------------------------------------------------------

fn spawn_type_checker(files: &HashSet<String>) {
    // Detect dominant language from changed files
    let mut py_count = 0;
    let mut ts_count = 0;
    let mut rs_count = 0;

    for f in files {
        match Path::new(f).extension().and_then(|e| e.to_str()) {
            Some("py" | "pyi") => py_count += 1,
            Some("ts" | "tsx") => ts_count += 1,
            Some("rs") => rs_count += 1,
            _ => {}
        }
    }

    // Spawn the appropriate type checker in the background
    if py_count > 0 {
        if tool_exists("pyright") {
            let file_list: Vec<&String> = files
                .iter()
                .filter(|f| {
                    Path::new(f.as_str())
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e == "py" || e == "pyi")
                        .unwrap_or(false)
                })
                .collect();
            if !file_list.is_empty() {
                let args: Vec<&str> = file_list.iter().map(|s| s.as_str()).collect();
                let _ = Command::new("pyright").args(&args).spawn();
            }
        }
    }

    if ts_count > 0 {
        if tool_exists("tsc") {
            let _ = Command::new("tsc").arg("--noEmit").spawn();
        }
    }

    if rs_count > 0 {
        if tool_exists("cargo") {
            let _ = Command::new("cargo")
                .args(["check", "--message-format=json", "--quiet"])
                .spawn();
        }
    }
}

// ---------------------------------------------------------------------------
// State file I/O
// ---------------------------------------------------------------------------

/// Write diagnostics atomically: write to tmp, then rename.
fn write_diagnostics(state_dir: &str, diagnostics: &[Diagnostic]) {
    let dir = Path::new(state_dir);
    if !dir.exists() {
        let _ = std::fs::create_dir_all(dir);
    }
    let path = dir.join("diagnostics.json");
    let tmp = path.with_extension("json.tmp");
    if let Ok(json) = serde_json::to_string_pretty(diagnostics) {
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Update HUD state with diagnostic summary counts.
fn update_hud_diagnostics(state_dir: &str, diagnostics: &[Diagnostic]) {
    let error_count = diagnostics
        .iter()
        .filter(|d| d.severity == "error")
        .count() as u64;
    let security_count = diagnostics
        .iter()
        .filter(|d| d.check == "security")
        .map(|d| d.issues as u64)
        .sum::<u64>();

    hud_state::update(state_dir, |state| {
        state.security.exposed = security_count;
        // Store total diagnostic errors in skills.fix_candidates as a proxy
        // (existing HUD schema doesn't have a dedicated diagnostics field)
        state.skills.fix_candidates = error_count;
    });
}

fn tool_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostic_serialization() {
        let d = Diagnostic {
            file: "src/main.py".into(),
            check: "syntax".into(),
            severity: "error".into(),
            tool: "tree-sitter".into(),
            detail: Some("syntax errors detected".into()),
            issues: 1,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"severity\":\"error\""));
        assert!(json.contains("\"file\":\"src/main.py\""));
    }

    #[test]
    fn test_diagnostic_no_detail_skips() {
        let d = Diagnostic {
            file: "clean.py".into(),
            check: "syntax".into(),
            severity: "info".into(),
            tool: "tree-sitter".into(),
            detail: None,
            issues: 0,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("detail"));
    }

    #[test]
    fn test_write_diagnostics_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();
        let diagnostics = vec![Diagnostic {
            file: "test.py".into(),
            check: "syntax".into(),
            severity: "error".into(),
            tool: "tree-sitter".into(),
            detail: Some("parse error".into()),
            issues: 1,
        }];

        write_diagnostics(state_dir, &diagnostics);

        let path = dir.path().join("diagnostics.json");
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Vec<Diagnostic> = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].severity, "error");
        assert_eq!(parsed[0].file, "test.py");

        // tmp file should not remain
        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists());
    }

    #[test]
    fn test_write_diagnostics_empty() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        write_diagnostics(state_dir, &[]);

        let path = dir.path().join("diagnostics.json");
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Vec<Diagnostic> = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_update_hud_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        let diagnostics = vec![
            Diagnostic {
                file: "a.py".into(),
                check: "syntax".into(),
                severity: "error".into(),
                tool: "tree-sitter".into(),
                detail: None,
                issues: 1,
            },
            Diagnostic {
                file: "b.py".into(),
                check: "security".into(),
                severity: "error".into(),
                tool: "forge-scan".into(),
                detail: Some("2 secret(s) detected".into()),
                issues: 2,
            },
            Diagnostic {
                file: "c.py".into(),
                check: "cross_file".into(),
                severity: "warning".into(),
                tool: "forge".into(),
                detail: None,
                issues: 1,
            },
        ];

        update_hud_diagnostics(state_dir, &diagnostics);

        let state = hud_state::read(state_dir);
        assert_eq!(state.security.exposed, 2);
        assert_eq!(state.skills.fix_candidates, 2); // 2 errors (syntax + security)
    }

    #[test]
    fn test_check_syntax_valid_python_watch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("valid.py");
        std::fs::write(&file, "def foo():\n    return 42\n").unwrap();

        let result = check_syntax_for_watch(file.to_str().unwrap(), &Language::Python);
        assert_eq!(result.status, "pass");
        assert_eq!(result.issues, 0);
    }

    #[test]
    fn test_check_syntax_invalid_python_watch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.py");
        std::fs::write(&file, "def foo(\n    return 42\n").unwrap();

        let result = check_syntax_for_watch(file.to_str().unwrap(), &Language::Python);
        assert_eq!(result.status, "fail");
        assert!(result.issues > 0);
    }

    #[test]
    fn test_check_syntax_unsupported_language() {
        let result = check_syntax_for_watch("test.go", &Language::Go);
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_check_security_clean_watch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("clean.py");
        std::fs::write(&file, "x = 42\nprint(x)\n").unwrap();

        let result = check_security_for_watch(file.to_str().unwrap());
        assert_eq!(result.status, "pass");
        assert_eq!(result.issues, 0);
    }

    #[test]
    fn test_check_security_with_secret_watch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.py");
        std::fs::write(&file, "key = AKIAIOSFODNN7EXAMPLE\n").unwrap();

        let result = check_security_for_watch(file.to_str().unwrap());
        assert_eq!(result.status, "fail");
        assert!(result.issues > 0);
    }

    #[test]
    fn test_check_cross_file_no_cache_watch() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "def foo(): pass").unwrap();

        let result = check_cross_file_for_watch(
            file.to_str().unwrap(),
            &Language::Python,
            dir.path().to_str().unwrap(),
        );
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_check_cross_file_unsupported_watch() {
        let result = check_cross_file_for_watch("test.rs", &Language::Rust, "/tmp");
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_ignored_dirs_list() {
        assert!(IGNORED_DIRS.contains(&".git"));
        assert!(IGNORED_DIRS.contains(&"node_modules"));
        assert!(IGNORED_DIRS.contains(&"target"));
        assert!(IGNORED_DIRS.contains(&"__pycache__"));
        assert!(IGNORED_DIRS.contains(&".venv"));
    }

    #[test]
    fn test_watch_extensions() {
        assert!(WATCH_EXTENSIONS.contains(&"py"));
        assert!(WATCH_EXTENSIONS.contains(&"ts"));
        assert!(WATCH_EXTENSIONS.contains(&"tsx"));
        assert!(WATCH_EXTENSIONS.contains(&"js"));
        assert!(WATCH_EXTENSIONS.contains(&"jsx"));
        assert!(WATCH_EXTENSIONS.contains(&"rs"));
        assert!(WATCH_EXTENSIONS.contains(&"go"));
    }

    #[test]
    fn test_process_changes_with_nonexistent_files() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        let mut files = HashSet::new();
        files.insert("/nonexistent/path/fake.py".to_string());

        // Should not panic — gracefully skips missing files
        process_changes(&files, state_dir);

        let path = dir.path().join("diagnostics.json");
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: Vec<Diagnostic> = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_process_changes_with_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // Create a file with a syntax error
        let bad_file = dir.path().join("bad.py");
        std::fs::write(&bad_file, "def foo(\n    return\n").unwrap();

        let mut files = HashSet::new();
        files.insert(bad_file.to_string_lossy().to_string());

        process_changes(&files, state_dir);

        let diag_path = dir.path().join("diagnostics.json");
        assert!(diag_path.exists());
        let content = std::fs::read_to_string(&diag_path).unwrap();
        let parsed: Vec<Diagnostic> = serde_json::from_str(&content).unwrap();
        // Should have at least a syntax error
        assert!(!parsed.is_empty());
        assert!(parsed.iter().any(|d| d.check == "syntax" && d.severity == "error"));
    }

    #[test]
    fn test_process_changes_clean_file() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        let good_file = dir.path().join("good.py");
        std::fs::write(&good_file, "def foo():\n    return 42\n").unwrap();

        let mut files = HashSet::new();
        files.insert(good_file.to_string_lossy().to_string());

        process_changes(&files, state_dir);

        let diag_path = dir.path().join("diagnostics.json");
        let content = std::fs::read_to_string(&diag_path).unwrap();
        let parsed: Vec<Diagnostic> = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_empty());
    }
}
