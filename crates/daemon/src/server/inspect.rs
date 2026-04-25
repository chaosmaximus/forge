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
    BenchRunRow, ErrorRateRow, InspectData, InspectFilter, InspectGroupBy, InspectShape,
    LatencyRow, PhaseRunRow, Response, ResponseData, ThroughputRow,
};
use rusqlite::{named_params, Connection};

/// Default hard ceiling for window duration (Tier 2 shapes). Overridden per
/// shape via `window_cap_secs_for_shape`.
const MAX_WINDOW_SECS: u64 = 7 * 86_400;

/// Tier 3 §3.3 (D8): `bench_run_summary` permits a 180d window to surface
/// seasonal regression trends without hitting per-event-type retention early.
const BENCH_RUN_SUMMARY_MAX_WINDOW_SECS: u64 = 180 * 86_400;

/// D8: return the per-shape window ceiling in seconds. All Tier 2 shapes keep
/// the 7-day cap; `BenchRunSummary` is the only Tier 3 exception.
pub fn window_cap_secs_for_shape(shape: &InspectShape) -> u64 {
    match shape {
        InspectShape::BenchRunSummary => BENCH_RUN_SUMMARY_MAX_WINDOW_SECS,
        _ => MAX_WINDOW_SECS,
    }
}

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
/// as long as the total is >0 and ≤ the per-shape ceiling (D8):
/// * `BenchRunSummary` → 180 days
/// * all other shapes  → 7 days
pub fn parse_window_secs(s: &str, shape: &InspectShape) -> Result<u64, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("window is empty".to_string());
    }
    let dur =
        humantime::parse_duration(trimmed).map_err(|e| format!("invalid window '{s}': {e}"))?;
    let secs = dur.as_secs();
    if secs == 0 {
        return Err(format!("window '{s}' parses to zero duration"));
    }
    let cap_secs = window_cap_secs_for_shape(shape);
    if secs > cap_secs {
        let cap_days = cap_secs / 86_400;
        return Err(format!(
            "window '{s}' exceeds {cap_days}-day ceiling ({secs}s > {cap_secs}s)"
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
        (RowCount, Some(g)) => Err(format!("group_by={g:?} not supported for shape=row_count")),

        (Latency, None) => Ok(Some(Phase)),
        (Latency, Some(Phase)) | (Latency, Some(RunId)) => Ok(provided),
        (Latency, Some(g)) => Err(format!("group_by={g:?} not valid for shape=latency")),

        (ErrorRate, None) => Ok(Some(Phase)),
        (ErrorRate, Some(Phase)) | (ErrorRate, Some(EventType)) => Ok(provided),
        (ErrorRate, Some(g)) => Err(format!("group_by={g:?} not valid for shape=error_rate")),

        (Throughput, None) => Ok(Some(EventType)),
        (Throughput, Some(Phase)) | (Throughput, Some(EventType)) | (Throughput, Some(Project)) => {
            Ok(provided)
        }
        (Throughput, Some(g)) => Err(format!("group_by={g:?} not valid for shape=throughput")),

        (PhaseRunSummary, None) => Ok(None),
        (PhaseRunSummary, Some(g)) => Err(format!(
            "group_by={g:?} not supported for shape=phase_run_summary (run_id is implicit)"
        )),

        // Tier 3 §3.3: BenchRunSummary defaults to grouping by bench_name; also
        // accepts commit_sha and seed. All Tier 2 dimensions (phase/event_type/
        // project/run_id) are rejected.
        (BenchRunSummary, None) => Ok(Some(BenchName)),
        (BenchRunSummary, Some(BenchName))
        | (BenchRunSummary, Some(CommitSha))
        | (BenchRunSummary, Some(Seed)) => Ok(provided),
        (BenchRunSummary, Some(g)) => Err(format!(
            "group_by={g:?} not valid for shape=bench_run_summary"
        )),
        // The Tier 2 arms above already reject BenchName/CommitSha/Seed via
        // their catch-all `Some(g)` clauses with shape-specific error text.
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
        InspectShape::BenchRunSummary => {
            // Tier 3 §3.3: only bench_name + commit_sha are honored; event_type
            // is implicit (`bench_run_completed`), other fields are dropped.
            out.bench_name = filter.bench_name.clone();
            out.commit_sha = filter.commit_sha.clone();
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
    let window_secs = match parse_window_secs(&window, &shape) {
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
        InspectShape::BenchRunSummary => {
            match shape_bench_run_summary(conn, window_start_secs, &eff_filter, effective_group_by)
            {
                Ok(rows) => (InspectData::BenchRunSummary { rows }, false, false),
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
                Ok((
                    key.unwrap_or_else(|| "unknown".to_string()),
                    lat.max(0) as u64,
                ))
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

// ─────────────────────────────────────────────────────────────
// Shape: bench_run_summary (Tier 3 §3.3 / D8)
// ─────────────────────────────────────────────────────────────

/// Build the per-group aggregate rollup for `bench_run_completed` events.
///
/// Strategy: two passes over `kpi_events`. Pass 1 uses SQL to aggregate
/// `runs`, `pass_rate`, `composite_mean`, and `first_ts`/`last_ts` per group.
/// Pass 2 pulls raw `composite` values (with the same filter) so we can run
/// the ceiling-rank percentile helper from Rust (matches `LatencyRow` semantics).
/// The per-group + absolute row caps from `MAX_ROWS_PER_GROUP` / `MAX_TOTAL_ROWS`
/// are honored; excess percentile samples are dropped silently but the mean +
/// pass_rate stay accurate (they come from the SQL aggregate).
pub fn shape_bench_run_summary(
    conn: &Connection,
    window_start_secs: u64,
    filter: &InspectFilter,
    group_by: Option<InspectGroupBy>,
) -> Result<Vec<BenchRunRow>, String> {
    // Default group_by for BenchRunSummary is BenchName; resolve_group_by
    // already normalizes None → Some(BenchName) before we get here, but be
    // defensive for callers that bypass it.
    let group_expr = match group_by.unwrap_or(InspectGroupBy::BenchName) {
        InspectGroupBy::BenchName => "CAST(json_extract(metadata_json, '$.bench_name') AS TEXT)",
        InspectGroupBy::CommitSha => "CAST(json_extract(metadata_json, '$.commit_sha') AS TEXT)",
        InspectGroupBy::Seed => "CAST(json_extract(metadata_json, '$.seed') AS TEXT)",
        other => {
            return Err(format!(
                "group_by={other:?} not valid for bench_run_summary"
            ))
        }
    };

    // Pass 1: SQL aggregate rollup.
    let sql = format!(
        r#"SELECT
              CAST(json_extract(metadata_json, '$.bench_name') AS TEXT) AS bench_name,
              {group_expr} AS group_key,
              COUNT(*) AS runs,
              AVG(success) AS pass_rate,
              AVG(CAST(json_extract(metadata_json, '$.composite') AS REAL)) AS composite_mean,
              MIN(timestamp) AS first_ts,
              MAX(timestamp) AS last_ts
           FROM kpi_events
           WHERE event_type = 'bench_run_completed'
             AND timestamp >= :window_start_secs
             AND (:bench_name IS NULL OR json_extract(metadata_json, '$.bench_name') = :bench_name)
             AND (:commit_sha IS NULL OR json_extract(metadata_json, '$.commit_sha') = :commit_sha)
           GROUP BY bench_name, group_key
           HAVING runs > 0
           ORDER BY bench_name, group_key
           LIMIT :absolute_cap"#
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| format!("prepare: {e}"))?;
    #[derive(Debug)]
    struct Aggregate {
        bench_name: String,
        group_key: String,
        runs: u64,
        pass_rate: f64,
        composite_mean: f64,
        first_ts: i64,
        last_ts: i64,
    }
    let aggregates: Vec<Aggregate> = stmt
        .query_map(
            named_params! {
                ":window_start_secs": window_start_secs as i64,
                ":bench_name": filter.bench_name.as_deref(),
                ":commit_sha": filter.commit_sha.as_deref(),
                // Enforce the absolute ceiling (MAX_TOTAL_ROWS) on rollup rows —
                // 200 000 distinct (bench_name, group_key) pairs is already pathological.
                ":absolute_cap": MAX_TOTAL_ROWS as i64,
            },
            |row| {
                let bench_name: Option<String> = row.get(0)?;
                let group_key: Option<String> = row.get(1)?;
                let runs: i64 = row.get(2)?;
                let pass_rate: Option<f64> = row.get(3)?;
                let composite_mean: Option<f64> = row.get(4)?;
                let first_ts: i64 = row.get(5)?;
                let last_ts: i64 = row.get(6)?;
                Ok(Aggregate {
                    bench_name: bench_name.unwrap_or_else(|| "unknown".to_string()),
                    group_key: group_key.unwrap_or_else(|| "unknown".to_string()),
                    runs: runs.max(0) as u64,
                    pass_rate: pass_rate.unwrap_or(0.0),
                    composite_mean: composite_mean.unwrap_or(0.0),
                    first_ts,
                    last_ts,
                })
            },
        )
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("row: {e}"))?;

    if aggregates.is_empty() {
        return Ok(Vec::new());
    }

    // Pass 2: per-group composite percentiles. Pull raw samples with a
    // per-group cap and compute p50/p95 in Rust via the ceiling-rank helper.
    let percentile_sql = format!(
        r#"SELECT {group_expr} AS group_key,
                  CAST(json_extract(metadata_json, '$.composite') AS REAL) AS composite
           FROM kpi_events
           WHERE event_type = 'bench_run_completed'
             AND timestamp >= :window_start_secs
             AND (:bench_name IS NULL OR json_extract(metadata_json, '$.bench_name') = :bench_name)
             AND (:commit_sha IS NULL OR json_extract(metadata_json, '$.commit_sha') = :commit_sha)
             AND json_extract(metadata_json, '$.composite') IS NOT NULL
           ORDER BY group_key
           LIMIT :total_cap"#
    );
    let mut p_stmt = conn
        .prepare(&percentile_sql)
        .map_err(|e| format!("prepare: {e}"))?;
    let raw_iter = p_stmt
        .query_map(
            named_params! {
                ":window_start_secs": window_start_secs as i64,
                ":bench_name": filter.bench_name.as_deref(),
                ":commit_sha": filter.commit_sha.as_deref(),
                ":total_cap": (MAX_TOTAL_ROWS + 1) as i64,
            },
            |row| {
                let key: Option<String> = row.get(0)?;
                let v: Option<f64> = row.get(1)?;
                Ok((
                    key.unwrap_or_else(|| "unknown".to_string()),
                    v.unwrap_or(0.0),
                ))
            },
        )
        .map_err(|e| format!("query: {e}"))?;

    let mut buckets: std::collections::BTreeMap<String, Vec<f64>> =
        std::collections::BTreeMap::new();
    let mut total_seen: u64 = 0;
    for r in raw_iter {
        let (k, v) = r.map_err(|e| format!("row: {e}"))?;
        total_seen += 1;
        if total_seen > MAX_TOTAL_ROWS {
            break;
        }
        let bucket = buckets.entry(k).or_default();
        if (bucket.len() as u64) < MAX_ROWS_PER_GROUP {
            bucket.push(v);
        }
    }

    // Sort each bucket once so percentile() is correct.
    for samples in buckets.values_mut() {
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    }

    let rows: Vec<BenchRunRow> = aggregates
        .into_iter()
        .map(|agg| {
            let samples = buckets.get(&agg.group_key);
            let (p50, p95) = match samples {
                Some(s) if !s.is_empty() => (percentile_f64(s, 0.50), percentile_f64(s, 0.95)),
                _ => (0.0, 0.0),
            };
            BenchRunRow {
                bench_name: agg.bench_name,
                group_key: agg.group_key,
                runs: agg.runs,
                pass_rate: agg.pass_rate,
                composite_mean: agg.composite_mean,
                composite_p50: p50,
                composite_p95: p95,
                first_ts_secs: agg.first_ts,
                last_ts_secs: agg.last_ts,
            }
        })
        .collect();

    Ok(rows)
}

/// Ceiling-rank percentile on an already-sorted slice of f64, mirroring
/// `percentile` for u64 samples. `p` ∈ [0.0, 1.0].
fn percentile_f64(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * sorted.len() as f64).ceil() as usize;
    let idx = rank.clamp(1, sorted.len()) - 1;
    sorted[idx]
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
    //
    // Helpers: pick any non-bench shape to get the 7d cap. All Tier 2 shapes
    // share the same ceiling so `Latency` works as a proxy.
    const T2: InspectShape = InspectShape::Latency;
    const BENCH: InspectShape = InspectShape::BenchRunSummary;

    #[test]
    fn parse_window_accepts_simple_forms() {
        assert_eq!(parse_window_secs("5m", &T2).unwrap(), 300);
        assert_eq!(parse_window_secs("1h", &T2).unwrap(), 3600);
        assert_eq!(parse_window_secs("24h", &T2).unwrap(), 86_400);
        assert_eq!(parse_window_secs("7d", &T2).unwrap(), 7 * 86_400);
    }

    #[test]
    fn parse_window_accepts_compound_forms() {
        assert_eq!(parse_window_secs("1h30m", &T2).unwrap(), 5400);
        assert_eq!(parse_window_secs("2h 15m", &T2).unwrap(), 8100);
    }

    #[test]
    fn parse_window_rejects_zero() {
        assert!(parse_window_secs("0s", &T2).is_err());
        assert!(parse_window_secs("0m", &T2).is_err());
    }

    #[test]
    fn parse_window_rejects_over_ceiling() {
        // Tier 2 shapes reject anything beyond 7 days.
        assert!(parse_window_secs("8d", &T2).is_err());
        assert!(parse_window_secs("2w", &T2).is_err());
        assert!(parse_window_secs("365d", &T2).is_err());
    }

    #[test]
    fn non_bench_shapes_still_reject_8d() {
        // T10 invariant: only BenchRunSummary unlocks >7d windows.
        for shape in [
            InspectShape::RowCount,
            InspectShape::Latency,
            InspectShape::ErrorRate,
            InspectShape::Throughput,
            InspectShape::PhaseRunSummary,
        ] {
            assert!(
                parse_window_secs("8d", &shape).is_err(),
                "{shape:?} should still cap at 7d"
            );
        }
    }

    #[test]
    fn parse_window_one_week_equals_seven_days_and_is_accepted() {
        // humantime parses "1w" as exactly 7 days; our ceiling is `<= 7d`, so accepted.
        assert_eq!(parse_window_secs("1w", &T2).unwrap(), 7 * 86_400);
    }

    #[test]
    fn parse_window_rejects_empty_and_whitespace() {
        assert!(parse_window_secs("", &T2).is_err());
        assert!(parse_window_secs("   ", &T2).is_err());
    }

    #[test]
    fn parse_window_rejects_bare_integer() {
        assert!(parse_window_secs("5", &T2).is_err());
        assert!(parse_window_secs("3600", &T2).is_err());
    }

    #[test]
    fn bench_run_summary_with_180d_window_works() {
        // D8: BenchRunSummary accepts windows up to 180 days.
        assert_eq!(parse_window_secs("180d", &BENCH).unwrap(), 180 * 86_400);
        assert_eq!(parse_window_secs("90d", &BENCH).unwrap(), 90 * 86_400);
        assert_eq!(parse_window_secs("30d", &BENCH).unwrap(), 30 * 86_400);
    }

    #[test]
    fn bench_run_summary_rejects_200d() {
        // D8: 200 days exceeds the BenchRunSummary ceiling of 180 days.
        assert!(parse_window_secs("200d", &BENCH).is_err());
        assert!(parse_window_secs("365d", &BENCH).is_err());
        let err = parse_window_secs("200d", &BENCH).unwrap_err();
        assert!(
            err.contains("180-day ceiling"),
            "error message should cite 180-day ceiling, got: {err}"
        );
    }

    #[test]
    fn bench_run_summary_181d_boundary() {
        // T14 H1: pin the off-by-one boundary. 180d × 86_400 = 15_552_000;
        // 181d = 15_638_400; both must be deterministic across humantime
        // releases.
        assert!(parse_window_secs("181d", &BENCH).is_err());
        assert_eq!(parse_window_secs("180d", &BENCH).unwrap(), 180 * 86_400);
    }

    #[test]
    fn non_bench_shapes_error_messages_cite_7d_ceiling() {
        // T14 H1: confirm the parameterized error message is correct for
        // non-BenchRunSummary shapes. T10's window_cap_secs_for_shape
        // refactor previously passed a uniform 7-day cap into the error
        // string regardless of shape — now each shape's error message
        // must cite its own cap.
        for shape in [
            InspectShape::RowCount,
            InspectShape::Latency,
            InspectShape::ErrorRate,
            InspectShape::Throughput,
            InspectShape::PhaseRunSummary,
        ] {
            let err = parse_window_secs("30d", &shape).unwrap_err();
            assert!(
                err.contains("7-day ceiling"),
                "{shape:?} error should cite 7-day ceiling, got: {err}"
            );
        }
    }

    // ── resolve_group_by validity matrix ──

    #[test]
    fn resolve_group_by_row_count_accepts_only_none() {
        assert_eq!(
            resolve_group_by(InspectShape::RowCount, None).unwrap(),
            None
        );
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
        assert!(resolve_group_by(InspectShape::Throughput, Some(InspectGroupBy::RunId)).is_err());
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

    #[test]
    fn bench_run_summary_resolve_group_by_defaults_to_bench_name() {
        assert_eq!(
            resolve_group_by(InspectShape::BenchRunSummary, None).unwrap(),
            Some(InspectGroupBy::BenchName)
        );
        assert_eq!(
            resolve_group_by(
                InspectShape::BenchRunSummary,
                Some(InspectGroupBy::CommitSha)
            )
            .unwrap(),
            Some(InspectGroupBy::CommitSha)
        );
        assert_eq!(
            resolve_group_by(InspectShape::BenchRunSummary, Some(InspectGroupBy::Seed)).unwrap(),
            Some(InspectGroupBy::Seed)
        );
    }

    #[test]
    fn bench_run_summary_resolve_group_by_rejects_phase() {
        // Tier 2 dimensions are not valid for BenchRunSummary.
        assert!(
            resolve_group_by(InspectShape::BenchRunSummary, Some(InspectGroupBy::Phase)).is_err()
        );
        assert!(resolve_group_by(
            InspectShape::BenchRunSummary,
            Some(InspectGroupBy::EventType)
        )
        .is_err());
        assert!(
            resolve_group_by(InspectShape::BenchRunSummary, Some(InspectGroupBy::Project)).is_err()
        );
        assert!(
            resolve_group_by(InspectShape::BenchRunSummary, Some(InspectGroupBy::RunId)).is_err()
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
            InspectShape::BenchRunSummary,
        ] {
            let resp = run_inspect(
                &conn,
                shape,
                "1h".into(),
                InspectFilter::default(),
                None,
                None,
            );
            match resp {
                Response::Ok {
                    data: ResponseData::Inspect { data, .. },
                } => match data {
                    InspectData::Latency { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    InspectData::ErrorRate { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    InspectData::Throughput { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    InspectData::PhaseRunSummary { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    InspectData::BenchRunSummary { rows } => assert!(rows.is_empty(), "{shape:?}"),
                    _ => panic!("unexpected data variant for {shape:?}"),
                },
                other => panic!("expected Ok, got {other:?}"),
            }
        }
    }

    /// Seed a `bench_run_completed` kpi_event with the fields T10 aggregates.
    /// T8 will wire daemon-side emission; these rows are synthesized directly
    /// so the leaderboard shape can be unit-tested in isolation.
    #[allow(clippy::too_many_arguments)]
    fn insert_bench_event(
        conn: &Connection,
        id: &str,
        ts: i64,
        bench_name: &str,
        commit_sha: Option<&str>,
        seed: Option<&str>,
        composite: f64,
        success: bool,
    ) {
        let metadata = serde_json::json!({
            "bench_name": bench_name,
            "commit_sha": commit_sha,
            "seed": seed,
            "composite": composite,
        });
        conn.execute(
            "INSERT INTO kpi_events (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)
             VALUES (?1, ?2, 'bench_run_completed', NULL, NULL, 1, ?3, ?4)",
            rusqlite::params![
                id,
                ts,
                i64::from(success),
                metadata.to_string()
            ],
        )
        .unwrap();
    }

    #[test]
    fn bench_run_summary_empty_db_returns_empty_rows() {
        let conn = seed_conn();
        let rows = shape_bench_run_summary(
            &conn,
            0,
            &InspectFilter::default(),
            Some(InspectGroupBy::BenchName),
        )
        .expect("query should succeed");
        assert!(rows.is_empty());
    }

    #[test]
    fn bench_run_summary_aggregates_seeded_runs() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10; // inside 1h window
        insert_bench_event(
            &conn,
            "br1",
            ts,
            "forge-identity",
            Some("abc"),
            Some("1"),
            0.97,
            true,
        );
        insert_bench_event(
            &conn,
            "br2",
            ts + 1,
            "forge-identity",
            Some("abc"),
            Some("2"),
            0.95,
            true,
        );
        insert_bench_event(
            &conn,
            "br3",
            ts + 2,
            "forge-identity",
            Some("abc"),
            Some("3"),
            0.93,
            false,
        );

        let resp = run_inspect(
            &conn,
            InspectShape::BenchRunSummary,
            "1h".into(),
            InspectFilter::default(),
            None,
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::BenchRunSummary { rows },
                    effective_group_by,
                    ..
                },
        } = resp
        else {
            panic!("expected Ok BenchRunSummary, got {resp:?}");
        };
        assert_eq!(effective_group_by, Some(InspectGroupBy::BenchName));
        assert_eq!(rows.len(), 1, "one bench_name → one rollup row");
        let r = &rows[0];
        assert_eq!(r.bench_name, "forge-identity");
        assert_eq!(r.group_key, "forge-identity");
        assert_eq!(r.runs, 3);
        // 2/3 successes → ≈0.6667
        assert!(
            (r.pass_rate - 2.0 / 3.0).abs() < 1e-6,
            "pass_rate: got {}",
            r.pass_rate
        );
        // composite mean ≈ (0.97+0.95+0.93)/3 = 0.95
        assert!(
            (r.composite_mean - 0.95).abs() < 1e-6,
            "composite_mean: got {}",
            r.composite_mean
        );
        // sorted composites [0.93, 0.95, 0.97]: p50 = sorted[ceil(1.5)-1] = sorted[1] = 0.95
        assert!(
            (r.composite_p50 - 0.95).abs() < 1e-6,
            "composite_p50: got {}",
            r.composite_p50
        );
        // p95: ceil(0.95*3)-1 = ceil(2.85)-1 = 3-1 = 2 → 0.97
        assert!(
            (r.composite_p95 - 0.97).abs() < 1e-6,
            "composite_p95: got {}",
            r.composite_p95
        );
    }

    #[test]
    fn bench_run_summary_filters_by_bench_name() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10;
        insert_bench_event(&conn, "a", ts, "forge-identity", None, None, 0.9, true);
        insert_bench_event(&conn, "b", ts, "forge-retrieval", None, None, 0.5, false);
        let resp = run_inspect(
            &conn,
            InspectShape::BenchRunSummary,
            "1h".into(),
            InspectFilter {
                bench_name: Some("forge-identity".into()),
                ..Default::default()
            },
            None,
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::BenchRunSummary { rows },
                    ..
                },
        } = resp
        else {
            panic!("expected Ok BenchRunSummary");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bench_name, "forge-identity");
    }

    #[test]
    fn bench_run_summary_groups_by_commit_sha() {
        let conn = seed_conn();
        let ts = now_secs() as i64 - 10;
        insert_bench_event(
            &conn,
            "a",
            ts,
            "forge-identity",
            Some("sha1"),
            None,
            0.9,
            true,
        );
        insert_bench_event(
            &conn,
            "b",
            ts,
            "forge-identity",
            Some("sha2"),
            None,
            0.8,
            true,
        );
        let resp = run_inspect(
            &conn,
            InspectShape::BenchRunSummary,
            "1h".into(),
            InspectFilter::default(),
            Some(InspectGroupBy::CommitSha),
            None,
        );
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    data: InspectData::BenchRunSummary { rows },
                    effective_group_by,
                    ..
                },
        } = resp
        else {
            panic!("expected Ok BenchRunSummary");
        };
        assert_eq!(effective_group_by, Some(InspectGroupBy::CommitSha));
        assert_eq!(rows.len(), 2, "one row per commit_sha");
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
        insert_phase_event(
            &conn,
            "ra1",
            ts,
            "phase_1",
            "runA",
            100,
            0,
            Some("trace_aaa"),
        );
        insert_phase_event(
            &conn,
            "ra2",
            ts + 1,
            "phase_2",
            "runA",
            200,
            1,
            Some("trace_aaa"),
        );
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
        let phase = rows
            .iter()
            .find(|r| r.group_key == "phase_completed")
            .unwrap();
        assert_eq!(phase.count, 2);
        let bench = rows
            .iter()
            .find(|r| r.group_key == "bench_run_completed")
            .unwrap();
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
        assert!(
            matches!(resp, Response::Error { .. }),
            "expected Response::Error, got: {resp:?}"
        );
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
        assert!(
            matches!(resp, Response::Error { .. }),
            "expected Response::Error, got: {resp:?}"
        );
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
            bench_name: None,
            commit_sha: None,
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
