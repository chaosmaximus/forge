//! Bench telemetry — emit `bench_run_completed` rows into `kpi_events`.
//!
//! Phase 2A-4d.3 T8: per spec §3.2, every bench run writes a single row
//! into `kpi_events` with `event_type = 'bench_run_completed'` and a
//! v1 `metadata_json` schema describing the run (bench name, seed,
//! dimensions, composite, hardware/commit context, ...).
//!
//! This is a side-effect-only helper used by `forge-bench` after each
//! sub-bench produces its `summary.json`. It opens a short-lived
//! `rusqlite::Connection` to `${FORGE_DIR}/forge.db`, INSERTs the row,
//! and closes the connection. No state is shared with the bench harness
//! (whose scoring DB is typically `:memory:`). Concurrent-writer safety
//! comes from WAL mode + a 5s `busy_timeout`.
//!
//! When `FORGE_DIR` is unset the function is a no-op (CI misconfig is
//! visible via a one-shot stderr note, mirroring the Tier 2 reaper
//! precedent). All other failures are returned as `Result<(), String>`
//! and the caller is expected to log them via `tracing::warn!` rather
//! than fail the bench — telemetry is observability, not correctness.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// Per-dimension entry inside `metadata_json.dimensions`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DimensionEntry {
    pub name: String,
    pub score: f64,
    pub min: f64,
    pub pass: bool,
}

/// All inputs needed to emit one `bench_run_completed` row.
#[derive(Debug, Clone)]
pub struct BenchRunPayload {
    pub bench_name: String,
    pub seed: u64,
    pub composite: f64,
    pub pass: bool,
    pub dimensions: Vec<DimensionEntry>,
    pub dimension_scores: HashMap<String, f64>,
    pub bench_specific_stats: serde_json::Value,
    pub wall_duration_ms: u64,
    pub result_count: u64,
}

/// Detect the canonical hardware-profile string used across spec §3.2.
///
/// Order:
///   1. `FORGE_HARDWARE_PROFILE` if set (allows ops override)
///   2. `ubuntu-latest-ci` / `macos-latest-ci` if `GITHUB_ACTIONS == "true"`
///   3. `local` otherwise
pub fn detect_hardware_profile() -> String {
    if let Ok(p) = std::env::var("FORGE_HARDWARE_PROFILE") {
        if !p.is_empty() {
            return p;
        }
    }
    if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        // GitHub Actions sets RUNNER_OS to "Linux" / "macOS" / "Windows".
        match std::env::var("RUNNER_OS").as_deref() {
            Ok("macOS") => return "macos-latest-ci".to_string(),
            _ => return "ubuntu-latest-ci".to_string(),
        }
    }
    "local".to_string()
}

/// Detect commit SHA, dirty flag, and commit timestamp (unix secs).
///
/// Order for SHA:
///   1. `GITHUB_SHA` if set
///   2. `git rev-parse HEAD` (no-fail; returns `None` if not a git repo)
///
/// Dirty flag: `git status --porcelain` non-empty (best-effort; `false` if git fails).
/// Commit timestamp: `git show -s --format=%ct HEAD` (best-effort).
pub fn detect_commit_metadata() -> (Option<String>, bool, Option<i64>) {
    let sha = match std::env::var("GITHUB_SHA") {
        Ok(s) if !s.is_empty() => Some(s),
        _ => Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s)
                    }
                } else {
                    None
                }
            }),
    };

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| o.status.success() && !o.stdout.is_empty())
        .unwrap_or(false);

    let commit_ts = Command::new("git")
        .args(["show", "-s", "--format=%ct", "HEAD"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<i64>()
                    .ok()
            } else {
                None
            }
        });

    (sha, dirty, commit_ts)
}

/// One-shot stderr note when FORGE_DIR is unset — mirrors the Tier 2
/// reaper precedent so CI misconfig is visible without spamming logs.
fn note_forge_dir_unset_once() {
    static NOTED: OnceLock<()> = OnceLock::new();
    NOTED.get_or_init(|| {
        eprintln!("forge-bench telemetry: FORGE_DIR unset — bench_run_completed event NOT emitted");
    });
}

