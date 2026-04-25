// workers/reaper.rs — Session reaper worker
//
// Periodically checks for sessions whose heartbeats have stopped and reaps them.
// Only targets sessions that have sent at least one heartbeat (last_heartbeat_at IS NOT NULL).
// Sessions that never sent heartbeats are left alone (backward compatibility).

use crate::config::ForgeConfig;
use crate::events::{self, EventSender};
use rusqlite::Connection;
use std::time::Duration;
use tokio::sync::watch;

/// Background worker that transitions sessions through the
/// `active → idle → ended` lifecycle based on heartbeat freshness.
///
/// Phase 2A-4d.3.1 #7 — operators saw "many active sessions" because
/// the previous reaper only had a binary `active → ended` transition.
/// Now sessions move:
///
/// * `active` → `idle` after `heartbeat_idle_secs` (default 600s = 10 min)
///   if `last_heartbeat_at` is older than the threshold but still within
///   the ended window.
/// * `active` or `idle` → `ended` after `heartbeat_timeout_secs`
///   (default 14400s = 4h) if `last_heartbeat_at` is older than the
///   ended threshold.
/// * `active` → `ended` (orphan path, unchanged) if `last_heartbeat_at`
///   IS NULL and `started_at` is older than 24 hours.
///
/// Set `heartbeat_idle_secs = 0` to disable the idle phase (sessions go
/// straight `active → ended` like the previous behavior).
pub async fn run_session_reaper(
    db_path: String,
    config: ForgeConfig,
    events: EventSender,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let workers = config.workers.validated();
    let interval = workers.session_reaper_interval_secs;
    let timeout = workers.heartbeat_timeout_secs;
    let idle = workers.heartbeat_idle_secs;

    tracing::info!(
        target: "forge::reaper",
        interval_s = interval,
        timeout_s = timeout,
        idle_s = idle,
        "reaper started"
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {
                if let Err(e) = reap_stale_sessions(&db_path, timeout, idle, &events) {
                    tracing::error!(target: "forge::reaper", error = %e, "reaper error");
                }
            }
            _ = shutdown_rx.changed() => {
                tracing::info!(target: "forge::reaper", "shutdown received");
                return;
            }
        }
    }
}

