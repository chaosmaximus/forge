//! `forge verify <file>` — unified verification command.
//!
//! Runs all verification checks in sequence: syntax, format, lint, security,
//! cross-file breakage, and optionally type checking.

use crate::verify::detect::{detect_language, Language};
use serde::Serialize;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Instant;
use tree_sitter::Parser;

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct CheckResult {
    pub name: String,
    pub status: String, // "pass", "fail", "warn", "fixed", "skip"
    pub tool: String,
    pub duration_ms: u128,
    pub issues: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Run all verification checks on a file or directory.
pub fn run(path: &str, fix: bool, format: &str, state_dir: &str, run_types: bool) {
    let language = detect_language(path);
    let mut checks: Vec<CheckResult> = Vec::new();

    // 1. Syntax (tree-sitter) — always for supported languages
    checks.push(check_syntax(path, &language));

    // 2. Format (ruff/prettier) — if tool available
    checks.push(check_format(path, &language, fix));

    // 3. Lint (ruff/eslint) — if tool available
    checks.push(check_lint(path, &language));

    // 4. Security (secret scan) — always
    checks.push(check_security(path));

    // 5. Cross-file (signature diff) — if caches exist and language supported
    checks.push(check_cross_file(path, &language, state_dir));

    // 6. Types (mypy/tsc) — optional (slower)
    checks.push(check_types(path, &language, run_types));

    // Aggregate results
    let status = if checks.iter().any(|c| c.status == "fail") {
        "fail"
    } else if checks.iter().any(|c| c.status == "warn") {
        "warn"
    } else {
        "pass"
    };

    let active_checks = checks.iter().filter(|c| c.status != "skip").count();
    let total_checks = checks.len();
    let pass_count = checks
        .iter()
        .filter(|c| c.status == "pass" || c.status == "fixed")
        .count();
    let warn_count = checks.iter().filter(|c| c.status == "warn").count();
    let fail_count = checks.iter().filter(|c| c.status == "fail").count();

    let summary = if fail_count > 0 {
        format!(
            "{}/{} pass, {} failed",
            pass_count, active_checks, fail_count
        )
    } else if warn_count > 0 {
        format!(
            "{}/{} pass, {} warning",
            pass_count, active_checks, warn_count
        )
    } else {
        format!("{}/{} pass", pass_count, total_checks)
    };

    if format == "text" {
        println!("forge verify {}", path);
        for check in &checks {
            let icon = match check.status.as_str() {
                "pass" | "fixed" => "\u{2713}",  // checkmark
                "warn" => "\u{26a0}",             // warning
                "fail" => "\u{2717}",             // x mark
                "skip" => "\u{25cb}",             // circle
                _ => " ",
            };
            let duration_str = if check.status == "skip" {
                "skip".to_string()
            } else {
                format!("{}ms", check.duration_ms)
            };
            let detail = if let Some(ref d) = check.detail {
                format!("  {}", d)
            } else if check.issues > 0 {
                format!("  {} issue(s)", check.issues)
            } else {
                String::new()
            };
            println!(
                "  {} {:<11} {:<14} {}{}",
                icon, check.name, check.tool, duration_str, detail
            );
        }
        println!();
        println!("  {}", summary);
    } else {
        let output = json!({
            "status": status,
            "file": path,
            "checks": checks,
            "summary": summary,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap_or_default());
    }
}

// ---------------------------------------------------------------------------
// Individual checks
// ---------------------------------------------------------------------------

fn check_syntax(path: &str, language: &Language) -> CheckResult {
    let start = Instant::now();

    let ts_lang = match language {
        Language::Python => Some(("python", tree_sitter_python::LANGUAGE.into())),
        Language::TypeScript => Some((
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        )),
        Language::JavaScript => Some(("javascript", tree_sitter_javascript::LANGUAGE.into())),
        _ => None,
    };

    let ts_lang: Option<(&str, tree_sitter::Language)> = match ts_lang {
        Some((name, lang)) => Some((name, lang)),
        None => {
            return CheckResult {
                name: "syntax".into(),
                status: "skip".into(),
                tool: "tree-sitter".into(),
                duration_ms: 0,
                issues: 0,
                detail: Some("unsupported language".into()),
            }
        }
    };

    let (_, ts_language) = ts_lang.unwrap();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return CheckResult {
                name: "syntax".into(),
                status: "fail".into(),
                tool: "tree-sitter".into(),
                duration_ms: start.elapsed().as_millis(),
                issues: 1,
                detail: Some("cannot read file".into()),
            }
        }
    };

    let mut parser = Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return CheckResult {
            name: "syntax".into(),
            status: "skip".into(),
            tool: "tree-sitter".into(),
            duration_ms: start.elapsed().as_millis(),
            issues: 0,
            detail: Some("parser init failed".into()),
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
    let duration_ms = start.elapsed().as_millis();

    CheckResult {
        name: "syntax".into(),
        status: if has_errors { "fail" } else { "pass" }.into(),
        tool: "tree-sitter".into(),
        duration_ms,
        issues: if has_errors { 1 } else { 0 },
        detail: if has_errors {
            Some("syntax errors detected".into())
        } else {
            None
        },
    }
}

fn check_format(path: &str, language: &Language, fix: bool) -> CheckResult {
    let (tool_name, tool_cmd, args_check, args_fix) = match language {
        Language::Python => ("ruff", "ruff", vec!["format", "--check"], vec!["format"]),
        Language::TypeScript | Language::JavaScript => (
            "prettier",
            if tool_exists("prettier") {
                "prettier"
            } else {
                "npx"
            },
            if tool_exists("prettier") {
                vec!["--check"]
            } else {
                vec!["prettier", "--check"]
            },
            if tool_exists("prettier") {
                vec!["--write"]
            } else {
                vec!["prettier", "--write"]
            },
        ),
        Language::Rust => ("rustfmt", "rustfmt", vec!["--check"], vec![]),
        Language::Go => ("gofmt", "gofmt", vec!["-l"], vec!["-w"]),
        _ => {
            return CheckResult {
                name: "format".into(),
                status: "skip".into(),
                tool: "none".into(),
                duration_ms: 0,
                issues: 0,
                detail: Some("unsupported language".into()),
            }
        }
    };

    if !tool_exists(tool_cmd) {
        return CheckResult {
            name: "format".into(),
            status: "skip".into(),
            tool: tool_name.into(),
            duration_ms: 0,
            issues: 0,
            detail: Some(format!("{} not installed", tool_name)),
        };
    }

    let start = Instant::now();

    if fix && !args_fix.is_empty() {
        // Run in fix mode
        let mut fix_args = args_fix;
        fix_args.push(path);
        let output = Command::new(tool_cmd).args(&fix_args).output();
        let duration_ms = start.elapsed().as_millis();

        match output {
            Ok(o) if o.status.success() => CheckResult {
                name: "format".into(),
                status: "fixed".into(),
                tool: tool_name.into(),
                duration_ms,
                issues: 0,
                detail: Some("auto-formatted".into()),
            },
            Ok(_) => CheckResult {
                name: "format".into(),
                status: "pass".into(),
                tool: tool_name.into(),
                duration_ms,
                issues: 0,
                detail: None,
            },
            Err(_) => CheckResult {
                name: "format".into(),
                status: "skip".into(),
                tool: tool_name.into(),
                duration_ms,
                issues: 0,
                detail: Some("tool execution failed".into()),
            },
        }
    } else {
        // Run in check mode
        let mut check_args = args_check;
        check_args.push(path);
        let output = Command::new(tool_cmd).args(&check_args).output();
        let duration_ms = start.elapsed().as_millis();

        match output {
            Ok(o) => {
                let needs_formatting = !o.status.success();
                CheckResult {
                    name: "format".into(),
                    status: if needs_formatting { "warn" } else { "pass" }.into(),
                    tool: tool_name.into(),
                    duration_ms,
                    issues: if needs_formatting { 1 } else { 0 },
                    detail: if needs_formatting {
                        Some("needs formatting".into())
                    } else {
                        None
                    },
                }
            }
            Err(_) => CheckResult {
                name: "format".into(),
                status: "skip".into(),
                tool: tool_name.into(),
                duration_ms,
                issues: 0,
                detail: Some("tool execution failed".into()),
            },
        }
    }
}

fn check_lint(path: &str, language: &Language) -> CheckResult {
    let (tool_name, tool_cmd, args) = match language {
        Language::Python => ("ruff", "ruff", vec!["check", "--output-format", "json"]),
        Language::TypeScript | Language::JavaScript => ("eslint", "eslint", vec!["-f", "json"]),
        Language::Rust => ("clippy", "cargo", vec!["clippy", "--message-format=json", "--quiet"]),
        Language::Go => (
            "golangci-lint",
            "golangci-lint",
            vec!["run", "--out-format", "json"],
        ),
        _ => {
            return CheckResult {
                name: "lint".into(),
                status: "skip".into(),
                tool: "none".into(),
                duration_ms: 0,
                issues: 0,
                detail: Some("unsupported language".into()),
            }
        }
    };

    if !tool_exists(tool_cmd) {
        return CheckResult {
            name: "lint".into(),
            status: "skip".into(),
            tool: tool_name.into(),
            duration_ms: 0,
            issues: 0,
            detail: Some(format!("{} not installed", tool_name)),
        };
    }

    let start = Instant::now();
    let mut cmd_args = args;
    // For cargo clippy, don't append the file path (it's project-level)
    if tool_cmd != "cargo" {
        cmd_args.push(path);
    }

    let output = Command::new(tool_cmd).args(&cmd_args).output();
    let duration_ms = start.elapsed().as_millis();

    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let issue_count = count_lint_issues(&stdout, language);

            CheckResult {
                name: "lint".into(),
                status: if issue_count > 0 { "warn" } else { "pass" }.into(),
                tool: tool_name.into(),
                duration_ms,
                issues: issue_count,
                detail: if issue_count > 0 {
                    Some(format!("{} issue(s)", issue_count))
                } else {
                    None
                },
            }
        }
        Err(_) => CheckResult {
            name: "lint".into(),
            status: "skip".into(),
            tool: tool_name.into(),
            duration_ms,
            issues: 0,
            detail: Some("tool execution failed".into()),
        },
    }
}

