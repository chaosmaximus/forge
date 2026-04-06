# Cloud Deployment

This guide covers deploying Forge to Kubernetes with Helm, or running it standalone with Docker. Forge runs as a single-binary daemon backed by SQLite, so it does not require an external database.

## Architecture Overview

```
                  ┌──────────────────────────┐
                  │       Kubernetes          │
                  │                           │
Developer A ─────►│  ┌─────────────────────┐  │
(forge-next       │  │   forge-daemon pod   │  │
 --endpoint)      │  │   (StatefulSet)      │  │
                  │  │                      │  │
Developer B ─────►│  │  HTTP :8420          │  │
                  │  │  SQLite + WAL        │  │
                  │  │  Litestream sidecar  │──┼──► S3 backup
                  │  └─────────────────────┘  │
                  │                           │
                  │  Prometheus ──► Grafana   │
                  └──────────────────────────┘
```

A single Forge daemon serves the entire team. Each developer connects from their laptop using `forge-next --endpoint`.

## Prerequisites

- Kubernetes cluster (1.24+)
- Helm 3
- kubectl configured for the target cluster
- cert-manager (optional, for TLS)

## Install with Helm

### Minimal install

```bash
helm install forge deploy/helm/forge/
```

This creates a StatefulSet with a 10Gi PersistentVolume, a ClusterIP Service on port 8420, and enables HTTP transport and Prometheus metrics by default.

### Production install

```bash
helm install forge deploy/helm/forge/ \
  --set auth.enabled=true \
  --set auth.issuerUrl="https://login.company.com" \
  --set auth.audience="forge-api" \
  --set auth.adminEmails='{admin@company.com}' \
  --set backup.enabled=true \
  --set backup.s3.bucket="my-forge-backups" \
  --set backup.s3.region="us-east-1" \
  --set backup.s3.existingSecret="forge-aws-creds" \
  --set persistence.size=50Gi \
  --set persistence.storageClass=gp3
```

### Using a values file

Create `my-values.yaml`:

```yaml
auth:
  enabled: true
  issuerUrl: "https://login.company.com"
  audience: "forge-api"
  adminEmails:
    - admin@company.com

backup:
  enabled: true
  s3:
    bucket: "my-forge-backups"
    region: "us-east-1"
    existingSecret: "forge-aws-creds"

persistence:
  size: 50Gi
  storageClass: gp3

resources:
  requests:
    memory: "256Mi"
    cpu: "200m"
  limits:
    memory: "1Gi"
    cpu: "2000m"
```

```bash
helm install forge deploy/helm/forge/ -f my-values.yaml
```

## Verify Deployment

Check the pod is running:

```bash
kubectl get pods -l app.kubernetes.io/name=forge
```

Port-forward to test locally:

```bash
kubectl port-forward svc/forge 8420:8420
curl http://localhost:8420/healthz
curl http://localhost:8420/readyz
```

## Expose with Ingress

Create an Ingress resource for external access:

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: forge
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
spec:
  tls:
    - hosts:
        - forge.company.com
      secretName: forge-tls
  rules:
    - host: forge.company.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: forge
                port:
                  number: 8420
```

```bash
kubectl apply -f forge-ingress.yaml
```

## Connect from Laptop

Once the daemon is accessible over HTTPS, connect from any developer machine:

```bash
forge-next --endpoint https://forge.company.com --token <JWT> health
```

Store a memory on the shared server:

```bash
forge-next --endpoint https://forge.company.com --token <JWT> \
  remember --type decision --title "Use gRPC for service mesh" \
  --content "Team agreed to use gRPC between microservices for type safety and performance."
```

Recall from the shared server:

```bash
forge-next --endpoint https://forge.company.com --token <JWT> \
  recall "service mesh"
```

## Authentication

Forge supports OIDC/JWT authentication. When enabled, every HTTP request must include a valid Bearer token.

### Helm configuration

```bash
helm upgrade forge deploy/helm/forge/ \
  --set auth.enabled=true \
  --set auth.issuerUrl="https://login.company.com" \
  --set auth.audience="forge-api" \
  --set auth.adminEmails='{admin@company.com,lead@company.com}' \
  --set auth.viewerEmails='{dev1@company.com,dev2@company.com}'
