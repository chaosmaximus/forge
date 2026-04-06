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
/// Returns proper HTTP status codes: 200 for success, 503 for infrastructure failures.
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
            Err(e) => {
                tracing::error!("failed to open read-only connection: {e}");
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    Json(Response::Error {
                        message: "database unavailable".to_string(),
                    }),
                )
                    .into_response();
            }
        }
    } else {
        // Send write request through the writer actor with timeout (same as socket.rs)
        let (reply_tx, reply_rx) = oneshot::channel();
        match tokio::time::timeout(
            Duration::from_secs(30),
            state.write_tx.send(WriteCommand::Raw {
                request,
                reply: reply_tx,
            }),
        )
        .await
        {
            Ok(Ok(())) => {
                match tokio::time::timeout(Duration::from_secs(30), reply_rx).await {
                    Ok(Ok(resp)) => resp,
                    Ok(Err(_)) => {
                        tracing::error!("writer actor closed unexpectedly");
                        return (
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            Json(Response::Error {
                                message: "writer unavailable".to_string(),
                            }),
                        )
                            .into_response();
                    }
                    Err(_) => {
                        tracing::error!("write request timed out after 30s");
                        return (
                            axum::http::StatusCode::GATEWAY_TIMEOUT,
                            Json(Response::Error {
                                message: "write request timed out".to_string(),
                            }),
                        )
                            .into_response();
                    }
                }
            }
            Ok(Err(_)) => {
                tracing::error!("daemon writer channel closed");
                return (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    Json(Response::Error {
                        message: "writer unavailable".to_string(),
                    }),
                )
                    .into_response();
            }
            Err(_) => {
                tracing::error!("write channel send timed out after 30s");
                return (
                    axum::http::StatusCode::GATEWAY_TIMEOUT,
                    Json(Response::Error {
                        message: "write request timed out".to_string(),
                    }),
                )
                    .into_response();
            }
        }
    };

    // Protocol-level errors still return 200 (they're valid protocol responses)
    // Infrastructure failures above return 503/504
    Json(response).into_response()
}

/// Build the CORS layer from config.
/// When auth is disabled and origins contain "*", log a security warning.
fn build_cors_layer(config: &ForgeConfig) -> CorsLayer {
    let is_wildcard = config.cors.allowed_origins.contains(&"*".to_string());
    let layer = if is_wildcard {
        if !config.auth.enabled {
            tracing::warn!(
                "CORS wildcard (*) is active with auth DISABLED — \
                 the API is browser-callable from any origin. \
                 Set cors.allowed_origins to specific origins or enable auth for production."
            );
        }
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

/// Build the axum router with all routes.
///
/// Health probes are EXEMPT from auth (K8s must access them without tokens).
/// When `config.auth.enabled` is true, POST /api requires a valid JWT.
pub fn build_router(config: &ForgeConfig, state: AppState) -> Router {
    let cors = build_cors_layer(config);

    // Health probes — always unauthenticated
    let health_routes = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/startupz", get(startupz))
        .with_state(state.clone());

    // API routes — optionally protected by JWT auth
    let api_routes = if config.auth.enabled {
        let jwks_cache = super::auth::new_jwks_cache();
        let auth_config = config.auth.clone();
        tracing::info!(
            issuer = %config.auth.issuer_url,
            audience = %config.auth.audience,
            "JWT auth enabled for POST /api"
        );
        Router::new()
            .route("/api", post(api_handler))
            .layer(axum::middleware::from_fn(move |req, next| {
                let cache = jwks_cache.clone();
                let cfg = auth_config.clone();
                super::auth::auth_middleware(req, next, cache, cfg)
            }))
            .with_state(state)
    } else {
        Router::new()
            .route("/api", post(api_handler))
            .with_state(state)
    };

    Router::new()
        .merge(health_routes)
        .merge(api_routes)
        .layer(cors)
}

/// Start the HTTP server with a pre-bound listener and graceful shutdown.
/// main.rs binds the listener early so bind failures are caught at startup.
#[allow(clippy::too_many_arguments)]
pub async fn run_http_server_with_listener(
    config: &ForgeConfig,
    db_path: String,
    events: EventSender,
    hlc: Arc<crate::sync::Hlc>,
    started_at: Instant,
    write_tx: mpsc::Sender<WriteCommand>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    let state = AppState {
        db_path,
        events,
        hlc,
        started_at,
        write_tx,
    };

    let app = build_router(config, state);

    // Graceful shutdown: drain in-flight requests when shutdown signal received
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.changed().await;
            tracing::info!("HTTP server shutting down gracefully");
        })
        .await?;
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
        let (write_tx, write_rx) = mpsc::channel(16);
        // Keep temp file and write_rx alive (test only)
        std::mem::forget(tmp);
        std::mem::forget(write_rx);
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
        build_router(&config, state)
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

    fn test_router_with_auth(state: AppState) -> Router {
        let mut config = ForgeConfig::default();
        config.auth.enabled = true;
        config.auth.issuer_url = "https://test-issuer.example.com".to_string();
        config.auth.audience = "forge-api".to_string();
        build_router(&config, state)
    }

    #[tokio::test]
    async fn test_health_probes_exempt_from_auth() {
        let state = test_app_state();
        let app = test_router_with_auth(state);

        // healthz should work without any auth token
        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "healthz should be exempt from auth"
        );
    }

    #[tokio::test]
    async fn test_readyz_exempt_from_auth() {
        let state = test_app_state();
        let app = test_router_with_auth(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "readyz should be exempt from auth"
        );
    }

    #[tokio::test]
    async fn test_api_requires_auth_when_enabled() {
        let state = test_app_state();
        let app = test_router_with_auth(state);

        // POST /api without Bearer token should return 401
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

        assert_eq!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "POST /api without token should be 401 when auth enabled"
        );

        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(json["error"].as_str().is_some());
    }

    #[tokio::test]
    async fn test_api_no_auth_when_disabled() {
        // Default config has auth.enabled = false
        let state = test_app_state();
        let app = test_router(state);

        // POST /api without Bearer token should work when auth is disabled
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

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "POST /api should work without auth when auth is disabled"
        );
    }
}
