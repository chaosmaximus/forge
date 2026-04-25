// workers/kpi_reaper.rs — Retention reaper for the `kpi_events` namespace.
//
// Phase 2A-4d.2 T7 (Tier 2 v4). Every `phase_completed` row landed by the
// consolidator (and any future writers listed in
// `docs/architecture/kpi_events-namespace.md`) persists forever unless
// something deletes it. Without retention the table grows unbounded:
// 23 phases × 48 passes/day = ~1,100 rows/day per project; on a multi-month
// running daemon this compounds into millions of rows and latency on the
// `/inspect` GROUP BY queries.
//
// Design choice: batched `DELETE … WHERE rowid IN (SELECT … LIMIT ?)` with
// a 50 ms yield between batches. The index on `timestamp` makes the inner
// SELECT cheap, and the batch size bounds the transaction so we never
// hold the writer lock for more than a few hundred ms on realistic tables.
//
// This worker does NOT touch `kpi_snapshots` / `kpi_benchmarks` /
// `kpi_uat_runs` — those tables have different lifecycles and are managed
// elsewhere.

use crate::server::handler::DaemonState;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{watch, Mutex};

/// Maximum rows deleted per transaction. Bounds lock hold time on the
/// writer connection and keeps the WAL growth manageable.
pub const BATCH_SIZE: usize = 10_000;

/// Default retention window (days). A row older than this at reap time is
/// eligible for deletion.
pub const DEFAULT_RETENTION_DAYS: u32 = 30;

