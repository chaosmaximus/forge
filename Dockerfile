# =============================================================================
# Forge Daemon — Multi-stage Docker Build
# =============================================================================
#
# Build:   docker build -t forge .
# Run:     docker run -d -p 8420:8420 -v forge-data:/var/lib/forge forge
#
# Litestream integration (continuous SQLite replication):
#   1. Copy litestream binary into the runtime image (see litestream stage below)
#   2. On startup, restore from replica if DB doesn't exist:
#        litestream restore -if-db-not-exists /var/lib/forge/forge.db
#   3. Run daemon as Litestream subprocess for continuous replication:
#        litestream replicate -config /etc/litestream.yml -exec "forge-daemon"
#
#   This ensures every WAL frame is replicated to S3/GCS/Azure in near-realtime,
#   and the DB is automatically restored on first container boot.
# =============================================================================

# ---------------------------------------------------------------------------
# Stage 1: Builder — compile Rust release binaries
# ---------------------------------------------------------------------------
FROM rust:1.88-bookworm AS builder

WORKDIR /build

# Install protoc for gRPC/tonic proto compilation
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# The workspace includes app/forge/src-tauri, but we only need daemon + CLI.
# Create a stub so Cargo resolves the workspace without the Tauri dependency.
RUN mkdir -p app/forge/src-tauri/src && \
    echo '[package]\nname = "forge-app"\nversion = "0.1.0"\nedition = "2021"' > app/forge/src-tauri/Cargo.toml && \
    echo 'fn main() {}' > app/forge/src-tauri/src/main.rs

# Build only the packages we ship
RUN cargo build --release -p forge-daemon -p forge-cli

# ---------------------------------------------------------------------------
# Stage 2: Litestream (optional — uncomment COPY below to enable)
# ---------------------------------------------------------------------------
FROM litestream/litestream:0.3 AS litestream

# ---------------------------------------------------------------------------
# Stage 3: Runtime — minimal Debian slim
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim

# Install minimal runtime dependencies
# SQLite is bundled in the Rust binary (rusqlite bundled feature), so we only
# need libc, TLS certs, and curl for the healthcheck.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd -r forge && useradd -r -g forge -m -d /home/forge forge

# Copy binaries from builder
COPY --from=builder /build/target/release/forge-daemon /usr/local/bin/
COPY --from=builder /build/target/release/forge-next /usr/local/bin/

# Uncomment to include Litestream for continuous SQLite replication:
# COPY --from=litestream /usr/local/bin/litestream /usr/local/bin/
# COPY deploy/litestream.yml /etc/litestream.yml

# Create data directory with correct ownership
RUN mkdir -p /var/lib/forge && chown forge:forge /var/lib/forge

# Switch to non-root user
USER forge
WORKDIR /home/forge

# Environment defaults for container deployment.
# SECURITY: 0.0.0.0 is required for K8s Service routing. Protect with:
#   - NetworkPolicy to restrict ingress
#   - FORGE_AUTH_ENABLED=true for production
#   - Ingress controller for external TLS termination
ENV FORGE_DB=/var/lib/forge/forge.db \
    FORGE_SOCKET=/var/lib/forge/forge.sock \
    FORGE_HTTP_ENABLED=true \
    FORGE_HTTP_BIND=0.0.0.0 \
    FORGE_HTTP_PORT=8420

# HTTP transport port
EXPOSE 8420

# Persistent data volume
VOLUME ["/var/lib/forge"]

# Healthcheck via HTTP /healthz endpoint
HEALTHCHECK --interval=10s --timeout=3s --start-period=30s --retries=3 \
    CMD curl -sf http://localhost:8420/healthz || exit 1

ENTRYPOINT ["forge-daemon"]
