//! Phase 2A-4d.2 T3 — Observability API handler.
//!
//! Serves `Request::Inspect { shape, window, filter, group_by }` by dispatching
//! to one of 5 shape handlers and returning a `ResponseData::Inspect`. Reads
//! `kpi_events` (with the `idx_kpi_events_phase` expression index) plus, for
//! `shape=row_count`, the atomic `GaugeSnapshot` written by `ForgeMetrics::refresh_gauges`.
//!
//! The `row_count` shape returns zero-valued rows and `stale: true` when the
//! snapshot has never been refreshed — T4 wires up the snapshot; T3 reads it.
//!
//! Window grammar uses `humantime::parse_duration` with a 7-day ceiling.
//!
//! Row cap: each non-row-count shape selects up to `MAX_ROWS_PER_GROUP` rows
//! per group plus an absolute `MAX_TOTAL_ROWS` ceiling. When any limit is hit,
//! `truncated: true` is set on the response and `truncated_samples` on the
//! affected latency row (`LatencyRow` only — other shapes are strictly
//! summary aggregates that the cap doesn't affect in the current design).

use forge_core::protocol::{
    ErrorRateRow, InspectData, InspectFilter, InspectGroupBy, InspectShape, LatencyRow,
    PhaseRunRow, Response, ResponseData, ThroughputRow,
};
use rusqlite::{named_params, Connection};

/// Hard ceiling for window duration. Anything longer is rejected.
const MAX_WINDOW_SECS: u64 = 7 * 86_400;

/// Soft cap for per-group latency sample counts. Latency queries pull at
/// most this many rows per group_key before computing percentiles.
const MAX_ROWS_PER_GROUP: u64 = 20_000;

/// Absolute ceiling on total rows loaded for latency shape (per query).
const MAX_TOTAL_ROWS: u64 = 200_000;

/// Max rows returned by `phase_run_summary` shape (hard-coded; future: paginate).
const PHASE_RUN_SUMMARY_LIMIT: u64 = 100;

/// Staleness threshold for `row_count` gauge snapshot.
const GAUGE_STALE_SECS: u64 = 60;

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Parse the `window` string into a count of seconds. Accepts any
/// `humantime::parse_duration`-valid form (`5m`, `1h30m`, `2h`, `7d`, etc.)
/// as long as the total is >0 and ≤ 7 days.
pub fn parse_window_secs(s: &str) -> Result<u64, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("window is empty".to_string());
    }
    let dur = humantime::parse_duration(trimmed)
        .map_err(|e| format!("invalid window '{s}': {e}"))?;
    let secs = dur.as_secs();
    if secs == 0 {
        return Err(format!("window '{s}' parses to zero duration"));
    }
    if secs > MAX_WINDOW_SECS {
        return Err(format!(
            "window '{s}' exceeds 7-day ceiling ({secs}s > {MAX_WINDOW_SECS}s)"
        ));
    }
    Ok(secs)
}

/// Validate `shape × group_by`. Returns the effective group_by to use
/// (handler default when the caller supplied `None`).
pub fn resolve_group_by(
    shape: InspectShape,
    provided: Option<InspectGroupBy>,
) -> Result<Option<InspectGroupBy>, String> {
    use InspectGroupBy::*;
    use InspectShape::*;
    match (shape, provided) {
        (RowCount, None) => Ok(None),
        (RowCount, Some(g)) => Err(format!(
            "group_by={g:?} not supported for shape=row_count"
        )),

        (Latency, None) => Ok(Some(Phase)),
        (Latency, Some(Phase)) | (Latency, Some(RunId)) => Ok(provided),
        (Latency, Some(g)) => Err(format!("group_by={g:?} not valid for shape=latency")),

        (ErrorRate, None) => Ok(Some(Phase)),
        (ErrorRate, Some(Phase)) | (ErrorRate, Some(EventType)) => Ok(provided),
        (ErrorRate, Some(g)) => Err(format!("group_by={g:?} not valid for shape=error_rate")),

        (Throughput, None) => Ok(Some(EventType)),
        (Throughput, Some(Phase))
        | (Throughput, Some(EventType))
        | (Throughput, Some(Project)) => Ok(provided),
        (Throughput, Some(g)) => Err(format!("group_by={g:?} not valid for shape=throughput")),

        (PhaseRunSummary, None) => Ok(None),
        (PhaseRunSummary, Some(g)) => Err(format!(
            "group_by={g:?} not supported for shape=phase_run_summary (run_id is implicit)"
        )),
    }
}

