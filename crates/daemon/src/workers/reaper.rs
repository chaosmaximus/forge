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

/// Background worker that reaps sessions whose heartbeats have stopped.
pub async fn run_session_reaper(
    db_path: String,
    config: ForgeConfig,
    events: EventSender,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let interval = config.workers.session_reaper_interval_secs;
    let timeout = config.workers.heartbeat_timeout_secs;

    eprintln!(
        "[reaper] started — interval={}s, timeout={}s",
        interval, timeout
    );

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {
                if let Err(e) = reap_stale_sessions(&db_path, timeout, &events) {
                    eprintln!("[reaper] error: {}", e);
                }
            }
            _ = shutdown_rx.changed() => {
                eprintln!("[reaper] shutdown received");
                return;
            }
        }
    }
}

fn reap_stale_sessions(
    db_path: &str,
    timeout_secs: u64,
    events: &EventSender,
) -> Result<(), String> {
    let conn = Connection::open(db_path).map_err(|e| format!("db open: {}", e))?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
        .map_err(|e| format!("pragma: {}", e))?;

    // Phase 1: Reap sessions whose heartbeats have stopped.
    // Atomic: UPDATE with full WHERE clause to avoid TOCTOU race.
    let reap_sql = format!(
        "UPDATE session SET status = 'ended', ended_at = datetime('now') \
         WHERE status = 'active' \
         AND last_heartbeat_at IS NOT NULL \
         AND last_heartbeat_at < datetime('now', '-{} seconds') \
         RETURNING id",
        timeout_secs
    );
    let mut stmt = conn.prepare(&reap_sql).map_err(|e| format!("prepare: {}", e))?;
    let stale_ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("query: {}", e))?
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
        eprintln!("[reaper] reaped stale session: {}", session_id);
    }

    // Phase 2: Reap sessions that never heartbeated AND are older than 24 hours.
    // These are typically hook-test sessions or leaked registrations where
    // the session-end hook never fired. Without this, they accumulate forever.
    let orphan_sql =
        "UPDATE session SET status = 'ended', ended_at = datetime('now') \
         WHERE status = 'active' \
         AND last_heartbeat_at IS NULL \
         AND started_at < datetime('now', '-86400 seconds') \
         RETURNING id";
    let mut stmt2 = conn.prepare(orphan_sql).map_err(|e| format!("prepare orphan: {}", e))?;
    let orphan_ids: Vec<String> = stmt2
        .query_map([], |row| row.get(0))
        .map_err(|e| format!("query orphan: {}", e))?
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
        eprintln!("[reaper] reaped orphan session (no heartbeat, >24h): {}", session_id);
    }

    let total = stale_ids.len() + orphan_ids.len();
    if total > 0 {
        eprintln!("[reaper] reaped {} session(s) ({} stale heartbeat, {} orphan)",
            total, stale_ids.len(), orphan_ids.len());
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
        reap_stale_sessions(&path, 1, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| r.get(0))
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
        reap_stale_sessions(&path, 60, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| r.get(0))
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
        ).unwrap();
        reap_stale_sessions(&path, 60, &tx).unwrap();
        let status: String = conn
            .query_row("SELECT status FROM session WHERE id = 's1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "active");
    }

    #[test]
    fn test_reap_orphan_sessions_no_heartbeat_24h() {
        let (path, conn, _dir) = setup_db();
        let tx = create_event_bus();
        let mut rx = tx.subscribe();

        // Register a session with no heartbeat, created >24h ago
        crate::sessions::register_session(&conn, "orphan1", "hook-test", None, None, None, None).unwrap();
        conn.execute(
            "UPDATE session SET started_at = datetime('now', '-90000 seconds') WHERE id = 'orphan1'",
            [],
        ).unwrap();

        // Register a recent session with no heartbeat (should NOT be reaped)
        crate::sessions::register_session(&conn, "recent1", "hook-test", None, None, None, None).unwrap();

        reap_stale_sessions(&path, 300, &tx).unwrap();

        let orphan: String = conn.query_row("SELECT status FROM session WHERE id = 'orphan1'", [], |r| r.get(0)).unwrap();
        let recent: String = conn.query_row("SELECT status FROM session WHERE id = 'recent1'", [], |r| r.get(0)).unwrap();

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

        crate::sessions::register_session(&conn, "s1", "claude-code", None, None, None, None).unwrap();
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now', '-600 seconds') WHERE id = 's1'",
            [],
        ).unwrap();

        crate::sessions::register_session(&conn, "s2", "cline", None, None, None, None).unwrap();

        crate::sessions::register_session(&conn, "s3", "codex", None, None, None, None).unwrap();
        conn.execute(
            "UPDATE session SET last_heartbeat_at = datetime('now') WHERE id = 's3'",
            [],
        ).unwrap();

        reap_stale_sessions(&path, 300, &tx).unwrap();

        let s1: String = conn.query_row("SELECT status FROM session WHERE id = 's1'", [], |r| r.get(0)).unwrap();
        let s2: String = conn.query_row("SELECT status FROM session WHERE id = 's2'", [], |r| r.get(0)).unwrap();
        let s3: String = conn.query_row("SELECT status FROM session WHERE id = 's3'", [], |r| r.get(0)).unwrap();

        assert_eq!(s1, "ended", "stale heartbeat should be reaped");
        assert_eq!(s2, "active", "no heartbeat should be left alone");
        assert_eq!(s3, "active", "recent heartbeat should be left alone");
    }
}