/// Cooperative yield between batches. `std::thread::sleep` is intentional
/// here — `reap_once` is synchronous; the async wrapper
/// (`run_kpi_reaper`) owns the tokio integration and runs this body on
/// its own dedicated task (never the hot request path).
pub const BATCH_SLEEP_MS: u64 = 50;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Delete `kpi_events` rows older than retention bounds, in batches of
/// `BATCH_SIZE`. Returns the total number of rows deleted across all passes.
///
/// Two-pass strategy (Phase 2A-4d.3 T11, D9):
/// - Pass A — for each `(event_type, retention_days)` entry in
///   `retention_by_type`, delete rows of that event_type older than its
///   per-type cutoff. This lets `bench_run_completed` rows survive 180
///   days for the leaderboard while phase_completed et al. still expire
///   at the global default.
/// - Pass B — delete rows whose event_type is NOT in the override map and
///   whose timestamp is older than the global `retention_days` cutoff.
///
/// Sync on purpose — see module docs. Safe to call with any `Connection`
/// (writer-mode). Never panics on an empty table.
pub fn reap_once(
    conn: &Connection,
    retention_days: u32,
    retention_by_type: &HashMap<String, u32>,
) -> rusqlite::Result<usize> {
    let global_cutoff = now_secs().saturating_sub(u64::from(retention_days) * 86_400);
    tracing::info!(
        target: "forge::kpi_reaper",
        global_cutoff,
        retention_days,
        override_keys = retention_by_type.len(),
        "reap pass starting"
    );

    let mut total: usize = 0;

    // Pass A — per-event-type overrides. Iterate keys deterministically by
    // sorting so logs stay reproducible across runs (HashMap iteration is
    // non-deterministic in Rust).
    let mut ordered: Vec<(&String, &u32)> = retention_by_type.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    for (event_type, days) in ordered {
        let cutoff = now_secs().saturating_sub(u64::from(*days) * 86_400);
        // Phase 2A-4d.3.1 #6 L3 (W8): downgrade per-type pass-start to
        // `debug` — it fires once per configured override regardless
        // of whether any rows are reapable, which floods info-level
        // logs. The actual delete work below logs at info only when
        // `batch_deleted > 0`, so operators still see the meaningful
        // signal.
        tracing::debug!(
            target: "forge::kpi_reaper",
            event_type = %event_type,
            retention_days = *days,
            cutoff,
            "per-type reap pass"
        );
        loop {
            let n = conn.execute(
                "DELETE FROM kpi_events
                  WHERE rowid IN (
                      SELECT rowid FROM kpi_events
                       WHERE event_type = ?1
                         AND timestamp < ?2
                       LIMIT ?3
                  )",
                rusqlite::params![event_type, cutoff as i64, BATCH_SIZE as i64],
            )?;
            total += n;

            if n > 0 {
                tracing::info!(
                    target: "forge::kpi_reaper",
                    event_type = %event_type,
                    batch_deleted = n,
                    total,
                    "per-type reap batch"
                );
            }

            if n < BATCH_SIZE {
                break;
            }
            std::thread::sleep(Duration::from_millis(BATCH_SLEEP_MS));
        }
    }

    // Pass B — global default for everything NOT in the override map.
    // Build the NOT IN clause from sorted keys so the SQL plan is stable
    // and the parameter binding order matches the placeholders.
    let mut keys: Vec<&String> = retention_by_type.keys().collect();
    keys.sort();

    loop {
        // Compose the SQL with a placeholder count matching `keys.len()`.
        // For the empty-map case we degrade to the original single-pass
        // semantic (delete all rows older than the global cutoff).
        let mut sql = String::from(
            "DELETE FROM kpi_events
              WHERE rowid IN (
                  SELECT rowid FROM kpi_events
                   WHERE timestamp < ?1",
        );
        if !keys.is_empty() {
            sql.push_str(" AND event_type NOT IN (");
            for i in 0..keys.len() {
                if i > 0 {
                    sql.push_str(", ");
                }
                // Placeholders ?2, ?3, ... — ?1 is the cutoff.
                sql.push_str(&format!("?{}", i + 2));
            }
            sql.push(')');
        }
        sql.push_str(&format!(" LIMIT ?{})", keys.len() + 2));

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(keys.len() + 2);
        params.push(Box::new(global_cutoff as i64));
        for k in &keys {
            params.push(Box::new((*k).clone()));
        }
        params.push(Box::new(BATCH_SIZE as i64));

        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let n = conn.execute(&sql, param_refs.as_slice())?;
        total += n;

        if n > 0 {
            tracing::info!(
                target: "forge::kpi_reaper",
                batch_deleted = n,
                total,
                "global reap batch"
            );
        }

        if n < BATCH_SIZE {
            break;
        }
        std::thread::sleep(Duration::from_millis(BATCH_SLEEP_MS));
    }

    tracing::info!(
        target: "forge::kpi_reaper",
        total_deleted = total,
        "reap pass complete"
    );
    Ok(total)
}

/// Background worker that periodically reaps stale `kpi_events` rows.
///
/// Runs `reap_once` at startup, then on every `interval_secs` tick.
/// The reaper body acquires the shared `DaemonState` lock for the duration
/// of the DELETE batches. `reap_once` uses `std::thread::sleep` between
/// batches, which can block the executor if this task is multiplexed on
/// a tokio worker thread — to keep the reactor free we wrap the sync call
/// in `tokio::task::spawn_blocking`, and move the write-path access into
/// a fresh `Connection::open(db_path)` so we don't have to send the
/// non-Send `DaemonState` handle across threads.
pub async fn run_kpi_reaper(
    _state: Arc<Mutex<DaemonState>>,
    db_path: String,
    mut shutdown_rx: watch::Receiver<bool>,
    interval_secs: u64,
    retention_days: u32,
    retention_by_type: HashMap<String, u32>,
) {
    tracing::info!(
        target: "forge::kpi_reaper",
        interval_s = interval_secs,
        retention_days,
        override_keys = retention_by_type.len(),
        "kpi_reaper started"
    );

    // Startup pass — clear any backlog from a previously-running daemon
    // whose reaper never fired (e.g., T7 rolled out after weeks of run).
    run_reap_blocking(&db_path, retention_days, &retention_by_type).await;

    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval_secs)) => {
                run_reap_blocking(&db_path, retention_days, &retention_by_type).await;
            }
            _ = shutdown_rx.changed() => {
                tracing::info!(target: "forge::kpi_reaper", "shutdown received");
                return;
            }
        }
    }
}