fn check_security(path: &str) -> CheckResult {
    use crate::scan::rules::RULES;

    let start = Instant::now();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            // If it's a directory, run scan on all files — but for simplicity,
            // just report skip for non-readable paths
            return CheckResult {
                name: "security".into(),
                status: "skip".into(),
                tool: "forge-scan".into(),
                duration_ms: 0,
                issues: 0,
                detail: Some("cannot read file".into()),
            };
        }
    };

    let mut issue_count = 0;
    for line in content.lines() {
        for rule in RULES.iter() {
            if rule.regex.is_match(line) {
                issue_count += 1;
                break;
            }
        }
    }

    let duration_ms = start.elapsed().as_millis();

    CheckResult {
        name: "security".into(),
        status: if issue_count > 0 { "fail" } else { "pass" }.into(),
        tool: "forge-scan".into(),
        duration_ms,
        issues: issue_count,
        detail: if issue_count > 0 {
            Some(format!("{} secret(s) detected", issue_count))
        } else {
            None
        },
    }
}

fn check_cross_file(path: &str, language: &Language, state_dir: &str) -> CheckResult {
    let lang_str = match language {
        Language::Python => "python",
        Language::TypeScript => "typescript",
        Language::JavaScript => "javascript",
        _ => {
            return CheckResult {
                name: "cross_file".into(),
                status: "skip".into(),
                tool: "forge".into(),
                duration_ms: 0,
                issues: 0,
                detail: Some("unsupported language".into()),
            }
        }
    };

    // Check if signature cache exists
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
            detail: Some("no signature cache (run forge index first)".into()),
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
                detail: Some("cannot read file".into()),
            }
        }
    };

    let start = Instant::now();
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
            Some(format!("{} signature change(s) detected", issue_count))
        } else {
            None
        },
    }
}

