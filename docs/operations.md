# Operations Guide

This guide is for SRE/DevOps teams managing Forge in production. It covers monitoring, alerting, backups, health probes, capacity planning, and troubleshooting.

## Architecture Overview

Forge runs as a single `forge-daemon` binary backed by a SQLite database in WAL mode. It exposes three transport layers:

- **Unix socket** (default, local only) -- NDJSON protocol, auto-started by the CLI
- **HTTP/REST** (opt-in) -- axum-based, for remote access, health probes, and metrics
- **gRPC** (opt-in) -- JSON-over-gRPC for HTTP/2 + mTLS environments

The daemon uses an actor architecture with hot/cold path separation:
- **Socket handler**: per-connection read-only SQLite (never blocks)
- **Writer actor**: serializes all writes via an mpsc channel
- **Workers**: 8 background workers for extraction, embedding, consolidation, indexing, perception, disposition, diagnostics, and file watching

---

## Monitoring

### Prometheus Metrics

Forge exposes 7 metric families at `GET /metrics` in standard Prometheus text format. Metrics are enabled by default (`FORGE_METRICS_ENABLED=true`).

| Metric | Type | Labels | Description | Normal Range |
|--------|------|--------|-------------|--------------|
| `forge_memories_total` | Gauge | -- | Total number of memories in the database | Grows over time. Alert if unchanged for > 1h during active use. |
| `forge_recall_latency_seconds` | Histogram | -- | Recall query latency. Buckets: 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s. | p50 < 10ms, p99 < 100ms |
| `forge_extraction_duration_seconds` | Histogram | -- | Auto-extraction duration. Buckets: 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s, 30s, 60s. | p50 < 5s, p99 < 30s |
| `forge_worker_healthy` | Gauge | `worker` | Whether a background worker is healthy (1=yes, 0=no). Workers: `watcher`, `extractor`, `embedder`, `consolidator`, `indexer`, `perception`, `disposition`, `diagnostics`. | All = 1. Alert if any = 0 for > 5 minutes. |
| `forge_active_sessions` | Gauge | -- | Number of active agent sessions | Varies by usage. 0 during off-hours is normal. |
| `forge_edges_total` | Gauge | -- | Total number of knowledge graph edges | Grows with memories. Typically 2-5x memory count. |
| `forge_embeddings_total` | Gauge | -- | Total number of stored embeddings (768-dim vectors) | Should match or approach memory count. |

All gauge values are refreshed from the database on each Prometheus scrape, so they are always current.

### Grafana Dashboard

Import the pre-built dashboard:

```
deploy/grafana/forge-dashboard.json
```

The dashboard contains 15 panels organized in rows:

| Row | Panels |
|-----|--------|
| **Overview** | Total Memories, Active Sessions, Edges Total, Embeddings Total, Worker Health |
| **Memory & Recall** | Memory Growth (time series), Recall Latency (heatmap) |
| **Extraction & Workers** | Extraction Duration (histogram), Worker Health Detail (table) |
| **Rates** | Operation Rate (rate of recalls/extractions), Active Sessions Over Time |

To import:
1. Open Grafana at `http://localhost:3000`
2. Navigate to Dashboards > Import
3. Upload `deploy/grafana/forge-dashboard.json`
4. Select your Prometheus data source

### Alerting Rules

Import the alert rules into Prometheus or Grafana:

```
deploy/grafana/forge-alerts.yml
```

Six alert rules are defined:

| Alert | Expression | For | Severity | Description |
|-------|-----------|-----|----------|-------------|
| `ForgeWorkerDown` | `forge_worker_healthy == 0` | 5m | critical | A specific worker has been unhealthy for 5 minutes. |
| `ForgeExtractionSlow` | `histogram_quantile(0.95, rate(forge_extraction_duration_seconds_bucket[5m])) > 60` | 5m | warning | Extraction p95 exceeds 60 seconds. |
| `ForgeMemoryStale` | `delta(forge_memories_total[1h]) == 0` | 1h | warning | Memory count unchanged for 1 hour during active use. |
| `ForgeHighRecallLatency` | `histogram_quantile(0.99, rate(forge_recall_latency_seconds_bucket[5m])) > 5` | 10m | warning | Recall p99 exceeds 5 seconds. |
| `ForgeNoActiveSessions` | `forge_active_sessions == 0` | 30m | info | No active sessions for 30 minutes. May be expected off-hours. |
| `ForgeAllWorkersDown` | `count(forge_worker_healthy == 1) == 0` | 1m | critical | All workers are down. Daemon may be non-functional. |