/// Which filter fields are honored for each shape. Dropped fields are nulled
/// in the returned `effective_filter`.
fn effective_filter(shape: InspectShape, filter: &InspectFilter) -> InspectFilter {
    let mut out = InspectFilter::default();
    match shape {
        InspectShape::RowCount => {
            out.layer = filter.layer.clone();
        }
        InspectShape::Latency | InspectShape::ErrorRate => {
            out.phase = filter.phase.clone();
            out.event_type = filter.event_type.clone();
            out.project = filter.project.clone();
        }
        InspectShape::Throughput => {
            out.phase = filter.phase.clone();
            out.event_type = filter.event_type.clone();
            out.project = filter.project.clone();
        }
        InspectShape::PhaseRunSummary => {
            // run-summary always operates on phase_completed; filters other
            // than event_type have no effect on the grouping. Honor phase /
            // project as scoping hints.
            out.phase = filter.phase.clone();
            out.event_type = filter.event_type.clone();
            out.project = filter.project.clone();
        }
    }
    out
}

/// Top-level entry. Opens a reader connection at the caller's request level;
/// returns a fully-typed `Response`.
pub fn run_inspect(
    conn: &Connection,
    shape: InspectShape,
    window: String,
    filter: InspectFilter,
    group_by: Option<InspectGroupBy>,
    // Carries the snapshot for shape=row_count. T4 wires this; for T3 callers
    // who don't yet have the snapshot, pass None and the row_count shape
    // returns empty rows + stale: true.
    snapshot: Option<&crate::server::metrics::GaugeSnapshot>,
) -> Response {
    let window_secs = match parse_window_secs(&window) {
        Ok(n) => n,
        Err(e) => return Response::Error { message: e },
    };
    let effective_group_by = match resolve_group_by(shape, group_by) {
        Ok(g) => g,
        Err(e) => return Response::Error { message: e },
    };
    let eff_filter = effective_filter(shape, &filter);
    let event_type = eff_filter
        .event_type
        .clone()
        .unwrap_or_else(|| "phase_completed".to_string());
    let window_start_secs = now_secs().saturating_sub(window_secs);

    let (data, truncated, stale) = match shape {
        InspectShape::RowCount => {
            let (d, stale) = row_count_from_snapshot(snapshot, &eff_filter);
            (d, false, stale)
        }
        InspectShape::Latency => match shape_latency(
            conn,
            window_start_secs,
            &event_type,
            &eff_filter,
            effective_group_by,
        ) {
            Ok((d, t)) => (d, t, false),
            Err(e) => return Response::Error { message: e },
        },
        InspectShape::ErrorRate => match shape_error_rate(
            conn,
            window_start_secs,
            &event_type,
            &eff_filter,
            effective_group_by,
        ) {
            Ok(d) => (d, false, false),
            Err(e) => return Response::Error { message: e },
        },
        InspectShape::Throughput => {
            match shape_throughput(conn, window_start_secs, &eff_filter, effective_group_by) {
                Ok(d) => (d, false, false),
                Err(e) => return Response::Error { message: e },
            }
        }
        InspectShape::PhaseRunSummary => {
            match shape_phase_run_summary(conn, window_start_secs, &eff_filter) {
                Ok(d) => (d, false, false),
                Err(e) => return Response::Error { message: e },
            }
        }
    };

    Response::Ok {
        data: ResponseData::Inspect {
            shape,
            window,
            window_secs,
            generated_at_secs: now_secs(),
            effective_filter: eff_filter,
            effective_group_by,
            stale,
            truncated,
            data,
        },
    }
}

// ─────────────────────────────────────────────────────────────
// Shape: row_count
// ─────────────────────────────────────────────────────────────

fn row_count_from_snapshot(
    snapshot: Option<&crate::server::metrics::GaugeSnapshot>,
    filter: &InspectFilter,
) -> (InspectData, bool) {
    let Some(snap) = snapshot else {
        return (InspectData::RowCount { rows: vec![] }, true);
    };
    let age = now_secs().saturating_sub(snap.refreshed_at_secs);
    let stale = snap.refreshed_at_secs == 0 || age > GAUGE_STALE_SECS;
    let all_rows = snap.tables.to_layer_rows(age);
    let rows = if let Some(wanted) = &filter.layer {
        all_rows
            .into_iter()
            .filter(|r| &r.layer == wanted)
            .collect()
    } else {
        all_rows
    };
    (InspectData::RowCount { rows }, stale)
}

// ─────────────────────────────────────────────────────────────
// Shape: latency
// ─────────────────────────────────────────────────────────────

