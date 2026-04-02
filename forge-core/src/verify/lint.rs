//! `forge lint <file>` -- auto-detect language, run the right linter, normalize output.

use crate::verify::detect::{detect_language, Language};
use serde_json::json;
use std::process::Command;
use std::time::Instant;

/// Normalized diagnostic from any linter.
#[derive(Debug)]
struct Diagnostic {
    file: String,
    line: u64,
    col: u64,
    severity: String,
    code: String,
    message: String,
}

pub fn run(path: &str, format: &str) {
    let lang = detect_language(path);
    let result = match lang {
        Language::Python => run_ruff_check(path),
        Language::TypeScript | Language::JavaScript => run_eslint(path),
        Language::Rust => run_clippy(path),
        Language::Go => run_golangci_lint(path),
        Language::Unknown => {
            let output = json!({
                "error": "Cannot detect language",
                "file": path,
                "hint": "Ensure the file has a recognized extension (.py, .ts, .js, .rs, .go)"
            });
            if format == "text" {
                eprintln!("Error: Cannot detect language for {}", path);
            } else {
                println!("{}", output);
            }
            return;
        }
    };

    match result {
        Ok((diagnostics, tool, duration_ms)) => {
            let diag_json: Vec<serde_json::Value> = diagnostics
                .iter()
                .map(|d| {
                    json!({
                        "file": d.file,
                        "line": d.line,
                        "col": d.col,
                        "severity": d.severity,
                        "code": d.code,
                        "message": d.message,
                    })
                })
                .collect();

            let output = json!({
                "diagnostics": diag_json,
                "tool": tool,
                "duration_ms": duration_ms,
                "count": diagnostics.len(),
            });

            if format == "text" {
                if diagnostics.is_empty() {
                    println!("No lint issues found ({}, {}ms)", tool, duration_ms);
                } else {
                    for d in &diagnostics {
                        println!(
                            "{}:{}:{}: {} [{}] {}",
                            d.file, d.line, d.col, d.severity, d.code, d.message
                        );
                    }
                    println!(
                        "\n{} issue(s) found ({}, {}ms)",
                        diagnostics.len(),
                        tool,
                        duration_ms
                    );
                }
            } else {
                println!("{}", serde_json::to_string(&output).unwrap_or_default());
            }
        }
        Err(err_json) => {
            if format == "text" {
                if let Some(msg) = err_json.get("error").and_then(|v| v.as_str()) {
                    eprintln!("Error: {}", msg);
                    if let Some(install) = err_json.get("install").and_then(|v| v.as_str()) {
                        eprintln!("Install: {}", install);
                    }
                }
            } else {
                println!("{}", err_json);
            }
        }
    }
}

