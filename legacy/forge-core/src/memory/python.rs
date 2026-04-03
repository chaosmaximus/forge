//! Shared Python subprocess caller for graph operations.
//!
//! Calls `python3 -m forge_graph.cli --db <path> <subcommand> [args]`
//! Each invocation opens the DB, operates, closes. No persistent process.

use std::path::Path;
use std::process::Command;

/// Find the Python interpreter — prefer the plugin venv, fallback to system.
fn find_python(state_dir: &str) -> String {
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();

    let venv_candidates = [
        // Via CLAUDE_PLUGIN_ROOT (set by Claude Code for hooks)
        format!("{}/forge-graph/.venv/bin/python", plugin_root),
        // Via state_dir (CLAUDE_PLUGIN_DATA) — go up to plugin root
        format!("{}/../../forge-graph/.venv/bin/python", state_dir),
        // Auto-detect from known cache paths
        format!("{}/.claude/plugins/cache/forge-marketplace/forge/0.3.0/forge-graph/.venv/bin/python", home),
        format!("{}/.claude/plugins/cache/forge-marketplace/forge/0.2.0/forge-graph/.venv/bin/python", home),
        // Development: repo root
        "forge-graph/.venv/bin/python".to_string(),
    ];
    for candidate in &venv_candidates {
        if !candidate.is_empty() && Path::new(candidate).exists() {
            return candidate.clone();
        }
    }
    "python3".to_string()
}

/// Find PYTHONPATH for forge_graph module.
fn find_pythonpath(state_dir: &str) -> String {
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();

    let candidates = [
        format!("{}/forge-graph/src", plugin_root),
        format!("{}/../../forge-graph/src", state_dir),
        format!("{}/.claude/plugins/cache/forge-marketplace/forge/0.3.0/forge-graph/src", home),
        format!("{}/.claude/plugins/cache/forge-marketplace/forge/0.2.0/forge-graph/src", home),
        "forge-graph/src".to_string(),
    ];
    for candidate in &candidates {
        if !candidate.is_empty() && Path::new(candidate).join("forge_graph").is_dir() {
            return candidate.clone();
        }
    }
    String::new()
}

/// Call a forge_graph.cli subcommand and return stdout as JSON string.
pub fn call_graph(state_dir: &str, args: &[&str]) -> Result<String, String> {
    let db_path = format!("{}/graph/forge.lbdb", state_dir);
    let python = find_python(state_dir);
    let pythonpath = find_pythonpath(state_dir);

    let mut cmd = Command::new(&python);
    cmd.args(["-m", "forge_graph.cli", "--db", &db_path]);
    cmd.args(args);

    if !pythonpath.is_empty() {
        cmd.env("PYTHONPATH", &pythonpath);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Python not available: {}", e))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("Python error: {}", stderr))
    }
}
