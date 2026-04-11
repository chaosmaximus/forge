# Security Overview

Forge is designed for teams that cannot send data to a third party. Every byte stays on infrastructure you control. This document covers authentication, authorization, audit logging, data residency, transport security, and the threat model.

## Authentication

Forge uses JWT RS256 tokens validated against OIDC-discovered JWKS keys. Authentication is disabled by default (local development) and enabled via configuration for networked deployments.

### JWT Validation

| Property | Value |
|----------|-------|
| Algorithm | RS256 only |
| Clock leeway | 5 seconds (not the jsonwebtoken default of 60) |
| Required claims | `exp`, `sub` (always); `email`, `org`, `groups` (configurable) |
| `nbf` validation | Enabled |
| Audience validation | Enabled when `auth.audience` is set |
| Issuer validation | Enabled when `auth.issuer_url` is set |

When the JWKS keyset contains multiple keys, the JWT `kid` header is required. If the keyset contains exactly one key and the JWT has no `kid`, the single key is accepted. Multiple keys with no `kid` is rejected as ambiguous.

### JWKS Caching

JWKS keys are cached in memory behind a `RwLock<Option<JwksCache>>` (Tokio async RwLock) with a configurable TTL (default: 3600 seconds).

- **Fresh:** Cache is within TTL. Requests use cached keys with zero network I/O.
- **Stale-on-error:** If the OIDC/JWKS endpoint is unreachable, cached keys continue to be served for up to 2x TTL. This prevents transient IdP outages from locking out users.
- **Expired:** Beyond 2x TTL with no successful refresh, validation fails hard.
- **Refresh path:** Network fetch happens outside the write lock. The read lock checks freshness, then releases. The fetch runs without holding any lock. Only the final write to update the cache takes the write lock. This prevents head-of-line blocking on JWKS refresh.

### Offline JWKS Fallback

For air-gapped or disconnected environments, configure `auth.offline_jwks_path` to point to a local JWKS JSON file. When OIDC discovery fails (or `issuer_url` is empty), the daemon loads keys from this file.

```toml
[auth]
enabled = true
offline_jwks_path = "/etc/forge/jwks.json"
```

### HTTPS Enforcement

Both the OIDC issuer URL and the discovered `jwks_uri` must use HTTPS. The only exception is `localhost` and `127.0.0.1`, which are allowed over HTTP for local development.

```
https://login.company.com     -- accepted
http://localhost:8080          -- accepted (development)
http://10.0.0.5:8080           -- rejected
```

### Health Probe Exemption

The following endpoints are always accessible without authentication. Kubernetes liveness, readiness, and startup probes must function without JWT tokens.

| Endpoint | Purpose |
|----------|---------|
| `GET /healthz` | Liveness probe |
| `GET /readyz` | Readiness probe |
| `GET /startupz` | Startup probe |
| `GET /metrics` | Prometheus scrape target |

### Configuration Reference

```toml
[auth]
enabled = true
issuer_url = "https://login.company.com"
audience = "forge-api"
required_claims = ["email"]
admin_emails = ["admin@company.com"]
viewer_emails = ["auditor@company.com"]
jwks_cache_secs = 3600
offline_jwks_path = ""  # path to local JWKS file for air-gapped deployments
```

## Authorization (RBAC)

Forge implements role-based access control with three roles. Role assignment is email-based, resolved from JWT claims.

### Roles

| Role | How Assigned | Access Level |
|------|--------------|-------------|
| **Admin** | Email in `auth.admin_emails` | Full access to all operations |
| **Member** | Default for authenticated users not in admin or viewer lists | Read + write, blocked from administrative operations |
| **Viewer** | Email in `auth.viewer_emails` | Read-only |

If a user's email appears in both `admin_emails` and `viewer_emails`, Admin takes precedence.

### Permission Matrix

Admin-only operations (Members and Viewers are denied):

| Operation | Description |
|-----------|-------------|
| `shutdown` | Stop the daemon |
| `set_config` | Change global configuration |
| `set_scoped_config` | Set scoped configuration |
| `delete_scoped_config` | Delete scoped configuration |
| `cleanup_sessions` | Bulk-end active sessions |
| `grant_permission` | Grant A2A inter-session permission |
| `revoke_permission` | Revoke A2A permission |
| `import` | Import data |
| `sync_import` | Import sync data from remote |
| `force_index` | Trigger code indexing |