fn run_ruff_check(file: &str) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    if !tool_exists("ruff") {
        return Err(json!({
            "error": "ruff not installed",
            "install": "pip install ruff"
        }));
    }

    let start = Instant::now();
    let output = Command::new("ruff")
        .args(["check", "--output-format", "json", file])
        .output()
        .map_err(|e| json!({"error": format!("Failed to run ruff: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let diagnostics = parse_ruff_output(&stdout, file);
    Ok((diagnostics, "ruff", duration_ms))
}

fn parse_ruff_output(stdout: &str, default_file: &str) -> Vec<Diagnostic> {
    let parsed: Vec<serde_json::Value> = serde_json::from_str(stdout).unwrap_or_default();
    parsed
        .iter()
        .map(|item| Diagnostic {
            file: item
                .get("filename")
                .and_then(|v| v.as_str())
                .unwrap_or(default_file)
                .to_string(),
            line: item
                .get("location")
                .and_then(|l| l.get("row"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            col: item
                .get("location")
                .and_then(|l| l.get("column"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            severity: ruff_severity(
                item.get("code").and_then(|v| v.as_str()).unwrap_or(""),
            ),
            code: item
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            message: item
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .collect()
}

fn ruff_severity(code: &str) -> String {
    // E/W = error/warning style codes, F = pyflakes, etc.
    if code.starts_with('E') || code.starts_with('F') {
        "error".to_string()
    } else if code.starts_with('W') {
        "warning".to_string()
    } else {
        "warning".to_string()
    }
}

fn run_eslint(file: &str) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    if !tool_exists("eslint") {
        return Err(json!({
            "error": "eslint not installed",
            "install": "npm install -g eslint"
        }));
    }

    let start = Instant::now();
    let output = Command::new("eslint")
        .args(["-f", "json", file])
        .output()
        .map_err(|e| json!({"error": format!("Failed to run eslint: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let diagnostics = parse_eslint_output(&stdout, file);
    Ok((diagnostics, "eslint", duration_ms))
}

fn parse_eslint_output(stdout: &str, default_file: &str) -> Vec<Diagnostic> {
    let parsed: Vec<serde_json::Value> = serde_json::from_str(stdout).unwrap_or_default();
    let mut diagnostics = Vec::new();

    for file_result in &parsed {
        let file_path = file_result
            .get("filePath")
            .and_then(|v| v.as_str())
            .unwrap_or(default_file);
        if let Some(messages) = file_result.get("messages").and_then(|v| v.as_array()) {
            for msg in messages {
                diagnostics.push(Diagnostic {
                    file: file_path.to_string(),
                    line: msg.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
                    col: msg.get("column").and_then(|v| v.as_u64()).unwrap_or(0),
                    severity: match msg.get("severity").and_then(|v| v.as_u64()) {
                        Some(2) => "error".to_string(),
                        _ => "warning".to_string(),
                    },
                    code: msg
                        .get("ruleId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    message: msg
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
        }
    }

    diagnostics
}

fn run_clippy(path: &str) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    // Clippy is project-level. Find the Cargo.toml directory.
    let cargo_dir = find_cargo_dir(path).ok_or_else(|| {
        json!({
            "error": "No Cargo.toml found",
            "hint": "clippy requires a Cargo.toml in a parent directory"
        })
    })?;

    if !tool_exists("cargo") {
        return Err(json!({
            "error": "cargo not installed",
            "install": "curl https://sh.rustup.rs -sSf | sh"
        }));
    }

    let start = Instant::now();
    let output = Command::new("cargo")
        .args(["clippy", "--message-format=json", "--quiet"])
        .current_dir(&cargo_dir)
        .output()
        .map_err(|e| json!({"error": format!("Failed to run clippy: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let diagnostics = parse_cargo_messages(&stdout, path);
    Ok((diagnostics, "clippy", duration_ms))
}

fn run_golangci_lint(
    file: &str,
) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    if !tool_exists("golangci-lint") {
        return Err(json!({
            "error": "golangci-lint not installed",
            "install": "go install github.com/golangci/golangci-lint/cmd/golangci-lint@latest"
        }));
    }

    let start = Instant::now();
    let output = Command::new("golangci-lint")
        .args(["run", "--out-format", "json", file])
        .output()
        .map_err(|e| json!({"error": format!("Failed to run golangci-lint: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let diagnostics = parse_golangci_output(&stdout, file);
    Ok((diagnostics, "golangci-lint", duration_ms))
}

fn parse_golangci_output(stdout: &str, default_file: &str) -> Vec<Diagnostic> {
    let parsed: serde_json::Value = serde_json::from_str(stdout).unwrap_or_default();
    let mut diagnostics = Vec::new();

    if let Some(issues) = parsed.get("Issues").and_then(|v| v.as_array()) {
        for issue in issues {
            diagnostics.push(Diagnostic {
                file: issue
                    .get("Pos")
                    .and_then(|p| p.get("Filename"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(default_file)
                    .to_string(),
                line: issue
                    .get("Pos")
                    .and_then(|p| p.get("Line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                col: issue
                    .get("Pos")
                    .and_then(|p| p.get("Column"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                severity: issue
                    .get("Severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("warning")
                    .to_string(),
                code: issue
                    .get("FromLinter")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                message: issue
                    .get("Text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }

    diagnostics
}

/// Parse cargo/clippy JSON messages into normalized diagnostics.
fn parse_cargo_messages(stdout: &str, filter_file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in stdout.lines() {
        let msg: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Only process compiler messages
        if msg.get("reason").and_then(|v| v.as_str()) != Some("compiler-message") {
            continue;
        }

        if let Some(message) = msg.get("message") {
            let level = message
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("warning");

            // Skip notes and help
            if level == "note" || level == "help" {
                continue;
            }

            let code = message
                .get("code")
                .and_then(|c| c.get("code"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let text = message
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Get primary span
            if let Some(spans) = message.get("spans").and_then(|v| v.as_array()) {
                for span in spans {
                    let is_primary = span
                        .get("is_primary")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !is_primary {
                        continue;
                    }

                    let file = span
                        .get("file_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    // If we're targeting a specific file, filter
                    if !filter_file.ends_with('/') && !filter_file.eq(".") && !file.contains(filter_file.trim_start_matches("./")) {
                        continue;
                    }

                    diagnostics.push(Diagnostic {
                        file: file.to_string(),
                        line: span
                            .get("line_start")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        col: span
                            .get("column_start")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        severity: level.to_string(),
                        code: code.to_string(),
                        message: text.to_string(),
                    });
                }
            }
        }
    }

    diagnostics
}

/// Find the directory containing Cargo.toml, walking up from the given path.
fn find_cargo_dir(path: &str) -> Option<String> {
    let mut dir = std::path::Path::new(path);
    if dir.is_file() {
        dir = dir.parent()?;
    }
    for _ in 0..10 {
        if dir.join("Cargo.toml").exists() {
            return Some(dir.to_string_lossy().to_string());
        }
        dir = dir.parent()?;
    }
    None
}

fn tool_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ruff_output_empty() {
        let diags = parse_ruff_output("[]", "test.py");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_ruff_output_with_issues() {
        let ruff_json = r#"[
            {
                "code": "E302",
                "message": "expected 2 blank lines, got 1",
                "filename": "test.py",
                "location": {"row": 42, "column": 1}
            }
        ]"#;
        let diags = parse_ruff_output(ruff_json, "test.py");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].code, "E302");
        assert_eq!(diags[0].severity, "error");
    }

    #[test]
    fn test_parse_eslint_output_empty() {
        let diags = parse_eslint_output("[]", "test.js");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_eslint_output_with_issues() {
        let eslint_json = r#"[{
            "filePath": "test.js",
            "messages": [{
                "ruleId": "no-unused-vars",
                "severity": 2,
                "message": "'x' is assigned a value but never used.",
                "line": 10,
                "column": 5
            }]
        }]"#;
        let diags = parse_eslint_output(eslint_json, "test.js");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, "error");
        assert_eq!(diags[0].code, "no-unused-vars");
    }

    #[test]
    fn test_parse_golangci_output_empty() {
        let diags = parse_golangci_output("{}", "test.go");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_golangci_output_with_issues() {
        let golangci_json = r#"{"Issues": [{
            "FromLinter": "govet",
            "Text": "printf: non-constant format string",
            "Severity": "warning",
            "Pos": {"Filename": "main.go", "Line": 15, "Column": 3}
        }]}"#;
        let diags = parse_golangci_output(golangci_json, "main.go");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, "govet");
        assert_eq!(diags[0].line, 15);
    }

    #[test]
    fn test_parse_cargo_messages_empty() {
        let diags = parse_cargo_messages("", ".");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_ruff_severity_mapping() {
        assert_eq!(ruff_severity("E302"), "error");
        assert_eq!(ruff_severity("F401"), "error");
        assert_eq!(ruff_severity("W291"), "warning");
        assert_eq!(ruff_severity("D100"), "warning");
    }

    #[test]
    fn test_tool_detection() {
        // This just verifies the function doesn't panic
        let _ = tool_exists("nonexistent-tool-xyz");
    }

    #[test]
    fn test_find_cargo_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let result = find_cargo_dir(file.to_str().unwrap());
        assert!(result.is_some());
        assert!(result.unwrap().contains(dir.path().to_str().unwrap()));
    }

    #[test]
    fn test_unknown_language_json_output() {
        // Use a temp dir with no config files to ensure Unknown detection
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Makefile");
        std::fs::write(&file, "all:\n\techo hi").unwrap();
        let lang = detect_language(file.to_str().unwrap());
        assert_eq!(lang, Language::Unknown);
    }
}