```

### Environment variables

| Variable | Description |
|----------|-------------|
| `FORGE_AUTH_ENABLED` | Enable JWT validation (`true` / `false`) |
| `FORGE_AUTH_ISSUER_URL` | OIDC issuer URL (must serve `.well-known/openid-configuration`) |
| `FORGE_AUTH_AUDIENCE` | Expected `aud` claim in the JWT |
| `FORGE_AUTH_ADMIN_EMAILS` | Comma-separated list of admin email addresses |
| `FORGE_AUTH_VIEWER_EMAILS` | Comma-separated list of viewer email addresses |
| `FORGE_AUTH_REQUIRED_CLAIMS` | Additional required JWT claims (comma-separated `key=value`) |
| `FORGE_AUTH_JWKS_CACHE_SECS` | JWKS cache TTL in seconds |
| `FORGE_AUTH_OFFLINE_JWKS_PATH` | Path to a local JWKS file for air-gapped environments |

## Backup with Litestream

Forge uses SQLite in WAL mode. Litestream runs as a sidecar container that continuously replicates every WAL frame to S3, GCS, or Azure Blob Storage.

### Enable backup

```bash
helm upgrade forge deploy/helm/forge/ \
  --set backup.enabled=true \
  --set backup.s3.bucket="my-forge-backups" \
  --set backup.s3.region="us-east-1" \
  --set backup.s3.existingSecret="forge-aws-creds"
```

The `existingSecret` must contain `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`:

```bash
kubectl create secret generic forge-aws-creds \
  --from-literal=AWS_ACCESS_KEY_ID=AKIA... \
  --from-literal=AWS_SECRET_ACCESS_KEY=...
```

### How it works

1. On pod startup, an init container runs `litestream restore -if-db-not-exists` to restore from the latest replica if no local DB exists.
2. A sidecar container runs `litestream replicate` to continuously stream WAL frames to S3.
3. On pod restart, the cycle repeats -- restore if needed, then replicate.

### Manual restore

```bash
litestream restore \
  -config deploy/litestream.yml \
  -o /tmp/forge-restored.db \
  /var/lib/forge/forge.db
```

### Supported backends

| Backend | Config key | Notes |
|---------|-----------|-------|
| AWS S3 | `s3` | Default. Also works with MinIO. |
| Google Cloud Storage | `gcs` | Set `GOOGLE_APPLICATION_CREDENTIALS` env var. |
| Azure Blob Storage | `abs` | Set `AZURE_STORAGE_ACCOUNT` and `AZURE_STORAGE_KEY`. |
| Cloudflare R2 | `s3` | S3-compatible. Set `endpoint` to R2 URL. |

See `deploy/litestream.yml` for full configuration examples.

## Monitoring

### Prometheus

Forge exposes a `/metrics` endpoint on port 8420 when `metrics.enabled=true` (the default). A `ServiceMonitor` resource is created automatically for Prometheus Operator.

Key metrics:

| Metric | Type | Description |
|--------|------|-------------|
| `forge_memories_total` | Gauge | Total memory count |
| `forge_active_sessions` | Gauge | Currently active sessions |
| `forge_recall_latency_seconds` | Histogram | Recall query latency |
| `forge_extraction_duration_seconds` | Histogram | Extraction pipeline duration |
| `forge_worker_healthy` | Gauge | Per-worker health status (1 = healthy) |

### Grafana dashboard

Import the pre-built dashboard:

```bash
kubectl create configmap forge-dashboard \
  --from-file=deploy/grafana/forge-dashboard.json
```

Or load `deploy/grafana/forge-dashboard.json` directly into Grafana via the UI (Dashboards > Import > Upload JSON).

### Alerts

Pre-built Prometheus alert rules are in `deploy/grafana/forge-alerts.yml`. Key alerts:

| Alert | Severity | Condition |
|-------|----------|-----------|
| `ForgeWorkerDown` | critical | Any worker unhealthy for 5 minutes |
| `ForgeAllWorkersDown` | critical | All workers down for 1 minute |
| `ForgeExtractionSlow` | warning | Extraction p95 > 60 seconds for 5 minutes |
| `ForgeMemoryStale` | warning | No new memories for 1 hour |
| `ForgeHighRecallLatency` | warning | Recall p99 > 5 seconds for 10 minutes |

### OpenTelemetry

Export traces to Jaeger, Datadog, or any OTLP-compatible collector:

```bash
helm upgrade forge deploy/helm/forge/ \
  --set otlp.enabled=true \
  --set otlp.endpoint="http://jaeger-collector:4317"
