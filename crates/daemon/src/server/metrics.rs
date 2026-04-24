//! Prometheus /metrics endpoint for enterprise observability.
//!
//! Exposes 7 metric families in standard Prometheus text format:
//!   - forge_memories_total (gauge)
//!   - forge_recall_latency_seconds (histogram)
//!   - forge_extraction_duration_seconds (histogram)
//!   - forge_worker_healthy (gauge vec, label: worker)
//!   - forge_active_sessions (gauge)
//!   - forge_edges_total (gauge)
//!   - forge_embeddings_total (gauge)

use axum::extract::State;
use axum::response::IntoResponse;
use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
    TextEncoder,
};

use super::http::AppState;

/// Atomic snapshot of gauge values for `/inspect shape=row_count` consumers.
///
/// Distinct from the live Prometheus gauges (which `/metrics` scrapes and
/// which are still written non-atomically). `refresh_gauges` builds a new
/// `GaugeSnapshot` from the single 13-subquery SELECT and swaps it under a
/// write lock so `/inspect` readers see a single point-in-time view.
///
/// T3 defines the types; T4 wires `Arc<RwLock<GaugeSnapshot>>` onto
/// `ForgeMetrics` and populates the snapshot inside `refresh_gauges`. Until
/// T4 lands, `/inspect row_count` returns empty rows + `stale: true` (the
/// inspect handler accepts `Option<&GaugeSnapshot>` and handles `None`).
#[derive(Clone, Debug, Default)]
pub struct GaugeSnapshot {
    /// Unix seconds when the snapshot was last built. `0` = never refreshed.
    pub refreshed_at_secs: u64,
    pub tables: TableGauges,
    pub memories_total: i64,
    pub edges_total: i64,
    pub embeddings_total: i64,
    pub active_sessions: i64,
}

/// Per-Manas-table gauges. Named struct (not a `BTreeMap<String, i64>`) because
/// the 11 tables are compile-time-known; adding a table is a compile-time change.
#[derive(Clone, Debug, Default)]
pub struct TableGauges {
    pub memory: RowAndFreshness,
    pub skill: RowAndFreshness,
    pub edge: RowAndFreshness,
    pub identity: RowAndFreshness,
    pub disposition: RowAndFreshness,
    pub platform: RowAndFreshness,
    pub tool: RowAndFreshness,
    pub perception: RowAndFreshness,
    pub declared: RowAndFreshness,
    pub domain_dna: RowAndFreshness,
    pub entity: RowAndFreshness,
}

impl TableGauges {
    /// Flatten the named struct into the `LayerRow` wire format used by
    /// `/inspect row_count`. `snapshot_age_secs` is passed in by the caller
    /// (computed once from `GaugeSnapshot.refreshed_at_secs`).
    pub fn to_layer_rows(&self, snapshot_age_secs: u64) -> Vec<forge_core::protocol::LayerRow> {
        use forge_core::protocol::LayerRow;
        vec![
            ("memory", &self.memory),
            ("skill", &self.skill),
            ("edge", &self.edge),
            ("identity", &self.identity),
            ("disposition", &self.disposition),
            ("platform", &self.platform),
            ("tool", &self.tool),
            ("perception", &self.perception),
            ("declared", &self.declared),
            ("domain_dna", &self.domain_dna),
            ("entity", &self.entity),
        ]
        .into_iter()
        .map(|(name, rf)| LayerRow {
            layer: name.to_string(),
            count: rf.count,
            snapshot_age_secs,
            freshness_secs: rf.freshness_secs,
        })
        .collect()
    }
}

/// One table's row count + time-since-last-write. `freshness_secs = None`
/// when the table is empty (Prometheus uses `-1` sentinel at T4; internal
/// types keep `Option<i64>` for honesty).
#[derive(Clone, Debug, Default)]
pub struct RowAndFreshness {
    pub count: i64,
    pub freshness_secs: Option<i64>,
}

/// Holds all Prometheus metric collectors and the registry that owns them.
#[derive(Clone)]
pub struct ForgeMetrics {
    pub registry: Registry,
    pub memories_total: IntGauge,
    pub recall_latency: Histogram,
    pub extraction_duration: Histogram,
    pub worker_healthy: IntGaugeVec,
    pub active_sessions: IntGauge,
    pub edges_total: IntGauge,
    pub embeddings_total: IntGauge,

