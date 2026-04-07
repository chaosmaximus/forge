//! JWT validation and OIDC discovery for HTTP transport.
//!
//! When `config.auth.enabled` is true, all POST /api requests require a valid
//! `Authorization: Bearer <JWT>` header. Health probes are always exempt.
//!
//! - JWKS fetched from OIDC discovery endpoint (`{issuer_url}/.well-known/openid-configuration`)
//! - JWKS cached in `RwLock<Option<JwksCache>>` with configurable TTL
//! - Offline fallback: if OIDC discovery fails, load from `config.auth.offline_jwks_path`
//! - Validates: RS256 signature, exp, aud, iss, required_claims
//! - Unix socket NEVER requires auth (this middleware is HTTP-only)

use crate::config::AuthConfig;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Claims extracted from a validated JWT.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthClaims {
    pub sub: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub org: Option<String>,
    // Standard JWT fields used during validation
    #[serde(default)]
    pub iss: Option<String>,
    #[serde(default)]
    pub aud: Option<serde_json::Value>,
    #[serde(default)]
    pub exp: Option<u64>,
}

/// Cached JWKS with TTL-based refresh.
pub struct JwksCache {
    pub keys: JwkSet,
    pub fetched_at: Instant,
    pub ttl: Duration,
}

impl JwksCache {
    /// Returns true if the cache entry has expired.
    pub fn is_expired(&self) -> bool {
        self.fetched_at.elapsed() > self.ttl
    }

    pub fn is_fresh(&self) -> bool {
        !self.is_expired()
    }
}

/// Thread-safe shared JWKS cache.
pub type SharedJwksCache = Arc<RwLock<Option<JwksCache>>>;

/// Create a new empty shared JWKS cache.
pub fn new_jwks_cache() -> SharedJwksCache {
    Arc::new(RwLock::new(None))
}

