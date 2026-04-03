//! Agent start handler — creates JSONL log and updates HUD state.

use crate::hud_state;
use super::validate;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Fast ISO-ish UTC timestamp without spawning a process or pulling in chrono.
fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Days since epoch → date (simplified: handles leap years correctly via algorithm)
    let days = secs / 86400;
    let day_secs = secs % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;

    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm)
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

pub fn run(state_dir: &str, agent_id: &str, agent_type: &str) {
    let agents_dir = Path::new(state_dir).join("agents");

    // Create agents dir with 0o700
    if !agents_dir.exists() {
        if fs::create_dir_all(&agents_dir).is_err() {
            return;
        }
        let _ = fs::set_permissions(&agents_dir, fs::Permissions::from_mode(0o700));
    }

    // Validate the agents dir
    if !validate::safe_dir(&agents_dir, Path::new(state_dir)) {
        return;
    }

    let jsonl_path = agents_dir.join(format!("{}.jsonl", agent_id));

    // Check not symlink — delete if so
    if jsonl_path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        let _ = fs::remove_file(&jsonl_path);
    }

    let ts = now_iso();

    // Write first line to JSONL
    let start_event = serde_json::json!({
        "event": "start",
        "ts": ts,
        "type": agent_type,
    });

    match File::create(&jsonl_path) {
        Ok(mut f) => {
            let _ = fs::set_permissions(&jsonl_path, fs::Permissions::from_mode(0o600));
            let line = format!("{}\n", start_event);
            let _ = f.write_all(line.as_bytes());
        }
        Err(_) => return,
    }

    // Update HUD state
    let ts_clone = ts.clone();
    let agent_type_owned = agent_type.to_string();
    let agent_id_owned = agent_id.to_string();

    hud_state::update(state_dir, |state| {
        // Enforce max 20 team entries — evict oldest "done" agents first
        while state.team.len() >= 20 {
            // Try to remove a "done" agent
            let done_key = state
                .team
                .iter()
                .find(|(_, v)| v.status.as_deref() == Some("done"))
                .map(|(k, _)| k.clone());
            if let Some(key) = done_key {
                state.team.remove(&key);
            } else {
                // Remove any agent if no "done" agents exist
                let any_key = state.team.keys().next().cloned();
                if let Some(key) = any_key {
                    state.team.remove(&key);
                } else {
                    break;
                }
            }
        }

        state.team.insert(
            agent_id_owned,
            hud_state::AgentEntry {
                agent_type: Some(agent_type_owned),
                status: Some("running".to_string()),
                started_at: Some(ts_clone),
                tool_calls: 0,
                ..Default::default()
            },
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_iso_format() {
        let ts = now_iso();
        // Should match YYYY-MM-DDTHH:MM:SSZ
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {}", ts);
        assert!(ts.contains('T'), "timestamp should contain T: {}", ts);
        assert_eq!(ts.len(), 20, "timestamp should be 20 chars: {}", ts);
    }

    #[test]
    fn test_start_creates_jsonl_and_hud() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        run(state_dir, "test-agent-1", "planner");

        // Check JSONL file exists
        let jsonl_path = dir.path().join("agents").join("test-agent-1.jsonl");
        assert!(jsonl_path.exists(), "JSONL file should be created");

        let content = fs::read_to_string(&jsonl_path).unwrap();
        let event: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(event["event"], "start");
        assert_eq!(event["type"], "planner");

        // Check HUD state
        let state = hud_state::read(state_dir);
        let agent = state.team.get("test-agent-1").unwrap();
        assert_eq!(agent.status, Some("running".to_string()));
        assert_eq!(agent.agent_type, Some("planner".to_string()));
        assert_eq!(agent.tool_calls, 0);
    }

    #[test]
    fn test_start_max_team_entries() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // Create 20 agents
        for i in 0..20 {
            run(state_dir, &format!("agent-{}", i), "worker");
        }

        let state = hud_state::read(state_dir);
        assert_eq!(state.team.len(), 20);

        // 21st agent should evict one
        run(state_dir, "agent-overflow", "worker");
        let state = hud_state::read(state_dir);
        assert!(state.team.len() <= 20);
        assert!(state.team.contains_key("agent-overflow"));
    }

    #[test]
    fn test_start_symlink_jsonl_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();
        let agents_dir = dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        // Create a symlink where the JSONL would go
        let jsonl_path = agents_dir.join("evil-agent.jsonl");
        let target = dir.path().join("target.txt");
        fs::write(&target, "sensitive data").unwrap();
        std::os::unix::fs::symlink(&target, &jsonl_path).unwrap();

        run(state_dir, "evil-agent", "planner");

        // The symlink should have been removed and replaced with a real file
        assert!(!jsonl_path.symlink_metadata().unwrap().file_type().is_symlink());
        // Original target should still exist
        assert!(target.exists());
    }
}