    // ── Phase 2A-4d.1 Instrumentation tier (4 new families) ──
    /// Consolidator phase duration, labelled by phase.
    pub phase_duration: HistogramVec,
    /// Output-row count per phase × action (succeeded|errored). `action` is
    /// driven by `PhaseOutcome::error_count`, which reflects errors INSIDE
    /// the phase body. `kpi_events` persistence failures are tracked
    /// separately in `phase_persistence_errors_total` to avoid polluting
    /// the phase-level SLI.
    pub phase_output_rows: IntCounterVec,
    /// Row count per Manas-layer SQL table (memory, skill, edge, identity, …).
    pub table_rows: IntGaugeVec,
    /// `kpi_events` row write failures, labelled by phase and kind.
    /// `kind` is one of `insert_error` (SQL failed) or `ulid_collision`
    /// (INSERT OR IGNORE absorbed the row). Separating this from
    /// `phase_output_rows{action="errored"}` prevents double-counting when a
    /// phase also had an internal error.
    pub phase_persistence_errors: IntCounterVec,
    /// `/metrics` scrape refreshes that skipped whole-snapshot reads.
    /// Label `reason` takes one of:
    /// - `open_reader` — failed to open the read-only SQLite connection.
    /// - `query_failed` — the single-SELECT gauge refresh returned an
    ///   error (table missing, schema drift, corruption).
    ///
    /// Without this counter, a silent gauge freeze (refresh_gauges returns
    /// early, Prometheus keeps serving the last-known values) would be
    /// indistinguishable from a stable database. Operators should alert
    /// on any non-zero rate.
    pub gauge_refresh_failures: IntCounterVec,

    // ── Phase 2A-4d.2 Observability API (2 new families + 1 atomic snapshot) ──
    /// Seconds since the most recent write to each Manas table, labelled
    /// by table name. Prometheus cannot emit NULL, so empty tables report
    /// `-1` as a sentinel; consumers should filter `> 0`. Internal Rust /
    /// JSON consumers (via `/inspect row_count`) see `Option<i64>` with
    /// `None` for empty tables — no sentinel ambiguity on that path.
    pub layer_freshness: IntGaugeVec,

    /// Atomic snapshot of the most recent gauge refresh. Read by
    /// `/inspect row_count` to serve a point-in-time view without the torn
    /// reads the serial `.set()` calls above would produce. Distinct from
    /// the live Prometheus gauges (which `/metrics` scrapes and which are
    /// still written non-atomically — acceptable because Prometheus
    /// aggregation tolerates sub-scrape drift).
    pub snapshot: std::sync::Arc<parking_lot::RwLock<GaugeSnapshot>>,
}

