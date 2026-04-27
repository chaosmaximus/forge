// db/pragma.rs — Canonical runtime PRAGMAs for every Forge SQLite
// connection.
//
// Why this exists (W23 review MED-4 / P3-4 W1.30):
//
// Forge opens SQLite connections from many sites — `DaemonState::new`
// (writer-actor's connection), `DaemonState::new_writer`, the read-only
// per-request handles in `new_reader`, the kpi_reaper background pass,
// the force-index spawn_blocking task, the Wave X auto-create one-shot,
// the consolidator, the bench telemetry, etc. Pre-fix each site
// open-coded its own PRAGMA batch:
//
//   * `DaemonState::new`        → `journal_mode=WAL; busy_timeout=5000;`
//   * `DaemonState::new_writer` → `journal_mode=WAL; busy_timeout=5000;`
//   * `DaemonState::new_reader` → `journal_mode=WAL; busy_timeout=5000;`
//   * `kpi_reaper`              → `journal_mode=WAL; busy_timeout=5000;`
//   * `force-index` (W22)       → `journal_mode=WAL; busy_timeout=10000;`
//   * Auto-create (X1)          → `journal_mode=WAL; busy_timeout=10000;`
//
// Two values for `busy_timeout` (5000 ms vs 10000 ms) crept in without a
// documented rationale — W22 picked 10000 for the heavy indexer pass,
// kpi_reaper used 5000, and X1 inherited 10000 from the indexer
// precedent. Functionally each site is fine in isolation, but the
// pattern makes future drift likely (next contributor copies whichever
// site they happened to read first), and the divergence makes
// "this query timed out at busy_timeout" debugging harder.
//
// Fix: one helper, one canonical value, one source of truth. Every
// `Connection::open` site calls `apply_runtime_pragmas(&conn)` instead
// of running a literal PRAGMA string. The helper:
//
//   1. Sets `journal_mode=WAL`. WAL is a persistent file-level setting
//      (the WAL header survives connection close), so this is a
//      no-op on already-WAL DBs but inexpensive (~µs). Ensuring it
//      every open guards against the rare case where a fresh DB file
//      was created without WAL — e.g., a test that copies a stock
//      `forge.db` into a tempdir, or a backup restore. Returning
//      "wal" verifies the mode actually engaged; we treat any other
//      result as `tracing::warn!`-worthy because lock contention on
//      non-WAL DBs is much sharper.
//   2. Sets `busy_timeout=10000` (10 seconds). The 10-second value
//      matches the heaviest-write site (force-index) so all sites
//      retry SQLITE_BUSY identically. This is the upper bound; sites
//      that legitimately want shorter timeouts can set their own
//      AFTER calling `apply_runtime_pragmas`. (Today none do.)
//
// The helper is `Result`-typed so callers can decide whether a PRAGMA
// failure is fatal (e.g. `DaemonState::new`'s schema-create flow MUST
// have WAL or the multi-conn pattern breaks) or best-effort (e.g.
// per-request reader handles that already gracefully degrade on
// `Connection::open` errors).
//
// Unit tests live in this file's `mod tests`. A CI-level grep that
// every production `Connection::open` site routes through this helper
// (instead of open-coding a PRAGMA literal) is a follow-up — could be
// scripts/check-pragma-consistency.sh in the next polish wave; for now
// the W1.30 commit touched all sites by hand and the helper is the
// only path forward.

use rusqlite::Connection;

/// Canonical busy-timeout for every Forge SQLite connection.
///
/// 10 seconds covers the heaviest-write site (the W22 indexer pass +
/// the consolidator's batched commits). Sites that want a shorter
/// timeout can override AFTER calling `apply_runtime_pragmas`, but
/// none do today; centralizing the default avoids the 5000-vs-10000
/// drift the W23 review surfaced.
pub const BUSY_TIMEOUT_MS: u32 = 10_000;

/// Apply Forge's canonical runtime PRAGMAs to a freshly-opened SQLite
/// connection.
///
/// Sets `journal_mode=WAL` and `busy_timeout=10000`. WAL is persistent
/// at the file level, so this is a no-op on already-WAL DBs; the
/// busy_timeout is per-connection and must be set every time. Returns
/// `Ok(())` on success or a `rusqlite::Error` if either PRAGMA fails.
///
/// Callers MUST decide whether to treat a failure as fatal (writer
/// connections + schema-create flows) or best-effort (read-only
/// per-request handles that already gracefully degrade).
pub fn apply_runtime_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    // PRAGMA journal_mode returns the new mode as a string row; we
    // verify it's "wal" because the helper's contract is "ensure WAL
    // is engaged after this call." A non-"wal" result usually means
    // the file is locked by another process at journal-mode-change
    // time (rare but observed during concurrent `cargo test` runs
    // pre-WAL persistence).
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| {
        row.get::<_, String>(0)
    })?;
    if !mode.eq_ignore_ascii_case("wal") {
        tracing::warn!(
            target: "forge::db",
            mode = %mode,
            "PRAGMA journal_mode=WAL did not engage; lock contention will be sharper than expected"
        );
    }
    // PRAGMA busy_timeout doesn't return a row; use `execute_batch`.
    conn.execute_batch(&format!("PRAGMA busy_timeout = {BUSY_TIMEOUT_MS};"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_runtime_pragmas_engages_wal_and_timeout() {
        // Use a tempfile DB — `:memory:` has its own pragma quirks
        // (WAL is not supported on in-memory DBs in the same way).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = Connection::open(&path).unwrap();
        apply_runtime_pragmas(&conn).unwrap();

        // Verify journal_mode is WAL.
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "wal");

        // Verify busy_timeout matches the canonical value.
        let timeout: i64 = conn
            .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
            .unwrap();
        assert_eq!(timeout, BUSY_TIMEOUT_MS as i64);
    }

    #[test]
    fn apply_runtime_pragmas_is_idempotent() {
        // Calling the helper twice must succeed and leave the DB in
        // the same state. WAL is persistent so the second call is a
        // no-op for journal_mode; busy_timeout is overwritten with
        // the same value.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let conn = Connection::open(&path).unwrap();
        apply_runtime_pragmas(&conn).unwrap();
        apply_runtime_pragmas(&conn).unwrap();

        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_ascii_lowercase(), "wal");
    }
}