Read-only operations (Viewers are allowed, all roles are allowed):

All `list_*`, `get_*`, `recall`, `batch_recall`, `health`, `doctor`, `export`, `sync_export`, `compile_context`, `guardrails_check`, `blast_radius`, `sessions`, `lsp_status`, and diagnostic queries.

Write operations (Members and Admins are allowed):

`remember`, `forget`, `register_session`, `end_session`, `session_send`, `session_ack`, `store_identity`, `deactivate_identity`, `create_meeting`, `meeting_synthesize`, `meeting_decide`, and all other mutating operations not in the admin-only list.

### Fail-Closed Design

New `Request` variants added to the codebase default to **denied for Members** unless explicitly classified. The `is_admin_only` function uses an explicit allowlist pattern -- operations must be affirmatively listed as member-safe to be accessible.

### Unix Socket Bypass

Connections over the Unix domain socket bypass RBAC entirely. The rationale: if a process can access the socket file (which has `umask 0177`, owner-only permissions), it already has filesystem-level trust equivalent to root on that machine.

## Audit Trail

Every write operation is logged to an append-only `audit_log` table in the SQLite database.

### Schema

| Column | Type | Description |
|--------|------|-------------|
| `id` | TEXT (ULID) | Unique log entry identifier |
| `timestamp` | TEXT | ISO 8601 timestamp |
| `user_id` | TEXT | JWT `sub` claim (or `local` for socket) |
| `email` | TEXT | JWT `email` claim |
| `role` | TEXT | Resolved role: `admin`, `member`, or `viewer` |
| `request_type` | TEXT | Operation name (e.g., `Remember`, `Forget`) |
| `request_summary` | TEXT | Short description of the request |
| `source` | TEXT | `http` or `socket` |
| `source_ip` | TEXT | Client IP address (empty for socket) |
| `response_status` | TEXT | `ok` or error description |

Additional columns from the base audit schema: `actor_type`, `actor_id`, `action`, `resource_type`, `resource_id`, `scope_path`, `details`.

### Append-Only Enforcement

SQLite triggers block `UPDATE` and `DELETE` operations on the `audit_log` table:

```sql
CREATE TRIGGER audit_log_no_update
BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only: UPDATE not allowed');
END;

CREATE TRIGGER audit_log_no_delete
BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only: DELETE not allowed');
END;
```

These triggers are enforced at the SQLite engine level. Even if application code attempts a modification, the database rejects it.

### Indexes

| Index | Columns |
|-------|---------|
| `idx_audit_log_timestamp` | `timestamp` |
| `idx_audit_log_user` | `user_id` |
| `idx_audit_scope` | `scope_path`, `timestamp` |
| `idx_audit_actor` | `actor_id`, `timestamp` |

### Audit Path

Both HTTP and Unix socket writes flow through the writer actor. When auth is enabled, HTTP requests carry an `AuditContext` (user_id, email, role, source, source_ip) that the writer uses to create the audit record after the operation completes. Socket writes log with `user_id = "local"`.

## Data Residency

Forge is 100% self-hosted. There is no telemetry, no phone-home, no cloud dependency.

| Component | Where data lives |
|-----------|-----------------|
| **Database** | Single SQLite file on local filesystem or Kubernetes PersistentVolumeClaim |
| **Backups** | Litestream replicates to YOUR S3/GCS/Azure storage bucket |
| **Telemetry** | OTLP traces go to YOUR OpenTelemetry collector (opt-in, disabled by default) |
| **Metrics** | Prometheus scrapes your daemon's `/metrics` endpoint |
| **Embeddings** | sqlite-vec stores 768-dim vectors in the same SQLite file |
| **OIDC/JWKS** | Outbound HTTPS to your IdP for key discovery (cached, offline fallback available) |

No data is sent to Anthropic, OpenAI, or any third party unless you explicitly configure an extraction backend that calls an external API. The default extraction backend is Ollama (local).

## Transport Security

Forge supports three transport layers. Choose based on your deployment topology.

### Unix Domain Socket (Default)

- File permissions: `umask 0177` (owner read/write only)
- Protocol: NDJSON over Unix stream socket
- Auth: Filesystem permissions (no JWT required)
- Use case: Local development, same-machine agent integration

### HTTP

