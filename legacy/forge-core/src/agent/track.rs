//! Agent track handler — appends tool-use events to JSONL, updates HUD state.

use crate::hud_state;
use super::validate;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Fast ISO UTC timestamp (same algorithm as start.rs).
fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days = secs / 86400;
    let day_secs = secs % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;

    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if mo <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", yr, mo, d, h, m, s)
}

pub fn run(state_dir: &str, agent_id: &str, agent_type: &str, tool: &str, file: Option<&str>) {
    // Validate tool name
    let safe_tool = if validate::valid_tool_name(tool) {
        tool.to_string()
    } else {
        return; // Invalid tool name — skip silently
    };

    // Extract safe basename from file
    let safe_file = file.map(|f| validate::safe_basename(f));

    let agents_dir = Path::new(state_dir).join("agents");
    let jsonl_path = agents_dir.join(format!("{}.jsonl", agent_id));

    // Ensure agents dir exists
    if !agents_dir.exists() {
        if fs::create_dir_all(&agents_dir).is_err() {
            return;
        }
        let _ = fs::set_permissions(&agents_dir, fs::Permissions::from_mode(0o700));
    }

    // Check JSONL is not symlink — delete if so
    if jsonl_path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&jsonl_path);
    }

    let ts = now_iso();

    // Build event
    let mut event = serde_json::json!({
        "event": "tool",
        "ts": ts,
        "tool": safe_tool,
    });
    if let Some(ref f) = safe_file {
        if !f.is_empty() {
            event["file"] = serde_json::Value::String(f.clone());
        }
    }

    // Append to JSONL — O_APPEND is atomic for small writes
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&jsonl_path)
    {
        Ok(mut f) => {
            let _ = fs::set_permissions(&jsonl_path, fs::Permissions::from_mode(0o600));
            let line = format!("{}\n", event);
            let _ = f.write_all(line.as_bytes());
        }
        Err(_) => return,
    }

    // Update HUD state under flock
    let safe_tool_owned = safe_tool;
    let safe_file_owned = safe_file;
    let agent_id_owned = agent_id.to_string();
    let agent_type_owned = agent_type.to_string();
    let ts_clone = ts;

    hud_state::update(state_dir, |state| {
        let entry = state
            .team
            .entry(agent_id_owned.clone())
            .or_insert_with(|| {
                // Orphaned track — start hasn't completed yet; create minimal entry
                hud_state::AgentEntry {
                    agent_type: Some(agent_type_owned.clone()),
                    status: Some("running".to_string()),
                    started_at: Some(ts_clone.clone()),
                    ..Default::default()
                }
            });

        entry.tool_calls += 1;
        entry.last_tool = Some(safe_tool_owned.clone());

        if let Some(ref f) = safe_file_owned {
            if !f.is_empty() {
                entry.current_file = Some(f.clone());
                // Dedup files list, max 100
                if !entry.files.contains(f) {
                    if entry.files.len() < 100 {
                        entry.files.push(f.clone());
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_track_appends_to_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // First start the agent
        super::super::start::run(state_dir, "track-test", "planner");

        // Track two tool uses
        run(state_dir, "track-test", "planner", "Read", Some("/src/main.rs"));
        run(state_dir, "track-test", "planner", "Edit", Some("/src/lib.rs"));

        // Check JSONL has 3 lines (start + 2 tracks)
        let jsonl_path = dir.path().join("agents").join("track-test.jsonl");
        let content = fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 3, "should have start + 2 tool events");

        // Parse the tool events
        let tool1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(tool1["event"], "tool");
        assert_eq!(tool1["tool"], "Read");
        assert_eq!(tool1["file"], "main.rs");

        let tool2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(tool2["tool"], "Edit");
        assert_eq!(tool2["file"], "lib.rs");
    }

    #[test]
    fn test_track_updates_hud() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        super::super::start::run(state_dir, "hud-track", "generator");
        run(state_dir, "hud-track", "generator", "Read", Some("/a/b.rs"));
        run(state_dir, "hud-track", "generator", "Edit", Some("/a/c.rs"));
        run(state_dir, "hud-track", "generator", "Read", Some("/a/b.rs")); // dup file

        let state = hud_state::read(state_dir);
        let agent = state.team.get("hud-track").unwrap();
        assert_eq!(agent.tool_calls, 3);
        assert_eq!(agent.last_tool, Some("Read".to_string()));
        assert_eq!(agent.current_file, Some("b.rs".to_string()));
        // files should be deduped: b.rs, c.rs
        assert_eq!(agent.files.len(), 2);
    }

    #[test]
    fn test_track_invalid_tool_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        super::super::start::run(state_dir, "invalid-tool", "planner");
        run(state_dir, "invalid-tool", "planner", "bad tool name", None);

        // JSONL should only have start event
        let jsonl_path = dir.path().join("agents").join("invalid-tool.jsonl");
        let content = fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn test_track_orphan_creates_minimal_entry() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // Track without a prior start
        run(state_dir, "orphan-agent", "evaluator", "Read", Some("/foo.rs"));

        let state = hud_state::read(state_dir);
        let agent = state.team.get("orphan-agent").unwrap();
        assert_eq!(agent.status, Some("running".to_string()));
        assert_eq!(agent.tool_calls, 1);
    }
}
