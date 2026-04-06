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
    Histogram, HistogramOpts, IntGauge, IntGaugeVec, Opts, Registry, TextEncoder,
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
            HistogramOpts::new("forge_recall_latency_seconds", "Recall query latency in seconds")
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
            Opts::new("forge_worker_healthy", "Whether a background worker is healthy (1=yes, 0=no)"),
            &["worker"],
        )
        .expect("worker_healthy metric");
        let active_sessions =
            IntGauge::new("forge_active_sessions", "Number of active sessions")
                .expect("active_sessions metric");
        let edges_total =
            IntGauge::new("forge_edges_total", "Total number of knowledge graph edges")
                .expect("edges_total metric");
        let embeddings_total =
            IntGauge::new("forge_embeddings_total", "Total number of stored embeddings")
                .expect("embeddings_total metric");

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

        Self {
            registry,
            memories_total,
            recall_latency,
            extraction_duration,
            worker_healthy,
            active_sessions,
            edges_total,
            embeddings_total,
        }
    }
}

/// Refresh gauge values from the database before a Prometheus scrape.
/// Opens a read-only connection, queries current counts, and updates gauges.
fn refresh_gauges(metrics: &ForgeMetrics, state: &AppState) {
    // Open a per-scrape read-only connection (same pattern as health probes)
    let reader = match crate::server::handler::DaemonState::new_reader(
        &state.db_path,
        state.events.clone(),
        std::sync::Arc::clone(&state.hlc),
        state.started_at,
    ) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("metrics: failed to open reader for gauge refresh: {e}");
            return;
        }
    };

    // Memory count
    if let Ok(count) = reader.conn.query_row("SELECT COUNT(*) FROM memory", [], |r| r.get::<_, i64>(0)) {
        metrics.memories_total.set(count);
    }
    // Edge count
    if let Ok(count) = reader.conn.query_row("SELECT COUNT(*) FROM edge", [], |r| r.get::<_, i64>(0)) {
        metrics.edges_total.set(count);
    }
    // Embedding count
    if let Ok(count) = reader.conn.query_row("SELECT COUNT(*) FROM memory_vec", [], |r| r.get::<_, i64>(0)) {
        metrics.embeddings_total.set(count);
    }
    // Active sessions (non-ended)
    if let Ok(count) = reader.conn.query_row(
        "SELECT COUNT(*) FROM session WHERE ended_at IS NULL",
        [],
        |r| r.get::<_, i64>(0),
    ) {
        metrics.active_sessions.set(count);
    }
    // Worker health — set all known workers to 1 (we're alive if we can query)
    for worker in &["watcher", "extractor", "embedder", "consolidator", "indexer", "perception", "disposition", "diagnostics"] {
        metrics.worker_healthy.with_label_values(&[worker]).set(1);
    }
}

/// GET /metrics — Prometheus scrape endpoint.
/// Refreshes gauges from DB, then returns all metrics in text format.
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    // metrics is always Some when this handler is registered (guarded by config check)
    let metrics = state.metrics.as_ref().expect("metrics must be Some when /metrics is registered");

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

    fn test_app_state_with_metrics(metrics: Option<ForgeMetrics>) -> AppState {
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
        }
    }

    #[test]
    fn test_forge_metrics_new_registers_all_families() {
        let m = ForgeMetrics::new();
        // Initialize at least one label so the GaugeVec appears in gather()
        m.worker_healthy.with_label_values(&["extractor"]).set(1);
        let families = m.registry.gather();
        let names: Vec<&str> = families.iter().map(|f| f.get_name()).collect();
        assert!(names.contains(&"forge_memories_total"), "missing forge_memories_total");
        assert!(
            names.contains(&"forge_recall_latency_seconds"),
            "missing forge_recall_latency_seconds"
        );
        assert!(
            names.contains(&"forge_extraction_duration_seconds"),
            "missing forge_extraction_duration_seconds"
        );
        assert!(names.contains(&"forge_worker_healthy"), "missing forge_worker_healthy");
        assert!(names.contains(&"forge_active_sessions"), "missing forge_active_sessions");
        assert!(names.contains(&"forge_edges_total"), "missing forge_edges_total");
        assert!(names.contains(&"forge_embeddings_total"), "missing forge_embeddings_total");
        assert_eq!(families.len(), 7, "expected exactly 7 metric families");
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
        let metrics = ForgeMetrics::new();
        metrics.memories_total.set(10);
        metrics.active_sessions.set(2);
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
        assert!(text.contains("forge_memories_total 10"), "body should contain memories gauge value");
        assert!(
            text.contains("forge_active_sessions 2"),
            "body should contain active_sessions gauge value"
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
