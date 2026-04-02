//! `forge remember` — append a memory entry to pending.jsonl.
//!
//! The MCP server picks these up on startup and writes them to the graph.
//! This avoids DB lock issues and keeps the CLI fast (<5ms).

use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::hud_state;

/// Append a memory entry to pending.jsonl, update HUD, optionally sync to graph.
pub fn run(state_dir: &str, mem_type: &str, title: &str, content: &str, confidence: f64, sync: bool) {
    let memory_dir = Path::new(state_dir).join("memory");

    // Create directory with 0o700
    if fs::create_dir_all(&memory_dir).is_err() {
        eprintln!("{{\"error\":\"Cannot create memory directory\"}}");
        return;
    }
    let _ = fs::set_permissions(&memory_dir, fs::Permissions::from_mode(0o700));

    // Validate type
    let valid_types = ["decision", "pattern", "lesson", "preference"];
    if !valid_types.contains(&mem_type) {
        eprintln!("{{\"error\":\"Invalid type. Must be one of: decision, pattern, lesson, preference\"}}");
        return;
    }

    // Generate ID
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let id = format!("{}-{}", mem_type, ts);

    // Build the entry
    let entry = json!({
        "id": id,
        "type": mem_type,
        "title": title,
        "content": content,
        "confidence": confidence,
        "status": "active",
        "timestamp": now_iso(),
        "synced": false
    });

    // Append to pending.jsonl (O_APPEND is atomic for small writes)
    let pending_path = memory_dir.join("pending.jsonl");
    match OpenOptions::new().create(true).append(true).open(&pending_path) {
        Ok(mut f) => {
            let _ = fs::set_permissions(&pending_path, fs::Permissions::from_mode(0o600));
            if writeln!(f, "{}", entry).is_err() {
                eprintln!("{{\"error\":\"Failed to write to pending.jsonl\"}}");
                return;
            }
        }
        Err(_) => {
            eprintln!("{{\"error\":\"Cannot open pending.jsonl\"}}");
            return;
        }
    }

    // Also write to the cache (readable by recall without DB)
    update_memory_cache(state_dir, mem_type, &entry);

    // Update HUD memory counts
    hud_state::update(state_dir, |state| {
        match mem_type {
            "decision" => state.memory.decisions += 1,
            "pattern" => state.memory.patterns += 1,
            "lesson" => state.memory.lessons += 1,
            _ => {}
        }
    });

    // Sync to graph DB if requested
    let was_synced = if sync {
        match crate::memory::python::call_graph(state_dir, &[
            "remember",
            "--type", mem_type,
            "--data", &serde_json::to_string(&json!({
                "id": id,
                "title": title,
                "content": content,
                "confidence": confidence,
                "status": "active"
            })).unwrap_or_default(),
        ]) {
            Ok(_) => true,
            Err(_) => false, // Cache still has it; will sync later
        }
    } else {
        false
    };

    // Output success
    println!("{}", json!({
        "status": "stored",
        "id": id,
        "type": mem_type,
        "synced": was_synced
    }));
}

/// Update the memory cache file with a new entry.
fn update_memory_cache(state_dir: &str, _mem_type: &str, entry: &serde_json::Value) {
    let cache_path = Path::new(state_dir).join("memory").join("cache.json");

    // Read existing cache
    let mut cache: serde_json::Value = fs::read_to_string(&cache_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({"entries": []}));

    // Append entry
    if let Some(entries) = cache.get_mut("entries").and_then(|e| e.as_array_mut()) {
        entries.push(entry.clone());
        // Cap at 500 entries
        if entries.len() > 500 {
            *entries = entries.split_off(entries.len() - 500);
        }
    }

    // Atomic write
    let tmp = cache_path.with_extension("tmp");
    if let Ok(mut f) = File::create(&tmp) {
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        if f.write_all(serde_json::to_string(&cache).unwrap_or_default().as_bytes()).is_ok() {
            let _ = fs::rename(&tmp, &cache_path);
        }
    }
}

fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let day_secs = secs % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;
    format!("T{:02}:{:02}:{:02}Z", h, m, s)
}
