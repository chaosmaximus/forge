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
            return;
        }
    };

    // Per-table row gauges. Labels match actual SQLite table names (verified
    // 2026-04-24 in schema.rs); adding a label that doesn't correspond to a
    // real table would produce a perpetually-zero series that misleads
    // operators. `memory` and `edge` intentionally appear here *and* feed
    // the dedicated memories_total / edges_total gauges — the single SELECT
    // below returns each count exactly once and we reuse it for both sinks.
    let tables = [
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
    ];

    // Single-row SELECT with scalar subqueries — all 15 counts come back in
    // one round-trip, evaluated against one implicit read snapshot.
    const COUNTS_SQL: &str = "SELECT \
        (SELECT COUNT(*) FROM memory)                          AS mem, \
        (SELECT COUNT(*) FROM edge)                            AS edg, \
        (SELECT COUNT(*) FROM memory_vec)                      AS vec, \
        (SELECT COUNT(*) FROM session WHERE ended_at IS NULL)  AS active_sess, \
        (SELECT COUNT(*) FROM skill)                           AS skl, \
        (SELECT COUNT(*) FROM identity)                        AS idn, \
        (SELECT COUNT(*) FROM disposition)                     AS dsp, \
        (SELECT COUNT(*) FROM platform)                        AS plt, \
        (SELECT COUNT(*) FROM tool)                            AS tol, \
        (SELECT COUNT(*) FROM perception)                      AS prc, \
        (SELECT COUNT(*) FROM declared)                        AS dcl, \
        (SELECT COUNT(*) FROM domain_dna)                      AS dna, \
        (SELECT COUNT(*) FROM entity)                          AS ent";

    let row = reader.conn.query_row(COUNTS_SQL, [], |r| {
        Ok((
            r.get::<_, i64>(0)?,  // memory
            r.get::<_, i64>(1)?,  // edge
            r.get::<_, i64>(2)?,  // memory_vec
            r.get::<_, i64>(3)?,  // active sessions
            r.get::<_, i64>(4)?,  // skill
            r.get::<_, i64>(5)?,  // identity
            r.get::<_, i64>(6)?,  // disposition
            r.get::<_, i64>(7)?,  // platform
            r.get::<_, i64>(8)?,  // tool
            r.get::<_, i64>(9)?,  // perception
            r.get::<_, i64>(10)?, // declared
            r.get::<_, i64>(11)?, // domain_dna
            r.get::<_, i64>(12)?, // entity
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
    ) = match row {
        Ok(r) => r,
        Err(e) => {
            // Any table missing / schema drift → skip this scrape entirely
            // rather than partially-update gauges (preserves all-or-nothing
            // semantics from the prior BEGIN/COMMIT design).
            tracing::warn!(error = %e, "metrics: failed to collect DB counts; skipping gauge refresh");
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
    let per_table_counts: [i64; 11] = [
        count_memory,
        count_skill,
        count_edge,
        count_identity,
        count_disposition,
        count_platform,
        count_tool,
        count_perception,
        count_declared,
        count_domain_dna,
        count_entity,
    ];
    for (table, count) in tables.iter().zip(per_table_counts.iter()) {
        metrics.table_rows.with_label_values(&[*table]).set(*count);
    }
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
        assert_eq!(families.len(), 11, "expected exactly 11 metric families");
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
}