fn check_types(path: &str, language: &Language, run_types: bool) -> CheckResult {
    let tool_name = match language {
        Language::Python => "mypy",
        Language::TypeScript => "tsc",
        Language::Rust => "cargo-check",
        Language::Go => "go-vet",
        _ => "none",
    };

    if !run_types {
        return CheckResult {
            name: "types".into(),
            status: "skip".into(),
            tool: tool_name.into(),
            duration_ms: 0,
            issues: 0,
            detail: Some("run with --types".into()),
        };
    }

    let (tool_cmd, args): (&str, Vec<&str>) = match language {
        Language::Python => ("mypy", vec!["--no-error-summary", path]),
        Language::TypeScript => ("npx", vec!["tsc", "--noEmit", "--pretty", "false"]),
        Language::Rust => ("cargo", vec!["check", "--message-format=json", "--quiet"]),
        Language::Go => ("go", vec!["vet", "./..."]),
        _ => {
            return CheckResult {
                name: "types".into(),
                status: "skip".into(),
                tool: "none".into(),
                duration_ms: 0,
                issues: 0,
                detail: Some("unsupported language".into()),
            }
        }
    };

    if !tool_exists(tool_cmd) {
        return CheckResult {
            name: "types".into(),
            status: "skip".into(),
            tool: tool_name.into(),
            duration_ms: 0,
            issues: 0,
            detail: Some(format!("{} not installed", tool_name)),
        };
    }

    let start = Instant::now();
    let output = Command::new(tool_cmd).args(&args).output();
    let duration_ms = start.elapsed().as_millis();

    match output {
        Ok(o) => {
            let success = o.status.success();
            CheckResult {
                name: "types".into(),
                status: if success { "pass" } else { "fail" }.into(),
                tool: tool_name.into(),
                duration_ms,
                issues: if success { 0 } else { 1 },
                detail: if !success {
                    Some("type errors found".into())
                } else {
                    None
                },
            }
        }
        Err(_) => CheckResult {
            name: "types".into(),
            status: "skip".into(),
            tool: tool_name.into(),
            duration_ms,
            issues: 0,
            detail: Some("tool execution failed".into()),
        },
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn tool_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Count lint issues from tool output.
fn count_lint_issues(stdout: &str, language: &Language) -> usize {
    match language {
        Language::Python => {
            // ruff JSON output is an array
            serde_json::from_str::<Vec<serde_json::Value>>(stdout)
                .map(|v| v.len())
                .unwrap_or(0)
        }
        Language::TypeScript | Language::JavaScript => {
            // eslint JSON is array of file results
            serde_json::from_str::<Vec<serde_json::Value>>(stdout)
                .map(|files| {
                    files
                        .iter()
                        .filter_map(|f| f.get("messages").and_then(|m| m.as_array()))
                        .map(|msgs| msgs.len())
                        .sum()
                })
                .unwrap_or(0)
        }
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_syntax_valid_python() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "def foo():\n    pass\n").unwrap();

        let result = check_syntax(file.to_str().unwrap(), &Language::Python);
        assert_eq!(result.status, "pass");
        assert_eq!(result.issues, 0);
        assert_eq!(result.tool, "tree-sitter");
    }

    #[test]
    fn test_check_syntax_invalid_python() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bad.py");
        std::fs::write(&file, "def foo(\n    pass\n").unwrap();

        let result = check_syntax(file.to_str().unwrap(), &Language::Python);
        assert_eq!(result.status, "fail");
        assert!(result.issues > 0);
    }

    #[test]
    fn test_check_syntax_unsupported() {
        let result = check_syntax("test.rs", &Language::Rust);
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_check_security_clean() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("clean.py");
        std::fs::write(&file, "x = 42\nprint(x)\n").unwrap();

        let result = check_security(file.to_str().unwrap());
        assert_eq!(result.status, "pass");
        assert_eq!(result.issues, 0);
    }

    #[test]
    fn test_check_security_with_secret() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("secrets.py");
        // Use an AWS Access Key ID pattern (AKIA + 16 uppercase chars) that matches scan rules
        std::fs::write(
            &file,
            "key = AKIAIOSFODNN7EXAMPLE\n",
        )
        .unwrap();

        let result = check_security(file.to_str().unwrap());
        assert_eq!(result.status, "fail");
        assert!(result.issues > 0);
    }

    #[test]
    fn test_check_cross_file_no_cache() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.py");
        std::fs::write(&file, "def foo(): pass").unwrap();

        let result = check_cross_file(
            file.to_str().unwrap(),
            &Language::Python,
            dir.path().to_str().unwrap(),
        );
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_check_cross_file_unsupported() {
        let result = check_cross_file("test.rs", &Language::Rust, "/tmp");
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_check_types_skipped_by_default() {
        let result = check_types("test.py", &Language::Python, false);
        assert_eq!(result.status, "skip");
        assert!(result.detail.as_ref().unwrap().contains("--types"));
    }

    #[test]
    fn test_check_format_unsupported() {
        let result = check_format("test.xyz", &Language::Unknown, false);
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_check_lint_unsupported() {
        let result = check_lint("test.xyz", &Language::Unknown);
        assert_eq!(result.status, "skip");
    }

    #[test]
    fn test_count_lint_issues_ruff_empty() {
        assert_eq!(count_lint_issues("[]", &Language::Python), 0);
    }

    #[test]
    fn test_count_lint_issues_ruff_with_issues() {
        let json = r#"[{"code": "E302"}, {"code": "F401"}]"#;
        assert_eq!(count_lint_issues(json, &Language::Python), 2);
    }

    #[test]
    fn test_count_lint_issues_eslint() {
        let json = r#"[{"filePath": "x.js", "messages": [{"ruleId": "no-unused-vars"}]}]"#;
        assert_eq!(count_lint_issues(json, &Language::JavaScript), 1);
    }

    #[test]
    fn test_check_result_serialization() {
        let result = CheckResult {
            name: "syntax".into(),
            status: "pass".into(),
            tool: "tree-sitter".into(),
            duration_ms: 2,
            issues: 0,
            detail: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"name\":\"syntax\""));
        // detail should be omitted when None
        assert!(!json.contains("detail"));
    }
}
