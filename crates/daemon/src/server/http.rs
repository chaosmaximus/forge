//! Axum HTTP server — thin wrapper around the same handle_request used by socket.rs.
//!
//! POST /api accepts the same JSON body as the Unix socket protocol.
//! Read/write routing follows the identical pattern:
//!   - Read-only requests: per-request DaemonState::new_reader
//!   - Write requests: sent through write_tx channel to the WriterActor
//!
//! When auth is enabled, RBAC is enforced before handling:
//!   - Admin: all operations
//!   - Member: read + write, no admin operations
//!   - Viewer: read-only operations only

use crate::config::ForgeConfig;
use crate::events::EventSender;
use crate::server::handler::{handle_request, DaemonState};
use crate::server::rbac::{check_permission, resolve_role};
use crate::server::writer::{is_read_only, AuditContext, WriteCommand};
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
    /// Admin email list for RBAC role resolution (empty = no admins configured).
    pub admin_emails: Vec<String>,
    /// Whether auth (and thus RBAC) is enabled.
    pub auth_enabled: bool,
}

/// POST /api — accepts JSON matching the protocol Request type.
/// Routes reads to per-request read-only connections, writes through the writer actor.
/// Returns proper HTTP status codes: 200 for success, 503 for infrastructure failures.
///
/// When auth is enabled:
/// - RBAC is enforced before handling (returns 403 if denied)
/// - Write operations carry AuditContext for the writer actor to log
async fn api_handler(
    State(state): State<AppState>,
    http_req: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    // Extract auth claims from extensions (injected by auth_middleware)
    let claims = http_req
        .extensions()
        .get::<super::auth::AuthClaims>()
        .cloned();

    // Parse JSON body
    let body_bytes = match axum::body::to_bytes(http_req.into_body(), 10 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(Response::Error {
                    message: format!("invalid request body: {e}"),
                }),
            )
                .into_response();
        }
    };
    let request: Request = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => {
            return (
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                Json(Response::Error {
                    message: format!("invalid JSON: {e}"),
                }),
            )
                .into_response();
        }
    };

    // RBAC check: only when auth is enabled and claims are present
    let audit_ctx = if state.auth_enabled {
        if let Some(ref c) = claims {
            let role = resolve_role(c, &state.admin_emails);
            if let Err(reason) = check_permission(&role, &request) {
                return (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": reason})),
                )
                    .into_response();
            }
            // Build audit context for writes
            Some(AuditContext {
                user_id: c.sub.clone(),
                email: c.email.clone().unwrap_or_default(),
                role: role.as_str().to_string(),
                source: "http".to_string(),
                source_ip: String::new(), // TODO: extract from ConnectInfo if needed
            })
        } else {
            None
        }
    } else {
        None
    };

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
        // Send write request through the writer actor with timeout
        let (reply_tx, reply_rx) = oneshot::channel();
        let cmd = if let Some(audit) = audit_ctx {
            WriteCommand::Audited {
                request,
                reply: reply_tx,
                audit,
            }
        } else {
            WriteCommand::Raw {
                request,
                reply: reply_tx,
            }
        };
        match tokio::time::timeout(Duration::from_secs(30), state.write_tx.send(cmd)).await {
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
        if config.auth.admin_emails.is_empty() {
            tracing::warn!(
                "auth is enabled but admin_emails is empty — no user can be assigned Admin role. \
                 All authenticated users will be Members."
            );
        }
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
        admin_emails: config.auth.admin_emails.clone(),
        auth_enabled: config.auth.enabled,
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
            admin_emails: Vec::new(),
            auth_enabled: false,
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

    // ── Wave 5: Full-stack HTTP+Auth+RBAC integration tests ──

    // Test RSA key pair (same as auth.rs tests — deterministic, offline-only).
    const TEST_RSA_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQCgGb81p6iwoRNK
f0dSsbm/x9pZRhT02u6vsRcuzHgZ1AuShGNpEedX0r0wkUb9hLAC+RpbkKDTjQkA
poBRYRIIC2Y0rfDPfk7D1JX8p/XRqV40XQIAq42hrAZdpZAjPlvb2Isycm7qtTuT
2U04p0ZuBiOm9p1CL6yz1jC9NW8kZlzN1d6oGjFwzMbLB9YWxyP+7VS/mupF0KWO
9RCwoynX7fwSbHs0c4N1eFgievNWU5Fx86NAUTNOaO2V+VcUZWWWvkXYoKiWRDE2
8nu+UIwS+0ir5jdTUQWLNd6TROcL+Cdsdzm3yyYBRovtgb0WjT8J14/01YjeJjFx
OlGewM8lAgMBAAECggEAN9JSV4BiMlevNLnlIeGi3MnviVIEq40MTQjnhuM2+vZy
pH7xdGiQK5Boc58ry+gwQJEfTg7C7JAPtADZ28YHNfPXioWdYZNuHhyowSPE83nk
xUgqkxY9t0GWJJ+9/nPXLnO1sPyyTLatE7NgF+FHDsSoOKZjFXku87M6YjZXzq4u
vm4yhr4Jlhcc2nzgozszsqq2LlH9hiOwD8IskSIWNi7cTtf0DcWQZ4hveW7LWaw6
CH0+ugJ4gNujBwMz/x5iF4ZSbRhYIe9FLV6gjlObTGKi994pSVPfDt00lEY8FwAR
F7lR8iW9p/NmTcs4vGqzAD6IBBVOrixkJ4Fb7SdPnwKBgQDW8yCsY8ShBtq+I0pk
4hh+JuXPN3M4fMB8GqNXo5W2k9T7L4PHHnyO98Yl+r4KRqwz59YlbMB4zof3wXFV
fFPc/S6H5NBWxHiJEdWNphGBKfRRBH9+UEIIfivmIBJtkKECPivcA+ZGZGqajIQz
hG2xUrxhAD2hEkO3vxLURfvfKwKBgQC+rQyxu47+cfhoukXw92yzXh9GMxHsNXxi
FPLpYk1PgI+Svq+aA2e4LVv8ncib6QkIdxVWtoenWuadFPm1PX9C80LmQF5ASIXr
v9w0PpIedFW3e0rnPgfdTzmOlcXCeVbAiHtJOqfpxZ2wa5PBg0BswvKaMRTs3EqB
ULa6yQdi7wKBgGIREnsUGYWN5waQe0SDksEbZgWgOsUuxXLZhGRbkdZ2o9jl2K1j
z1g62wBA4as2iyIzR5RThYyYTZhPfTGPQ4OzTyNY1WSAxq1ioZe6iInxZjIAZ1pt
q3LMfaLERyQNtCedzczXSpwa/Df+m+IVLSaVpLRss7Fk79hJKIIIW915AoGAPmhR
QVLMCIew8EYXYjj5QPPLdKR+dztCTK/imXRtLVo8o6D5xITcy7E87D+QS0dIh5bC
SzFO0P21gTA+Uo2gO393I/lpX8zc2D5hik/4bzNQYs9dwrXQySSHCB4JLg+cz0Nc
ZqlmD+N4KyfqommdCnv7/2+VE7k+QXjzdcsaOc0CgYEAttOGVcTaLhWnIzRxBkyh
5wYljDRR0GaWSZYp5m4ACTfl2/TyqCfY+JEs6NnYuqzWbkxf/PJpbLrPIHHkWzrg
XLhoZtxJDPlUab39y3G0qYZu5aTFSGNbnJGHC/kczw069Wd/GZ17Gxx0G0kMNT5S
Pfkte+2kAeYPMK9Sa+apqqE=
-----END PRIVATE KEY-----"#;

    const TEST_JWKS_JSON: &str = r#"{
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": "test-key-1",
            "n": "oBm_NaeosKETSn9HUrG5v8faWUYU9Nrur7EXLsx4GdQLkoRjaRHnV9K9MJFG_YSwAvkaW5Cg040JAKaAUWESCAtmNK3wz35Ow9SV_Kf10aleNF0CAKuNoawGXaWQIz5b29iLMnJu6rU7k9lNOKdGbgYjpvadQi-ss9YwvTVvJGZczdXeqBoxcMzGywfWFscj_u1Uv5rqRdCljvUQsKMp1-38Emx7NHODdXhYInrzVlORcfOjQFEzTmjtlflXFGVllr5F2KColkQxNvJ7vlCMEvtIq-Y3U1EFizXek0TnC_gnbHc5t8smAUaL7YG9Fo0_CdeP9NWI3iYxcTpRnsDPJQ",
            "e": "AQAB"
        }]
    }"#;

    /// Create a JWT with the given claims, signed with the test RSA private key.
    fn make_jwt(claims: &serde_json::Value) -> String {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
        let key = EncodingKey::from_rsa_pem(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".to_string());
        encode(&header, claims, &key).unwrap()
    }

    /// Create valid JWT claims for the given email.
    fn jwt_claims_for(email: &str) -> serde_json::Value {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        serde_json::json!({
            "sub": format!("user-{}", email.split('@').next().unwrap_or("unknown")),
            "email": email,
            "groups": [],
            "iss": "https://test-issuer.example.com",
            "aud": "forge-api",
            "exp": now + 3600,
            "iat": now
        })
    }

    /// Build an app state with a running WriterActor (needed for write requests).
    /// Returns (AppState, JoinHandle) — drop handle to stop the actor.
    fn test_app_state_with_writer() -> (AppState, tokio::task::JoinHandle<()>) {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().to_string();
        // Create DB with full schema
        let _init_state = DaemonState::new(&db_path).unwrap();
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let hlc = Arc::new(crate::sync::Hlc::new("test"));
        let started_at = Instant::now();
        let (write_tx, write_rx) = mpsc::channel(16);

        // Create writer actor with its own connection
        let writer_state = DaemonState::new_writer(
            &db_path,
            events.clone(),
            Arc::clone(&hlc),
            started_at,
        )
        .unwrap();
        let actor = crate::server::writer::WriterActor { state: writer_state };
        let handle = tokio::spawn(async move { actor.run(write_rx).await });

        // Keep temp file alive
        std::mem::forget(tmp);

        let state = AppState {
            db_path,
            events,
            hlc,
            started_at,
            write_tx,
            admin_emails: Vec::new(),
            auth_enabled: false,
        };
        (state, handle)
    }

    /// Build a router with auth enabled and the test JWKS loaded from a temp file.
    /// `admin_emails`: list of email addresses that get Admin role.
    fn test_authed_router(mut state: AppState, admin_emails: Vec<String>) -> Router {
        // Write JWKS to a temp file for offline fallback
        let mut jwks_file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut jwks_file, TEST_JWKS_JSON.as_bytes()).unwrap();
        std::io::Write::flush(&mut jwks_file).unwrap();
        let jwks_path = jwks_file.path().to_string_lossy().to_string();
        // Keep the file alive
        std::mem::forget(jwks_file);

        state.auth_enabled = true;
        state.admin_emails = admin_emails;

        let mut config = ForgeConfig::default();
        config.auth.enabled = true;
        config.auth.issuer_url = String::new(); // Skip OIDC discovery
        config.auth.audience = "forge-api".to_string();
        config.auth.offline_jwks_path = Some(jwks_path);

        build_router(&config, state)
    }

    // ── AC1: Admin token can write ──

    #[tokio::test]
    async fn test_admin_can_write_via_http() {
        let (state, _writer_handle) = test_app_state_with_writer();
        let app = test_authed_router(state, vec!["admin@example.com".to_string()]);

        let token = make_jwt(&jwt_claims_for("admin@example.com"));
        let body = serde_json::json!({
            "method": "remember",
            "params": {
                "memory_type": "decision",
                "title": "admin write test",
                "content": "admin content"
            }
        });

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "Admin should be able to write via HTTP"
        );
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(json["status"], "ok");
    }

    // ── AC1 (continued): Admin can read ──

    #[tokio::test]
    async fn test_admin_can_read_via_http() {
        let state = test_app_state();
        let app = test_authed_router(state, vec!["admin@example.com".to_string()]);

        let token = make_jwt(&jwt_claims_for("admin@example.com"));
        let body = serde_json::json!({"method": "health"});

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "Admin should be able to read via HTTP"
        );
    }

    // ── AC2: Non-admin token blocked from admin operations via HTTP ──

    #[tokio::test]
    async fn test_non_admin_blocked_from_admin_ops_via_http() {
        let (state, _writer_handle) = test_app_state_with_writer();
        // The "viewer" email is not in admin list → resolves to Member role.
        // Member + admin-only op → 403.
        let app = test_authed_router(state, vec!["boss@example.com".to_string()]);

        let token = make_jwt(&jwt_claims_for("viewer@example.com"));
        let body = serde_json::json!({"method": "shutdown"});

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::FORBIDDEN,
            "Non-admin user should be blocked from admin operations via HTTP"
        );
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let error_msg = json["error"]
            .as_str()
            .unwrap_or_else(|| panic!("expected 'error' field in JSON, got: {json}"));
        assert!(
            error_msg.contains("insufficient permissions"),
            "expected 'insufficient permissions' in error, got: {error_msg}"
        );
    }

    // ── AC3: Member token blocked from Shutdown ──

    #[tokio::test]
    async fn test_member_blocked_from_shutdown_via_http() {
        let (state, _writer_handle) = test_app_state_with_writer();
        let app = test_authed_router(state, vec!["other@example.com".to_string()]);

        let token = make_jwt(&jwt_claims_for("member@example.com"));
        let body = serde_json::json!({"method": "shutdown"});

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::FORBIDDEN,
            "Member should be blocked from Shutdown via HTTP"
        );
    }

    // ── AC3 (additional): Member CAN write regular ops ──

    #[tokio::test]
    async fn test_member_can_write_regular_ops_via_http() {
        let (state, _writer_handle) = test_app_state_with_writer();
        let app = test_authed_router(state, vec!["admin-only@example.com".to_string()]);

        let token = make_jwt(&jwt_claims_for("member@example.com"));
        let body = serde_json::json!({
            "method": "remember",
            "params": {
                "memory_type": "decision",
                "title": "member write",
                "content": "member content"
            }
        });

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::OK,
            "Member should be able to write regular operations via HTTP"
        );
    }

    // ── AC4: Health probes without auth (verify startupz too) ──

    #[tokio::test]
    async fn test_startupz_exempt_from_auth() {
        let state = test_app_state();
        let app = test_router_with_auth(state);

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/startupz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // startupz may return 503 (no indexed memories in test DB) or 200,
        // but it must NOT return 401 — health probes are exempt from auth.
        assert_ne!(
            response.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "startupz should be exempt from auth (must not return 401)"
        );
        // Verify it returns a valid JSON body (not an auth error)
        let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert!(
            json["status"].as_str().is_some(),
            "startupz should return a status field, not an auth error"
        );
    }

    // ── AC5: Member blocked from all admin-only operations via HTTP ──

    #[tokio::test]
    async fn test_member_blocked_from_set_config_via_http() {
        let (state, _writer_handle) = test_app_state_with_writer();
        let app = test_authed_router(state, vec!["admin@example.com".to_string()]);

        let token = make_jwt(&jwt_claims_for("member@example.com"));
        let body = serde_json::json!({
            "method": "set_config",
            "params": { "key": "k", "value": "v" }
        });

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::FORBIDDEN,
            "Member blocked from SetConfig via HTTP"
        );
    }

    #[tokio::test]
    async fn test_member_blocked_from_cleanup_sessions_via_http() {
        let (state, _writer_handle) = test_app_state_with_writer();
        let app = test_authed_router(state, vec!["admin@example.com".to_string()]);

        let token = make_jwt(&jwt_claims_for("member@example.com"));
        let body = serde_json::json!({"method": "cleanup_sessions", "params": {"prefix": null}});

        let response = app
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::FORBIDDEN,
            "Member blocked from CleanupSessions via HTTP"
        );
    }

    // ── AC7: Socket writes still work (regression) ──

    #[tokio::test]
    async fn test_socket_write_still_works_regression() {
        // Verify that the writer actor still processes Raw (non-audited) commands,
        // which is the path used by Unix socket connections.
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let db_path = tmp.path().to_string_lossy().to_string();
        let _init_state = DaemonState::new(&db_path).unwrap();
        let (events, _rx) = tokio::sync::broadcast::channel(16);
        let hlc = Arc::new(crate::sync::Hlc::new("test"));
        let started_at = Instant::now();

        let writer_state = DaemonState::new_writer(
            &db_path,
            events.clone(),
            Arc::clone(&hlc),
            started_at,
        )
        .unwrap();
        let actor = crate::server::writer::WriterActor { state: writer_state };
        let (tx, rx) = mpsc::channel(16);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        // Send a Raw write command (socket path — no audit context)
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Raw {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "socket regression test".into(),
                content: "must still work".into(),
                confidence: None,
                tags: None,
                project: None,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok for socket-path write, got {:?}", other),
        }

        drop(tx);
        handle.await.unwrap();
    }
}
