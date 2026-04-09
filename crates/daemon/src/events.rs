// events.rs — Event bus for real-time streaming to subscribers
//
// Uses tokio::broadcast for fan-out to multiple subscribers (Mac app, CLI, etc.).
// Events are best-effort: if a subscriber is slow, it skips (Lagged).

use serde::{Serialize, Deserialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeEvent {
    pub event: String,     // "extraction" | "consolidation" | "guardrail" | "agent"
    pub data: serde_json::Value,
    pub timestamp: String,
}

pub type EventSender = broadcast::Sender<ForgeEvent>;

pub fn create_event_bus() -> EventSender {
    let (tx, _) = broadcast::channel(256);
    tx
}

/// Helper to emit an event (best-effort, never blocks).
pub fn emit(tx: &EventSender, event: &str, data: serde_json::Value) {
    let _ = tx.send(ForgeEvent {
        event: event.to_string(),
        data,
        timestamp: timestamp_now(),
    });
}

fn timestamp_now() -> String {
    forge_core::time::timestamp_now()
}

/// Spawn a background task that writes HUD state on every daemon event.
/// The HUD binary (forge-hud) reads this file to render the status line.
///
/// Opens a read-only DB connection to query memory/security/team stats.
/// Also reads K8s context and HUD config for the renderer.
pub fn spawn_hud_writer(tx: &EventSender) {
    let mut rx = tx.subscribe();
    let db_path = std::env::var("FORGE_DB")
        .unwrap_or_else(|_| forge_core::default_db_path());

    tokio::spawn(async move {
        // Write to the same path the HUD binary reads from.
        // Priority: CLAUDE_PLUGIN_DATA > known plugin data dirs > ~/.forge/
        let hud_dir = resolve_hud_dir();
        let _ = std::fs::create_dir_all(&hud_dir);
        let hud_path = hud_dir.join("hud-state.json");

        // Track agent team state across events (persists between events)
        let mut team_state: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();

        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Update agent team state from events
                    if event.event == "agent_status" || event.event == "agent_status_changed" {
                        if let Some(obj) = event.data.as_object() {
                            let agent_id = obj.get("agent_id").or(obj.get("transcript"))
                                .and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
                            let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
                            let last_tool = obj.get("last_tool").and_then(|v| v.as_str());
                            let agent_type = obj.get("agent").and_then(|v| v.as_str());

                            team_state.insert(agent_id, serde_json::json!({
                                "status": if status == "working" || status == "thinking" { "running" } else { status },
                                "agent_type": agent_type,
                                "last_tool": last_tool,
                            }));
                        }
                    }

                    // Build full HUD state by querying the DB
                    let mut state = build_hud_state(&db_path, &event);
                    if let Some(team) = state.get_mut("team") {
                        *team = serde_json::json!(team_state);
                    }

                    // Write to the shared HUD state file (all sessions see the same file).
                    // The HUD binary uses stdin.cwd to scope stats to the current project,
                    // so the shared file contains global + per-project breakdowns.
                    let _ = std::fs::write(&hud_path, state.to_string());
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(skipped = n, "HUD writer lagged, catching up");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Resolve the HUD directory — matches the path the HUD binary reads from.
/// Checks: CLAUDE_PLUGIN_DATA, known plugin data dirs, ~/.forge/
fn resolve_hud_dir() -> std::path::PathBuf {
    // 1. CLAUDE_PLUGIN_DATA (set by Claude Code)
    if let Ok(d) = std::env::var("CLAUDE_PLUGIN_DATA") {
        if !d.is_empty() && !d.contains("codex") {
            return std::path::PathBuf::from(d).join("hud");
        }
    }

    // 2. Known plugin data directories
    if let Ok(home) = std::env::var("HOME") {
        let candidates = [
            format!("{home}/.claude/plugins/data/forge-forge-marketplace"),
            format!("{home}/.claude/plugins/data/forge"),
            format!("{home}/.claude/plugin-data/forge"),
        ];
        for c in &candidates {
            if std::path::Path::new(c).is_dir() {
                return std::path::PathBuf::from(c).join("hud");
            }
        }
    }

    // 3. Fallback to ~/.forge/
    std::path::PathBuf::from(forge_core::forge_dir()).join("hud")
}

/// Build per-project memory stats: {project_name: {decisions, lessons, patterns}}.
fn build_project_stats(conn: &rusqlite::Connection) -> serde_json::Value {
    let mut projects = serde_json::Map::new();
    let mut stmt = match conn.prepare(
        "SELECT COALESCE(project, ''), memory_type, COUNT(*)
         FROM memory WHERE status = 'active' AND project IS NOT NULL AND project != ''
         GROUP BY project, memory_type"
    ) {
        Ok(s) => s,
        Err(_) => return serde_json::Value::Object(projects),
    };

    let rows: Vec<(String, String, u64)> = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u64>(2)?))
    }).ok()
        .map(|r| r.flatten().collect())
        .unwrap_or_default();

    for (proj, mtype, count) in rows {
        let entry = projects.entry(proj).or_insert_with(|| serde_json::json!({
            "decisions": 0, "lessons": 0, "patterns": 0
        }));
        if let Some(obj) = entry.as_object_mut() {
            match mtype.as_str() {
                "decision" => { obj.insert("decisions".into(), count.into()); }
                "lesson" => { obj.insert("lessons".into(), count.into()); }
                "pattern" => { obj.insert("patterns".into(), count.into()); }
                _ => {}
            }
        }
    }

    serde_json::Value::Object(projects)
}

