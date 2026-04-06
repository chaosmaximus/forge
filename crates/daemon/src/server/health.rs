//! Health probe endpoints for Kubernetes-style liveness, readiness, and startup checks.
//!
//! - GET /healthz  -> liveness (always OK)
//! - GET /readyz   -> readiness (verifies DB connection, reports worker count)
//! - GET /startupz -> startup (reports whether initial indexing is complete)

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use super::http::AppState;

/// Liveness probe response — always returns OK.
#[derive(Serialize)]
struct LivenessResponse {
    status: &'static str,
}

/// Readiness probe response — verifies DB is accessible.
#[derive(Serialize)]
struct ReadinessResponse {
    status: &'static str,
    workers: usize,
}

/// Startup probe response — reports indexing status.
#[derive(Serialize)]
struct StartupResponse {
    status: &'static str,
    indexed: bool,
}

/// GET /healthz — liveness probe. Always returns 200 OK.
pub async fn healthz() -> impl IntoResponse {
    Json(LivenessResponse { status: "ok" })
}

/// GET /readyz — readiness probe. Verifies both the database connection
/// and the write path (writer actor channel) are healthy.
pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    // Check 1: Read path — open a read-only connection and verify it works
    match crate::server::handler::DaemonState::new_reader(
        &state.db_path,
        state.events.clone(),
        std::sync::Arc::clone(&state.hlc),
        state.started_at,
    ) {
        Ok(reader) => {
            // Verify DB is actually responsive with a simple query
            match reader.conn.query_row("SELECT 1", [], |row| row.get::<_, i32>(0)) {
                Ok(_) => {
                    // Check 2: Write path — verify the writer actor channel is open
                    // (if closed, all writes would fail)
                    if state.write_tx.is_closed() {
                        tracing::error!("readyz: writer actor channel is closed");
                        return (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({"status": "error", "message": "writer unavailable"})),
                        )
                            .into_response();
                    }
                    // 8 workers: watcher, extractor, embedder, consolidator,
                    // indexer, perception, disposition, diagnostics
                    let workers = 8;
                    Json(ReadinessResponse {
                        status: "ok",
                        workers,
                    })
                    .into_response()
                }
                Err(e) => {
                    tracing::error!("readyz: db query failed: {e}");
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(serde_json::json!({"status": "error", "message": "database not responding"})),
                    )
                        .into_response()
                }
            }
        }
        Err(e) => {
            tracing::error!("readyz: db connection failed: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"status": "error", "message": "database unavailable"})),
            )
                .into_response()
        }
    }
}

/// GET /startupz — startup probe. Reports whether initial indexing has completed.
/// Returns 503 while startup is in progress (indexed=false), 200 when ready.
/// K8s uses this to know when the container is ready to accept traffic.
pub async fn startupz(State(state): State<AppState>) -> impl IntoResponse {
    match crate::server::handler::DaemonState::new_reader(
        &state.db_path,
        state.events.clone(),
        std::sync::Arc::clone(&state.hlc),
        state.started_at,
    ) {
        Ok(reader) => {
            // Check if we have any indexed memories (indicates startup indexing is done)
            let indexed = reader
                .conn
                .query_row("SELECT COUNT(*) FROM memories", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap_or(0)
                > 0;
            if indexed {
                Json(StartupResponse {
                    status: "ok",
                    indexed,
                })
                .into_response()
            } else {
                // Return 503 while startup is incomplete — K8s will retry
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(StartupResponse {
                        status: "starting",
                        indexed,
                    }),
                )
                    .into_response()
            }
        }
        Err(e) => {
            tracing::error!("startupz: db connection failed: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"status": "error", "message": "database unavailable"})),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handler::DaemonState;
    use crate::sync::Hlc;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use axum::Router;
    use http_body_util::BodyExt;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    fn test_app_state() -> AppState {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().to_string();
        // Create the DB with schema so reads work
        let _state = DaemonState::new(&db_path).unwrap();
        let (events, _) = tokio::sync::broadcast::channel(16);
        let hlc = Arc::new(Hlc::new("test"));
        let (write_tx, write_rx) = mpsc::channel(16);
        // Keep temp file and write_rx alive (test only — prevents is_closed() returning true)
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
            metrics: None,
        }
    }

    fn test_router(state: AppState) -> Router {
        Router::new()
            .route("/healthz", axum::routing::get(healthz))
            .route("/readyz", axum::routing::get(readyz))
            .route("/startupz", axum::routing::get(startupz))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_healthz_returns_ok() {
        let state = test_app_state();
        let app = test_router(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_readyz_returns_ok_with_workers() {
        let state = test_app_state();
        let app = test_router(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "ok");
        assert!(json["workers"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_startupz_returns_503_when_not_indexed() {
        let state = test_app_state();
        let app = test_router(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/startupz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Empty test DB has no memories → indexed=false → 503
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "starting");
        assert_eq!(json["indexed"], false);
    }
}