impl Default for ForgeMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl ForgeMetrics {
    /// Create a new metrics instance with all collectors registered.
    pub fn new() -> Self {
        let registry = Registry::new();

        let memories_total = IntGauge::new("forge_memories_total", "Total number of memories")
            .expect("memories_total metric");
        let recall_latency = Histogram::with_opts(
            HistogramOpts::new(
                "forge_recall_latency_seconds",
                "Recall query latency in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
        )
        .expect("recall_latency metric");
        let extraction_duration = Histogram::with_opts(
            HistogramOpts::new(
                "forge_extraction_duration_seconds",
                "Auto-extraction duration in seconds",
            )
            .buckets(vec![0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]),
        )
        .expect("extraction_duration metric");
        let worker_healthy = IntGaugeVec::new(
            Opts::new(
                "forge_worker_healthy",
                "Whether a background worker is healthy (1=yes, 0=no)",
            ),
            &["worker"],
        )
        .expect("worker_healthy metric");
        let active_sessions = IntGauge::new("forge_active_sessions", "Number of active sessions")
            .expect("active_sessions metric");
        let edges_total =
            IntGauge::new("forge_edges_total", "Total number of knowledge graph edges")
                .expect("edges_total metric");
        let embeddings_total = IntGauge::new(
            "forge_embeddings_total",
            "Total number of stored embeddings",
        )
        .expect("embeddings_total metric");

        // ── Phase 2A-4d.1 Instrumentation — 3 new families ──
        let phase_duration = HistogramVec::new(
            HistogramOpts::new(
                "forge_phase_duration_seconds",
                "Consolidator phase duration in seconds, labelled by phase",
            )
            // Buckets span sub-ms (phase 1 dedup, phase 10 decay) through
            // multi-minute (phase 2 semantic_dedup, phase 7 embedding_merge,
            // phase 14 reweave on warm DBs routinely exceed 30s).
            .buckets(vec![
                0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0, 60.0, 120.0, 300.0,
            ]),
            &["phase"],
        )
        .expect("phase_duration metric");
        let phase_output_rows = IntCounterVec::new(
            Opts::new(
                "forge_phase_output_rows_total",
                "Output rows produced per consolidator phase × action",
            ),
            &["phase", "action"],
        )
        .expect("phase_output_rows metric");
        let table_rows = IntGaugeVec::new(
            Opts::new(
                "forge_table_rows",
                "Row count per Manas-layer SQL table (gauge, not a counter)",
            ),
            &["table"],
        )
        .expect("table_rows metric");
        let phase_persistence_errors = IntCounterVec::new(
            Opts::new(
                "forge_phase_persistence_errors_total",
                "kpi_events row write failures per phase × kind (insert_error | ulid_collision)",
            ),
            &["phase", "kind"],
        )
        .expect("phase_persistence_errors metric");
        let gauge_refresh_failures = IntCounterVec::new(
            Opts::new(
                "forge_gauge_refresh_failures_total",
                "/metrics scrapes where the gauge refresh returned early (open_reader | query_failed)",
            ),
            &["reason"],
        )
        .expect("gauge_refresh_failures metric");

        // ── Phase 2A-4d.2 Observability API ──
        let layer_freshness = IntGaugeVec::new(
            Opts::new(
                "forge_layer_freshness_seconds",
                "Seconds since the most recent write to each Manas table (-1 when empty or time column is NULL)",
            ),
            &["table"],
        )
        .expect("layer_freshness metric");

        registry
            .register(Box::new(memories_total.clone()))
            .expect("register memories_total");
        registry
            .register(Box::new(recall_latency.clone()))
            .expect("register recall_latency");
        registry
            .register(Box::new(extraction_duration.clone()))
            .expect("register extraction_duration");
        registry
            .register(Box::new(worker_healthy.clone()))
            .expect("register worker_healthy");
        registry
            .register(Box::new(active_sessions.clone()))
            .expect("register active_sessions");
        registry
            .register(Box::new(edges_total.clone()))
            .expect("register edges_total");
        registry
            .register(Box::new(embeddings_total.clone()))
            .expect("register embeddings_total");
        registry
            .register(Box::new(phase_duration.clone()))
            .expect("register phase_duration");
        registry
            .register(Box::new(phase_output_rows.clone()))
            .expect("register phase_output_rows");
        registry
            .register(Box::new(table_rows.clone()))
            .expect("register table_rows");
        registry
            .register(Box::new(phase_persistence_errors.clone()))
            .expect("register phase_persistence_errors");
        registry
            .register(Box::new(gauge_refresh_failures.clone()))
            .expect("register gauge_refresh_failures");
        registry
            .register(Box::new(layer_freshness.clone()))
            .expect("register layer_freshness");

        Self {
            registry,
            memories_total,
            recall_latency,
            extraction_duration,
            worker_healthy,
            active_sessions,
            edges_total,
            embeddings_total,
            phase_duration,
            phase_output_rows,
            table_rows,
            phase_persistence_errors,
            gauge_refresh_failures,
            layer_freshness,
            snapshot: std::sync::Arc::new(parking_lot::RwLock::new(GaugeSnapshot::default())),
        }
    }
}

/// Refresh gauge values from the database before a Prometheus scrape.
/// Opens a read-only connection and collects all 15 COUNT(*) values in a
/// single SELECT with scalar subqueries. SQLite evaluates every subquery
/// within one implicit read transaction on an auto-commit SELECT, so the
/// whole row reflects one DB snapshot — no explicit BEGIN/COMMIT is needed
/// and the WAL snapshot window is ~1 round-trip instead of 15, which
/// avoids blocking WAL checkpoint on busy DBs (HIGH-2 adversarial review).
/// Without the single-snapshot property, Phase 1 dedup or Phase 7
/// embedding_merge could commit between statements and produce torn
/// snapshots (e.g. memory count updated, edge count still stale) that
/// trip operator alerts.
fn refresh_gauges(metrics: &ForgeMetrics, state: &AppState) {
    // Open a per-scrape read-only connection (same pattern as health probes)
    let reader = match crate::server::handler::DaemonState::new_reader(
        &state.db_path,
        state.events.clone(),
        std::sync::Arc::clone(&state.hlc),
        state.started_at,
        None,
    ) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("metrics: failed to open reader for gauge refresh: {e}");
            metrics
                .gauge_refresh_failures
                .with_label_values(&["open_reader"])
                .inc();
            return;
        }
    };
    refresh_gauges_from_conn(metrics, &reader.conn);
}