- Default port: 8420
- Protocol: JSON over HTTP POST
- Auth: JWT Bearer tokens when `auth.enabled = true`
- TLS: Provided by reverse proxy (nginx, Ingress, cloud LB) or service mesh
- CORS: Configurable origins; warns on wildcard `*` with auth disabled
- Use case: Remote developers connecting via `forge-next --endpoint`, web dashboards

### gRPC (Planned)

- Designed for in-cluster service-to-service communication
- mTLS via cert-manager
- Use case: Sidecar patterns, service mesh integration

### CORS Configuration

```toml
[cors]
allowed_origins = ["https://dashboard.company.com", "https://app.company.com"]
max_age_secs = 3600
```

When `allowed_origins` contains `*` and auth is disabled, the daemon logs a security warning:

> CORS wildcard (*) is active with auth DISABLED -- the API is browser-callable from any origin. Set cors.allowed_origins to specific origins or enable auth for production.

## Threat Model

### SSRF Prevention

- OIDC issuer URLs and JWKS URIs must use HTTPS (except localhost)
- HTTP redirects are disabled on the OIDC/JWKS client (`redirect::Policy::none()`)
- OIDC discovery issuer is cross-checked against the configured issuer to prevent discovery document spoofing

### Secret Scanning

Forge includes a built-in secret scanner that detects credentials in code and configuration files.

- Secrets are never stored in plain text. The scanner stores SHA256 fingerprints only.
- Detection covers: API keys, AWS credentials, private keys, JWTs, connection strings, and more.
- Integrated into `post_edit_check` guardrails and available as a standalone scan (`forge scan .`).

### File Protections

| Protection | Detail |
|------------|--------|
| Size limit | 50 MB maximum file size |
| Symlink defense | Symlinks are not followed outside the workspace boundary |
| Workspace boundary | File operations are restricted to the project directory |

### SQLite Hardening

| Measure | Detail |
|---------|--------|
| Parameterized queries | All SQL uses parameterized bindings (`?1` syntax), never string interpolation |
| Property key validation | Regex `^[A-Za-z_][A-Za-z0-9_]{0,63}$` for user-supplied keys |
| WAL mode | Write-Ahead Logging for concurrent read/write safety |
| Cypher sandbox | `axon_cypher` blocks memory node labels and write keywords in graph queries |

### Request Body Limits

HTTP request bodies are limited to 10 MB (`axum::body::to_bytes` with 10 * 1024 * 1024 limit). This prevents memory exhaustion from oversized payloads.

### Write Timeout

Write operations have a 30-second timeout. If the writer actor does not respond within 30 seconds, the HTTP handler returns 504 Gateway Timeout. This prevents hung write operations from consuming connections indefinitely.

## Adversarial Testing

Forge has undergone 15 rounds of adversarial review using Codex (GPT-5.2) and internal evaluator agents across Sessions 6-15. Each round produced findings that were fixed before the next round.

| Metric | Count |
|--------|-------|
| Adversarial review rounds | 15 (6 Codex + 9 evaluator) |
| Total findings (all fixed) | 80+ |
| Rust unit tests | 1,223 |
| Live UAT tests | 77 |
| K8s cluster tests | 15 |
| Clippy warnings | 0 |

### Test Coverage Areas

- JWT validation edge cases (expired tokens, wrong audience, wrong issuer, missing claims, garbage tokens)
- RBAC permission checks for all role/operation combinations
- Audit log append-only trigger enforcement
- JWKS cache expiry and stale-on-error behavior
- Offline JWKS fallback
- CORS header presence and wildcard warnings
- HTTP status codes for auth failures, RBAC denials, and infrastructure errors
- Secret scanner detection accuracy
- Socket permission enforcement

## Compliance Notes

### SOC 2

- **Access control:** JWT-based authentication with RBAC
- **Audit logging:** Append-only, tamper-evident audit trail
- **Encryption in transit:** TLS via reverse proxy, mTLS for gRPC
- **Data residency:** All data on customer infrastructure

### GDPR

- No data processed outside customer infrastructure
- No telemetry or analytics
- Audit log provides data access records
- SQLite `VACUUM` can be used for data deletion compliance

### HIPAA

- BAA-compatible: no PHI leaves the deployment boundary
- Audit trail meets access logging requirements
- Encryption at rest is the responsibility of the underlying storage layer (e.g., LUKS, EBS encryption, PVC encryption)