```

## Docker Standalone

For single-node deployment without Kubernetes:

```bash
docker run -d \
  --name forge \
  -p 8420:8420 \
  -v forge-data:/var/lib/forge \
  ghcr.io/chaosmaximus/forge-daemon:latest
```

Verify:

```bash
curl http://localhost:8420/healthz
```

### With monitoring stack

```bash
cd deploy
docker compose up -d                        # Forge daemon only
docker compose --profile monitor up -d      # Forge + Prometheus + Grafana
```

Grafana is available at `http://localhost:3000` (default credentials: admin / changeme).

### Build the image locally

```bash
docker build -t forge .
docker run -d -p 8420:8420 -v forge-data:/var/lib/forge forge
```

## Health Endpoints

| Endpoint | Purpose |
|----------|---------|
| `GET /healthz` | Liveness probe. Returns 200 if the process is alive. |
| `GET /readyz` | Readiness probe. Returns 200 when the memory system is initialized. |
| `GET /startupz` | Startup probe. Returns 200 once initial setup is complete. |
| `GET /metrics` | Prometheus metrics (when `metrics.enabled=true`). |

## Environment Variables Reference

All configuration can be set via environment variables, which take precedence over `config.toml`.

### Core

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_DB` | `~/.forge/forge.db` | Path to SQLite database |
| `FORGE_SOCKET` | `~/.forge/forge.sock` | Path to Unix domain socket |

### HTTP Transport

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_HTTP_ENABLED` | `false` | Enable HTTP transport |
| `FORGE_HTTP_BIND` | `127.0.0.1` | HTTP bind address |
| `FORGE_HTTP_PORT` | `8420` | HTTP port |

### gRPC Transport

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_GRPC_ENABLED` | `false` | Enable gRPC transport |
| `FORGE_GRPC_BIND` | `127.0.0.1` | gRPC bind address |
| `FORGE_GRPC_PORT` | `8421` | gRPC port |

### CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_CORS_ALLOWED_ORIGINS` | (none) | Comma-separated allowed origins |
| `FORGE_CORS_MAX_AGE_SECS` | `3600` | CORS preflight cache duration |

### Authentication

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_AUTH_ENABLED` | `false` | Enable JWT authentication |
| `FORGE_AUTH_ISSUER_URL` | (none) | OIDC issuer URL |
| `FORGE_AUTH_AUDIENCE` | (none) | Expected JWT audience |
| `FORGE_AUTH_REQUIRED_CLAIMS` | (none) | Required claims (comma-separated `k=v`) |
| `FORGE_AUTH_ADMIN_EMAILS` | (none) | Admin email addresses (comma-separated) |
| `FORGE_AUTH_VIEWER_EMAILS` | (none) | Viewer email addresses (comma-separated) |
| `FORGE_AUTH_JWKS_CACHE_SECS` | `3600` | JWKS cache TTL |
| `FORGE_AUTH_OFFLINE_JWKS_PATH` | (none) | Local JWKS file for air-gapped environments |

### Observability

| Variable | Default | Description |
|----------|---------|-------------|
| `FORGE_METRICS_ENABLED` | `true` | Enable `/metrics` endpoint |
| `FORGE_OTLP_ENABLED` | `false` | Enable OpenTelemetry export |
| `FORGE_OTLP_ENDPOINT` | (none) | OTLP collector endpoint |
| `FORGE_OTLP_SERVICE_NAME` | `forge-daemon` | Service name in traces |

## Upgrading

### Helm

```bash
helm upgrade forge deploy/helm/forge/ -f my-values.yaml
```

The StatefulSet will perform a rolling update. The pod annotation `checksum/config` triggers a restart when the ConfigMap changes.

### Docker

```bash
docker pull ghcr.io/chaosmaximus/forge-daemon:latest
docker stop forge && docker rm forge
docker run -d --name forge -p 8420:8420 -v forge-data:/var/lib/forge ghcr.io/chaosmaximus/forge-daemon:latest
```

The SQLite database is stored on the persistent volume and survives container replacement.

## Next Steps

- [Getting Started](getting-started.md) -- local installation and first use
- [Agent Development](agent-development.md) -- build custom agents that connect to Forge
