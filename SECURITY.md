# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.4.x (current) | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in Forge, please report it responsibly:

1. **Do NOT** open a public GitHub issue
2. Use GitHub's private vulnerability reporting at <https://github.com/chaosmaximus/forge/security/advisories/new>
3. Include: description, steps to reproduce, potential impact
4. We will respond within 48 hours
5. Fix timeline: Critical 24h, High 7 days, Medium 30 days

## Security Architecture

### Local-First Design
All data stays on the user's machine. No cloud services, no telemetry, no data collection. The daemon stores everything in a single SQLite file with WAL mode.

### Daemon (forge-daemon)
- **Socket permissions:** Unix socket at `~/.forge/forge.sock` with `0600` permissions (owner-only)
- **Parameterized queries:** All SQLite operations use parameterized statements (no SQL injection)
- **Property validation:** Memory property keys validated against `^[A-Za-z_][A-Za-z0-9_]{0,63}$`
- **Secret scanning:** SHA256 fingerprints only — never stores actual secret values
- **Symlink defense:** `symlink_metadata()` verification before file operations
- **UTF-8 safe truncation:** Content truncated at valid UTF-8 boundaries
- **50MB file limit:** Prevents memory exhaustion from large inputs
- **PID locking:** Exclusive flock with /proc liveness check for stale lock recovery
- **Multi-tenant isolation:** All memory queries, recall results, and context compilation are scoped by `organization_id`. Memory sync exports filter by org_id, preventing cross-tenant data leakage. Cross-tier sync policies control which memory types propagate between local, team, and organization levels.

### HTTP Transport Security
- **JWT/OIDC authentication:** RS256 signature validation, JWKS with TTL cache, OIDC discovery
- **RBAC:** Admin/Member/Viewer roles, fail-closed (unknown operations denied)
- **Rate limiting:** 100 req/min/IP using real TCP peer IP (ConnectInfo, not spoofable headers)
- **Auth failure lockout:** 5 failed JWT validations per IP triggers lockout
- **CORS:** Restricted to localhost origins by default (numeric port validation)
- **gRPC:** Binds 127.0.0.1 by default (never 0.0.0.0)
- **TLS:** Self-signed certificate generation via rcgen, axum-server tls-rustls integration

### Terminal Security (WebSocket)
- **Authentication:** JWT required via `?token=` query parameter when auth enabled
- **Fail-closed:** Returns 500 if auth infrastructure misconfigured (no silent pass-through)
- **Session limits:** Max 8 concurrent PTY sessions
- **Idle timeout:** 15-minute idle timeout with 60-second background reaper
- **TOCTOU prevention:** Session limit checked atomically inside lock during PTY creation
- **Audit logging:** All PTY spawns recorded in append-only audit_log table
- **Rate limiting:** Terminal WebSocket endpoint rate-limited via same mechanism as HTTP API

### License Tier Gating
- Feature-level access control: Free/Pro/Team/Enterprise tiers
- 402 responses with upgrade URL for tier-locked features
- Configuration-driven (config.toml), no phone-home license server required

### Audit Trail
- Append-only `audit_log` table with SQLite triggers blocking UPDATE/DELETE
- Records: actor, action, resource, timestamp, source IP, role, response status
- Terminal sessions audited separately with PTY ID and working directory

### Hook Scripts
- `set -euo pipefail` on all scripts
- Session-start hook uses `forge-next compile-context` for proactive context injection
- Pre-edit hook runs guardrails check via daemon socket
- All file paths resolved via `readlink` to prevent symlink traversal

## Security Reviews

- **15 adversarial security reviews** completed (Sessions 6-15)
  - 9 internal evaluator reviews (Sessions 6-10)
  - 6 Codex (GPT-5.2) adversarial rounds (Sessions 6-9)
  - 3 CRITICAL findings fixed: auth bypass path, IP spoofing, dead lockout code
  - 6 IMPORTANT findings fixed: WS rate limit, PTY TOCTOU, CORS port validation, PID truncation
  - All HIGH/MEDIUM findings addressed through Session 15
- **1,245 Rust tests, 0 clippy warnings** (Session 16)

## Data Protection

- **Local-first:** All data stays on the user's machine. No cloud services, no telemetry.
- **No secrets stored:** Memory extraction filters credential-like content. Secret scanner uses SHA256 fingerprints.
- **Memory self-healing:** Stale memories auto-faded, duplicates auto-superseded via cosine similarity.
