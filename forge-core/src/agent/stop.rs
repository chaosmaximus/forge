//! Agent stop handler — records stop event, updates HUD state.

use crate::hud_state;
use super::validate;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Fast ISO UTC timestamp (same algorithm as start.rs / track.rs).
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

pub fn run(
    state_dir: &str,
    agent_id: &str,
    agent_type: &str,
    transcript_path: Option<&str>,
    last_message: Option<&str>,
) {
    // Validate transcript path
    let safe_transcript = transcript_path.and_then(|p| validate::safe_transcript_path(p));

    // Sanitize last message
    let safe_msg = last_message.map(|m| validate::safe_message(m, 200));

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

    // Build stop event
    let mut event = serde_json::json!({
        "event": "stop",
        "ts": ts,
        "type": agent_type,
    });
    if let Some(ref t) = safe_transcript {
        event["transcript"] = serde_json::Value::String(t.clone());
    }
    if let Some(ref m) = safe_msg {
        if !m.is_empty() {
            event["message"] = serde_json::Value::String(m.clone());
        }
    }

    // Append to JSONL
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

    // Update HUD state
    let ts_clone = ts;
    let agent_id_owned = agent_id.to_string();
    let agent_type_owned = agent_type.to_string();
    let safe_transcript_owned = safe_transcript;

    hud_state::update(state_dir, |state| {
        let entry = state
            .team
            .entry(agent_id_owned.clone())
            .or_insert_with(|| {
                // Orphaned stop — no matching start; create minimal entry
                hud_state::AgentEntry {
                    agent_type: Some(agent_type_owned.clone()),
                    started_at: Some("unknown".to_string()),
                    ..Default::default()
                }
            });

        entry.status = Some("done".to_string());
        entry.ended_at = Some(ts_clone.clone());
        entry.last_tool = None;
        entry.current_file = None;
        if let Some(ref t) = safe_transcript_owned {
            entry.transcript_path = Some(t.clone());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stop_appends_event_and_updates_hud() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // Start then stop
        super::super::start::run(state_dir, "stop-test", "evaluator");
        run(state_dir, "stop-test", "evaluator", None, Some("All done"));

        // Check JSONL
        let jsonl_path = dir.path().join("agents").join("stop-test.jsonl");
        let content = fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 2, "should have start + stop");

        let stop_event: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(stop_event["event"], "stop");
        assert_eq!(stop_event["type"], "evaluator");
        assert_eq!(stop_event["message"], "All done");

        // Check HUD
        let state = hud_state::read(state_dir);
        let agent = state.team.get("stop-test").unwrap();
        assert_eq!(agent.status, Some("done".to_string()));
        assert!(agent.ended_at.is_some());
        assert!(agent.last_tool.is_none());
        assert!(agent.current_file.is_none());
    }

    #[test]
    fn test_stop_sanitizes_message() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        super::super::start::run(state_dir, "sanitize-test", "planner");
        run(
            state_dir,
            "sanitize-test",
            "planner",
            None,
            Some("msg\x00with\x01control\nchars"),
        );

        let jsonl_path = dir.path().join("agents").join("sanitize-test.jsonl");
        let content = fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        let stop_event: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(stop_event["message"], "msgwithcontrolchars");
    }

    #[test]
    fn test_stop_orphan_creates_entry() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // Stop without start
        run(state_dir, "orphan-stop", "generator", None, None);

        let state = hud_state::read(state_dir);
        let agent = state.team.get("orphan-stop").unwrap();
        assert_eq!(agent.status, Some("done".to_string()));
        assert_eq!(agent.started_at, Some("unknown".to_string()));
    }

    #[test]
    fn test_stop_message_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        let long_msg = "x".repeat(500);
        super::super::start::run(state_dir, "truncate-test", "planner");
        run(
            state_dir,
            "truncate-test",
            "planner",
            None,
            Some(&long_msg),
        );

        let jsonl_path = dir.path().join("agents").join("truncate-test.jsonl");
        let content = fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        let stop_event: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        let msg = stop_event["message"].as_str().unwrap();
        assert_eq!(msg.len(), 200);
    }

    #[test]
    fn test_full_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // Start
        super::super::start::run(state_dir, "lifecycle", "generator");

        // Track several tool calls
        super::super::track::run(state_dir, "lifecycle", "generator", "Read", Some("/a/b.rs"));
        super::super::track::run(state_dir, "lifecycle", "generator", "Edit", Some("/a/c.rs"));
        super::super::track::run(state_dir, "lifecycle", "generator", "Bash", None);

        // Stop
        run(state_dir, "lifecycle", "generator", None, Some("Build complete"));

        // Verify JSONL: 1 start + 3 track + 1 stop = 5 lines
        let jsonl_path = dir.path().join("agents").join("lifecycle.jsonl");
        let content = fs::read_to_string(&jsonl_path).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 5, "lifecycle should have 5 events");

        // Verify HUD
        let state = hud_state::read(state_dir);
        let agent = state.team.get("lifecycle").unwrap();
        assert_eq!(agent.status, Some("done".to_string()));
        assert_eq!(agent.agent_type, Some("generator".to_string()));
        assert_eq!(agent.tool_calls, 3);
        assert_eq!(agent.files.len(), 2); // b.rs, c.rs
        assert!(agent.last_tool.is_none()); // cleared by stop
        assert!(agent.current_file.is_none()); // cleared by stop
        assert!(agent.ended_at.is_some());
    }
}
