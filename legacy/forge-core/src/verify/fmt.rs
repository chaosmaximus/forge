//! `forge fmt <file>` -- auto-detect language, format, report if changed.

use crate::verify::detect::{detect_language, Language};
use serde_json::json;
use std::process::Command;
use std::time::Instant;

pub fn run(path: &str, check_only: bool, format: &str) {
    let lang = detect_language(path);
    let result = match lang {
        Language::Python => run_ruff_format(path, check_only),
        Language::TypeScript | Language::JavaScript => run_prettier(path, check_only),
        Language::Rust => run_rustfmt(path, check_only),
        Language::Go => run_gofmt(path, check_only),
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
        Ok(fmt_result) => {
            if format == "text" {
                if fmt_result.changed {
                    if check_only {
                        println!("{} needs formatting ({})", fmt_result.file, fmt_result.tool);
                        if !fmt_result.diff.is_empty() {
                            println!("{}", fmt_result.diff);
                        }
                    } else {
                        println!("{} formatted ({})", fmt_result.file, fmt_result.tool);
                    }
                } else {
                    println!("{} already formatted ({})", fmt_result.file, fmt_result.tool);
                }
            } else {
                let mut output = json!({
                    "formatted": fmt_result.changed,
                    "file": fmt_result.file,
                    "tool": fmt_result.tool,
                    "duration_ms": fmt_result.duration_ms,
                    "check_only": check_only,
                });
                if !fmt_result.changed {
                    output["unchanged"] = json!(true);
                }
                if !fmt_result.diff.is_empty() {
                    output["diff"] = json!(fmt_result.diff);
                }
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

struct FmtResult {
    changed: bool,
    file: String,
    tool: &'static str,
    duration_ms: u128,
    diff: String,
}

fn run_ruff_format(file: &str, check_only: bool) -> Result<FmtResult, serde_json::Value> {
    if !tool_exists("ruff") {
        return Err(json!({
            "error": "ruff not installed",
            "install": "pip install ruff"
        }));
    }

    let start = Instant::now();

    if check_only {
        // --check returns non-zero if file needs formatting
        // --diff shows what would change
        let output = Command::new("ruff")
            .args(["format", "--check", "--diff", file])
            .output()
            .map_err(|e| json!({"error": format!("Failed to run ruff format: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();
        let needs_formatting = !output.status.success();
        let diff = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(FmtResult {
            changed: needs_formatting,
            file: file.to_string(),
            tool: "ruff",
            duration_ms,
            diff,
        })
    } else {
        // Read file content before formatting to detect changes
        let before = std::fs::read_to_string(file).unwrap_or_default();

        let output = Command::new("ruff")
            .args(["format", file])
            .output()
            .map_err(|e| json!({"error": format!("Failed to run ruff format: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(json!({"error": format!("ruff format failed: {}", stderr)}));
        }

        let after = std::fs::read_to_string(file).unwrap_or_default();
        let changed = before != after;

        Ok(FmtResult {
            changed,
            file: file.to_string(),
            tool: "ruff",
            duration_ms,
            diff: String::new(),
        })
    }
}

fn run_prettier(file: &str, check_only: bool) -> Result<FmtResult, serde_json::Value> {
    // Try npx prettier first, then global prettier
    let (cmd, args_prefix) = if tool_exists("prettier") {
        ("prettier", vec![])
    } else if tool_exists("npx") {
        ("npx", vec!["prettier"])
    } else {
        return Err(json!({
            "error": "prettier not installed",
            "install": "npm install -g prettier"
        }));
    };

    let start = Instant::now();

    if check_only {
        let mut args = args_prefix.clone();
        args.extend(["--check", file]);
        let output = Command::new(cmd)
            .args(&args)
            .output()
            .map_err(|e| json!({"error": format!("Failed to run prettier: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();
        let needs_formatting = !output.status.success();

        Ok(FmtResult {
            changed: needs_formatting,
            file: file.to_string(),
            tool: "prettier",
            duration_ms,
            diff: String::new(),
        })
    } else {
        let mut args = args_prefix;
        args.extend(["--write", file]);
        let before = std::fs::read_to_string(file).unwrap_or_default();

        let output = Command::new(cmd)
            .args(&args)
            .output()
            .map_err(|e| json!({"error": format!("Failed to run prettier: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(json!({"error": format!("prettier failed: {}", stderr)}));
        }

        let after = std::fs::read_to_string(file).unwrap_or_default();
        let changed = before != after;

        Ok(FmtResult {
            changed,
            file: file.to_string(),
            tool: "prettier",
            duration_ms,
            diff: String::new(),
        })
    }
}

fn run_rustfmt(file: &str, check_only: bool) -> Result<FmtResult, serde_json::Value> {
    if !tool_exists("rustfmt") {
        return Err(json!({
            "error": "rustfmt not installed",
            "install": "rustup component add rustfmt"
        }));
    }

    let start = Instant::now();

    if check_only {
        let output = Command::new("rustfmt")
            .args(["--check", file])
            .output()
            .map_err(|e| json!({"error": format!("Failed to run rustfmt: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();
        let needs_formatting = !output.status.success();
        let diff = String::from_utf8_lossy(&output.stdout).to_string();

        Ok(FmtResult {
            changed: needs_formatting,
            file: file.to_string(),
            tool: "rustfmt",
            duration_ms,
            diff,
        })
    } else {
        let output = Command::new("rustfmt")
            .arg(file)
            .output()
            .map_err(|e| json!({"error": format!("Failed to run rustfmt: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(json!({"error": format!("rustfmt failed: {}", stderr)}));
        }

        Ok(FmtResult {
            changed: true, // rustfmt doesn't easily tell us if it changed anything without --check
            file: file.to_string(),
            tool: "rustfmt",
            duration_ms,
            diff: String::new(),
        })
    }
}

fn run_gofmt(file: &str, check_only: bool) -> Result<FmtResult, serde_json::Value> {
    if !tool_exists("gofmt") {
        return Err(json!({
            "error": "gofmt not installed",
            "install": "Install Go from https://go.dev/dl/"
        }));
    }

    let start = Instant::now();

    if check_only {
        // gofmt -l lists files that differ from gofmt's formatting
        let output = Command::new("gofmt")
            .args(["-l", file])
            .output()
            .map_err(|e| json!({"error": format!("Failed to run gofmt: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let needs_formatting = !stdout.trim().is_empty();

        Ok(FmtResult {
            changed: needs_formatting,
            file: file.to_string(),
            tool: "gofmt",
            duration_ms,
            diff: String::new(),
        })
    } else {
        let output = Command::new("gofmt")
            .args(["-w", file])
            .output()
            .map_err(|e| json!({"error": format!("Failed to run gofmt: {}", e)}))?;

        let duration_ms = start.elapsed().as_millis();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(json!({"error": format!("gofmt failed: {}", stderr)}));
        }

        Ok(FmtResult {
            changed: true,
            file: file.to_string(),
            tool: "gofmt",
            duration_ms,
            diff: String::new(),
        })
    }
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
    fn test_unknown_language() {
        // Use a temp dir with no config files to ensure Unknown detection
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Makefile");
        std::fs::write(&file, "all:\n\techo hi").unwrap();
        let lang = detect_language(file.to_str().unwrap());
        assert_eq!(lang, Language::Unknown);
    }

    #[test]
    fn test_tool_exists_nonexistent() {
        assert!(!tool_exists("nonexistent-tool-xyz-12345"));
    }
}
