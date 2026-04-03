//! `forge check <file>` -- type-checking via mypy, tsc, cargo check, go vet.

use crate::verify::detect::{detect_language, detect_project_language, Language};
use serde_json::json;
use std::process::Command;
use std::time::Instant;

/// Normalized type-check diagnostic (same schema as lint).
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
    // For directories, use project detection; for files, use file extension
    let lang = if std::path::Path::new(path).is_dir() {
        detect_project_language(path)
    } else {
        detect_language(path)
    };

    let result = match lang {
        Language::Python => run_mypy(path),
        Language::TypeScript => run_tsc(path),
        Language::JavaScript => {
            let output = json!({
                "diagnostics": [],
                "tool": "none",
                "duration_ms": 0,
                "count": 0,
                "note": "JavaScript has no built-in type checker. Use TypeScript or add JSDoc types with tsconfig checkJs."
            });
            if format == "text" {
                println!("JavaScript has no built-in type checker.");
                println!("Hint: Use TypeScript, or enable checkJs in tsconfig.json for JSDoc-based checking.");
            } else {
                println!("{}", serde_json::to_string(&output).unwrap_or_default());
            }
            return;
        }
        Language::Rust => run_cargo_check(path),
        Language::Go => run_go_vet(path),
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
                    println!("No type errors found ({}, {}ms)", tool, duration_ms);
                } else {
                    for d in &diagnostics {
                        println!(
                            "{}:{}:{}: {} [{}] {}",
                            d.file, d.line, d.col, d.severity, d.code, d.message
                        );
                    }
                    println!(
                        "\n{} type error(s) found ({}, {}ms)",
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

fn run_mypy(file: &str) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    if !tool_exists("mypy") {
        return Err(json!({
            "error": "mypy not installed",
            "install": "pip install mypy"
        }));
    }

    let start = Instant::now();
    let output = Command::new("mypy")
        .args(["--output", "json", "--no-error-summary", file])
        .output()
        .map_err(|e| json!({"error": format!("Failed to run mypy: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let diagnostics = parse_mypy_output(&stdout, file);
    Ok((diagnostics, "mypy", duration_ms))
}

fn parse_mypy_output(stdout: &str, default_file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try JSON format first (mypy --output json)
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            diagnostics.push(Diagnostic {
                file: v
                    .get("file")
                    .and_then(|v| v.as_str())
                    .unwrap_or(default_file)
                    .to_string(),
                line: v.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
                col: v.get("column").and_then(|v| v.as_u64()).unwrap_or(0),
                severity: v
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("error")
                    .to_string(),
                code: v
                    .get("code")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                message: v
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
            continue;
        }

        // Fallback: parse classic mypy format "file.py:line:col: severity: message [code]"
        if let Some(diag) = parse_mypy_classic_line(line, default_file) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

fn parse_mypy_classic_line(line: &str, default_file: &str) -> Option<Diagnostic> {
    // Format: file.py:10:5: error: Message text [error-code]
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let file = parts[0].trim();
    let line_num = parts[1].trim().parse::<u64>().unwrap_or(0);
    let col_or_severity = parts[2].trim();

    // Could be col:severity:message or just severity:message
    let (col, rest) = if let Ok(c) = col_or_severity.parse::<u64>() {
        // parts[3] should be " severity: message"
        (c, parts[3].trim().to_string())
    } else {
        // col_or_severity is actually severity
        (0, format!("{}: {}", col_or_severity, parts[3]))
    };

    let (severity, message) = if let Some(pos) = rest.find(':') {
        let sev = rest[..pos].trim().to_lowercase();
        let msg = rest[pos + 1..].trim().to_string();
        (sev, msg)
    } else {
        ("error".to_string(), rest)
    };

    // Extract code from [code] at end of message
    let (msg, code) = if let Some(bracket_start) = message.rfind('[') {
        if message.ends_with(']') {
            let code = &message[bracket_start + 1..message.len() - 1];
            let msg = message[..bracket_start].trim().to_string();
            (msg, code.to_string())
        } else {
            (message, String::new())
        }
    } else {
        (message, String::new())
    };

    Some(Diagnostic {
        file: if file.is_empty() {
            default_file.to_string()
        } else {
            file.to_string()
        },
        line: line_num,
        col,
        severity,
        code,
        message: msg,
    })
}

fn run_tsc(path: &str) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    if !tool_exists("npx") {
        return Err(json!({
            "error": "npx not installed (Node.js required)",
            "install": "Install Node.js from https://nodejs.org/"
        }));
    }

    // tsc is project-level, find the tsconfig.json directory
    let project_dir = find_tsconfig_dir(path).unwrap_or_else(|| ".".to_string());

    let start = Instant::now();
    let output = Command::new("npx")
        .args(["tsc", "--noEmit", "--pretty", "false"])
        .current_dir(&project_dir)
        .output()
        .map_err(|e| json!({"error": format!("Failed to run tsc: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let diagnostics = parse_tsc_output(&stdout, path);
    Ok((diagnostics, "tsc", duration_ms))
}

fn parse_tsc_output(stdout: &str, filter_file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // tsc format: file.ts(10,5): error TS2322: message
        if let Some(diag) = parse_tsc_line(line) {
            // Filter to target file if specific
            if filter_file != "." && !diag.file.contains(filter_file.trim_start_matches("./")) {
                continue;
            }
            diagnostics.push(diag);
        }
    }

    diagnostics
}

fn parse_tsc_line(line: &str) -> Option<Diagnostic> {
    // Format: file.ts(line,col): error TScode: message
    let paren_start = line.find('(')?;
    let paren_end = line.find(')')?;
    if paren_start >= paren_end {
        return None;
    }

    let file = &line[..paren_start];
    let loc = &line[paren_start + 1..paren_end];
    let rest = &line[paren_end + 1..];

    let loc_parts: Vec<&str> = loc.split(',').collect();
    let line_num = loc_parts.first().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let col = loc_parts.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

    // rest should be ": error TS2322: message" or ": warning TS...: message"
    let rest = rest.trim_start_matches(':').trim();
    let (severity, rest) = if rest.starts_with("error") {
        ("error", rest.trim_start_matches("error").trim())
    } else if rest.starts_with("warning") {
        ("warning", rest.trim_start_matches("warning").trim())
    } else {
        ("error", rest)
    };

    // Code is "TSxxxx:"
    let (code, message) = if let Some(colon_pos) = rest.find(':') {
        let code = rest[..colon_pos].trim();
        let msg = rest[colon_pos + 1..].trim();
        (code, msg)
    } else {
        ("", rest)
    };

    Some(Diagnostic {
        file: file.to_string(),
        line: line_num,
        col,
        severity: severity.to_string(),
        code: code.to_string(),
        message: message.to_string(),
    })
}

fn run_cargo_check(
    path: &str,
) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    let cargo_dir = find_cargo_dir(path).ok_or_else(|| {
        json!({
            "error": "No Cargo.toml found",
            "hint": "cargo check requires a Cargo.toml in a parent directory"
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
        .args(["check", "--message-format=json", "--quiet"])
        .current_dir(&cargo_dir)
        .output()
        .map_err(|e| json!({"error": format!("Failed to run cargo check: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stdout = String::from_utf8_lossy(&output.stdout);

    let diagnostics = parse_cargo_messages(&stdout, path);
    Ok((diagnostics, "cargo-check", duration_ms))
}

fn run_go_vet(path: &str) -> Result<(Vec<Diagnostic>, &'static str, u128), serde_json::Value> {
    if !tool_exists("go") {
        return Err(json!({
            "error": "go not installed",
            "install": "Install Go from https://go.dev/dl/"
        }));
    }

    let work_dir = if std::path::Path::new(path).is_dir() {
        path.to_string()
    } else {
        std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string())
    };

    let start = Instant::now();
    let output = Command::new("go")
        .args(["vet", "-json", "./..."])
        .current_dir(&work_dir)
        .output()
        .map_err(|e| json!({"error": format!("Failed to run go vet: {}", e)}))?;

    let duration_ms = start.elapsed().as_millis();
    let stderr = String::from_utf8_lossy(&output.stderr);

    let diagnostics = parse_go_vet_output(&stderr, path);
    Ok((diagnostics, "go-vet", duration_ms))
}

fn parse_go_vet_output(stderr: &str, filter_file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // go vet -json outputs JSON objects on stderr
    // Also handle classic format: file.go:line:col: message
    for line in stderr.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Try JSON
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(file) = v.get("posn").and_then(|p| p.as_str()) {
                // posn format: "file.go:10:5"
                let parts: Vec<&str> = file.splitn(3, ':').collect();
                let fname = parts.first().copied().unwrap_or("");
                let line_num = parts.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                let col = parts.get(2).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);

                if !filter_file.ends_with('/') && filter_file != "." && !fname.contains(filter_file.trim_start_matches("./")) {
                    continue;
                }

                diagnostics.push(Diagnostic {
                    file: fname.to_string(),
                    line: line_num,
                    col,
                    severity: "warning".to_string(),
                    code: "vet".to_string(),
                    message: v
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string(),
                });
            }
            continue;
        }

        // Classic format: file.go:line:col: message
        if let Some(diag) = parse_go_classic_line(line, filter_file) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

fn parse_go_classic_line(line: &str, filter_file: &str) -> Option<Diagnostic> {
    // Format: ./file.go:10:5: message
    let parts: Vec<&str> = line.splitn(4, ':').collect();
    if parts.len() < 4 {
        return None;
    }

    let file = parts[0].trim();
    let line_num = parts[1].trim().parse::<u64>().ok()?;
    let col = parts[2].trim().parse::<u64>().unwrap_or(0);
    let message = parts[3].trim().to_string();

    if filter_file != "." && !file.contains(filter_file.trim_start_matches("./")) {
        return None;
    }

    Some(Diagnostic {
        file: file.to_string(),
        line: line_num,
        col,
        severity: "warning".to_string(),
        code: "vet".to_string(),
        message,
    })
}

/// Parse cargo JSON messages into normalized diagnostics.
fn parse_cargo_messages(stdout: &str, filter_file: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for line in stdout.lines() {
        let msg: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if msg.get("reason").and_then(|v| v.as_str()) != Some("compiler-message") {
            continue;
        }

        if let Some(message) = msg.get("message") {
            let level = message
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("warning");

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

                    if !filter_file.ends_with('/') && filter_file != "." && !file.contains(filter_file.trim_start_matches("./")) {
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

fn find_tsconfig_dir(path: &str) -> Option<String> {
    let mut dir = std::path::Path::new(path);
    if dir.is_file() {
        dir = dir.parent()?;
    }
    for _ in 0..10 {
        if dir.join("tsconfig.json").exists() {
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
    fn test_parse_mypy_classic_line() {
        let diag = parse_mypy_classic_line(
            "test.py:10:5: error: Incompatible types [assignment]",
            "test.py",
        );
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.file, "test.py");
        assert_eq!(d.line, 10);
        assert_eq!(d.col, 5);
        assert_eq!(d.severity, "error");
        assert_eq!(d.code, "assignment");
        assert_eq!(d.message, "Incompatible types");
    }

    #[test]
    fn test_parse_mypy_json() {
        let json_line = r#"{"file": "test.py", "line": 42, "column": 10, "severity": "error", "code": "attr-defined", "message": "Module has no attribute 'foo'"}"#;
        let diags = parse_mypy_output(json_line, "test.py");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].code, "attr-defined");
    }

    #[test]
    fn test_parse_tsc_line() {
        let diag = parse_tsc_line("src/app.ts(10,5): error TS2322: Type 'string' is not assignable to type 'number'.");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.file, "src/app.ts");
        assert_eq!(d.line, 10);
        assert_eq!(d.col, 5);
        assert_eq!(d.severity, "error");
        assert_eq!(d.code, "TS2322");
        assert_eq!(
            d.message,
            "Type 'string' is not assignable to type 'number'."
        );
    }

    #[test]
    fn test_parse_tsc_line_no_match() {
        let diag = parse_tsc_line("Found 3 errors.");
        assert!(diag.is_none());
    }

    #[test]
    fn test_parse_go_classic_line() {
        let diag = parse_go_classic_line("./main.go:15:3: printf: non-constant format string", ".");
        assert!(diag.is_some());
        let d = diag.unwrap();
        assert_eq!(d.file, "./main.go");
        assert_eq!(d.line, 15);
        assert_eq!(d.col, 3);
    }

    #[test]
    fn test_parse_go_vet_output_empty() {
        let diags = parse_go_vet_output("", ".");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_parse_cargo_messages_empty() {
        let diags = parse_cargo_messages("", ".");
        assert!(diags.is_empty());
    }

    #[test]
    fn test_find_cargo_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("lib.rs");
        std::fs::write(&file, "").unwrap();

        let result = find_cargo_dir(file.to_str().unwrap());
        assert!(result.is_some());
    }

    #[test]
    fn test_find_tsconfig_dir_exists() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir_all(&sub).unwrap();
        let file = sub.join("app.ts");
        std::fs::write(&file, "").unwrap();

        let result = find_tsconfig_dir(file.to_str().unwrap());
        assert!(result.is_some());
    }

    #[test]
    fn test_tool_exists_nonexistent() {
        assert!(!tool_exists("nonexistent-tool-xyz-12345"));
    }
}