fn reap_stale_sessions(
    db_path: &str,
    timeout_secs: u64,
    idle_secs: u64,
    events: &EventSender,
) -> Result<(), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("db open: {e}"))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .map_err(|e| format!("pragma: {e}"))?;

    // Phase 0: Transition active → idle when heartbeat is older than
    // `heartbeat_idle_secs` but still within the ended window.
    // Skipped when idle_secs == 0 (idle phase disabled).
    let idle_count = if idle_secs > 0 {
        let idle_sql = format!(
            "UPDATE session SET status = 'idle' \
             WHERE status = 'active' \
             AND last_heartbeat_at IS NOT NULL \
             AND last_heartbeat_at < datetime('now', '-{idle_secs} seconds') \
             AND last_heartbeat_at >= datetime('now', '-{timeout_secs} seconds') \
             RETURNING id"
        );
        let mut stmt0 = conn
            .prepare(&idle_sql)
            .map_err(|e| format!("prepare idle: {e}"))?;
        let idle_ids: Vec<String> = stmt0
            .query_map([], |row| row.get(0))
            .map_err(|e| format!("query idle: {e}"))?
            .filter_map(|r| r.ok())
            .collect();

        for session_id in &idle_ids {
            events::emit(
                events,
                "session_idled",
                serde_json::json!({
                    "session_id": session_id,
                    "idle_secs": idle_secs,
                }),
            );
            tracing::info!(target: "forge::reaper", %session_id, idle_secs, "transitioned to idle");
        }
        idle_ids.len()
    } else {
        0
    };

    // Phase 1: Reap sessions (active OR idle) whose heartbeats have stopped
    // beyond the ended threshold. Atomic: full WHERE clause avoids TOCTOU.
    let reap_sql = format!(
        "UPDATE session SET status = 'ended', ended_at = datetime('now') \
         WHERE status IN ('active', 'idle') \
         AND last_heartbeat_at IS NOT NULL \
         AND last_heartbeat_at < datetime('now', '-{timeout_secs} seconds') \
         RETURNING id"
    );
    let mut stmt = conn
        .prepare(&reap_sql)
        .map_err(|e| format!("prepare: {e}"))?;
    let stale_ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("query: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    for session_id in &stale_ids {
        events::emit(
            events,
            "session_reaped",
            serde_json::json!({
                "session_id": session_id,
                "reason": "heartbeat_timeout"
            }),
        );
        tracing::info!(target: "forge::reaper", %session_id, reason = "heartbeat_timeout", "reaped stale session");
    }

    // Phase 2: Reap sessions that never heartbeated AND are older than 24 hours.
    // These are typically hook-test sessions or leaked registrations where
    // the session-end hook never fired. Without this, they accumulate forever.
    let orphan_sql = "UPDATE session SET status = 'ended', ended_at = datetime('now') \
         WHERE status = 'active' \
         AND last_heartbeat_at IS NULL \
         AND started_at < datetime('now', '-86400 seconds') \
         RETURNING id";
    let mut stmt2 = conn
        .prepare(orphan_sql)
        .map_err(|e| format!("prepare orphan: {e}"))?;
    let orphan_ids: Vec<String> = stmt2
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("query orphan: {e}"))?
        .filter_map(|r| r.ok())
        .collect();

    for session_id in &orphan_ids {
        events::emit(
            events,
            "session_reaped",
            serde_json::json!({
                "session_id": session_id,
                "reason": "no_heartbeat_24h"
            }),
        );
        tracing::info!(target: "forge::reaper", %session_id, reason = "no_heartbeat_24h", "reaped orphan session");
    }

    let total = idle_count + stale_ids.len() + orphan_ids.len();
    if total > 0 {
        tracing::info!(
            target: "forge::reaper",
            total,
            idled = idle_count,
            stale = stale_ids.len(),
            orphan = orphan_ids.len(),
            "session lifecycle pass"
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;
    use crate::events::create_event_bus;

    fn setup_db() -> (String, Connection, tempfile::TempDir) {
        crate::db::vec::init_sqlite_vec();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("test_reaper.db");
        let path_str = path.to_str().unwrap().to_string();
        let conn = Connection::open(&path_str).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        create_schema(&conn).unwrap();
        (path_str, conn, dir)
    }

    #[test]
    fn test_reap_skips_no_heartbeat_sessions() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        reap_stale_sessions(&path, 1, 0, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn test_reap_stale_heartbeated_session() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        let mut rx = tx.subscribe();
        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now', '-120 seconds') WHERE id = 's1'",
            [],
        ).unwrap();
        reap_stale_sessions(&path, 60, 0, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "ended");
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "session_reaped");
        assert_eq!(event.data["session_id"], "s1");
    }

    #[test]
    fn test_reap_leaves_recent_heartbeat() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now') WHERE id = 's1'",
            [],
        )
        .unwrap();
        reap_stale_sessions(&path, 60, 0, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn test_reap_orphan_sessions_no_heartbeat_24h() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        let mut rx = tx.subscribe();

        // Register a session with no heartbeat, created >24h ago.
        // Phase 2A-4d.3.1 #7: register_session now sets last_heartbeat_at,
        // so we manually clear it to simulate the "session created via raw
        // SQL or pre-#7 binary" path that the orphan reaper still needs
        // to handle.
        crate::sessions::register_session(&conn, "orphan1", "hook-test", None, None, None, None)
            .unwrap();
        conn.execute(
            "UPDATE session SET started_at = datetime('now', '-90000 seconds'), last_heartbeat_at = NULL WHERE id = 'orphan1'",
            [],
        ).unwrap();

        // Register a recent session with no heartbeat (should NOT be reaped).
        crate::sessions::register_session(&conn, "recent1", "hook-test", None, None, None, None)
            .unwrap();
        conn.execute(
            "UPDATE session SET last_heartbeat_at = NULL WHERE id = 'recent1'",
            [],
        )
        .unwrap();

        reap_stale_sessions(&path, 300, 0, &tx).unwrap();

        let orphan: String = conn
            .query_row("SELECT status FROM session WHERE id = 'orphan1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let recent: String = conn
            .query_row("SELECT status FROM session WHERE id = 'recent1'", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert_eq!(orphan, "ended", "orphan session >24h should be reaped");
        assert_eq!(recent, "active", "recent session should be left alone");

        // Verify event was emitted for orphan
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "session_reaped");
        assert_eq!(event.data["reason"], "no_heartbeat_24h");
    }

    #[test]
    fn test_reap_multiple_sessions_mixed() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();

        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now', '-600 seconds') WHERE id = 's1'",
            [],
        ).unwrap();

        crate::sessions::register_session(&conn, "s2", "cline", None, None, None, None).unwrap();

        crate::sessions::register_session(&conn, "s3", "codex", None, None, None, None).unwrap();
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now') WHERE id = 's3'",
            [],
        )
        .unwrap();

        reap_stale_sessions(&path, 300, 0, &tx).unwrap();

        let s1: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let s2: String = conn
            .query_row("SELECT status FROM session WHERE id = 's2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        let s3: String = conn
            .query_row("SELECT status FROM session WHERE id = 's3'", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert_eq!(s1, "ended", "stale heartbeat should be reaped");
        assert_eq!(s2, "active", "no heartbeat should be left alone");
        assert_eq!(s3, "active", "recent heartbeat should be left alone");
    }

    // ── Phase 2A-4d.3.1 #7 — idle phase tests ─────────────────────

    #[test]
    fn test_idle_transition_active_to_idle() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        let mut rx = tx.subscribe();
        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        // Heartbeat 60s ago — older than idle (30s) but newer than timeout (300s).
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now', '-60 seconds') WHERE id = 's1'",
            [],
        )
        .unwrap();
        reap_stale_sessions(&path, 300, 30, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            status, "idle",
            "active → idle when heartbeat in idle window"
        );
        // Verify session_idled event fired.
        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "session_idled");
        assert_eq!(event.data["session_id"], "s1");
    }

    #[test]
    fn test_idle_to_ended_transition() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        // Already idle, heartbeat 600s ago — beyond timeout (300s).
        conn.execute(
            "UPDATE session SET status = 'idle', last_heartbeat_at = datetime('now', '-600 seconds') WHERE id = 's1'",
            [],
        )
        .unwrap();
        reap_stale_sessions(&path, 300, 30, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            status, "ended",
            "idle → ended when heartbeat exceeds timeout"
        );
    }

    #[test]
    fn test_idle_disabled_when_secs_zero() {
        // idle_secs = 0 means "skip the idle phase entirely".
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        // Heartbeat 60s ago — would be idle if phase were enabled.
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now', '-60 seconds') WHERE id = 's1'",
            [],
        )
        .unwrap();
        reap_stale_sessions(&path, 300, 0, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            status, "active",
            "idle phase disabled — session stays active"
        );
    }

    #[test]
    fn test_register_session_seeds_heartbeat() {
        // Phase 2A-4d.3.1 #7: register_session sets last_heartbeat_at = now
        // so the idle/ended lifecycle has a starting point.
        let (_path, conn, _dir) = setup_db();
        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None)
            .unwrap();
        let hb: Option<String> = conn
            .query_row(
                "SELECT last_heartbeat_at FROM session WHERE id = 's1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(hb.is_some(), "register_session must seed last_heartbeat_at");
    }
}
