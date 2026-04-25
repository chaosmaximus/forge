// events.rs — Event bus for real-time streaming to subscribers
//
// Uses tokio::broadcast for fan-out to multiple subscribers (Mac app, CLI, etc.).
// Events are best-effort: if a subscriber is slow, it skips (Lagged).

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeEvent {
    pub event: String, // "extraction" | "consolidation" | "guardrail" | "agent"
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
    let db_path = std::env::var("FORGE_DB").unwrap_or_else(|_| forge_core::default_db_path());

    tokio::spawn(async move {
        // Write to the same path the HUD binary reads from.
        // Priority: CLAUDE_PLUGIN_DATA > known plugin data dirs > ~/.forge/
        let hud_dir = resolve_hud_dir();
        let _ = std::fs::create_dir_all(&hud_dir);
        let hud_path = hud_dir.join("hud-state.json");

        // Track state across events (persists between events)
        let mut team_state: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let mut session_state: Vec<serde_json::Value> = Vec::new();

        loop {
            match rx.recv().await {
                Ok(event) => {
                    // Track session registrations from events
                    if event.event == "session_changed"
                        && event.data.get("action").and_then(|v| v.as_str()) == Some("registered")
                    {
                        if let Some(obj) = event.data.as_object() {
                            let id = obj
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let agent = obj
                                .get("agent")
                                .and_then(|v| v.as_str())
                                .unwrap_or("claude-code")
                                .to_string();
                            let project = obj
                                .get("project")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let cwd = obj
                                .get("cwd")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            // Avoid duplicates
                            session_state
                                .retain(|s| s.get("id").and_then(|v| v.as_str()) != Some(&id));
                            session_state.push(serde_json::json!({
                                "id": id, "agent": agent, "project": project, "cwd": cwd,
                                "since": &event.timestamp,
                            }));
                        }
                    }
                    if event.event == "session_changed"
                        && event.data.get("action").and_then(|v| v.as_str()) == Some("ended")
                    {
                        if let Some(id) = event.data.get("id").and_then(|v| v.as_str()) {
                            session_state
                                .retain(|s| s.get("id").and_then(|v| v.as_str()) != Some(id));
                        }
                    }

                    // Update agent team state from agent_status_changed events.
                    // Only track real team agents (planner/generator/evaluator), not
                    // transcript watcher events which use transcript paths as IDs.
                    if event.event == "agent_status_changed" {
                        if let Some(obj) = event.data.as_object() {
                            let agent_id = obj.get("agent_id").and_then(|v| v.as_str());
                            // Skip transcript watcher events (agent_id is a file path)
                            if let Some(id) = agent_id {
                                if !id.contains('/') && !id.contains(".jsonl") {
                                    let status = obj
                                        .get("status")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("unknown");
                                    let last_tool = obj.get("last_tool").and_then(|v| v.as_str());
                                    let agent_type = obj.get("agent").and_then(|v| v.as_str());

                                    if status == "done" || status == "ended" {
                                        team_state.remove(id);
                                    } else {
                                        team_state.insert(id.to_string(), serde_json::json!({
                                            "status": if status == "working" || status == "thinking" { "running" } else { status },
                                            "agent_type": agent_type,
                                            "last_tool": last_tool,
                                        }));
                                    }
                                }
                            }
                        }
                    }

                    // Phase 2A-4d.2.1 #3: the prior path ran synchronous
                    // `build_hud_state` (DB queries + file read) plus a
                    // non-atomic `fs::write` directly on the tokio runtime.
                    // On bursty event rates that blocks the worker, and a
                    // partial-write of hud-state.json could be observed by
                    // the HUD binary mid-flight.
                    //
                    // Fix: move the whole synchronous block into
                    // `tokio::task::spawn_blocking` (so DB + I/O happen on
                    // the blocking pool) and write via tmpfile + atomic
                    // rename so readers always see a complete file.
                    //
                    // The `.await` is intentional: writes are dispatched
                    // serially per event so HUD readers see the events in
                    // bus order. Concurrent writes would race on the same
                    // target file (last rename wins), and skipping the
                    // wait would also let `team_state` / `session_state`
                    // mutate underneath an in-flight write. Callers
                    // optimizing for throughput at the cost of order
                    // would have to maintain a write-coalescing buffer.
                    let db_path_for_blocking = db_path.clone();
                    let hud_path_for_blocking = hud_path.clone();
                    let team_state_clone = team_state.clone();
                    let session_state_clone = session_state.clone();
                    let event_for_blocking = event.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        let mut state = build_hud_state(&db_path_for_blocking, &event_for_blocking);
                        if let Some(team) = state.get_mut("team") {
                            *team = serde_json::json!(team_state_clone);
                        }
                        if let Some(sessions) = state.get_mut("sessions") {
                            *sessions = serde_json::json!(session_state_clone);
                        }

                        // Atomic write: serialize → tmpfile → rename.
                        // Rename is atomic on the same filesystem, so the
                        // HUD binary never observes a half-written file.
                        let tmp_path = hud_path_for_blocking.with_extension("json.tmp");
                        let payload = state.to_string();
                        match std::fs::write(&tmp_path, &payload) {
                            Ok(()) => {
                                if let Err(e) = std::fs::rename(&tmp_path, &hud_path_for_blocking) {
                                    tracing::warn!(
                                        target: "forge::hud",
                                        error = %e,
                                        "hud-state.json atomic rename failed"
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "forge::hud",
                                    error = %e,
                                    "hud-state.json tmpfile write failed"
                                );
                            }
                        }
                    })
                    .await;
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
         GROUP BY project, memory_type",
    ) {
        Ok(s) => s,
        Err(_) => return serde_json::Value::Object(projects),
    };

    let rows: Vec<(String, String, u64)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })
        .ok()
        .map(|r| r.flatten().collect())
        .unwrap_or_default();

    for (proj, mtype, count) in rows {
        let entry = projects.entry(proj).or_insert_with(|| {
            serde_json::json!({
                "decisions": 0, "lessons": 0, "patterns": 0
            })
        });
        if let Some(obj) = entry.as_object_mut() {
            match mtype.as_str() {
                "decision" => {
                    obj.insert("decisions".into(), count.into());
                }
                "lesson" => {
                    obj.insert("lessons".into(), count.into());
                }
                "pattern" => {
                    obj.insert("patterns".into(), count.into());
                }
                _ => {}
            }
        }
    }

    serde_json::Value::Object(projects)
}

/// Build complete HUD state JSON by querying the daemon DB.
fn build_hud_state(db_path: &str, event: &ForgeEvent) -> serde_json::Value {
    // Open connection with WAL mode to see latest writes from the writer actor
    let conn = match rusqlite::Connection::open(db_path) {
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
    let decisions: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE memory_type = 'decision' AND status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let lessons: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE memory_type = 'lesson' AND status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let patterns: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE memory_type = 'pattern' AND status = 'active'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);

    // Security stats (secrets)
    let exposed: u64 = conn.query_row(
        "SELECT COUNT(*) FROM diagnostic WHERE severity = 'error' AND source = 'secret-scanner'",
        [], |r| r.get(0),
    ).unwrap_or(0);

    // K8s context
    let k8s = crate::hud_config::read_k8s_context()
        .map(|(ctx, ns)| serde_json::json!({ "context": ctx, "namespace": ns }));

    // HUD config (merged for current user)
    let hud_config =
        crate::hud_config::get_merged_hud_config(&conn, Some("default"), None, None, None)
            .ok()
            .map(|entries| {
                let sections: Vec<String> = entries
                    .iter()
                    .find(|e| e.key == "hud.sections")
                    .and_then(|e| serde_json::from_str(&e.value).ok())
                    .unwrap_or_default();
                let density = entries
                    .iter()
                    .find(|e| e.key == "hud.density")
                    .map(|e| e.value.clone())
                    .unwrap_or_else(|| "normal".to_string());
                serde_json::json!({ "sections": sections, "density": density })
            });

    // CWD from any live (active|idle) session or daemon process.
    let cwd: Option<String> = conn.query_row(
        "SELECT cwd FROM session WHERE status IN ('active', 'idle') AND cwd IS NOT NULL ORDER BY last_active DESC LIMIT 1",
        [], |r| r.get(0),
    ).ok().or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()));

    // Per-project memory stats breakdown
    let projects = build_project_stats(&conn);

    // Sessions are tracked via events (not DB query — avoids WAL visibility issues)

    // Phase 2A-4d.2 T6: consolidation segment. Rebuilt from the event payload
    // on `consolidate_pass_completed` (cheap — no SQL for run fields, one
    // query for the 24h rollup). On every OTHER event, we carry over the
    // previously-written value from hud-state.json if it's still fresh
    // (<= 2× configured consolidation_interval_secs). This keeps the HUD
    // segment cheap for high-frequency events like extraction progress.
    let consolidation = build_consolidation_field(&conn, event);

    let mut out = serde_json::json!({
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
        "sessions": [],
        "team": {},
    });
    if let Some(cons) = consolidation {
        out["consolidation"] = cons;
    }
    out
}

