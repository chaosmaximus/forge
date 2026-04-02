use serde_json::json;
use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::hud_state;

pub fn run(state_dir: &str) {
    // 1. Cleanup stale agents and JSONL files
    cleanup_agents(state_dir);

    // 2. Read current HUD state
    let state = hud_state::read(state_dir);

    // 3. Build context output
    let version = state
        .version
        .as_deref()
        .unwrap_or(env!("CARGO_PKG_VERSION"));

    let mut context_parts: Vec<String> = vec![format!("[Forge v{}]", version)];

    // Security/skill warnings from HUD state
    if state.security.stale > 0 {
        context_parts.push(format!("WARNING: {} secrets need rotation.", state.security.stale));
    }
    if state.skills.fix_candidates > 0 {
        context_parts.push(format!("{} skill(s) need attention.", state.skills.fix_candidates));
    }

    context_parts.push(
        "Tools: forge_remember, forge_recall, forge_link, forge_decisions, forge_patterns, forge_timeline, forge_forget, forge_usage, forge_scan, forge_index, forge_cypher.".to_string()
    );

    let output = json!({
        "hookSpecificOutput": {
            "additionalContext": context_parts.join(" ")
        }
    });

    println!("{}", output);
}

/// Clean up stale agent state and old transcript files at session start.
///
/// 1. Clear `.team` in HUD state (agents from previous session are stale).
/// 2. Delete `.jsonl` files in `{state_dir}/agents/` older than 24 hours.
/// 3. If more than 100 `.jsonl` files remain, delete the oldest by mtime.
fn cleanup_agents(state_dir: &str) {
    // Clear stale agent entries from HUD state
    hud_state::update(state_dir, |state| {
        state.team.clear();
    });

    // Prune old JSONL transcript files
    let agents_dir = Path::new(state_dir).join("agents");
    if !agents_dir.is_dir() {
        return;
    }

    let entries = match std::fs::read_dir(&agents_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let cutoff = SystemTime::now() - Duration::from_secs(24 * 60 * 60);

    // Collect JSONL files with their modification times
    let mut jsonl_files: Vec<(std::path::PathBuf, SystemTime)> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }

        let mtime = match entry.metadata().and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // Delete files older than 24 hours
        if mtime < cutoff {
            let _ = std::fs::remove_file(&path);
            continue;
        }

        jsonl_files.push((path, mtime));
    }

    // Cap at 100 files — delete oldest if over limit
    if jsonl_files.len() > 100 {
        // Sort oldest first
        jsonl_files.sort_by_key(|(_, mtime)| *mtime);
        let to_delete = jsonl_files.len() - 100;
        for (path, _) in &jsonl_files[..to_delete] {
            let _ = std::fs::remove_file(path);
        }
    }
}
