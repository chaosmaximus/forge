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

# Build only the packages we ship
RUN cargo build --release -p forge-daemon -p forge-cli

# Strip binaries for smaller image (release profile has strip=true but be explicit)
RUN strip --strip-all target/release/forge-daemon target/release/forge-next 2>/dev/null || true

# ---------------------------------------------------------------------------
# Stage 2: Litestream (optional — uncomment COPY below to enable)
# ---------------------------------------------------------------------------
FROM litestream/litestream:0.3 AS litestream

# ---------------------------------------------------------------------------
# Stage 3: Scratch-based runtime with only glibc runtime deps
# ---------------------------------------------------------------------------
# Use debian-slim to extract only the required shared libraries,
# then build a minimal filesystem from scratch.
FROM debian:bookworm-slim AS runtime-deps

# Copy only the shared libraries our binaries need + CA certs
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    mkdir -p /forge-data && chown 65532:65532 /forge-data

# ---------------------------------------------------------------------------
# Stage 4: Final minimal image
# ---------------------------------------------------------------------------
FROM gcr.io/distroless/cc-debian12:nonroot

# Copy CA certificates for TLS
COPY --from=runtime-deps /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/

# Create data directory (distroless has no shell, copy from builder)
COPY --from=runtime-deps --chown=nonroot:nonroot /forge-data /var/lib/forge

# Copy binaries from builder
COPY --from=builder /build/target/release/forge-daemon /usr/local/bin/
COPY --from=builder /build/target/release/forge-next /usr/local/bin/

# Uncomment to include Litestream for continuous SQLite replication:
# COPY --from=litestream /usr/local/bin/litestream /usr/local/bin/
# COPY deploy/litestream.yml /etc/litestream.yml

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

# NOTE: HEALTHCHECK not supported in distroless (no shell).
# Use K8s livenessProbe with httpGet to /healthz instead.

ENTRYPOINT ["forge-daemon"]