/// Fetch JWKS from OIDC discovery endpoint.
/// 1. GET {issuer_url}/.well-known/openid-configuration -> parse -> extract jwks_uri
/// 2. Validate jwks_uri (HTTPS required, no redirects to prevent SSRF)
/// 3. GET jwks_uri -> parse as JwkSet
async fn fetch_jwks_from_oidc(issuer_url: &str) -> Result<JwkSet, String> {
    // Security: require HTTPS for issuer URL (except localhost for dev)
    if !issuer_url.starts_with("https://") && !issuer_url.starts_with("http://localhost") && !issuer_url.starts_with("http://127.0.0.1") {
        return Err(format!("issuer_url must use HTTPS (got: {})", issuer_url));
    }

    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        issuer_url.trim_end_matches('/')
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none()) // No redirects — prevent SSRF
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let discovery: serde_json::Value = client
        .get(&discovery_url)
        .send()
        .await
        .map_err(|e| format!("OIDC discovery request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("OIDC discovery response invalid: {e}"))?;

    // Security: verify discovery issuer matches configured issuer
    if let Some(disc_issuer) = discovery["issuer"].as_str() {
        let normalized_config = issuer_url.trim_end_matches('/');
        let normalized_disc = disc_issuer.trim_end_matches('/');
        if normalized_config != normalized_disc {
            return Err(format!(
                "OIDC discovery issuer mismatch: configured={}, discovery={}",
                issuer_url, disc_issuer
            ));
        }
    }

    let jwks_uri = discovery["jwks_uri"]
        .as_str()
        .ok_or_else(|| "OIDC discovery response missing jwks_uri".to_string())?;

    // Security: jwks_uri must also use HTTPS (except localhost for dev)
    if !jwks_uri.starts_with("https://") && !jwks_uri.starts_with("http://localhost") && !jwks_uri.starts_with("http://127.0.0.1") {
        return Err(format!("jwks_uri must use HTTPS (got: {})", jwks_uri));
    }

    let jwks: JwkSet = client
        .get(jwks_uri)
        .send()
        .await
        .map_err(|e| format!("JWKS fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("JWKS response invalid: {e}"))?;

    Ok(jwks)
}

/// Load JWKS from an offline file (air-gapped fallback).
fn load_offline_jwks(path: &str) -> Result<JwkSet, String> {
    let contents =
        std::fs::read_to_string(path).map_err(|e| format!("failed to read offline JWKS: {e}"))?;
    serde_json::from_str(&contents).map_err(|e| format!("failed to parse offline JWKS: {e}"))
}

/// Fetch JWKS: try OIDC discovery first, fall back to offline file.
pub async fn fetch_jwks(
    issuer_url: &str,
    offline_path: Option<&str>,
) -> Result<JwkSet, String> {
    // Try OIDC discovery first (only if issuer_url is non-empty)
    if !issuer_url.is_empty() {
        match fetch_jwks_from_oidc(issuer_url).await {
            Ok(jwks) => return Ok(jwks),
            Err(e) => {
                tracing::warn!(error = %e, "OIDC discovery failed, trying offline fallback");
            }
        }
    }

    // Fallback to offline JWKS file
    if let Some(path) = offline_path {
        return load_offline_jwks(path);
    }

    Err("no JWKS source available: OIDC discovery failed and no offline_jwks_path configured"
        .to_string())
}

/// Refresh the JWKS cache if expired or empty.
/// Performs network I/O OUTSIDE the write lock to prevent head-of-line blocking.
/// On fetch failure, serves stale keys for up to 2x TTL (stale-on-error).
async fn ensure_jwks_cache(
    cache: &SharedJwksCache,
    config: &AuthConfig,
) -> Result<(), String> {
    // Fast path: read lock to check if cache is valid
    let needs_refresh = {
        let read_guard = cache.read().await;
        !read_guard.as_ref().is_some_and(|entry| entry.is_fresh())
    };

    if !needs_refresh {
        return Ok(());
    }

    // Fetch OUTSIDE any lock to prevent head-of-line blocking
    let fetch_result = fetch_jwks(
        &config.issuer_url,
        config.offline_jwks_path.as_deref(),
    )
    .await;

    match fetch_result {
        Ok(jwks) => {
            let mut write_guard = cache.write().await;
            *write_guard = Some(JwksCache {
                keys: jwks,
                fetched_at: Instant::now(),
                ttl: Duration::from_secs(config.jwks_cache_secs),
            });
            Ok(())
        }
        Err(e) => {
            // Stale-on-error: if we have expired-but-recent keys, keep using them
            // for up to 2x TTL before hard-failing
            let read_guard = cache.read().await;
            if let Some(ref entry) = *read_guard {
                let stale_limit = entry.ttl * 2;
                if entry.fetched_at.elapsed() < stale_limit {
                    tracing::warn!(
                        error = %e,
                        stale_secs = entry.fetched_at.elapsed().as_secs(),
                        "JWKS refresh failed, serving stale keys"
                    );
                    return Ok(());
                }
            }
            Err(e)
        }
    }
}

/// Validate a JWT token against the cached JWKS.
///
/// Checks: RS256 signature, expiry, audience, issuer, and required claims.
pub async fn validate_token(
    token: &str,
    jwks_cache: &SharedJwksCache,
    config: &AuthConfig,
) -> Result<AuthClaims, String> {
    // Ensure JWKS cache is fresh
    ensure_jwks_cache(jwks_cache, config).await?;

    // Decode JWT header to get kid
    let header =
        decode_header(token).map_err(|e| format!("invalid JWT header: {e}"))?;

    // Read the cached JWKS
    let cache_guard = jwks_cache.read().await;
    let cache_entry = cache_guard
        .as_ref()
        .ok_or_else(|| "JWKS cache is empty".to_string())?;

    // Find the matching key by kid
    let decoding_key = if let Some(ref kid) = header.kid {
        let jwk = cache_entry
            .keys
            .find(kid)
            .ok_or_else(|| "invalid token".to_string())?;
        DecodingKey::from_jwk(jwk)
            .map_err(|_| "invalid token".to_string())?
    } else if cache_entry.keys.keys.len() == 1 {
        // No kid in header — only accept if JWKS has exactly one key
        let jwk = cache_entry
            .keys
            .keys
            .first()
            .ok_or_else(|| "invalid token".to_string())?;
        DecodingKey::from_jwk(jwk)
            .map_err(|_| "invalid token".to_string())?
    } else {
        // Multiple keys but no kid — reject as ambiguous
        return Err("invalid token".to_string());
    };

    // Build validation parameters
    let mut validation = Validation::new(Algorithm::RS256);

    // Security: explicit leeway (5 seconds, not default 60)
    validation.leeway = 5;

    // Security: validate not-before claim if present
    validation.validate_nbf = true;

    // Set audience validation
    if !config.audience.is_empty() {
        validation.set_audience(&[&config.audience]);
    } else {
        validation.validate_aud = false;
    }

    // Set issuer validation
    if !config.issuer_url.is_empty() {
        validation.set_issuer(&[&config.issuer_url]);
    }

    // Set required claims
    validation.set_required_spec_claims(&["exp", "sub"]);

    // Decode and validate
    let token_data = decode::<AuthClaims>(token, &decoding_key, &validation)
        .map_err(|e| format!("JWT validation failed: {e}"))?;

    let claims = token_data.claims;

    // Check additional required claims from config
    for claim_name in &config.required_claims {
        match claim_name.as_str() {
            "email" => {
                if claims.email.is_none() {
                    return Err(format!("required claim missing: {claim_name}"));
                }
            }
            "org" => {
                if claims.org.is_none() {
                    return Err(format!("required claim missing: {claim_name}"));
                }
            }
            "groups" => {
                if claims.groups.is_empty() {
                    return Err(format!("required claim missing: {claim_name}"));
                }
            }
            // sub, iss, aud, exp are validated by jsonwebtoken itself
            _ => {}
        }
    }

    Ok(claims)
}

/// Axum middleware that validates JWT Bearer tokens.
///
/// Extracts the token from the `Authorization: Bearer <token>` header,
/// validates it against JWKS, and stores `AuthClaims` in request extensions.
pub async fn auth_middleware(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
    jwks_cache: SharedJwksCache,
    auth_config: AuthConfig,
    rate_limiter: Option<crate::server::rate_limit::RateLimiter>,
) -> axum::response::Response {
    // Extract real client IP for auth failure recording
    let client_ip = req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Exempt localhost from auth failure recording — consistent with API rate limiter (ISS-1).
    let is_localhost = client_ip == "127.0.0.1"
        || client_ip == "::1"
        || client_ip.starts_with("127.")
        || client_ip == "localhost";

    // Extract Bearer token from Authorization header
    let token = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let token = match token {
        Some(t) => t.to_string(),
        None => {
            if !is_localhost {
                if let Some(ref limiter) = rate_limiter {
                    limiter.record_auth_failure(&client_ip).await;
                }
            }
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "missing or invalid Authorization header"})),
            )
                .into_response();
        }
    };

    match validate_token(&token, &jwks_cache, &auth_config).await {
        Ok(claims) => {
            // Check if token is expiring soon (within 5 minutes)
            let expiring_soon = claims.exp.is_some_and(|exp| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                exp > now && (exp - now) < 300
            });
            req.extensions_mut().insert(claims);
            let mut response = next.run(req).await;
            if expiring_soon {
                response.headers_mut().insert(
                    "X-Token-Expiring-Soon",
                    axum::http::HeaderValue::from_static("true"),
                );
            }
            response
        }
        Err(e) => {
            // Record auth failure for rate-limit lockout (skip localhost — ISS-1)
            if !is_localhost {
                if let Some(ref limiter) = rate_limiter {
                    limiter.record_auth_failure(&client_ip).await;
                }
            }
            // Log detailed error server-side, return generic message to client
            tracing::warn!(error = %e, "JWT validation failed");
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "invalid token"})),
            )
                .into_response()
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::io::Write;
    use tower::ServiceExt;

    // Test RSA key pair (2048-bit, generated offline for deterministic tests).
    // THESE KEYS ARE FOR TESTING ONLY — never use in production.
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

    #[allow(dead_code)] // Kept as reference — tests use JWKS format instead
    const TEST_RSA_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAoBm/NaeosKETSn9HUrG5