### OTLP Tracing

Forge supports distributed tracing via OpenTelemetry Protocol (OTLP). When enabled, spans are exported via gRPC to any OTLP-compatible collector.

**Enable tracing:**

```bash
export FORGE_OTLP_ENABLED=true
export FORGE_OTLP_ENDPOINT=http://jaeger:4317
export FORGE_OTLP_SERVICE_NAME=forge-daemon  # optional, default: forge-daemon
```

Or via `~/.forge/config.toml`:

```toml
[otlp]
enabled = true
endpoint = "http://jaeger:4317"
service_name = "forge-daemon"
```

**Compatible collectors:**
- Jaeger (port 4317 for OTLP/gRPC)
- Datadog Agent (OTLP ingest)
- Honeycomb
- Grafana Tempo
- LangSmith (for LLM-specific tracing)
- Any OpenTelemetry Collector

Forge uses W3C `traceparent` propagation for distributed trace context. The CLI client does not currently propagate trace context, but HTTP/gRPC clients may include `traceparent` headers.

---

## Health Probes

Forge exposes three Kubernetes-style health probe endpoints on the HTTP transport.

### GET /healthz -- Liveness

Always returns `200 OK`. If this fails, the process is dead and should be restarted.

```json
{"status": "ok"}
```

### GET /readyz -- Readiness

Verifies both the database connection (read path) and the writer actor channel (write path). Returns `200` when healthy, `503` when degraded.

```json
// Healthy
{"status": "ok", "workers": 8}

// Database unavailable
{"status": "error", "message": "database not responding"}

// Writer unavailable
{"status": "error", "message": "writer unavailable"}
```

If `/readyz` returns `503`, stop routing traffic to this instance. The writer actor may have panicked.

### GET /startupz -- Startup

Returns `503` until initial memory indexing is complete (at least one memory exists), then `200`. Use this to delay traffic until the daemon has finished bootstrapping.

```json
// Starting up
{"status": "starting", "indexed": false}

// Ready
{"status": "ok", "indexed": true}
```

### Kubernetes Probe Configuration

The Helm chart (`deploy/helm/forge/templates/statefulset.yaml`) configures probes as follows:

```yaml
livenessProbe:
  httpGet:
    path: /healthz
    port: http
  initialDelaySeconds: 10
  periodSeconds: 10
  timeoutSeconds: 3
  failureThreshold: 3

readinessProbe:
  httpGet:
    path: /readyz
    port: http
  initialDelaySeconds: 5
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 3

startupProbe:
  httpGet:
    path: /healthz
    port: http
  periodSeconds: 5
  timeoutSeconds: 3
  failureThreshold: 30
```

The startup probe allows up to 150 seconds (30 failures x 5s period) for initial boot before the liveness probe kicks in. This accommodates large databases that need time to initialize.

### Docker Compose Health Check

```yaml
healthcheck:
  test: ["CMD", "curl", "-sf", "http://localhost:8420/healthz"]
  interval: 10s
  timeout: 3s
  retries: 3
  start_period: 30s
```

---

## Backup (Litestream)