/// Build complete HUD state JSON by querying the daemon DB.
fn build_hud_state(db_path: &str, event: &ForgeEvent) -> serde_json::Value {
    // Open read-only connection (lightweight, same as socket handler pattern)
    let conn = match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => {
            return serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "daemon_up": true,
                "last_event": event.event,
            });
        }
    };

    // Memory stats
    let decisions: u64 = conn.query_row(
        "SELECT COUNT(*) FROM memory WHERE memory_type = 'decision' AND status = 'active'",
        [], |r| r.get(0),
    ).unwrap_or(0);
    let lessons: u64 = conn.query_row(
        "SELECT COUNT(*) FROM memory WHERE memory_type = 'lesson' AND status = 'active'",
        [], |r| r.get(0),
    ).unwrap_or(0);
    let patterns: u64 = conn.query_row(
        "SELECT COUNT(*) FROM memory WHERE memory_type = 'pattern' AND status = 'active'",
        [], |r| r.get(0),
    ).unwrap_or(0);

    // Security stats (secrets)
    let exposed: u64 = conn.query_row(
        "SELECT COUNT(*) FROM diagnostic WHERE severity = 'error' AND source = 'secret-scanner'",
        [], |r| r.get(0),
    ).unwrap_or(0);

    // K8s context
    let k8s = crate::hud_config::read_k8s_context().map(|(ctx, ns)| {
        serde_json::json!({ "context": ctx, "namespace": ns })
    });

    // HUD config (merged for current user)
    let hud_config = crate::hud_config::get_merged_hud_config(&conn, Some("default"), None, None, None)
        .ok()
        .map(|entries| {
            let sections: Vec<String> = entries.iter()
                .find(|e| e.key == "hud.sections")
                .and_then(|e| serde_json::from_str(&e.value).ok())
                .unwrap_or_default();
            let density = entries.iter()
                .find(|e| e.key == "hud.density")
                .map(|e| e.value.clone())
                .unwrap_or_else(|| "normal".to_string());
            serde_json::json!({ "sections": sections, "density": density })
        });

    // CWD from active session or daemon process
    let cwd: Option<String> = conn.query_row(
        "SELECT cwd FROM session WHERE status = 'active' AND cwd IS NOT NULL ORDER BY last_active DESC LIMIT 1",
        [], |r| r.get(0),
    ).ok().or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()));

    // Per-project memory stats breakdown
    let projects = build_project_stats(&conn);

    serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "memory": {
            "decisions": decisions,
            "lessons": lessons,
            "patterns": patterns,
        },
        "security": {
            "total": 0,
            "stale": 0,
            "exposed": exposed,
        },
        "k8s": k8s,
        "hud_config": hud_config,
        "cwd": cwd,
        "projects": projects,
        "team": {},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_bus() {
        let tx = create_event_bus();
        let mut rx = tx.subscribe();

        emit(&tx, "extraction", serde_json::json!({"title": "test"}));

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "extraction");
        assert_eq!(event.data["title"], "test");
        assert!(!event.timestamp.is_empty());
    }

    #[test]
    fn test_event_bus_multiple() {
        let tx = create_event_bus();
        let mut rx = tx.subscribe();

        emit(&tx, "extraction", serde_json::json!({}));
        emit(&tx, "consolidation", serde_json::json!({}));

        // Both received (no filter at bus level — filtering is in socket handler)
        let e1 = rx.try_recv().unwrap();
        assert_eq!(e1.event, "extraction");
        let e2 = rx.try_recv().unwrap();
        assert_eq!(e2.event, "consolidation");
    }

    #[test]
    fn test_event_bus_no_subscriber_no_panic() {
        let tx = create_event_bus();
        // Emit with no subscribers — should not panic
        emit(&tx, "test", serde_json::json!({"ok": true}));
    }

    #[test]
    fn test_event_serde_roundtrip() {
        let event = ForgeEvent {
            event: "extraction".to_string(),
            data: serde_json::json!({"memory_id": "abc123"}),
            timestamp: "1712000000".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let restored: ForgeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.event, "extraction");
        assert_eq!(restored.data["memory_id"], "abc123");
    }
}
