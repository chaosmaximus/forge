//! Shared Python subprocess caller for graph operations.
//!
//! Calls `python3 -m forge_graph.cli --db <path> <subcommand> [args]`
//! Each invocation opens the DB, operates, closes. No persistent process.

use std::path::Path;
use std::process::Command;

/// Find the Python interpreter — prefer the plugin venv, fallback to system.
fn find_python(state_dir: &str) -> String {
    // Try plugin venv
    let venv_candidates = [
        // Plugin cache venv
        format!(
            "{}/../forge-graph/.venv/bin/python",
            std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default()
        ),
        // Development venv
        format!("{}/../../forge-graph/.venv/bin/python", state_dir),
    ];
    for candidate in &venv_candidates {
        if Path::new(candidate).exists() {
            return candidate.clone();
        }
    }
    "python3".to_string()
}

/// Find PYTHONPATH for forge_graph module.
fn find_pythonpath(state_dir: &str) -> String {
    let candidates = [
        format!(
            "{}/forge-graph/src",
            std::env::var("CLAUDE_PLUGIN_ROOT").unwrap_or_default()
        ),
        format!("{}/../../forge-graph/src", state_dir),
    ];
    for candidate in &candidates {
        if Path::new(candidate).join("forge_graph").is_dir() {
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