Forge uses [Litestream](https://litestream.io) for continuous SQLite replication. Litestream streams every WAL frame to a remote store in near-realtime without stopping the daemon.

### Configuration

The Litestream config is at `deploy/litestream.yml`. Uncomment the replica backend you want to use.

**AWS S3:**

```yaml
dbs:
  - path: /var/lib/forge/forge.db
    replicas:
      - type: s3
        bucket: my-forge-backups
        path: forge/backup
        region: us-east-1
        access-key-id: ${AWS_ACCESS_KEY_ID}
        secret-access-key: ${AWS_SECRET_ACCESS_KEY}
```

Required env vars: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`

**Google Cloud Storage:**

```yaml
dbs:
  - path: /var/lib/forge/forge.db
    replicas:
      - type: gcs
        bucket: my-forge-backups
        path: forge/backup
```

Required env var: `GOOGLE_APPLICATION_CREDENTIALS` (path to service account JSON key)

**Azure Blob Storage:**

```yaml
dbs:
  - path: /var/lib/forge/forge.db
    replicas:
      - type: abs
        account-name: ${AZURE_STORAGE_ACCOUNT}
        account-key: ${AZURE_STORAGE_KEY}
        bucket: ${AZURE_STORAGE_CONTAINER}
        path: forge/backup
```

**Cloudflare R2 (S3-compatible):**

```yaml
dbs:
  - path: /var/lib/forge/forge.db
    replicas:
      - type: s3
        bucket: ${R2_BUCKET}
        path: forge/backup
        endpoint: https://${R2_ACCOUNT_ID}.r2.cloudflarestorage.com
        access-key-id: ${R2_ACCESS_KEY_ID}
        secret-access-key: ${R2_SECRET_ACCESS_KEY}
        skip-verify: true
```

### Running with Litestream

**Docker / Kubernetes:**

```bash
# Restore on startup (no-op if DB already exists):
litestream restore -if-db-not-exists -config /etc/litestream.yml /var/lib/forge/forge.db

# Run forge-daemon as a Litestream subprocess for continuous replication:
litestream replicate -config /etc/litestream.yml -exec "forge-daemon"
```

In the Helm chart, backup is handled by an init container (restore) and a sidecar container (replicate). Enable it with `backup.enabled=true` in `values.yaml`.

### Restore from Backup

```bash
# Restore to default path
litestream restore -config /etc/litestream.yml /var/lib/forge/forge.db

# Restore to a different path
litestream restore -config /etc/litestream.yml -o /tmp/forge-restored.db /var/lib/forge/forge.db
```

### Verify Backup

Check Litestream logs for replication lag. Healthy replication shows generation snapshots and WAL segment uploads with minimal delay:

```
level=INFO msg="replicating" db=/var/lib/forge/forge.db
level=INFO msg="initialized" replica=s3
level=INFO msg="snapshot created" replica=s3 position=000000000001/00000001
```

If replication stops, you will see no new log entries. Monitor the Litestream process alongside the daemon.

---

## Capacity Planning

### SQLite Performance

| Metric | Typical Value |
|--------|--------------|
| Write throughput | ~50 writes/sec (WAL mode, single writer) |
| Read throughput | Unlimited concurrent readers |
| Memory count | 10,000+ memories without degradation |
| Concurrent sessions | 100+ (read-only per session, serialized writes) |

### Storage

| Component | Size Per Unit | Notes |
|-----------|--------------|-------|
| Memory record | ~1 KB | Title, content, metadata, timestamps |
| Memory + embedding | ~3 KB | Adds 768-dim float32 vector (3,072 bytes) |
| Knowledge graph edge | ~100 bytes | Source, target, relation, weight |
| WAL file | Variable | Checkpointed periodically; monitor size |

For 10,000 memories with embeddings and 30,000 edges: approximately 33 MB database file.

### Compute

| Resource | Requirement |
|----------|-------------|
| Memory (RAM) | ~128 MB base + ~1 MB per 1,000 memories |
| CPU (idle) | < 1% -- background workers sleep between intervals |
| CPU (extraction) | Depends on backend (Ollama: moderate, API: minimal) |
| CPU (recall) | Minimal -- SQLite FTS5 + vector search is fast |

### Helm Chart Defaults

```yaml
resources:
  requests:
    memory: "128Mi"
    cpu: "100m"
  limits:
    memory: "512Mi"
    cpu: "1000m"

persistence:
  size: 10Gi
```

---

## Session Lifecycle

The daemon tracks every connecting client in the `session` table and
moves rows through a three-state lifecycle as their heartbeats arrive
(or don't):

```
   ┌─────────────────────┐  no heartbeat ≥ heartbeat_idle_secs   ┌──────────┐
   │ active (last write) │ ─────────────────────────────────────▶ │   idle   │
   └──────────────────────┘ ◀───────── heartbeat ─────────────── └──────────┘
            │                                                          │
            │   no heartbeat ≥ heartbeat_timeout_secs                  │
            ▼                                                          │
       ┌──────────┐                                                    │
       │  ended   │ ◀──────────────────────────────────────────────────┘
       └──────────┘     (no heartbeat ≥ heartbeat_timeout_secs)
```

**A live heartbeat from an `idle` session atomically revives it back to
`active` in the same UPDATE that refreshes `last_heartbeat_at`** — so a
client that was quiet for ten minutes keeps its session ID instead of
being forced to re-register on the next message.

### Tunables (Phase 2A-4d.3.1 #7)

Both knobs live under `[workers]` in the daemon config and can be
hot-reloaded by editing the file (no daemon restart required):

| Setting                    | Default      | Earlier default | Meaning                                                             |
|----------------------------|--------------|-----------------|---------------------------------------------------------------------|
| `heartbeat_idle_secs`      | `600` (10 m) | _(new in 0.5)_  | After this gap, `active → idle` and `session_idled` event fires.    |
| `heartbeat_timeout_secs`   | `14400` (4 h)| `60` (1 m)      | After this gap from any live state, the session is reaped to `ended`. |

> **Migration note for 0.5.x:** the `heartbeat_timeout_secs` default
> jumped from **60 seconds** to **14400 seconds** (4 hours) because
> the old value was ending healthy sessions during 5-minute user
> breaks. Operators who relied on the old aggressive reap should set
> `heartbeat_timeout_secs = 60` explicitly in their config.

Setting `heartbeat_idle_secs = 0` disables the `active → idle` phase
(sessions stay `active` until they hit the ended threshold). Setting
`heartbeat_timeout_secs = 0` disables ended-reaping entirely (rows
accumulate forever — only do this for forensic test runs).

`validated()` clamps `heartbeat_idle_secs` to be **strictly less than**
`heartbeat_timeout_secs`; an invalid pair is silently corrected at
load time and a warning is logged.

### Observability

- `forge_session_state{state}` Prometheus counter increments on every
  state transition.
- `session_idled` is emitted on the in-process bus (see
  `docs/architecture/events-namespace.md`) once per session that
  passes Phase 0 of a reaper tick. The payload is
  `{event_schema_version: 1, session_id, idle_secs}`.
- `forge-next inspect sessions` shows the current row state for any
  session id.

### Operational caveats

- **Long-lived agents** (e.g. Claude Code in a developer's terminal)
  should not be surprised by the `idle` state — they remain
  addressable via FISP, and any inbound message revives them. Only
  the `ended` state forecloses further message delivery.
- The reaper does **not** transition `idle → active` on its own. Only
  `update_heartbeat` does that, so a quiet session legitimately stays
  visible as `idle` in the inspector until the client checks in.
- When you increase `heartbeat_timeout_secs`, ended sessions take
  longer to clear from the table — this is normal. The retention
  reaper still trims `kpi_events` independently per its own schedule.

---

## Troubleshooting

### Pod CrashLoopBackOff

**Symptoms:** Pod restarts repeatedly with CrashLoopBackOff status.

**Common causes:**
1. **Missing HOME env var**: The daemon needs `HOME` to resolve `~/.forge/`. Set `HOME=/var/lib/forge` in the container.
2. **PVC not mounted**: The SQLite database path (`/var/lib/forge/forge.db`) must be on a persistent volume. Check that the PVC is bound and mounted.
3. **Socket permissions**: If running as non-root, ensure the socket directory is writable. The daemon creates `$HOME/.forge/forge.sock`.
4. **Read-only filesystem**: The container uses `readOnlyRootFilesystem: true`. Ensure `/tmp` is an emptyDir and `/var/lib/forge` is the PVC.

**Debug:**
```bash
kubectl logs <pod> -c forge-daemon --previous
kubectl describe pod <pod>
kubectl exec <pod> -c forge-daemon -- ls -la /var/lib/forge/
```

### 503 on All Requests

**Symptoms:** `/readyz` returns `503`, all API calls fail.

**Common causes:**
1. **Writer actor dead**: The writer channel is closed. Check logs for panic traces. Restart the pod.
2. **Database corruption**: SQLite WAL corruption (rare). Restore from Litestream backup.
3. **Disk full**: Check PVC usage. SQLite needs space for WAL and temporary files.

**Debug:**
```bash
# Check readyz details
curl -s http://localhost:8420/readyz | jq .

# Check disk usage
kubectl exec <pod> -c forge-daemon -- df -h /var/lib/forge/

# Check WAL size
kubectl exec <pod> -c forge-daemon -- ls -la /var/lib/forge/forge.db*
```

### Stale Metrics (All Zeros)

**Symptoms:** All metric values are 0, even though the daemon has data.

**Common causes:**
1. **Metrics disabled**: Check `FORGE_METRICS_ENABLED`. Must be `true`.
2. **HTTP not enabled**: Metrics are served on the HTTP transport. Set `FORGE_HTTP_ENABLED=true`.
3. **Wrong endpoint**: Ensure Prometheus is scraping the correct port (default: 8420).

**Debug:**
```bash
curl -s http://localhost:8420/metrics | head -20
```

### Auth Failures

**Symptoms:** `401 Unauthorized` on API calls despite valid tokens.

**Common causes:**
1. **JWKS cache stale**: The daemon caches JWKS keys. Default TTL is 3600 seconds. Restart the daemon or wait for cache expiry.
2. **Issuer URL misconfigured**: The `FORGE_AUTH_ISSUER_URL` must match the `iss` claim in the JWT exactly.
3. **Audience mismatch**: The `FORGE_AUTH_AUDIENCE` must match the `aud` claim in the JWT.
4. **Clock skew**: JWTs have `exp` and `nbf` claims. Ensure pod clock is synchronized (NTP).
5. **Offline JWKS**: For air-gapped environments, set `FORGE_AUTH_OFFLINE_JWKS_PATH` to a local JWKS JSON file.

**Debug:**
```bash
# Decode the JWT (without verification) to check claims
echo "$JWT" | cut -d. -f2 | base64 -d 2>/dev/null | jq .

# Check auth config
forge-next config show | grep -A5 auth
```

### Slow Recall

**Symptoms:** `forge_recall_latency_seconds` p99 exceeds 1 second.

**Common causes:**
1. **Large embedding count**: Vector search scales linearly with embedding count. Check `forge_embeddings_total`.
2. **WAL file bloat**: A large WAL file slows reads. Force a checkpoint:
   ```bash
   sqlite3 /var/lib/forge/forge.db "PRAGMA wal_checkpoint(TRUNCATE);"
   ```
3. **Missing FTS index**: Run `forge-next force-index` to rebuild indexes.
4. **High concurrent load**: While SQLite supports unlimited concurrent readers, very high load can cause contention on the WAL.

**Debug:**
```bash
# Check database and WAL sizes
ls -la /var/lib/forge/forge.db*

# Check memory and embedding counts
forge-next health
forge-next manas-health
```

### Extraction Not Running

**Symptoms:** New transcripts are not being processed into memories.

**Common causes:**
1. **No extraction backend**: Check `extraction.backend` config. If `auto`, the daemon tries Ollama, then Claude CLI, then falls back.
2. **Ollama not reachable**: If using Ollama, ensure `http://localhost:11434` is accessible from the daemon.
3. **API key missing**: For `claude_api`, `openai`, or `gemini` backends, ensure the API key is set via config or env var.
4. **Debounce delay**: Extraction has a configurable debounce (default: 15 seconds). Wait or use `forge-next extract --force`.

**Debug:**
```bash
forge-next config show | grep -A10 extraction
forge-next stats --hours 1
forge-next extract --force
```

---

## Environment Variables

All environment variables override `~/.forge/config.toml` values. Invalid values are silently ignored (the config file value remains).

### HTTP Transport

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_HTTP_ENABLED` | `false` | Enable HTTP transport |
| `FORGE_HTTP_BIND` | `127.0.0.1` | HTTP bind address |
| `FORGE_HTTP_PORT` | `8420` | HTTP listen port |

### gRPC Transport

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_GRPC_ENABLED` | `false` | Enable gRPC transport (JSON-over-gRPC) |
| `FORGE_GRPC_BIND` | `0.0.0.0` | gRPC bind address |
| `FORGE_GRPC_PORT` | `8421` | gRPC listen port |

### CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_CORS_ALLOWED_ORIGINS` | `*` | Comma-separated allowed origins |
| `FORGE_CORS_MAX_AGE_SECS` | `3600` | CORS preflight cache duration |

### Authentication (JWT/OIDC)

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_AUTH_ENABLED` | `false` | Enable JWT/OIDC authentication |
| `FORGE_AUTH_ISSUER_URL` | *(empty)* | OIDC issuer URL (must match JWT `iss` claim) |
| `FORGE_AUTH_AUDIENCE` | *(empty)* | Expected JWT audience (must match `aud` claim) |
| `FORGE_AUTH_REQUIRED_CLAIMS` | *(empty)* | Comma-separated required JWT claims |
| `FORGE_AUTH_ADMIN_EMAILS` | *(empty)* | Comma-separated admin email addresses (full access) |
| `FORGE_AUTH_VIEWER_EMAILS` | *(empty)* | Comma-separated viewer email addresses (read-only) |
| `FORGE_AUTH_JWKS_CACHE_SECS` | `3600` | JWKS key cache TTL in seconds |
| `FORGE_AUTH_OFFLINE_JWKS_PATH` | *(none)* | Path to local JWKS JSON file (for air-gapped environments) |

### Metrics

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_METRICS_ENABLED` | `true` | Enable Prometheus `/metrics` endpoint |

### OTLP Tracing

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_OTLP_ENABLED` | `false` | Enable OpenTelemetry trace export |
| `FORGE_OTLP_ENDPOINT` | *(empty)* | OTLP collector gRPC endpoint (e.g., `http://jaeger:4317`) |
| `FORGE_OTLP_SERVICE_NAME` | `forge-daemon` | Service name reported in traces |

### Daemon Paths

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_DB` | `~/.forge/forge.db` | SQLite database path |
| `FORGE_SOCKET` | `~/.forge/forge.sock` | Unix domain socket path |
| `HOME` | *(system)* | Home directory. Must be set in containers for `~/.forge/` resolution. |

### Extraction Backend API Keys

These are resolved with priority: config file value > environment variable.

| Variable | Description |
|----------|-------------|
| `ANTHROPIC_API_KEY` | Anthropic API key (for `claude_api` backend) |
| `OPENAI_API_KEY` | OpenAI API key (for `openai` backend) |
| `GEMINI_API_KEY` | Google Gemini API key (for `gemini` backend) |

### CLI-Only

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_ENDPOINT` | *(none)* | Remote daemon URL (same as `--endpoint` flag) |
| `FORGE_TOKEN` | *(none)* | JWT auth token (same as `--token` flag) |

---

## Deployment Options

### Docker Compose (Single Node)

```bash
# Start daemon only
docker compose -f deploy/docker-compose.yml up -d

# Start with monitoring stack (Prometheus + Grafana)
docker compose -f deploy/docker-compose.yml --profile monitor up -d

# View logs
docker compose -f deploy/docker-compose.yml logs -f forge
```

### Helm Chart (Kubernetes)

```bash
# Install
helm install forge deploy/helm/forge \
  --set auth.enabled=true \
  --set auth.issuerUrl=https://auth.company.com \
  --set auth.audience=forge-api

# Install with backup
helm install forge deploy/helm/forge \
  --set backup.enabled=true \
  --set backup.s3.bucket=my-forge-backups \
  --set backup.s3.existingSecret=forge-aws-credentials

# Upgrade
helm upgrade forge deploy/helm/forge -f custom-values.yaml
```

The Helm chart deploys Forge as a **StatefulSet** with a single replica (SQLite requires single-writer). Key features:
- PersistentVolumeClaim for database storage (default: 10Gi)
- Pod security context: non-root (UID 1000), read-only root filesystem, no privilege escalation
- ConfigMap for `config.toml`
- Optional Litestream sidecar for continuous backup
- ServiceMonitor for Prometheus Operator integration

### Systemd (Bare Metal / VM)

```bash
# Install as systemd service
forge-next service install

# Manage
forge-next service start
forge-next service stop
forge-next service status
forge-next service uninstall
```

---

## Security Considerations

- **Network exposure**: When `FORGE_HTTP_BIND` is set to anything other than `127.0.0.1` or `localhost`, enable authentication (`FORGE_AUTH_ENABLED=true`). The daemon logs a security warning if HTTP is exposed without auth.
- **Socket permissions**: The Unix socket is created with `umask 0177` (owner-only access).
- **Database file**: Ensure the SQLite database file has restrictive permissions (0600).
- **API keys**: Never store API keys in environment variables visible to other processes. Use Kubernetes secrets or a secrets manager.
- **RBAC**: When auth is enabled, three roles are enforced: Admin (full access), Viewer (read-only), Member (default, standard access). Assign via `FORGE_AUTH_ADMIN_EMAILS` and `FORGE_AUTH_VIEWER_EMAILS`.
- **File size limit**: The daemon enforces a 50 MB file size limit on imports.
- **UTF-8 safety**: All string inputs are safely truncated at UTF-8 boundaries.
- **Symlink defense**: File operations in the scanner and guardrails reject symlinks that escape workspace boundaries.
- **SQL injection**: All database queries use parameterized SQL. No string interpolation.
