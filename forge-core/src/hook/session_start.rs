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

    // Build XML context from memory cache
    let memory_xml = read_memory_context(state_dir, version);
    let mut context_parts: Vec<String> = vec![memory_xml];

    // Warnings
    if state.security.stale > 0 {
        context_parts.push(format!("WARNING: {} secrets need rotation.", state.security.stale));
    }

    context_parts.push(
        "CLI: forge remember, forge recall, forge index, forge scan, forge doctor, forge query, forge health, forge sync.".to_string()
    );

    let output = json!({
        "hookSpecificOutput": {
            "additionalContext": context_parts.join(" ")
        }
    });

    println!("{}", output);
}

/// Read memory cache and format as XML context for Claude.
fn read_memory_context(state_dir: &str, version: &str) -> String {
    let cache_path = Path::new(state_dir).join("memory").join("cache.json");
    let cache: serde_json::Value = match std::fs::read_to_string(&cache_path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or(json!({"entries": []})),
        Err(_) => return format!("<forge-context version=\"{}\"/>", version),
    };

    let entries = match cache.get("entries").and_then(|e| e.as_array()) {
        Some(e) => e,
        None => return format!("<forge-context version=\"{}\"/>", version),
    };

    // Collect active entries by type
    let mut decisions = Vec::new();
    let mut lessons = Vec::new();
    let mut patterns = Vec::new();

    for entry in entries {
        if entry.get("status").and_then(|v| v.as_str()) != Some("active") {
            continue;
        }
        let title = xml_escape(entry.get("title").and_then(|v| v.as_str()).unwrap_or(""));
        let content = xml_escape(entry.get("content").and_then(|v| v.as_str()).unwrap_or(""));
        let confidence = entry.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.5);

        match entry.get("type").and_then(|v| v.as_str()) {
            Some("decision") => decisions.push(format!(
                "    <decision title=\"{}\" confidence=\"{:.2}\">{}</decision>", title, confidence, content
            )),
            Some("lesson") => lessons.push(format!(
                "    <lesson>{}: {}</lesson>", title, content
            )),
            Some("pattern") => patterns.push(format!(
                "    <pattern name=\"{}\" confidence=\"{:.2}\">{}</pattern>", title, confidence, content
            )),
            _ => {}
        }
    }

    let mut xml = vec![format!("<forge-context version=\"{}\">", version)];
    if !decisions.is_empty() {
        xml.push(format!("  <decisions count=\"{}\">", decisions.len()));
        xml.extend(decisions.iter().take(10).cloned()); // Max 10
        xml.push("  </decisions>".to_string());
    }
    if !lessons.is_empty() {
        xml.push(format!("  <lessons count=\"{}\">", lessons.len()));
        xml.extend(lessons.iter().take(5).cloned()); // Max 5
        xml.push("  </lessons>".to_string());
    }
    if !patterns.is_empty() {
        xml.push(format!("  <patterns count=\"{}\">", patterns.len()));
        xml.extend(patterns.iter().take(5).cloned()); // Max 5
        xml.push("  </patterns>".to_string());
    }
    xml.push("</forge-context>".to_string());
    xml.join("\n")
}

/// Escape XML special characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
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