/// Phase 2A-4d.2 T9 fix: lazy-refresh entry point for `/inspect row_count`
/// when no Prometheus scraper is configured. Given any read-capable
/// connection (reader or writer — the SELECT is read-only), populate the
/// Prometheus gauges + atomic GaugeSnapshot. Same query as `refresh_gauges`;
/// shares all state-mutation logic with it via the split-off body.
pub fn refresh_gauges_from_conn(metrics: &ForgeMetrics, conn: &rusqlite::Connection) {
    refresh_gauges_impl(metrics, conn)
}

fn refresh_gauges_impl(metrics: &ForgeMetrics, conn: &rusqlite::Connection) {
    // Per-table row gauges. Labels match actual SQLite table names (verified
    // 2026-04-24 in schema.rs); adding a label that doesn't correspond to a
    // real table would produce a perpetually-zero series that misleads
    // operators. `memory` and `edge` intentionally appear here *and* feed
    // the dedicated memories_total / edges_total gauges — the single SELECT
    // below returns each count exactly once and we reuse it for both sinks.
    // The canonical (table, count, freshness) tuple list lives below in
    // `per_table` so labels and SELECT column order can't drift apart.

    // Single-row SELECT with scalar subqueries — 13 counts + 11 freshness
    // values all come back in one round-trip, evaluated against one implicit
    // read snapshot. Freshness expressions use `strftime('%s', ...)` because
    // the Manas tables store their timestamp columns as TEXT ISO strings;
    // naive integer subtraction would produce garbage. Empty tables (or the
    // all-NULL case for skill.last_used) evaluate to NULL, which rusqlite
    // decodes as `Option::None`.
    //
    // Per-table timestamp column (verified 2026-04-24):
    //   memory      → created_at        identity    → created_at
    //   skill       → last_used (NULL)  perception  → created_at
    //   edge        → created_at        declared    → ingested_at
    //   disposition → updated_at        domain_dna  → detected_at
    //   platform    → detected_at       entity      → last_seen
    //   tool        → discovered_at
    const COUNTS_SQL: &str = "SELECT \
        (SELECT COUNT(*) FROM memory)                                                   AS mem_c, \
        (SELECT COUNT(*) FROM edge)                                                     AS edg_c, \
        (SELECT COUNT(*) FROM memory_vec)                                               AS vec_c, \
        (SELECT COUNT(*) FROM session WHERE ended_at IS NULL)                           AS active_sess, \
        (SELECT COUNT(*) FROM skill)                                                    AS skl_c, \
        (SELECT COUNT(*) FROM identity)                                                 AS idn_c, \
        (SELECT COUNT(*) FROM disposition)                                              AS dsp_c, \
        (SELECT COUNT(*) FROM platform)                                                 AS plt_c, \
        (SELECT COUNT(*) FROM tool)                                                     AS tol_c, \
        (SELECT COUNT(*) FROM perception)                                               AS prc_c, \
        (SELECT COUNT(*) FROM declared)                                                 AS dcl_c, \
        (SELECT COUNT(*) FROM domain_dna)                                               AS dna_c, \
        (SELECT COUNT(*) FROM entity)                                                   AS ent_c, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(created_at)) AS INTEGER) END \
           FROM memory)                                                                 AS mem_f, \
        (SELECT CASE WHEN COUNT(last_used)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(last_used)) AS INTEGER) END \
           FROM skill)                                                                  AS skl_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(created_at)) AS INTEGER) END \
           FROM edge)                                                                   AS edg_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(created_at)) AS INTEGER) END \
           FROM identity)                                                               AS idn_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(updated_at)) AS INTEGER) END \
           FROM disposition)                                                            AS dsp_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(detected_at)) AS INTEGER) END \
           FROM platform)                                                               AS plt_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(discovered_at)) AS INTEGER) END \
           FROM tool)                                                                   AS tol_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(created_at)) AS INTEGER) END \
           FROM perception)                                                             AS prc_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(ingested_at)) AS INTEGER) END \
           FROM declared)                                                               AS dcl_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(detected_at)) AS INTEGER) END \
           FROM domain_dna)                                                             AS dna_f, \
        (SELECT CASE WHEN COUNT(*)=0 THEN NULL \
                     ELSE CAST(strftime('%s','now') AS INTEGER) - CAST(strftime('%s', MAX(last_seen)) AS INTEGER) END \
           FROM entity)                                                                 AS ent_f";

    type CountsRow = (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
    );
    let row: rusqlite::Result<CountsRow> = conn.query_row(COUNTS_SQL, [], |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
            r.get(10)?,
            r.get(11)?,
            r.get(12)?,
            r.get(13)?,
            r.get(14)?,
            r.get(15)?,
            r.get(16)?,
            r.get(17)?,
            r.get(18)?,
            r.get(19)?,
            r.get(20)?,
            r.get(21)?,
            r.get(22)?,
            r.get(23)?,
        ))
    });

    let (
        count_memory,
        count_edge,
        count_mem_vec,
        count_active_sessions,
        count_skill,
        count_identity,
        count_disposition,
        count_platform,
        count_tool,
        count_perception,
        count_declared,
        count_domain_dna,
        count_entity,
        fresh_memory,
        fresh_skill,
        fresh_edge,
        fresh_identity,
        fresh_disposition,
        fresh_platform,
        fresh_tool,
        fresh_perception,
        fresh_declared,
        fresh_domain_dna,
        fresh_entity,
    ) = match row {
        Ok(r) => r,
        Err(e) => {
            // Any table missing / schema drift → skip this scrape entirely
            // rather than partially-update gauges (preserves all-or-nothing
            // semantics from the prior BEGIN/COMMIT design). Counter allows
            // operators to alert on the silent freeze that would otherwise
            // look identical to a stable-but-idle database.
            tracing::warn!(error = %e, "metrics: failed to collect DB counts; skipping gauge refresh");
            metrics
                .gauge_refresh_failures
                .with_label_values(&["query_failed"])
                .inc();
            return;
        }
    };

    metrics.memories_total.set(count_memory);
    metrics.edges_total.set(count_edge);
    metrics.embeddings_total.set(count_mem_vec);
    metrics.active_sessions.set(count_active_sessions);

    // Worker health — set all known workers to 1 (we're alive if we can query)
    for worker in &[
        "watcher",
        "extractor",
        "embedder",
        "consolidator",
        "indexer",
        "perception",
        "disposition",
        "diagnostics",
    ] {
        metrics.worker_healthy.with_label_values(&[worker]).set(1);
    }

    // Iterate in the same order as `tables` so label → count pairing is
    // unambiguous and matches the SELECT's column order above.
    let per_table: [(&str, i64, Option<i64>); 11] = [
        ("memory", count_memory, fresh_memory),
        ("skill", count_skill, fresh_skill),
        ("edge", count_edge, fresh_edge),
        ("identity", count_identity, fresh_identity),
        ("disposition", count_disposition, fresh_disposition),
        ("platform", count_platform, fresh_platform),
        ("tool", count_tool, fresh_tool),
        ("perception", count_perception, fresh_perception),
        ("declared", count_declared, fresh_declared),
        ("domain_dna", count_domain_dna, fresh_domain_dna),
        ("entity", count_entity, fresh_entity),
    ];
    for (table, count, _fresh) in &per_table {
        metrics.table_rows.with_label_values(&[*table]).set(*count);
    }
    // Freshness — Prometheus cannot emit NULL, so empty tables report -1.
    // Consumers filter `> 0`; internal /inspect row_count sees Option<i64>.
    for (table, _count, fresh) in &per_table {
        metrics
            .layer_freshness
            .with_label_values(&[*table])
            .set(fresh.unwrap_or(-1));
    }

    // Build GaugeSnapshot and atomic-swap at the END so readers never see a
    // half-built snapshot. The 11 table_rows Prometheus gauges above are
    // still written non-atomically (accepted: /metrics tolerates sub-scrape
    // drift; /inspect row_count reads from this snapshot instead).
    let new_snapshot = GaugeSnapshot {
        refreshed_at_secs: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        tables: TableGauges {
            memory: RowAndFreshness {
                count: count_memory,
                freshness_secs: fresh_memory,
            },
            skill: RowAndFreshness {
                count: count_skill,
                freshness_secs: fresh_skill,
            },
            edge: RowAndFreshness {
                count: count_edge,
                freshness_secs: fresh_edge,
            },
            identity: RowAndFreshness {
                count: count_identity,
                freshness_secs: fresh_identity,
            },
            disposition: RowAndFreshness {
                count: count_disposition,
                freshness_secs: fresh_disposition,
            },
            platform: RowAndFreshness {
                count: count_platform,
                freshness_secs: fresh_platform,
            },
            tool: RowAndFreshness {
                count: count_tool,
                freshness_secs: fresh_tool,
            },
            perception: RowAndFreshness {
                count: count_perception,
                freshness_secs: fresh_perception,
            },
            declared: RowAndFreshness {
                count: count_declared,
                freshness_secs: fresh_declared,
            },
            domain_dna: RowAndFreshness {
                count: count_domain_dna,
                freshness_secs: fresh_domain_dna,
            },
            entity: RowAndFreshness {
                count: count_entity,
                freshness_secs: fresh_entity,
            },
        },
        memories_total: count_memory,
        edges_total: count_edge,
        embeddings_total: count_mem_vec,
        active_sessions: count_active_sessions,
    };
    *metrics.snapshot.write() = new_snapshot;
}

