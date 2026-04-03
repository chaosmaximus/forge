//! Shared HUD state module — atomic read/update for hud-state.json with flock.
//!
//! Used by agent lifecycle handlers and session hooks. The flock prevents
//! races when multiple hooks fire simultaneously (e.g. SubagentStart + PostToolUse).

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

// ---------------------------------------------------------------------------
// Structs — match the JSON schema written by forge-graph's HudStateWriter
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct HudState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub graph: GraphStats,
    #[serde(default)]
    pub memory: MemoryStats,
    #[serde(default)]
    pub session: SessionInfo,
    #[serde(default)]
    pub tokens: TokenStats,
    #[serde(default)]
    pub skills: SkillStats,
    #[serde(default)]
    pub team: HashMap<String, AgentEntry>,
    #[serde(default)]
    pub security: SecurityStats,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct GraphStats {
    pub nodes: u64,
    pub edges: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct MemoryStats {
    pub decisions: u64,
    pub patterns: u64,
    pub lessons: u64,
    pub secrets: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct SessionInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wave: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct TokenStats {
    pub input: u64,
    pub output: u64,
    pub llm_calls: u64,
    pub deterministic_ratio: f64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct SkillStats {
    pub active: u64,
    pub fix_candidates: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct AgentEntry {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct SecurityStats {
    pub total: u64,
    pub stale: u64,
    pub exposed: u64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read the current HUD state from `{state_dir}/hud/hud-state.json`.
/// Returns `HudState::default()` if the file is missing or unparseable.
pub fn read(state_dir: &str) -> HudState {
    let path = Path::new(state_dir).join("hud").join("hud-state.json");
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HudState::default(),
    }
}

/// Atomically update the HUD state file under an exclusive flock.
///
/// 1. Ensures `{state_dir}/hud/` exists (mode 0o700).
/// 2. Opens/creates a lock file at `{state_dir}/hud/hud-state.lock` (mode 0o600).
/// 3. Attempts `try_lock_exclusive()` — returns `false` immediately if contended.
/// 4. Reads current state (defaults on error).
/// 5. Applies the caller's mutation via `f`.
/// 6. Writes to a temp file (mode 0o600), fsync, then renames atomically.
/// 7. Unlocks and returns `true`.
pub fn update(state_dir: &str, f: impl FnOnce(&mut HudState)) -> bool {
    let hud_dir = Path::new(state_dir).join("hud");

    // Ensure hud directory exists with restricted permissions
    if !hud_dir.exists() {
        if fs::create_dir_all(&hud_dir).is_err() {
            return false;
        }
        // Set directory permissions to 0o700
        let _ = fs::set_permissions(&hud_dir, fs::Permissions::from_mode(0o700));
    }

    let lock_path = hud_dir.join("hud-state.lock");
    let state_path = hud_dir.join("hud-state.json");
    let tmp_path = hud_dir.join("hud-state.json.tmp");

    // Open/create lock file with 0o600 permissions
    let lock_file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => return false,
    };
    let _ = fs::set_permissions(&lock_path, fs::Permissions::from_mode(0o600));

    // Non-blocking exclusive lock — skip update if contended
    if lock_file.try_lock_exclusive().is_err() {
        return false;
    }

    // Read current state (default on any error)
    let mut state: HudState = match fs::read_to_string(&state_path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => HudState::default(),
    };

    // Apply the caller's mutation
    f(&mut state);

    // Serialize
    let json_bytes = match serde_json::to_string(&state) {
        Ok(s) => s,
        Err(_) => {
            let _ = lock_file.unlock();
            return false;
        }
    };

    // Atomic write: tmp file -> fsync -> rename
    let result = (|| -> std::io::Result<()> {
        let mut tmp_file = File::create(&tmp_path)?;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
        tmp_file.write_all(json_bytes.as_bytes())?;
        tmp_file.sync_all()?;
        fs::rename(&tmp_path, &state_path)?;
        Ok(())
    })();

    // Clean up tmp on failure
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }

    let _ = lock_file.unlock();
    result.is_ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_read_missing_file() {
        let state = read("/nonexistent/path/that/does/not/exist");
        assert_eq!(state.graph.nodes, 0);
        assert!(state.version.is_none());
        assert!(state.team.is_empty());
    }

    #[test]
    fn test_read_valid_state() {
        let dir = tempfile::tempdir().unwrap();
        let hud_dir = dir.path().join("hud");
        fs::create_dir_all(&hud_dir).unwrap();
        let state_path = hud_dir.join("hud-state.json");
        fs::write(
            &state_path,
            r#"{"version":"0.2.0","memory":{"decisions":5,"patterns":3,"lessons":1,"secrets":2}}"#,
        )
        .unwrap();

        let state = read(dir.path().to_str().unwrap());
        assert_eq!(state.version, Some("0.2.0".to_string()));
        assert_eq!(state.memory.decisions, 5);
        assert_eq!(state.memory.patterns, 3);
    }

    #[test]
    fn test_read_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let hud_dir = dir.path().join("hud");
        fs::create_dir_all(&hud_dir).unwrap();
        let state_path = hud_dir.join("hud-state.json");
        fs::write(&state_path, "not json at all").unwrap();

        let state = read(dir.path().to_str().unwrap());
        assert_eq!(state.graph.nodes, 0);
    }

    #[test]
    fn test_update_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        let ok = update(state_dir, |s| {
            s.version = Some("0.2.0".to_string());
            s.memory.decisions = 42;
        });
        assert!(ok);

        let state = read(state_dir);
        assert_eq!(state.version, Some("0.2.0".to_string()));
        assert_eq!(state.memory.decisions, 42);
    }

    #[test]
    fn test_update_preserves_existing() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        // First update
        assert!(update(state_dir, |s| {
            s.memory.decisions = 10;
            s.memory.patterns = 5;
        }));

        // Second update — only touch decisions
        assert!(update(state_dir, |s| {
            s.memory.decisions = 20;
        }));

        let state = read(state_dir);
        assert_eq!(state.memory.decisions, 20);
        assert_eq!(state.memory.patterns, 5); // preserved
    }

    #[test]
    fn test_update_agent_entry() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        assert!(update(state_dir, |s| {
            s.team.insert(
                "planner-abc123".to_string(),
                AgentEntry {
                    agent_type: Some("planner".to_string()),
                    status: Some("running".to_string()),
                    started_at: Some("2026-04-02T10:00:00Z".to_string()),
                    tool_calls: 3,
                    files: vec!["src/main.rs".to_string()],
                    last_tool: Some("Read".to_string()),
                    current_file: Some("src/main.rs".to_string()),
                    ..Default::default()
                },
            );
        }));

        let state = read(state_dir);
        let agent = state.team.get("planner-abc123").unwrap();
        assert_eq!(agent.agent_type, Some("planner".to_string()));
        assert_eq!(agent.status, Some("running".to_string()));
        assert_eq!(agent.tool_calls, 3);
        assert_eq!(agent.files.len(), 1);
    }

    #[test]
    fn test_serde_agent_type_rename() {
        let json = r#"{"type":"evaluator","status":"done","tool_calls":7}"#;
        let entry: AgentEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.agent_type, Some("evaluator".to_string()));

        // Serialize back — should use "type" not "agent_type"
        let serialized = serde_json::to_string(&entry).unwrap();
        assert!(serialized.contains(r#""type":"evaluator""#));
        assert!(!serialized.contains("agent_type"));
    }

    #[test]
    fn test_file_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path().to_str().unwrap();

        assert!(update(state_dir, |s| {
            s.version = Some("test".to_string());
        }));

        let hud_dir = dir.path().join("hud");
        let state_path = hud_dir.join("hud-state.json");
        let lock_path = hud_dir.join("hud-state.lock");

        // Check directory permissions
        let dir_perms = fs::metadata(&hud_dir).unwrap().permissions().mode();
        assert_eq!(dir_perms & 0o777, 0o700, "hud dir should be 0o700");

        // Check state file permissions
        let file_perms = fs::metadata(&state_path).unwrap().permissions().mode();
        assert_eq!(file_perms & 0o777, 0o600, "state file should be 0o600");

        // Check lock file permissions
        let lock_perms = fs::metadata(&lock_path).unwrap().permissions().mode();
        assert_eq!(lock_perms & 0o777, 0o600, "lock file should be 0o600");
    }

    #[test]
    fn test_skip_serializing_none_fields() {
        let state = HudState {
            version: None,
            session: SessionInfo {
                mode: Some("forge".to_string()),
                phase: None,
                wave: None,
            },
            ..Default::default()
        };
        let json = serde_json::to_string(&state).unwrap();
        // version should be omitted
        assert!(!json.contains("version"));
        // phase and wave should be omitted
        assert!(!json.contains("phase"));
        assert!(!json.contains("wave"));
        // mode should be present
        assert!(json.contains(r#""mode":"forge""#));
    }
}