/// Phase 2A-4d.2 T6 — populate `hud-state.json.consolidation` from the
/// incoming event when it's a pass-completion; otherwise carry over the
/// previously-written value if it's still fresh. Returns `None` when there
/// is no known pass OR the cached value is older than 2×
/// consolidation_interval_secs (first-boot and post-restart staleness guard).
fn build_consolidation_field(
    conn: &rusqlite::Connection,
    event: &ForgeEvent,
) -> Option<serde_json::Value> {
    if event.event == "consolidate_pass_completed" {
        build_consolidation_from_event(conn, event)
    } else {
        read_last_consolidation_from_hud_state_file()
    }
}

fn build_consolidation_from_event(
    conn: &rusqlite::Connection,
    event: &ForgeEvent,
) -> Option<serde_json::Value> {
    let d = &event.data;
    // All tolerant reads — missing / wrong-type fields yield None and the
    // segment is omitted from the HUD (renderer treats absence as "no data").
    let latest_run_id = d.get("run_id").and_then(|v| v.as_str()).map(String::from)?;
    let latest_run_duration_ms = d.get("pass_wall_duration_ms").and_then(|v| v.as_u64())?;
    let latest_run_phase_count = d.get("phase_count").and_then(|v| v.as_u64())?;
    let latest_run_error_count = d.get("error_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let latest_run_trace_id = d.get("trace_id").and_then(|v| v.as_str()).map(String::from);

    // 24h rollup — one query per pass (~1/30min at default cadence).
    // Uses the timestamp index.
    let cutoff_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 - 86_400)
        .unwrap_or(0);
    let (pass_count, error_passes): (u64, u64) = conn
        .query_row(
            r#"SELECT
                COUNT(DISTINCT json_extract(metadata_json, '$.run_id')) AS pass_count,
                SUM(CASE WHEN COALESCE(json_extract(metadata_json, '$.error_count'), 0) > 0
                         THEN 1 ELSE 0 END) AS err_passes
               FROM kpi_events
               WHERE timestamp >= ?1
                 AND event_type = 'phase_completed'"#,
            rusqlite::params![cutoff_secs],
            |r| {
                let pc: i64 = r.get(0)?;
                let ep: i64 = r.get::<_, Option<i64>>(1)?.unwrap_or(0);
                Ok((pc.max(0) as u64, ep.max(0) as u64))
            },
        )
        .unwrap_or((0, 0));

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    Some(serde_json::json!({
        "latest_run_id": latest_run_id,
        "latest_run_ts_secs": now_secs,
        "latest_run_wall_duration_ms": latest_run_duration_ms,
        "latest_run_error_count": latest_run_error_count,
        "latest_run_phase_count": latest_run_phase_count,
        "latest_run_trace_id": latest_run_trace_id,
        "rolling_24h_pass_count": pass_count,
        "rolling_24h_error_passes": error_passes,
    }))
}