/// GET /metrics — Prometheus scrape endpoint.
/// Refreshes gauges from DB, then returns all metrics in text format.
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    // metrics is always Some when this handler is registered (guarded by config check)
    let metrics = state
        .metrics
        .as_ref()
        .expect("metrics must be Some when /metrics is registered");

    // Refresh gauges from live DB data on each scrape
    refresh_gauges(metrics, &state);

    let encoder = TextEncoder::new();
    let metric_families = metrics.registry.gather();
    let mut buffer = String::new();
    encoder
        .encode_utf8(&metric_families, &mut buffer)
        .expect("prometheus text encoding");
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        buffer,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ForgeConfig;
    use crate::server::handler::DaemonState;
    use crate::server::http::build_router;
    use crate::sync::Hlc;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::http::StatusCode;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    fn test_app_state_with_metrics(metrics: Option<Arc<ForgeMetrics>>) -> AppState {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().to_string();
        let _state = DaemonState::new(&db_path).unwrap();
        let (events, _) = tokio::sync::broadcast::channel(16);
        let hlc = Arc::new(Hlc::new("test"));
        let (write_tx, write_rx) = mpsc::channel(16);
        std::mem::forget(tmp);
        std::mem::forget(write_rx);
        AppState {
            db_path,
            events,
            hlc,
            started_at: Instant::now(),
            write_tx,
            admin_emails: vec![],
            viewer_emails: vec![],
            auth_enabled: false,
            metrics,
            rate_limiter: None,
        }
    }

    #[test]
    fn test_forge_metrics_new_registers_all_families() {
        let m = ForgeMetrics::new();
        // Initialize at least one label so the GaugeVec appears in gather()
        m.worker_healthy.with_label_values(&["extractor"]).set(1);
        let families = m.registry.gather();
        let names: Vec<&str> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.contains(&"forge_memories_total"),
            "missing forge_memories_total"
        );
        assert!(
            names.contains(&"forge_recall_latency_seconds"),
            "missing forge_recall_latency_seconds"
        );
        assert!(
            names.contains(&"forge_extraction_duration_seconds"),
            "missing forge_extraction_duration_seconds"
        );
        assert!(
            names.contains(&"forge_worker_healthy"),
            "missing forge_worker_healthy"
        );
        assert!(
            names.contains(&"forge_active_sessions"),
            "missing forge_active_sessions"
        );
        assert!(
            names.contains(&"forge_edges_total"),
            "missing forge_edges_total"
        );
        assert!(
            names.contains(&"forge_embeddings_total"),
            "missing forge_embeddings_total"
        );
        // Phase 2A-4d.1 Instrumentation — 3 new families.
        // Initialize at least one label so the *Vec collectors appear in gather().
        m.phase_duration
            .with_label_values(&["phase_23_infer_skills_from_behavior"])
            .observe(0.0);
        m.phase_output_rows
            .with_label_values(&["phase_23_infer_skills_from_behavior", "succeeded"])
            .inc_by(0);
        m.table_rows.with_label_values(&["skill"]).set(0);
        m.phase_persistence_errors
            .with_label_values(&["phase_23_infer_skills_from_behavior", "insert_error"])
            .inc_by(0);
        m.gauge_refresh_failures
            .with_label_values(&["open_reader"])
            .inc_by(0);
        // Phase 2A-4d.2 — 1 new family (layer_freshness).
        m.layer_freshness.with_label_values(&["skill"]).set(-1);
        let families = m.registry.gather();
        let names: Vec<&str> = families.iter().map(|f| f.get_name()).collect();
        assert!(
            names.contains(&"forge_phase_duration_seconds"),
            "missing forge_phase_duration_seconds"
        );
        assert!(
            names.contains(&"forge_phase_output_rows_total"),
            "missing forge_phase_output_rows_total"
        );
        assert!(
            names.contains(&"forge_table_rows"),
            "missing forge_table_rows"
        );
        assert!(
            names.contains(&"forge_phase_persistence_errors_total"),
            "missing forge_phase_persistence_errors_total"
        );
        assert!(
            names.contains(&"forge_gauge_refresh_failures_total"),
            "missing forge_gauge_refresh_failures_total"
        );
        assert!(
            names.contains(&"forge_layer_freshness_seconds"),
            "missing forge_layer_freshness_seconds"
        );
        assert_eq!(families.len(), 13, "expected exactly 13 metric families");
    }

    #[test]
    fn test_forge_metrics_gauge_set_and_read() {
        let m = ForgeMetrics::new();
        m.memories_total.set(42);
        assert_eq!(m.memories_total.get(), 42);
        m.active_sessions.set(3);
        assert_eq!(m.active_sessions.get(), 3);
        m.edges_total.set(100);
        assert_eq!(m.edges_total.get(), 100);
        m.embeddings_total.set(500);
        assert_eq!(m.embeddings_total.get(), 500);
    }

    #[tokio::test]
    async fn test_metrics_endpoint_returns_prometheus_format() {
        let metrics = Arc::new(ForgeMetrics::new());
        let state = test_app_state_with_metrics(Some(metrics));

        let mut config = ForgeConfig::default();
        config.metrics.enabled = true;
        let app = build_router(&config, state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let content_type = response
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            content_type.contains("text/plain"),
            "content-type should be text/plain, got: {content_type}"
        );

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(body.to_vec()).unwrap();
        // refresh_gauges queries the real test DB — empty DB has 0 memories
        assert!(
            text.contains("forge_memories_total 0"),
            "body should contain memories gauge from DB query"
        );
        assert!(
            text.contains("forge_active_sessions 0"),
            "body should contain active_sessions gauge from DB query"
        );
        assert!(
            text.contains("forge_recall_latency_seconds"),
            "body should contain recall_latency histogram"
        );
    }

    #[tokio::test]
    async fn test_refresh_gauges_query_failure_bumps_counter() {
        // Regression test for T14 LOW-1: a schema-drift / table-missing
        // failure inside refresh_gauges must increment
        // forge_gauge_refresh_failures_total{reason="query_failed"} so
        // operators can alert on the otherwise-silent gauge freeze.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().to_string();
        {
            let state = DaemonState::new(&db_path).unwrap();
            // Drop `memory` to force the single-SELECT in refresh_gauges to fail.
            state.conn.execute("DROP TABLE memory", []).unwrap();
        }
        // Keep the tempfile alive for the read-only reopen below.
        std::mem::forget(tmp);

        let metrics = Arc::new(ForgeMetrics::new());
        let state = test_app_state_with_metrics_and_db(Some(metrics.clone()), db_path);

        // Trigger a refresh; should warn + increment "query_failed" counter.
        refresh_gauges(&metrics, &state);

        let got = metrics
            .gauge_refresh_failures
            .with_label_values(&["query_failed"])
            .get();
        assert_eq!(
            got, 1,
            "expected gauge_refresh_failures{{query_failed}}=1 after a dropped-table refresh, got {got}"
        );
    }

    fn test_app_state_with_metrics_and_db(
        metrics: Option<Arc<ForgeMetrics>>,
        db_path: String,
    ) -> AppState {
        let (events, _) = tokio::sync::broadcast::channel(16);
        let hlc = Arc::new(Hlc::new("test"));
        let (write_tx, write_rx) = mpsc::channel(16);
        std::mem::forget(write_rx);
        AppState {
            db_path,
            events,
            hlc,
            started_at: Instant::now(),
            write_tx,
            admin_emails: vec![],
            viewer_emails: vec![],
            auth_enabled: false,
            metrics,
            rate_limiter: None,
        }
    }

    #[tokio::test]
    async fn test_metrics_disabled_returns_404() {
        let state = test_app_state_with_metrics(None);
        let mut config = ForgeConfig::default();
        config.metrics.enabled = false;
        let app = build_router(&config, state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "/metrics should not be registered when disabled"
        );
    }

    #[test]
    fn test_worker_healthy_labels() {
        let m = ForgeMetrics::new();
        m.worker_healthy.with_label_values(&["extractor"]).set(1);
        m.worker_healthy.with_label_values(&["embedder"]).set(0);
        assert_eq!(m.worker_healthy.with_label_values(&["extractor"]).get(), 1);
        assert_eq!(m.worker_healthy.with_label_values(&["embedder"]).get(), 0);
    }

    #[test]
    fn test_histogram_observe() {
        let m = ForgeMetrics::new();
        m.recall_latency.observe(0.05);
        m.recall_latency.observe(0.1);
        m.extraction_duration.observe(2.5);
        // Verify histogram has recorded observations
        let families = m.registry.gather();
        let recall_family = families
            .iter()
            .find(|f| f.get_name() == "forge_recall_latency_seconds")
            .expect("recall family");
        let metric = &recall_family.get_metric()[0];
        assert_eq!(metric.get_histogram().get_sample_count(), 2);
    }

    // ── Phase 2A-4d.2 T4: GaugeSnapshot ──

    #[test]
    fn test_gauge_snapshot_initially_never_refreshed() {
        let m = ForgeMetrics::new();
        let snap = m.snapshot.read().clone();
        assert_eq!(
            snap.refreshed_at_secs, 0,
            "fresh ForgeMetrics should have refreshed_at_secs = 0"
        );
        assert_eq!(snap.tables.memory.count, 0);
        assert_eq!(snap.tables.memory.freshness_secs, None);
        assert_eq!(snap.memories_total, 0);
    }

    #[test]
    fn test_table_gauges_to_layer_rows_covers_all_eleven_tables() {
        let tg = TableGauges::default();
        let rows = tg.to_layer_rows(5);
        assert_eq!(rows.len(), 11, "expected exactly 11 Manas tables");
        let names: Vec<&str> = rows.iter().map(|r| r.layer.as_str()).collect();
        for expected in [
            "memory",
            "skill",
            "edge",
            "identity",
            "disposition",
            "platform",
            "tool",
            "perception",
            "declared",
            "domain_dna",
            "entity",
        ] {
            assert!(names.contains(&expected), "missing table: {expected}");
        }
        assert!(rows.iter().all(|r| r.snapshot_age_secs == 5));
        assert!(rows.iter().all(|r| r.freshness_secs.is_none()));
    }

    #[test]
    fn test_gauge_snapshot_torn_read_stress_sees_consistent_view() {
        // Reader side: repeatedly clone the snapshot and check that
        // (refreshed_at_secs, memories_total) come from the same generation
        // — writer picks monotonically increasing pairs so a torn read would
        // show (old_ts, new_count) or (new_ts, old_count).
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let m = ForgeMetrics::new();
        let stop = Arc::new(AtomicBool::new(false));

        let writer_snap = Arc::clone(&m.snapshot);
        let stop_w = Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            let mut gen: i64 = 1;
            while !stop_w.load(Ordering::Relaxed) {
                // Each generation has refreshed_at_secs == memories_total (a
                // trivially-invariant pairing that a torn read would break).
                let new_snap = GaugeSnapshot {
                    refreshed_at_secs: gen as u64,
                    memories_total: gen,
                    edges_total: gen,
                    embeddings_total: gen,
                    active_sessions: gen,
                    tables: TableGauges::default(),
                };
                *writer_snap.write() = new_snap;
                gen += 1;
            }
            gen
        });

        // Do a substantial number of reads on this thread while the writer
        // hammers the snapshot.
        for _ in 0..10_000 {
            let snap = m.snapshot.read().clone();
            assert_eq!(
                snap.refreshed_at_secs as i64, snap.memories_total,
                "torn read detected: {snap:?}"
            );
        }
        stop.store(true, Ordering::Relaxed);
        let _ = writer.join();
    }
}