/// Emit a single `bench_run_completed` row into `${FORGE_DIR}/forge.db`.
///
/// No-op (returns `Ok(())`) when `FORGE_DIR` is unset; logs a one-shot
/// stderr note so CI misconfig stays visible.
///
/// On the happy path:
///   * Opens a short-lived rusqlite connection
///   * Sets WAL mode + `busy_timeout = 5000ms` for concurrent-writer safety
///   * Calls the daemon's idempotent `create_schema` so the table exists
///     even when this is the first time anything has touched the DB
///     (CI starts from a clean tempdir; the daemon may not have run yet)
///   * Builds the v1 `metadata_json` blob
///   * INSERTs one row keyed by a fresh ULID
pub fn emit_bench_run_completed(payload: &BenchRunPayload) -> Result<(), String> {
    let forge_dir = match std::env::var("FORGE_DIR") {
        Ok(s) if !s.is_empty() => PathBuf::from(s),
        _ => {
            note_forge_dir_unset_once();
            return Ok(());
        }
    };

    std::fs::create_dir_all(&forge_dir)
        .map_err(|e| format!("create FORGE_DIR {}: {e}", forge_dir.display()))?;
    let db_path = forge_dir.join("forge.db");

    // Register the sqlite-vec virtual-table module BEFORE opening the
    // connection — `create_schema` declares `memory_vec` (vec0) and
    // fails on a connection that hasn't seen the extension. Idempotent
    // (safe to call repeatedly).
    crate::db::vec::init_sqlite_vec();

    let conn =
        Connection::open(&db_path).map_err(|e| format!("open {}: {e}", db_path.display()))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("set WAL: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(|e| format!("busy_timeout: {e}"))?;

    // Idempotent — no-op if the daemon already initialized the schema.
    crate::db::schema::create_schema(&conn).map_err(|e| format!("create_schema: {e}"))?;

    let (commit_sha, commit_dirty, commit_ts) = detect_commit_metadata();
    let hardware_profile = detect_hardware_profile();
    let run_id = ulid::Ulid::new().to_string();

    let metadata = serde_json::json!({
        "event_schema_version": 1,
        "bench_name": payload.bench_name,
        "seed": payload.seed,
        "composite": payload.composite,
        "pass": payload.pass,
        "dimensions": payload.dimensions,
        "dimension_scores": payload.dimension_scores,
        "commit_sha": commit_sha,
        "commit_dirty": commit_dirty,
        "commit_timestamp_secs": commit_ts,
        "hardware_profile": hardware_profile,
        "run_id": run_id,
        "bench_specific_stats": payload.bench_specific_stats,
    });

    let metadata_json =
        serde_json::to_string(&metadata).map_err(|e| format!("serialize metadata: {e}"))?;
    let timestamp = crate::db::ops::current_epoch_secs() as i64;
    let latency_ms = payload.wall_duration_ms as i64;
    let result_count = payload.result_count as i64;
    let success = i64::from(payload.pass);

    conn.execute(
        "INSERT INTO kpi_events
           (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)
         VALUES (?1, ?2, 'bench_run_completed', NULL, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            run_id,
            timestamp,
            latency_ms,
            result_count,
            success,
            metadata_json,
        ],
    )
    .map_err(|e| format!("INSERT kpi_events: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn sample_payload() -> BenchRunPayload {
        let mut scores = HashMap::new();
        scores.insert("dim_a".to_string(), 0.8);
        scores.insert("dim_b".to_string(), 0.9);
        BenchRunPayload {
            bench_name: "test-bench".to_string(),
            seed: 42,
            composite: 0.85,
            pass: true,
            dimensions: vec![
                DimensionEntry {
                    name: "dim_a".to_string(),
                    score: 0.8,
                    min: 0.7,
                    pass: true,
                },
                DimensionEntry {
                    name: "dim_b".to_string(),
                    score: 0.9,
                    min: 0.7,
                    pass: true,
                },
            ],
            dimension_scores: scores,
            bench_specific_stats: serde_json::json!({"foo": "bar"}),
            wall_duration_ms: 1234,
            result_count: 2,
        }
    }

    #[test]
    #[serial]
    fn emit_no_op_when_forge_dir_unset() {
        // SAFETY: tests run with serial_test serialization; this test
        // owns FORGE_DIR for its duration.
        let prev = std::env::var("FORGE_DIR").ok();
        std::env::remove_var("FORGE_DIR");

        let payload = sample_payload();
        let result = emit_bench_run_completed(&payload);
        assert!(result.is_ok(), "expected no-op, got {result:?}");

        if let Some(p) = prev {
            std::env::set_var("FORGE_DIR", p);
        }
    }

    #[test]
    #[serial]
    fn emit_inserts_one_row_when_forge_dir_set() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let prev = std::env::var("FORGE_DIR").ok();
        std::env::set_var("FORGE_DIR", tmp.path());

        let payload = sample_payload();
        let result = emit_bench_run_completed(&payload);
        assert!(result.is_ok(), "emit failed: {result:?}");

        let db_path = tmp.path().join("forge.db");
        assert!(db_path.exists(), "expected forge.db at {db_path:?}");

        let conn = Connection::open(&db_path).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kpi_events WHERE event_type = 'bench_run_completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        let success: i64 = conn
            .query_row(
                "SELECT success FROM kpi_events WHERE event_type = 'bench_run_completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(success, 1);

        let result_count: i64 = conn
            .query_row(
                "SELECT result_count FROM kpi_events WHERE event_type = 'bench_run_completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(result_count, 2);

        // Restore env for sibling tests.
        match prev {
            Some(p) => std::env::set_var("FORGE_DIR", p),
            None => std::env::remove_var("FORGE_DIR"),
        }
    }

    #[test]
    fn payload_serializes_with_v1_schema() {
        let payload = sample_payload();
        let mut scores = HashMap::new();
        scores.insert("dim_a".to_string(), 0.8);

        let v = serde_json::json!({
            "event_schema_version": 1,
            "bench_name": payload.bench_name,
            "seed": payload.seed,
            "composite": payload.composite,
            "pass": payload.pass,
            "dimensions": payload.dimensions,
            "dimension_scores": scores,
            "commit_sha": Option::<String>::None,
            "commit_dirty": false,
            "commit_timestamp_secs": Option::<i64>::None,
            "hardware_profile": "local",
            "run_id": "01XXX",
            "bench_specific_stats": payload.bench_specific_stats,
        });
        let s = serde_json::to_string(&v).expect("serialize");
        assert!(s.contains("\"event_schema_version\":1"));
        assert!(s.contains("\"bench_name\":\"test-bench\""));
        let round: serde_json::Value = serde_json::from_str(&s).expect("round-trip");
        assert_eq!(round["event_schema_version"], 1);
    }

    #[test]
    #[serial]
    fn detect_hardware_profile_handles_local() {
        let prev_force = std::env::var("FORGE_HARDWARE_PROFILE").ok();
        let prev_actions = std::env::var("GITHUB_ACTIONS").ok();
        std::env::remove_var("FORGE_HARDWARE_PROFILE");
        std::env::remove_var("GITHUB_ACTIONS");

        assert_eq!(detect_hardware_profile(), "local");

        if let Some(p) = prev_force {
            std::env::set_var("FORGE_HARDWARE_PROFILE", p);
        }
        if let Some(p) = prev_actions {
            std::env::set_var("GITHUB_ACTIONS", p);
        }
    }

    #[test]
    #[serial]
    fn detect_commit_metadata_returns_none_outside_git() {
        // Run from a tempdir so `git rev-parse HEAD` fails — and clear
        // GITHUB_SHA so the env-var fallback can't shadow the test.
        let tmp = tempfile::tempdir().expect("tempdir");
        let prev_cwd = std::env::current_dir().expect("cwd");
        let prev_sha = std::env::var("GITHUB_SHA").ok();
        std::env::remove_var("GITHUB_SHA");
        std::env::set_current_dir(tmp.path()).expect("chdir tempdir");

        let (sha, _dirty, ts) = detect_commit_metadata();
        assert!(
            sha.is_none(),
            "expected no SHA in non-git tempdir, got {sha:?}"
        );
        assert!(
            ts.is_none(),
            "expected no commit_ts in non-git tempdir, got {ts:?}"
        );

        // Restore cwd + env regardless of outcome.
        std::env::set_current_dir(&prev_cwd).expect("restore cwd");
        if let Some(p) = prev_sha {
            std::env::set_var("GITHUB_SHA", p);
        }
    }
}