v8faWUYU9Nrur7EXLsx4GdQLkoRjaRHnV9K9MJFG/YSwAvkaW5Cg040JAKaAUWES
CAtmNK3wz35Ow9SV/Kf10aleNF0CAKuNoawGXaWQIz5b29iLMnJu6rU7k9lNOKdG
bgYjpvadQi+ss9YwvTVvJGZczdXeqBoxcMzGywfWFscj/u1Uv5rqRdCljvUQsKMp
1+38Emx7NHODdXhYInrzVlORcfOjQFEzTmjtlflXFGVllr5F2KColkQxNvJ7vlCM
EvtIq+Y3U1EFizXek0TnC/gnbHc5t8smAUaL7YG9Fo0/CdeP9NWI3iYxcTpRnsDP
JQIDAQAB
-----END PUBLIC KEY-----"#;

    // JWKS JSON matching the test public key above.
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

    fn test_encoding_key() -> EncodingKey {
        EncodingKey::from_rsa_pem(TEST_RSA_PRIVATE_KEY.as_bytes()).unwrap()
    }

    fn test_auth_config() -> AuthConfig {
        AuthConfig {
            enabled: true,
            issuer_url: "https://test-issuer.example.com".to_string(),
            audience: "forge-api".to_string(),
            required_claims: vec![],
            admin_emails: vec![],
            viewer_emails: vec![],
            jwks_cache_secs: 3600,
            offline_jwks_path: None,
        }
    }

    /// Pre-populate the JWKS cache with our test key set (bypasses OIDC discovery).
    async fn prepopulate_cache(cache: &SharedJwksCache) {
        let jwks: JwkSet = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        let entry = JwksCache {
            keys: jwks,
            fetched_at: Instant::now(),
            ttl: Duration::from_secs(3600),
        };
        let mut guard = cache.write().await;
        *guard = Some(entry);
    }

    fn make_test_jwt(claims: &serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".to_string());
        encode(&header, claims, &test_encoding_key()).unwrap()
    }

    fn valid_claims() -> serde_json::Value {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        serde_json::json!({
            "sub": "user-123",
            "email": "alice@example.com",
            "groups": ["admin", "dev"],
            "org": "acme-corp",
            "iss": "https://test-issuer.example.com",
            "aud": "forge-api",
            "exp": now + 3600,
            "iat": now
        })
    }

    // ── validate_token tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_valid_token_accepted() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();
        let token = make_test_jwt(&valid_claims());

        let result = validate_token(&token, &cache, &config).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        let claims = result.unwrap();
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.email, Some("alice@example.com".to_string()));
        assert_eq!(claims.groups, vec!["admin", "dev"]);
        assert_eq!(claims.org, Some("acme-corp".to_string()));
    }

    #[tokio::test]
    async fn test_expired_token_rejected() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let mut claims = valid_claims();
        claims["exp"] = serde_json::json!(1_000_000); // long expired
        let token = make_test_jwt(&claims);

        let result = validate_token(&token, &cache, &config).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("ExpiredSignature"),
            "expected expiry error"
        );
    }

    #[tokio::test]
    async fn test_wrong_audience_rejected() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let mut claims = valid_claims();
        claims["aud"] = serde_json::json!("wrong-audience");
        let token = make_test_jwt(&claims);

        let result = validate_token(&token, &cache, &config).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("InvalidAudience"),
            "expected audience error"
        );
    }

    #[tokio::test]
    async fn test_wrong_issuer_rejected() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let mut claims = valid_claims();
        claims["iss"] = serde_json::json!("https://evil-issuer.example.com");
        let token = make_test_jwt(&claims);

        let result = validate_token(&token, &cache, &config).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("InvalidIssuer"),
            "expected issuer error"
        );
    }

    #[tokio::test]
    async fn test_missing_sub_rejected() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = serde_json::json!({
            "email": "alice@example.com",
            "iss": "https://test-issuer.example.com",
            "aud": "forge-api",
            "exp": now + 3600
        });
        let token = make_test_jwt(&claims);

        let result = validate_token(&token, &cache, &config).await;
        assert!(result.is_err(), "missing sub should fail");
    }

    #[tokio::test]
    async fn test_required_claims_enforced() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let mut config = test_auth_config();
        config.required_claims = vec!["email".to_string(), "org".to_string()];

        // Token with email but without org
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = serde_json::json!({
            "sub": "user-123",
            "email": "alice@example.com",
            "iss": "https://test-issuer.example.com",
            "aud": "forge-api",
            "exp": now + 3600
        });
        let token = make_test_jwt(&claims);

        let result = validate_token(&token, &cache, &config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("org"));
    }

    #[tokio::test]
    async fn test_garbage_token_rejected() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let result = validate_token("not.a.jwt", &cache, &config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_audience_skips_aud_validation() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let mut config = test_auth_config();
        config.audience = String::new(); // no audience check

        let mut claims = valid_claims();
        claims["aud"] = serde_json::json!("any-audience-at-all");
        let token = make_test_jwt(&claims);

        let result = validate_token(&token, &cache, &config).await;
        assert!(result.is_ok(), "empty audience config should skip aud check");
    }

    // ── JWKS cache tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn test_jwks_cache_expiry() {
        let cache = new_jwks_cache();
        // Set cache with 0-second TTL (already expired)
        let jwks: JwkSet = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        {
            let mut guard = cache.write().await;
            *guard = Some(JwksCache {
                keys: jwks,
                fetched_at: Instant::now() - Duration::from_secs(10),
                ttl: Duration::from_secs(1), // expired
            });
        }

        let entry = cache.read().await;
        assert!(entry.as_ref().unwrap().is_expired());
    }

    #[tokio::test]
    async fn test_jwks_cache_fresh() {
        let cache = new_jwks_cache();
        let jwks: JwkSet = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        {
            let mut guard = cache.write().await;
            *guard = Some(JwksCache {
                keys: jwks,
                fetched_at: Instant::now(),
                ttl: Duration::from_secs(3600),
            });
        }

        let entry = cache.read().await;
        assert!(!entry.as_ref().unwrap().is_expired());
    }

    // ── Offline JWKS fallback test ───────────────────────────────────────

    #[tokio::test]
    async fn test_offline_jwks_fallback() {
        // Write test JWKS to a temp file
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(TEST_JWKS_JSON.as_bytes()).unwrap();
        tmp.flush().unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        // fetch_jwks with empty issuer should fall back to offline file
        let result = fetch_jwks("", Some(&path)).await;
        assert!(result.is_ok(), "offline JWKS load failed: {result:?}");
        assert_eq!(result.unwrap().keys.len(), 1);
    }

    #[tokio::test]
    async fn test_offline_jwks_missing_file() {
        let result = fetch_jwks("", Some("/nonexistent/path/jwks.json")).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to read offline JWKS"));
    }

    #[tokio::test]
    async fn test_no_jwks_source_returns_error() {
        let result = fetch_jwks("", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no JWKS source available"));
    }

    // ── End-to-end with offline JWKS ─────────────────────────────────────

    #[tokio::test]
    async fn test_validate_token_with_offline_jwks() {
        // Write test JWKS to temp file
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        tmp.write_all(TEST_JWKS_JSON.as_bytes()).unwrap();
        tmp.flush().unwrap();
        let path = tmp.path().to_string_lossy().to_string();

        let cache = new_jwks_cache();
        let mut config = test_auth_config();
        config.issuer_url = String::new(); // no OIDC discovery
        config.offline_jwks_path = Some(path);
        config.audience = String::new(); // skip aud check since issuer is empty

        let mut claims = valid_claims();
        claims.as_object_mut().unwrap().remove("iss"); // no issuer check
        claims.as_object_mut().unwrap().remove("aud");
        let token = make_test_jwt(&claims);

        let mut config2 = config.clone();
        config2.issuer_url = String::new();

        let result = validate_token(&token, &cache, &config2).await;
        assert!(
            result.is_ok(),
            "validate_token with offline JWKS failed: {result:?}"
        );
    }

    // ── Middleware integration tests ─────────────────────────────────────

    /// Helper: build a minimal axum app with auth middleware for testing.
    fn build_authed_test_app(cache: SharedJwksCache, config: AuthConfig) -> axum::Router {
        axum::Router::new()
            .route(
                "/api",
                axum::routing::post(|| async { "ok" }),
            )
            .layer(axum::middleware::from_fn(move |req, next| {
                let c = cache.clone();
                let cfg = config.clone();
                auth_middleware(req, next, c, cfg, None)
            }))
    }

    #[tokio::test]
    async fn test_middleware_missing_auth_header() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let app = build_authed_test_app(cache, config);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(json["error"]
            .as_str()
            .unwrap()
            .contains("missing or invalid Authorization header"));
    }

    #[tokio::test]
    async fn test_middleware_invalid_token() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let app = build_authed_test_app(cache, config);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer garbage.token.here")
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_middleware_valid_token_passes_through() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();
        let token = make_test_jwt(&valid_claims());

        let app = build_authed_test_app(cache, config);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_middleware_expired_token() {
        let cache = new_jwks_cache();
        prepopulate_cache(&cache).await;
        let config = test_auth_config();

        let mut claims = valid_claims();
        claims["exp"] = serde_json::json!(1_000_000);
        let token = make_test_jwt(&claims);

        let app = build_authed_test_app(cache, config);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {token}"))
                    .body(axum::body::Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    // ── JwkSet parsing test ──────────────────────────────────────────────

    #[test]
    fn test_jwks_json_parses_correctly() {
        let jwks: JwkSet = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        assert_eq!(jwks.keys.len(), 1);
        assert_eq!(
            jwks.keys[0].common.key_id,
            Some("test-key-1".to_string())
        );
    }

    #[test]
    fn test_decoding_key_from_jwks() {
        let jwks: JwkSet = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        let jwk = jwks.find("test-key-1").unwrap();
        let key = DecodingKey::from_jwk(jwk);
        assert!(key.is_ok(), "DecodingKey::from_jwk failed: {}", key.err().map(|e| e.to_string()).unwrap_or_default());
    }

    // ── Round-trip: encode with PEM, decode with JWKS ────────────────────

    #[test]
    fn test_roundtrip_pem_encode_jwks_decode() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let claims = serde_json::json!({
            "sub": "roundtrip-user",
            "email": "rt@example.com",
            "groups": [],
            "iss": "https://test-issuer.example.com",
            "aud": "forge-api",
            "exp": now + 3600
        });

        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key-1".to_string());
        let token = encode(&header, &claims, &test_encoding_key()).unwrap();

        // Decode using JWKS-derived key
        let jwks: JwkSet = serde_json::from_str(TEST_JWKS_JSON).unwrap();
        let jwk = jwks.find("test-key-1").unwrap();
        let dk = DecodingKey::from_jwk(jwk).unwrap();

        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&["forge-api"]);
        validation.set_issuer(&["https://test-issuer.example.com"]);

        let decoded = decode::<AuthClaims>(&token, &dk, &validation);
        assert!(decoded.is_ok(), "roundtrip decode failed: {decoded:?}");
        assert_eq!(decoded.unwrap().claims.sub, "roundtrip-user");
    }
}
