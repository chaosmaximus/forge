//! Axum HTTP server — thin wrapper around the same handle_request used by socket.rs.
//!
//! POST /api accepts the same JSON body as the Unix socket protocol.
//! Read/write routing follows the identical pattern:
//!   - Read-only requests: per-request DaemonState::new_reader
//!   - Write requests: sent through write_tx channel to the WriterActor

use crate::config::ForgeConfig;
use crate::events::EventSender;
use crate::server::handler::{handle_request, DaemonState};
use crate::server::writer::{is_read_only, WriteCommand};
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::Method;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use forge_core::protocol::{Request, Response};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tower_http::cors::{Any, CorsLayer};

use super::health::{healthz, readyz, startupz};

/// Shared application state for all HTTP handlers.
#[derive(Clone)]
pub struct AppState {
    pub db_path: String,
    pub events: EventSender,
    pub hlc: Arc<crate::sync::Hlc>,
    pub started_at: Instant,
    pub write_tx: mpsc::Sender<WriteCommand>,
}

/// POST /api — accepts JSON matching the protocol Request type.
/// Routes reads to per-request read-only connections, writes through the writer actor.
async fn api_handler(
    State(state): State<AppState>,
    Json(request): Json<Request>,
) -> impl IntoResponse {
    let response = if is_read_only(&request) {
        // Open per-request read-only connection (same pattern as socket.rs)
        match DaemonState::new_reader(
            &state.db_path,
            state.events.clone(),
            Arc::clone(&state.hlc),
            state.started_at,
        ) {
            Ok(mut reader) => handle_request(&mut reader, request),
            Err(e) => Response::Error {
                message: format!("failed to open read-only connection: {e}"),
            },
        }
    } else {
        // Send write request through the writer actor (same as socket.rs)
        let (reply_tx, reply_rx) = oneshot::channel();
        match state
            .write_tx
            .send(WriteCommand::Raw {
                request,
                reply: reply_tx,
            })
            .await
        {
            Ok(()) => match reply_rx.await {
                Ok(resp) => resp,
                Err(_) => Response::Error {
                    message: "writer actor closed unexpectedly".to_string(),
                },
            },
            Err(_) => Response::Error {
                message: "daemon writer unavailable".to_string(),
            },
        }
    };

    Json(response)
}

/// Build the CORS layer from config.
fn build_cors_layer(config: &ForgeConfig) -> CorsLayer {
    let layer = if config.cors.allowed_origins.contains(&"*".to_string()) {
        CorsLayer::new().allow_origin(Any)
    } else {
        let origins: Vec<axum::http::HeaderValue> = config
            .cors
            .allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        CorsLayer::new().allow_origin(origins)
    };

    layer
        .allow_methods(vec![Method::GET, Method::POST])
        .allow_headers(vec![AUTHORIZATION, CONTENT_TYPE])
        .max_age(Duration::from_secs(config.cors.max_age_secs))
}

/// Start the HTTP server. Call from main.rs inside a tokio::spawn.
pub async fn run_http_server(
    config: &ForgeConfig,
    db_path: String,
    events: EventSender,
    hlc: Arc<crate::sync::Hlc>,
    started_at: Instant,
    write_tx: mpsc::Sender<WriteCommand>,
) -> std::io::Result<()> {
    let cors = build_cors_layer(config);

    let state = AppState {
        db_path,
        events,
        hlc,
        started_at,
        write_tx,
    };

    let app = Router::new()
        .route("/api", post(api_handler))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/startupz", get(startupz))
        .layer(cors)
        .with_state(state);

    let addr = format!("{}:{}", config.http.bind, config.http.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "HTTP server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handler::DaemonState;
    use crate::sync::Hlc;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_app_state() -> AppState {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().to_string();
        // Create DB with full schema so handler works
        let _state = DaemonState::new(&db_path).unwrap();
        let (events, _) = tokio::sync::broadcast::channel(16);
        let hlc = Arc::new(Hlc::new("test"));
        let (write_tx, _write_rx) = mpsc::channel(16);
        // Keep temp file alive
        std::mem::forget(tmp);
        AppState {
            db_path,
            events,
            hlc,
            started_at: Instant::now(),
            write_tx,
        }
    }

    fn test_router(state: AppState) -> Router {
        let config = ForgeConfig::default();
        let cors = build_cors_layer(&config);
        Router::new()
            .route("/api", post(api_handler))
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/startupz", get(startupz))
            .layer(cors)
            .with_state(state)
    }

    #[tokio::test]
    async fn test_post_api_health_roundtrip() {
        let state = test_app_state();
        let app = test_router(state);

        // Send a Health request (read-only) through POST /api
        let body = serde_json::json!({"method": "health"});
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        // Health response should have status field
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_post_api_doctor_roundtrip() {
        let state = test_app_state();
        let app = test_router(state);

        let body = serde_json::json!({"method": "doctor"});
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        // Doctor response should have the doctor data
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn test_cors_headers_present() {
        let state = test_app_state();
        let app = test_router(state);

        // Send an OPTIONS preflight request
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("OPTIONS")
                    .uri("/api")
                    .header("origin", "http://localhost:3000")
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-headers", "content-type")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // CORS should be active — check for access-control-allow-origin header
        let headers = response.headers();
        assert!(
            headers.contains_key("access-control-allow-origin"),
            "Expected CORS header access-control-allow-origin, got headers: {:?}",
            headers
        );
    }

    #[tokio::test]
    async fn test_post_api_invalid_json_returns_error() {
        let state = test_app_state();
        let app = test_router(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .body(Body::from("{invalid json}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should return 4xx for bad JSON
        assert!(response.status().is_client_error());
    }

    #[tokio::test]
    async fn test_get_api_returns_method_not_allowed() {
        let state = test_app_state();
        let app = test_router(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("GET")
                    .uri("/api")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // GET on /api should not work — POST only
        assert_eq!(
            response.status(),
            axum::http::StatusCode::METHOD_NOT_ALLOWED
        );
    }
}