fn shape_latency(
    conn: &Connection,
    window_start_secs: u64,
    event_type: &str,
    filter: &InspectFilter,
    group_by: Option<InspectGroupBy>,
) -> Result<(InspectData, bool), String> {
    // group_key is either phase_name, run_id, or a synthetic "all" bucket
    // when group_by=None.
    let group_expr = match group_by {
        Some(InspectGroupBy::Phase) => "json_extract(metadata_json, '$.phase_name')",
        Some(InspectGroupBy::RunId) => "json_extract(metadata_json, '$.run_id')",
        None => "'all'",
        Some(other) => return Err(format!("group_by={other:?} not valid for latency")),
    };

    // Pull raw (group_key, latency_ms) with MAX_TOTAL_ROWS ceiling.
    let sql = format!(
        r#"SELECT {group_expr} AS group_key, latency_ms
           FROM kpi_events
           WHERE timestamp >= :window_start_secs
             AND event_type = :event_type
             AND latency_ms IS NOT NULL
             AND (:phase IS NULL OR json_extract(metadata_json, '$.phase_name') = :phase)
             AND (:project IS NULL OR project = :project)
           ORDER BY group_key
           LIMIT :total_cap"#
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare: {e}"))?;
    let rows_iter = stmt
        .query_map(
            named_params! {
                ":window_start_secs": window_start_secs as i64,
                ":event_type": event_type,
                ":phase": filter.phase.as_deref(),
                ":project": filter.project.as_deref(),
                ":total_cap": (MAX_TOTAL_ROWS + 1) as i64,
            },
            |row| {
                let key: Option<String> = row.get(0)?;
                let lat: i64 = row.get(1)?;
                Ok((key.unwrap_or_else(|| "unknown".to_string()), lat.max(0) as u64))
            },
        )
        .map_err(|e| format!("query: {e}"))?;

    // Collect + group in Rust.
    let mut buckets: std::collections::BTreeMap<String, Vec<u64>> =
        std::collections::BTreeMap::new();
    let mut truncated_by_group: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    let mut total_seen: u64 = 0;
    let mut hit_total_cap = false;
    for r in rows_iter {
        let (k, v) = r.map_err(|e| format!("row: {e}"))?;
        total_seen += 1;
        if total_seen > MAX_TOTAL_ROWS {
            hit_total_cap = true;
            break;
        }
        let bucket = buckets.entry(k.clone()).or_default();
        if (bucket.len() as u64) < MAX_ROWS_PER_GROUP {
            bucket.push(v);
        } else {
            *truncated_by_group.entry(k).or_insert(0) += 1;
        }
    }

    let mut rows: Vec<LatencyRow> = buckets
        .into_iter()
        .map(|(group_key, mut samples)| {
            samples.sort_unstable();
            let count = samples.len() as u64;
            let truncated_samples = truncated_by_group.remove(&group_key).unwrap_or(0);
            let mean_ms = if samples.is_empty() {
                0.0
            } else {
                samples.iter().sum::<u64>() as f64 / samples.len() as f64
            };
            LatencyRow {
                group_key,
                count,
                p50_ms: percentile(&samples, 0.50),
                p95_ms: percentile(&samples, 0.95),
                p99_ms: percentile(&samples, 0.99),
                mean_ms,
                truncated_samples,
            }
        })
        .collect();
    // Stable output ordering by group_key.
    rows.sort_by(|a, b| a.group_key.cmp(&b.group_key));

    let truncated = hit_total_cap || rows.iter().any(|r| r.truncated_samples > 0);
    Ok((InspectData::Latency { rows }, truncated))
}

/// Ceiling-rank percentile on an already-sorted slice of u64 (in ms).
fn percentile(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    let idx = rank.clamp(1, sorted.len()) - 1;
    sorted[idx] as f64
}

// ─────────────────────────────────────────────────────────────
// Shape: error_rate
// ─────────────────────────────────────────────────────────────

fn shape_error_rate(
    conn: &Connection,
    window_start_secs: u64,
    event_type: &str,
    filter: &InspectFilter,
    group_by: Option<InspectGroupBy>,
) -> Result<InspectData, String> {
    let group_expr = match group_by {
        Some(InspectGroupBy::Phase) => "json_extract(metadata_json, '$.phase_name')",
        Some(InspectGroupBy::EventType) => "event_type",
        None => "'all'",
        Some(other) => return Err(format!("group_by={other:?} not valid for error_rate")),
    };
    let sql = format!(
        r#"SELECT {group_expr} AS group_key,
                  SUM(CASE WHEN COALESCE(json_extract(metadata_json, '$.error_count'), 0) > 0
                            THEN 1 ELSE 0 END) AS errored,
                  COUNT(*) AS total
           FROM kpi_events
           WHERE timestamp >= :window_start_secs
             AND event_type = :event_type
             AND (:phase IS NULL OR json_extract(metadata_json, '$.phase_name') = :phase)
             AND (:project IS NULL OR project = :project)
           GROUP BY group_key
           HAVING total > 0
           ORDER BY group_key"#
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare: {e}"))?;
    let rows: Vec<ErrorRateRow> = stmt
        .query_map(
            named_params! {
                ":window_start_secs": window_start_secs as i64,
                ":event_type": event_type,
                ":phase": filter.phase.as_deref(),
                ":project": filter.project.as_deref(),
            },
            |row| {
                let group_key: Option<String> = row.get(0)?;
                let errored: i64 = row.get(1)?;
                let total: i64 = row.get(2)?;
                let total = total.max(0) as u64;
                let errored = errored.max(0) as u64;
                let rate = if total == 0 {
                    0.0
                } else {
                    errored as f64 / total as f64
                };
                Ok(ErrorRateRow {
                    group_key: group_key.unwrap_or_else(|| "unknown".to_string()),
                    total,
                    errored,
                    rate,
                })
            },
        )
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("row: {e}"))?;
    Ok(InspectData::ErrorRate { rows })
}

// ─────────────────────────────────────────────────────────────
// Shape: throughput
// ─────────────────────────────────────────────────────────────

fn shape_throughput(
    conn: &Connection,
    window_start_secs: u64,
    filter: &InspectFilter,
    group_by: Option<InspectGroupBy>,
) -> Result<InspectData, String> {
    let group_expr = match group_by {
        Some(InspectGroupBy::Phase) => "json_extract(metadata_json, '$.phase_name')",
        Some(InspectGroupBy::EventType) => "event_type",
        Some(InspectGroupBy::Project) => "COALESCE(project, '')",
        None => "'all'",
        Some(other) => return Err(format!("group_by={other:?} not valid for throughput")),
    };
    let sql = format!(
        r#"SELECT {group_expr} AS group_key,
                  COUNT(*) AS count,
                  MIN(timestamp) AS first_ts_secs,
                  MAX(timestamp) AS last_ts_secs
           FROM kpi_events
           WHERE timestamp >= :window_start_secs
             AND (:phase IS NULL OR json_extract(metadata_json, '$.phase_name') = :phase)
             AND (:project IS NULL OR project = :project)
             AND (:event_type IS NULL OR event_type = :event_type)
           GROUP BY group_key
           HAVING count > 0
           ORDER BY group_key"#
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare: {e}"))?;
    let rows: Vec<ThroughputRow> = stmt
        .query_map(
            named_params! {
                ":window_start_secs": window_start_secs as i64,
                ":phase": filter.phase.as_deref(),
                ":project": filter.project.as_deref(),
                ":event_type": filter.event_type.as_deref(),
            },
            |row| {
                let group_key: Option<String> = row.get(0)?;
                let count: i64 = row.get(1)?;
                let first_ts: i64 = row.get(2)?;
                let last_ts: i64 = row.get(3)?;
                Ok(ThroughputRow {
                    group_key: group_key.unwrap_or_else(|| "unknown".to_string()),
                    count: count.max(0) as u64,
                    first_ts_secs: first_ts,
                    last_ts_secs: last_ts,
                })
            },
        )
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("row: {e}"))?;
    Ok(InspectData::Throughput { rows })
}

// ─────────────────────────────────────────────────────────────
// Shape: phase_run_summary
// ─────────────────────────────────────────────────────────────

fn shape_phase_run_summary(
    conn: &Connection,
    window_start_secs: u64,
    filter: &InspectFilter,
) -> Result<InspectData, String> {
    let sql = r#"
        SELECT
            json_extract(metadata_json, '$.run_id') AS run_id,
            MIN(timestamp) AS start_ts_secs,
            SUM(COALESCE(latency_ms, 0)) AS phases_duration_ms_sum,
            COUNT(*) AS phase_count,
            SUM(COALESCE(json_extract(metadata_json, '$.error_count'), 0)) AS error_count,
            MAX(json_extract(metadata_json, '$.trace_id')) AS trace_id,
            MAX(json_extract(metadata_json, '$.correlation_id')) AS correlation_id
        FROM kpi_events
        WHERE timestamp >= :window_start_secs
          AND event_type = 'phase_completed'
          AND (:project IS NULL OR project = :project)
          AND (:phase IS NULL OR json_extract(metadata_json, '$.phase_name') = :phase)
        GROUP BY run_id
        HAVING run_id IS NOT NULL
        ORDER BY start_ts_secs DESC
        LIMIT :limit
    "#;
    let mut stmt = conn.prepare(sql).map_err(|e| format!("prepare: {e}"))?;
    let rows: Vec<PhaseRunRow> = stmt
        .query_map(
            named_params! {
                ":window_start_secs": window_start_secs as i64,
                ":project": filter.project.as_deref(),
                ":phase": filter.phase.as_deref(),
                ":limit": PHASE_RUN_SUMMARY_LIMIT as i64,
            },
            |row| {
                let run_id: Option<String> = row.get(0)?;
                let start_ts: i64 = row.get(1)?;
                let dur_sum: i64 = row.get(2)?;
                let phase_count: i64 = row.get(3)?;
                let error_count: i64 = row.get(4)?;
                let trace_id: Option<String> = row.get(5)?;
                let correlation_id: Option<String> = row.get(6)?;
                Ok(PhaseRunRow {
                    run_id: run_id.unwrap_or_default(),
                    start_ts_secs: start_ts,
                    phases_duration_ms_sum: dur_sum.max(0) as u64,
                    phase_count: phase_count.max(0) as u64,
                    error_count: error_count.max(0) as u64,
                    trace_id,
                    correlation_id,
                })
            },
        )
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("row: {e}"))?;
    Ok(InspectData::PhaseRunSummary { rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::create_schema;

    fn seed_conn() -> Connection {
        // sqlite-vec registers the `vec0` virtual-table module that
        // create_schema depends on; must run before any connection is
        // opened in this process.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        conn
    }

    fn insert_phase_event(
        conn: &Connection,
        id: &str,
        ts: i64,
        phase: &str,
        run_id: &str,
        latency_ms: i64,
        error_count: i64,
        trace_id: Option<&str>,
    ) {
        let metadata = serde_json::json!({
            "metadata_schema_version": 1,
            "phase_name": phase,
            "run_id": run_id,
            "correlation_id": run_id,
            "trace_id": trace_id,
            "output_count": 1,
            "error_count": error_count,
            "extra": {}
        });
        conn.execute(
            "INSERT INTO kpi_events (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)
             VALUES (?1, ?2, 'phase_completed', NULL, ?3, 1, ?4, ?5)",
            rusqlite::params![
                id,
                ts,
                latency_ms,
                if error_count == 0 { 1 } else { 0 },
                metadata.to_string()
            ],
        )
        .unwrap();
    }

    // ── parse_window_secs ──

    #[test]
    fn parse_window_accepts_simple_forms() {
        assert_eq!(parse_window_secs("5m").unwrap(), 300);
        assert_eq!(parse_window_secs("1h").unwrap(), 3600);
        assert_eq!(parse_window_secs("24h").unwrap(), 86_400);
        assert_eq!(parse_window_secs("7d").unwrap(), 7 * 86_400);
    }

    #[test]
    fn parse_window_accepts_compound_forms() {
        assert_eq!(parse_window_secs("1h30m").unwrap(), 5400);
        assert_eq!(parse_window_secs("2h 15m").unwrap(), 8100);
    }

    #[test]
    fn parse_window_rejects_zero() {
        assert!(parse_window_secs("0s").is_err());
        assert!(parse_window_secs("0m").is_err());
    }

    #[test]
    fn parse_window_rejects_over_ceiling() {
        assert!(parse_window_secs("8d").is_err());
        assert!(parse_window_secs("2w").is_err());
        assert!(parse_window_secs("365d").is_err());
    }

    #[test]
    fn parse_window_one_week_equals_seven_days_and_is_accepted() {
        // humantime parses "1w" as exactly 7 days; our ceiling is `<= 7d`, so accepted.
        assert_eq!(parse_window_secs("1w").unwrap(), 7 * 86_400);
    }

    #[test]
    fn parse_window_rejects_empty_and_whitespace() {
        assert!(parse_window_secs("").is_err());
        assert!(parse_window_secs("   ").is_err());
    }

    #[test]
    fn parse_window_rejects_bare_integer() {
        assert!(parse_window_secs("5").is_err());
        assert!(parse_window_secs("3600").is_err());
    }

    // ── resolve_group_by validity matrix ──

    #[test]
    fn resolve_group_by_row_count_accepts_only_none() {
        assert_eq!(resolve_group_by(InspectShape::RowCount, None).unwrap(), None);
        assert!(resolve_group_by(InspectShape::RowCount, Some(InspectGroupBy::Phase)).is_err());
    }

    #[test]
    fn resolve_group_by_latency_defaults_to_phase() {
        assert_eq!(
            resolve_group_by(InspectShape::Latency, None).unwrap(),
            Some(InspectGroupBy::Phase)
        );
        assert!(resolve_group_by(InspectShape::Latency, Some(InspectGroupBy::Project)).is_err());
    }

    #[test]
    fn resolve_group_by_throughput_defaults_to_event_type() {
        assert_eq!(
            resolve_group_by(InspectShape::Throughput, None).unwrap(),
            Some(InspectGroupBy::EventType)
        );
        assert!(
            resolve_group_by(InspectShape::Throughput, Some(InspectGroupBy::RunId)).is_err()
        );
    }

    #[test]
    fn resolve_group_by_phase_run_summary_rejects_all() {
        assert_eq!(
            resolve_group_by(InspectShape::PhaseRunSummary, None).unwrap(),
            None
        );
        assert!(
            resolve_group_by(InspectShape::PhaseRunSummary, Some(InspectGroupBy::Phase)).is_err()
        );
    }

    // ── shape behavior on seeded DB ──

    #[test]
    fn empty_db_returns_empty_rows_for_all_non_rowcount_shapes() {
        let conn = seed_conn();
        for shape in [
            InspectShape::Latency,
            InspectShape::ErrorRate,
            InspectShape::Throughput,
            InspectShape::PhaseRunSummary,
        ] {
            let resp = run_inspect(&conn, shape, "1h".into(), InspectFilter::default(), None, None);
            match resp {
                Response::Ok {
                    data: ResponseData::Inspect { data, .. },
                } => match data {
                    InspectData::Latency { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    InspectData::ErrorRate { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    InspectData::Throughput { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    InspectData::PhaseRunSummary { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    _ => panic!("unexpected data variant for {shape:?}"),
                },
                other => panic!("expected Ok, got {other:?}"),
            }
        }
    }

    #[test]
    fn row_count_without_snapshot_is_stale_and_empty() {
        let conn = seed_conn();
        let resp = run_inspect(
            &conn,
            InspectShape::RowCount,
            "1h".into(),
            InspectFilter::default(),
            None,
            None,
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::Inspect {
                        stale,
                        data: InspectData::RowCount { rows },
                        ..
                    },
            } => {
                assert!(stale);
                assert!(rows.is_empty());
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[test]
    fn row_count_with_fresh_snapshot_returns_eleven_rows() {
        use crate::server::metrics::{GaugeSnapshot, RowAndFreshness, TableGauges};
        let conn = seed_conn();
        let now = now_secs();
        let snap = GaugeSnapshot {
            refreshed_at_secs: now,
            tables: TableGauges {
                memory: RowAndFreshness {
                    count: 42,
                    freshness_secs: Some(10),
                },
                skill: RowAndFreshness::default(), // empty → freshness_secs = None
                ..Default::default()
            },
            memories_total: 42,
            edges_total: 0,
            embeddings_total: 0,
            active_sessions: 0,
        };
        let resp = run_inspect(
            &conn,
            InspectShape::RowCount,
            "1h".into(),
            InspectFilter::default(),
            None,
            Some(&snap),
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    stale,
                    data: InspectData::RowCount { rows },
                    ..
                },
        } = resp
        else {
            panic!("expected Ok RowCount");
        };
        assert!(!stale, "fresh snapshot should not be stale");
        assert_eq!(rows.len(), 11);
        let memory = rows.iter().find(|r| r.layer == "memory").unwrap();
        assert_eq!(memory.count, 42);
        assert_eq!(memory.freshness_secs, Some(10));
        let skill = rows.iter().find(|r| r.layer == "skill").unwrap();
        assert_eq!(skill.count, 0);
        assert!(skill.freshness_secs.is_none(), "empty table → None");
    }

    #[test]
    fn row_count_with_layer_filter_narrows_to_one_row() {
        use crate::server::metrics::{GaugeSnapshot, RowAndFreshness, TableGauges};
        let conn = seed_conn();
        let now = now_secs();
        let snap = GaugeSnapshot {
            refreshed_at_secs: now,
            tables: TableGauges {
                entity: RowAndFreshness {
                    count: 7,
                    freshness_secs: Some(3),
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let resp = run_inspect(
            &conn,
            InspectShape::RowCount,
            "1h".into(),
            InspectFilter {
                layer: Some("entity".into()),
                ..Default::default()
            },
            None,
            Some(&snap),
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::RowCount { rows },
                    ..
                },
        } = resp
        else {
            panic!("expected Ok RowCount");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].layer, "entity");
        assert_eq!(rows[0].count, 7);
    }

    #[test]
    fn row_count_with_stale_snapshot_flags_stale() {
        use crate::server::metrics::{GaugeSnapshot, TableGauges};
        let conn = seed_conn();
        let snap = GaugeSnapshot {
            refreshed_at_secs: now_secs().saturating_sub(120), // 2 minutes old
            tables: TableGauges::default(),
            ..Default::default()
        };
        let resp = run_inspect(
            &conn,
            InspectShape::RowCount,
            "1h".into(),
            InspectFilter::default(),
            None,
            Some(&snap),
        );
        let Response::Ok {
            data: ResponseData::Inspect { stale, .. },
        } = resp
        else {
            panic!("expected Ok");
        };
        assert!(stale, "120s-old snapshot should be flagged stale (>60s)");
    }

    #[test]
    fn latency_shape_returns_percentiles_per_phase() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10; // inside the 1h window
        // phase_A: 100, 200, 300 ms → p50=200, p95≈300, mean=200
        insert_phase_event(&conn, "a1", ts, "phase_A", "run1", 100, 0, None);
        insert_phase_event(&conn, "a2", ts, "phase_A", "run1", 200, 0, None);
        insert_phase_event(&conn, "a3", ts, "phase_A", "run1", 300, 0, None);
        // phase_B: 10, 20 ms → p50=20, mean=15
        insert_phase_event(&conn, "b1", ts, "phase_B", "run1", 10, 0, None);
        insert_phase_event(&conn, "b2", ts, "phase_B", "run1", 20, 0, None);

        let resp = run_inspect(
            &conn,
            InspectShape::Latency,
            "1h".into(),
            InspectFilter::default(),
            Some(InspectGroupBy::Phase),
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::Latency { rows },
                    truncated,
                    ..
                },
        } = resp
        else {
            panic!("expected Ok Latency");
        };
        assert!(!truncated);
        assert_eq!(rows.len(), 2);
        let phase_a = rows.iter().find(|r| r.group_key == "phase_A").unwrap();
        assert_eq!(phase_a.count, 3);
        assert_eq!(phase_a.p50_ms, 200.0);
        assert_eq!(phase_a.mean_ms, 200.0);
        assert_eq!(phase_a.truncated_samples, 0);
        let phase_b = rows.iter().find(|r| r.group_key == "phase_B").unwrap();
        assert_eq!(phase_b.count, 2);
        // ceiling-rank percentile on sorted [10, 20]: p50 picks sorted[ceil(0.5*2)-1] = sorted[0] = 10.
        assert_eq!(phase_b.p50_ms, 10.0);
        assert_eq!(phase_b.mean_ms, 15.0);
    }

    #[test]
    fn error_rate_shape_counts_only_nonzero_error_events() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10;
        insert_phase_event(&conn, "c1", ts, "phase_C", "run1", 1, 0, None);
        insert_phase_event(&conn, "c2", ts, "phase_C", "run1", 1, 2, None); // errored
        insert_phase_event(&conn, "c3", ts, "phase_C", "run1", 1, 0, None);
        insert_phase_event(&conn, "d1", ts, "phase_D", "run1", 1, 0, None);

        let resp = run_inspect(
            &conn,
            InspectShape::ErrorRate,
            "1h".into(),
            InspectFilter::default(),
            Some(InspectGroupBy::Phase),
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::ErrorRate { rows },
                    ..
                },
        } = resp
        else {
            panic!("expected Ok ErrorRate");
        };
        let phase_c = rows.iter().find(|r| r.group_key == "phase_C").unwrap();
        assert_eq!(phase_c.total, 3);
        assert_eq!(phase_c.errored, 1);
        assert!((phase_c.rate - 1.0 / 3.0).abs() < 1e-9);
        let phase_d = rows.iter().find(|r| r.group_key == "phase_D").unwrap();
        assert_eq!(phase_d.errored, 0);
        assert!((phase_d.rate).abs() < 1e-9);
    }

    #[test]
    fn phase_run_summary_groups_by_run_id_and_sums_latency() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10;
        // run_A: two phases, 100ms + 200ms, 1 error total
        insert_phase_event(&conn, "ra1", ts, "phase_1", "runA", 100, 0, Some("trace_aaa"));
        insert_phase_event(&conn, "ra2", ts + 1, "phase_2", "runA", 200, 1, Some("trace_aaa"));
        // run_B: one phase, 50ms, 0 errors, no trace_id
        insert_phase_event(&conn, "rb1", ts, "phase_1", "runB", 50, 0, None);

        let resp = run_inspect(
            &conn,
            InspectShape::PhaseRunSummary,
            "1h".into(),
            InspectFilter::default(),
            None,
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::PhaseRunSummary { rows },
                    ..
                },
        } = resp
        else {
            panic!("expected Ok PhaseRunSummary");
        };
        assert_eq!(rows.len(), 2);
        let run_a = rows.iter().find(|r| r.run_id == "runA").unwrap();
        assert_eq!(run_a.phase_count, 2);
        assert_eq!(run_a.phases_duration_ms_sum, 300);
        assert_eq!(run_a.error_count, 1);
        assert_eq!(run_a.trace_id.as_deref(), Some("trace_aaa"));
        let run_b = rows.iter().find(|r| r.run_id == "runB").unwrap();
        assert_eq!(run_b.error_count, 0);
        assert!(run_b.trace_id.is_none());
    }

    #[test]
    fn throughput_groups_by_event_type() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10;
        // Two phase_completed rows + one manually-inserted different event_type row.
        insert_phase_event(&conn, "t1", ts, "phase_1", "run1", 1, 0, None);
        insert_phase_event(&conn, "t2", ts, "phase_1", "run1", 1, 0, None);
        conn.execute(
            "INSERT INTO kpi_events (id, timestamp, event_type, success) VALUES ('other', ?1, 'bench_run_completed', 1)",
            rusqlite::params![ts],
        )
        .unwrap();

        let resp = run_inspect(
            &conn,
            InspectShape::Throughput,
            "1h".into(),
            InspectFilter::default(),
            Some(InspectGroupBy::EventType),
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::Throughput { rows },
                    ..
                },
        } = resp
        else {
            panic!("expected Ok Throughput");
        };
        let phase = rows.iter().find(|r| r.group_key == "phase_completed").unwrap();
        assert_eq!(phase.count, 2);
        let bench = rows.iter().find(|r| r.group_key == "bench_run_completed").unwrap();
        assert_eq!(bench.count, 1);
    }

    #[test]
    fn invalid_window_returns_error_response() {
        let conn = seed_conn();
        let resp = run_inspect(
            &conn,
            InspectShape::Latency,
            "bogus".into(),
            InspectFilter::default(),
            None,
            None,
        );
        matches!(resp, Response::Error { .. });
    }

    #[test]
    fn invalid_shape_group_by_combo_returns_error_response() {
        let conn = seed_conn();
        let resp = run_inspect(
            &conn,
            InspectShape::RowCount,
            "1h".into(),
            InspectFilter::default(),
            Some(InspectGroupBy::Phase),
            None,
        );
        matches!(resp, Response::Error { .. });
    }

    #[test]
    fn sql_injection_in_phase_filter_is_bound_literal() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10;
        insert_phase_event(&conn, "s1", ts, "phase_safe", "run1", 10, 0, None);

        // Hostile filter — if treated as SQL, this would attempt to drop the table.
        let hostile = "phase_safe'); DROP TABLE kpi_events; --";
        let resp = run_inspect(
            &conn,
            InspectShape::Latency,
            "1h".into(),
            InspectFilter {
                phase: Some(hostile.to_string()),
                ..Default::default()
            },
            Some(InspectGroupBy::Phase),
            None,
        );
        // The filter is bound as a literal → no rows match → empty response.
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::Latency { rows },
                    ..
                },
        } = resp
        else {
            panic!("expected Ok Latency");
        };
        assert!(rows.is_empty());

        // Table still exists — a follow-up query returns the seeded row.
        let existed: i64 = conn
            .query_row("SELECT COUNT(*) FROM kpi_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(existed, 1);
    }

    #[test]
    fn effective_filter_drops_layer_for_non_row_count_shapes() {
        let filter = InspectFilter {
            layer: Some("memory".into()),
            phase: Some("p".into()),
            event_type: Some("phase_completed".into()),
            project: Some("proj".into()),
        };
        let eff = effective_filter(InspectShape::Latency, &filter);
        assert!(eff.layer.is_none());
        assert_eq!(eff.phase.as_deref(), Some("p"));
        assert_eq!(eff.event_type.as_deref(), Some("phase_completed"));
        assert_eq!(eff.project.as_deref(), Some("proj"));

        let eff_rc = effective_filter(InspectShape::RowCount, &filter);
        assert_eq!(eff_rc.layer.as_deref(), Some("memory"));
        assert!(eff_rc.phase.is_none());
    }

    #[test]
    fn response_metadata_reflects_inputs() {
        let conn = seed_conn();
        let resp = run_inspect(
            &conn,
            InspectShape::ErrorRate,
            "24h".into(),
            InspectFilter {
                phase: Some("phase_X".into()),
                ..Default::default()
            },
            None,
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    shape,
                    window,
                    window_secs,
                    effective_filter,
                    effective_group_by,
                    ..
                },
        } = resp
        else {
            panic!("expected Ok");
        };
        assert_eq!(shape, InspectShape::ErrorRate);
        assert_eq!(window, "24h");
        assert_eq!(window_secs, 86_400);
        assert_eq!(effective_filter.phase.as_deref(), Some("phase_X"));
        assert_eq!(effective_group_by, Some(InspectGroupBy::Phase));
    }
}