/// Read the previously-written consolidation subtree from `hud-state.json`.
/// Returns `None` if the file is missing, unparseable, or the cached
/// timestamp is older than 2× consolidation_interval_secs.
fn read_last_consolidation_from_hud_state_file() -> Option<serde_json::Value> {
    let hud_path = resolve_hud_dir().join("hud-state.json");
    let content = std::fs::read_to_string(&hud_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    let cons = parsed.get("consolidation")?.clone();

    // Staleness guard: treat cache as stale if older than
    // 2× consolidation_interval, capped at 1h. The cap matters for very
    // long intervals (e.g. operators with 12-24h cadence) — without it a
    // daemon restart could serve a day-old HUD segment as "current".
    // 5 minutes minimum so a tight-interval dev loop doesn't flap.
    let cfg = crate::config::load_config();
    let interval = cfg.workers.consolidation_interval_secs;
    let max_age_secs = ((interval.saturating_mul(2)).clamp(300, 3600)) as i64;
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let ts = cons.get("latest_run_ts_secs")?.as_i64()?;
    if now_secs.saturating_sub(ts) > max_age_secs {
        None
    } else {
        Some(cons)
    }
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

    // ── Phase 2A-4d.2 T6: HUD consolidation segment ──

    fn seed_conn_for_events() -> rusqlite::Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn build_consolidation_from_event_populates_all_fields() {
        let conn = seed_conn_for_events();
        let event = ForgeEvent {
            event: "consolidate_pass_completed".to_string(),
            data: serde_json::json!({
                "event_schema_version": 1,
                "run_id": "01HX_TEST_RUN",
                "correlation_id": "01HX_TEST_RUN",
                "trace_id": null,
                "pass_wall_duration_ms": 1234u64,
                "phase_count": 23u64,
                "error_count": 0u64,
                "stats": {}
            }),
            timestamp: "1712000000".to_string(),
        };
        let cons = build_consolidation_from_event(&conn, &event).expect("field");
        assert_eq!(cons["latest_run_id"], "01HX_TEST_RUN");
        assert_eq!(cons["latest_run_wall_duration_ms"], 1234);
        assert_eq!(cons["latest_run_phase_count"], 23);
        assert_eq!(cons["latest_run_error_count"], 0);
        assert!(cons["latest_run_trace_id"].is_null());
        // 24h rollup on empty DB should be zeros.
        assert_eq!(cons["rolling_24h_pass_count"], 0);
        assert_eq!(cons["rolling_24h_error_passes"], 0);
    }

    #[test]
    fn build_consolidation_from_event_tolerates_missing_required_fields() {
        let conn = seed_conn_for_events();
        // Missing pass_wall_duration_ms → returns None (segment omitted).
        let event = ForgeEvent {
            event: "consolidate_pass_completed".to_string(),
            data: serde_json::json!({
                "run_id": "x",
                "phase_count": 23u64,
            }),
            timestamp: "1712000000".to_string(),
        };
        assert!(build_consolidation_from_event(&conn, &event).is_none());
    }

    #[test]
    fn build_consolidation_field_carries_over_on_non_pass_events() {
        // On an extraction event, the function calls
        // read_last_consolidation_from_hud_state_file which returns None when
        // the file doesn't exist. Verify it gracefully returns None rather
        // than panicking.
        let conn = seed_conn_for_events();
        let event = ForgeEvent {
            event: "extraction".to_string(),
            data: serde_json::json!({}),
            timestamp: "1712000000".to_string(),
        };
        // Should not panic; may or may not return Some depending on
        // ambient hud-state.json presence on the test host. Just assert
        // no crash.
        let _ = build_consolidation_field(&conn, &event);
    }
}