async fn run_reap_blocking(
    db_path: &str,
    retention_days: u32,
    retention_by_type: &HashMap<String, u32>,
) {
    let db_path = db_path.to_string();
    let retention_by_type = retention_by_type.clone();
    let join = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
        // Open a dedicated writer connection. SQLite WAL mode serializes
        // concurrent writers internally, so this is safe alongside the
        // WriterActor + worker state connection.
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        reap_once(&conn, retention_days, &retention_by_type)
    })
    .await;

    match join {
        Ok(Ok(n)) => {
            if n > 0 {
                tracing::info!(target: "forge::kpi_reaper", deleted = n, "pass finished");
            }
        }
        Ok(Err(e)) => {
            tracing::error!(target: "forge::kpi_reaper", error = %e, "reap failed");
        }
        Err(e) => {
            tracing::error!(target: "forge::kpi_reaper", error = %e, "reap task panicked");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;

    fn setup_conn() -> Connection {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    fn insert_kpi_event(conn: &Connection, id: &str, ts: i64) {
        conn.execute(
            "INSERT INTO kpi_events (id, timestamp, event_type, success) \
             VALUES (?1, ?2, 'phase_completed', 1)",
            rusqlite::params![id, ts],
        )
        .unwrap();
    }

    fn insert_kpi_event_typed(conn: &Connection, id: &str, ts: i64, event_type: &str) {
        conn.execute(
            "INSERT INTO kpi_events (id, timestamp, event_type, success) \
             VALUES (?1, ?2, ?3, 1)",
            rusqlite::params![id, ts, event_type],
        )
        .unwrap();
    }

    fn empty_map() -> HashMap<String, u32> {
        HashMap::new()
    }

    fn bench_180_map() -> HashMap<String, u32> {
        let mut m = HashMap::new();
        m.insert("bench_run_completed".to_string(), 180);
        m
    }

    fn row_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM kpi_events", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn reap_once_deletes_rows_older_than_cutoff() {
        let conn = setup_conn();
        let now = now_secs() as i64;
        let retention_days: u32 = 30;
        // Seed 100 rows well past the cutoff.
        let old_ts = now - (60 * 86_400);
        for i in 0..100 {
            insert_kpi_event(&conn, &format!("old-{i}"), old_ts);
        }
        // Seed 10 rows within the retention window.
        let fresh_ts = now - 3600;
        for i in 0..10 {
            insert_kpi_event(&conn, &format!("fresh-{i}"), fresh_ts);
        }
        assert_eq!(row_count(&conn), 110);

        let deleted = reap_once(&conn, retention_days, &empty_map()).unwrap();
        assert_eq!(deleted, 100);
        assert_eq!(row_count(&conn), 10);

        // Verify only fresh rows remain.
        let fresh_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kpi_events WHERE id LIKE 'fresh-%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fresh_remaining, 10);
    }

    #[test]
    fn reap_once_respects_batch_size() {
        // Seed more than BATCH_SIZE rows to force multiple batches.
        //
        // 25_000 > 10_000 (BATCH_SIZE) => three batches:
        //   batch 1: 10_000 rows deleted (n == BATCH_SIZE → keep looping)
        //   batch 2: 10_000 rows deleted (n == BATCH_SIZE → keep looping)
        //   batch 3:  5_000 rows deleted (n < BATCH_SIZE  → break)
        let conn = setup_conn();
        let now = now_secs() as i64;
        let old_ts = now - (60 * 86_400);
        const N: usize = BATCH_SIZE * 2 + BATCH_SIZE / 2;
        // Use a single transaction to speed up the insert loop; this is a
        // test, the production writer takes a different path.
        conn.execute_batch("BEGIN").unwrap();
        for i in 0..N {
            insert_kpi_event(&conn, &format!("old-{i}"), old_ts);
        }
        conn.execute_batch("COMMIT").unwrap();
        assert_eq!(row_count(&conn), N as i64);

        let deleted = reap_once(&conn, 30, &empty_map()).unwrap();
        assert_eq!(deleted, N);
        assert_eq!(row_count(&conn), 0);
    }

    #[test]
    fn reap_once_ignores_fresh_rows() {
        let conn = setup_conn();
        let now = now_secs() as i64;
        // 50 rows all within the retention window.
        let fresh_ts = now - 60; // one minute ago
        for i in 0..50 {
            insert_kpi_event(&conn, &format!("fresh-{i}"), fresh_ts);
        }
        assert_eq!(row_count(&conn), 50);

        let deleted = reap_once(&conn, 30, &empty_map()).unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(row_count(&conn), 50);
    }

    #[test]
    fn reap_once_handles_empty_table() {
        let conn = setup_conn();
        assert_eq!(row_count(&conn), 0);
        let deleted = reap_once(&conn, 30, &empty_map()).unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(row_count(&conn), 0);
    }

    #[test]
    fn reap_once_respects_custom_retention() {
        // A 1-day retention window with a row 2 days old => deleted.
        // A 1-day retention window with a row 1 hour old => kept.
        let conn = setup_conn();
        let now = now_secs() as i64;
        insert_kpi_event(&conn, "two-day-old", now - (2 * 86_400));
        insert_kpi_event(&conn, "one-hour-old", now - 3600);

        let deleted = reap_once(&conn, 1, &empty_map()).unwrap();
        assert_eq!(deleted, 1);
        let remaining: String = conn
            .query_row("SELECT id FROM kpi_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, "one-hour-old");
    }

    // Phase 2A-4d.3 T11 (D9): per-event-type retention overrides.

    #[test]
    fn test_bench_run_completed_survives_31_days() {
        // With override map={"bench_run_completed": 180}:
        //   - 31-day-old bench_run_completed row => kept (within 180d).
        //   - 31-day-old phase_completed row => deleted (past 30d global).
        let conn = setup_conn();
        let now = now_secs() as i64;
        let ts_31d = now - (31 * 86_400);
        insert_kpi_event_typed(&conn, "bench-31d", ts_31d, "bench_run_completed");
        insert_kpi_event_typed(&conn, "phase-31d", ts_31d, "phase_completed");
        assert_eq!(row_count(&conn), 2);

        let deleted = reap_once(&conn, 30, &bench_180_map()).unwrap();
        assert_eq!(deleted, 1);

        // Bench row survives, phase row deleted.
        let surviving_id: String = conn
            .query_row("SELECT id FROM kpi_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(surviving_id, "bench-31d");
    }

    #[test]
    fn test_bench_run_completed_reaped_at_181_days() {
        // 181-day-old bench_run_completed row => deleted (past 180d override).
        let conn = setup_conn();
        let now = now_secs() as i64;
        let ts_181d = now - (181 * 86_400);
        insert_kpi_event_typed(&conn, "bench-181d", ts_181d, "bench_run_completed");
        assert_eq!(row_count(&conn), 1);

        let deleted = reap_once(&conn, 30, &bench_180_map()).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(row_count(&conn), 0);
    }

    #[test]
    fn test_global_default_30d_applies_to_other_types() {
        // No override map: a 31-day-old phase_completed row falls past the
        // 30-day global default and is deleted.
        let conn = setup_conn();
        let now = now_secs() as i64;
        let ts_31d = now - (31 * 86_400);
        insert_kpi_event_typed(&conn, "phase-31d", ts_31d, "phase_completed");
        assert_eq!(row_count(&conn), 1);

        let deleted = reap_once(&conn, 30, &empty_map()).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(row_count(&conn), 0);
    }
}
