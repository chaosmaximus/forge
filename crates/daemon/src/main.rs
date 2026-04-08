use forge_daemon::server::{DaemonState, WriterActor, run_grpc_server, run_http_server_with_listener, run_server};
use forge_daemon::server::http::{AppState, build_router};
use forge_daemon::server::tls;
use forge_daemon::server::metrics::ForgeMetrics;
use forge_core::{forge_dir, default_socket_path, default_db_path, default_pid_path};
use std::io::Write;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Acquire an exclusive PID lock file. If the lock is held by a dead process, clean up
/// the stale lock and retry. Returns the locked file handle (must be kept alive).
fn acquire_pid_lock(pid_path: &str) -> std::fs::File {
    use fs2::FileExt;
    use std::io::Seek;

    // I6: Open WITHOUT truncation first, attempt lock, THEN truncate+write
    // only after successful lock. This prevents destroying PID content before
    // the lock is acquired (which would make stale-PID detection fail).
    let try_open_and_lock = || -> Result<std::fs::File, String> {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false) // I6: intentionally no truncation — preserve PID for stale detection
            .open(pid_path)
            .map_err(|e| format!("failed to open PID file {pid_path}: {e}"))?;
        f.try_lock_exclusive()
            .map_err(|_| "lock held".to_string())?;
        // Lock acquired — now truncate the file before writing new PID.
        f.set_len(0).map_err(|e| format!("failed to truncate PID file: {e}"))?;
        let mut f = f;
        f.seek(std::io::SeekFrom::Start(0))
            .map_err(|e| format!("failed to seek PID file: {e}"))?;
        Ok(f)
    };

    match try_open_and_lock() {
        Ok(f) => f,
        Err(e) if e == "lock held" => {
            // Lock held — check if the PID in the file is actually alive
            if let Ok(contents) = std::fs::read_to_string(pid_path) {
                let pid_str = contents.trim();
                if let Ok(pid_num) = pid_str.parse::<i32>() {
                    #[cfg(unix)]
                    {
                        // Check if process exists by reading /proc/<pid>/status
                        let alive = std::path::Path::new(&format!("/proc/{}", pid_num)).exists();
                        if !alive {
                            tracing::warn!(
                                pid = pid_num,
                                "stale PID lock file found (process {} not alive) — cleaning up",
                                pid_num
                            );
                            let _ = std::fs::remove_file(pid_path);
                            match try_open_and_lock() {
                                Ok(f) => return f,
                                Err(e2) => {
                                    tracing::error!("failed to acquire PID lock after stale cleanup: {e2}");
                                    std::process::exit(1);
                                }
                            }
                        }
                    }
                }
            }
            tracing::error!("another forge-daemon is already running (PID file locked at {pid_path})");
            std::process::exit(1);
        }
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    }
}

/// Initialize the OpenTelemetry OTLP tracer provider and return a tracing-opentelemetry layer.
/// This is called only when FORGE_OTLP_ENABLED=true and FORGE_OTLP_ENDPOINT is set.
fn init_otlp_layer<S>(
    endpoint: &str,
    service_name: &str,
) -> Result<tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>, Box<dyn std::error::Error>>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry::KeyValue;
    use opentelemetry_otlp::WithExportConfig;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;

    let resource = opentelemetry_sdk::Resource::new(vec![
        KeyValue::new("service.name", service_name.to_string()),
    ]);

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(resource)
        .build();

    let tracer = provider.tracer(service_name.to_string());
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Keep the provider alive globally so spans are exported on shutdown.
    // opentelemetry::global is the canonical way to hold the provider.
    opentelemetry::global::set_tracer_provider(provider);

    Ok(layer)
}

#[tokio::main]
async fn main() {
    // Initialize structured JSON logging to stderr BEFORE anything else.
    // stdout is reserved for protocol output (NDJSON).
    //
    // The tracing subscriber is composed as layers so we can conditionally
    // add the OTLP export layer when FORGE_OTLP_ENABLED=true.
    // We read env vars directly (not ForgeConfig) to avoid a chicken-and-egg
    // problem — config loading logs, but the logger isn't initialized yet.
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("forge_daemon=info"));

    let json_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_target(true)
        .with_writer(std::io::stderr);

    let otlp_enabled = std::env::var("FORGE_OTLP_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);
    let otlp_endpoint = std::env::var("FORGE_OTLP_ENDPOINT").unwrap_or_default();
    let otlp_service = std::env::var("FORGE_OTLP_SERVICE_NAME")
        .unwrap_or_else(|_| "forge-daemon".to_string());

    // Build registry with json + optional OTLP layer.
    // tracing_subscriber::Option<Layer> is itself a Layer, so we can use .with(Option<L>).
    let otlp_layer = if otlp_enabled && !otlp_endpoint.is_empty() {
        match init_otlp_layer(&otlp_endpoint, &otlp_service) {
            Ok(layer) => {
                // Can't use tracing::info yet — subscriber isn't installed.
                eprintln!("[daemon] OTLP trace export enabled (endpoint={})", otlp_endpoint);
                Some(layer)
            }
            Err(e) => {
                eprintln!("[daemon] WARN: OTLP init failed (continuing without traces): {e}");
                None
            }
        }
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(json_layer)
        .with(otlp_layer)
        .init();

    let socket_path = std::env::var("FORGE_SOCKET").unwrap_or_else(|_| default_socket_path());
    let db_path = std::env::var("FORGE_DB").unwrap_or_else(|_| default_db_path());

    // Ensure ~/.forge/ directory exists
    let dir = forge_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::error!("failed to create {dir}: {e}");
        std::process::exit(1);
    }

    // I2: Set directory permissions to 0700 (owner-only access)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)) {
            tracing::warn!("failed to set permissions on {dir}: {e}");
        }
    }

    // C2: Write PID file with advisory lock to prevent multiple daemon instances.
    // If the lock fails, check if the PID in the file is actually alive.
    // Stale PID files from crashed daemons are auto-cleaned.
    let pid_path = default_pid_path();
    let pid_file = acquire_pid_lock(&pid_path);

    // Write PID — file is now locked exclusively
    let pid = std::process::id();
    if let Err(e) = write!(&pid_file, "{}", pid) {
        tracing::error!("failed to write PID to {pid_path}: {e}");
        std::process::exit(1);
    }

    // Keep pid_file alive (holds the advisory lock) for the lifetime of main()
    let _pid_file_guard = pid_file;

    // Create DaemonState for workers (opens/creates DB with write connection).
    // Workers use this via Arc<Mutex> for their background writes.
    let worker_state = match DaemonState::new(&db_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to open database {db_path}: {e}");
            // Best-effort cleanup of PID file
            if let Err(e2) = std::fs::remove_file(&pid_path) {
                tracing::warn!("failed to remove PID file on error: {e2}");
            }
            std::process::exit(1);
        }
    };

    // ISS-D9: Auto-vacuum on startup when DB exceeds 100MB.
    // Prevents unbounded growth from broad indexing.
    if let Ok(meta) = std::fs::metadata(&db_path) {
        let db_mb = meta.len() as f64 / 1_048_576.0;
        if db_mb > 100.0 {
            tracing::info!("DB is {db_mb:.1}MB — running auto-vacuum");
            if let Err(e) = worker_state.conn.execute_batch("VACUUM;") {
                tracing::warn!("auto-vacuum failed: {e}");
            }
            if let Ok(meta2) = std::fs::metadata(&db_path) {
                let freed = meta.len().saturating_sub(meta2.len());
                if freed > 0 {
                    tracing::info!("auto-vacuum freed {:.1}MB", freed as f64 / 1_048_576.0);
                }
            }
        }
    }

    // Extract shared resources BEFORE wrapping in Arc<Mutex>.
    // These are shared between the socket handler (read path), writer actor,
    // and workers so they all see the same events and HLC.
    let events = worker_state.events.clone();
    let hlc = Arc::clone(&worker_state.hlc);
    let started_at = worker_state.started_at;

    // Spawn HUD state writer — updates hud-state.json on every daemon event
    // so forge-hud can render current activity in the status line.
    forge_daemon::events::spawn_hud_writer(&events);

    let state = Arc::new(Mutex::new(worker_state));

    // C1: Create shutdown watch channel
    let (shutdown_tx, _shutdown_rx) = watch::channel(false);

    // Load config and apply environment variable overrides
    let mut config = forge_daemon::config::load_config();
    config.apply_env_overrides();
    tracing::info!(backend = %config.extraction.backend, "extraction backend configured");

    // Spawn background workers (they keep Arc<Mutex<DaemonState>> — unchanged)
    let _worker_handles = forge_daemon::workers::spawn_workers(
        Arc::clone(&state),
        config.clone(),
        &shutdown_tx,
        db_path.clone(),
        events.clone(),
    );

    // Create a SEPARATE DaemonState for the WriterActor. This is the key fix
    // for the write timeout bug: the writer owns its own DaemonState with an
    // independent SQLite connection, so it is NEVER blocked by workers holding
    // the Arc<Mutex<DaemonState>> during extraction (10-30s).
    // Both connections open the same db_path; SQLite WAL serializes writes internally.
    let (write_tx, write_rx) = mpsc::channel::<forge_daemon::server::WriteCommand>(256);
    let writer_state = match DaemonState::new_writer(
        &db_path,
        events.clone(),
        Arc::clone(&hlc),
        started_at,
    ) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to create writer state: {e}");
            std::process::exit(1);
        }
    };
    let writer = WriterActor { state: writer_state };
    tokio::spawn(async move { writer.run(write_rx).await });

    // I3: Spawn signal handler that sends on shutdown channel instead of process::exit
    let shutdown_for_signal = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("shutting down (signal)");
        let _ = shutdown_for_signal.send(true);
    });

    // Spawn startup tasks in background (consolidation, ingestion).
    // These run AFTER the server starts accepting connections, ensuring the
    // socket is available within ~100ms instead of waiting 2-5s for consolidation.
    //
    // IMPORTANT: Each task acquires and releases the lock independently.
    // This prevents a single long lock hold from blocking all API requests.
    // Like Docker — background maintenance never blocks the API.
    let startup_state = Arc::clone(&state);
    tokio::spawn(async move {
        // Phase 1: Consolidation (2-5s with many edges — short lock per phase)
        {
            let startup_consol_config = forge_daemon::config::load_config().consolidation.validated();
            let locked = startup_state.lock().await;
            let cs = forge_daemon::workers::consolidator::run_all_phases(&locked.conn, &startup_consol_config);
            eprintln!(
                "[daemon] startup consolidation: dedup={}, semantic={}, linked={}, faded={}, promoted={}, reconsolidated={}",
                cs.exact_dedup, cs.semantic_dedup, cs.linked, cs.faded, cs.promoted, cs.reconsolidated
            );
        } // lock released — API can serve requests between phases

        // Phase 2: Project ingestion (Layer 7 — Declared Knowledge) + Domain DNA (Layer 4)
        let project_dir = std::env::var("FORGE_PROJECT_DIR")
            .or_else(|_| std::env::current_dir().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_default();
        if !project_dir.is_empty() {
            {
                let locked = startup_state.lock().await;
                match forge_daemon::db::manas::ingest_project_declared(&locked.conn, &project_dir) {
                    Ok((ingested, _)) if ingested > 0 => eprintln!("[daemon] ingested {} declared knowledge files", ingested),
                    Ok(_) => {},
                    Err(e) => eprintln!("[daemon] WARN: declared knowledge ingestion failed: {e}"),
                }
            } // lock released

            {
                let locked = startup_state.lock().await;
                match forge_daemon::db::manas::detect_domain_dna(&locked.conn, &project_dir) {
                    Ok(n) if n > 0 => eprintln!("[daemon] detected {} project type markers", n),
                    Ok(_) => {},
                    Err(e) => eprintln!("[daemon] WARN: domain DNA detection failed: {e}"),
                }
            } // lock released
        }

        // Phase 3: Clean duplicate identity facets (fast, <100ms)
        {
            let locked = startup_state.lock().await;
            match locked.conn.execute(
                "DELETE FROM identity WHERE id NOT IN (
                    SELECT id FROM (
                        SELECT id, ROW_NUMBER() OVER (PARTITION BY agent, description ORDER BY strength DESC) as rn
                        FROM identity WHERE active = 1
                    ) WHERE rn = 1
                ) AND active = 1",
                [],
            ) {
                Ok(n) if n > 0 => eprintln!("[daemon] cleaned {} duplicate identity facets", n),
                Ok(_) => {},
                Err(e) => eprintln!("[daemon] WARN: identity dedup failed: {e}"),
            }
        } // lock released

        eprintln!("[daemon] startup tasks complete");
    });

    // Conditionally spawn HTTP server alongside Unix socket when enabled.
    // HTTP bind failure is fatal — if the operator explicitly enabled HTTP, we must serve it.
    if config.http.enabled {
        let http_config = config.clone();
        let http_db = db_path.clone();
        let http_events = events.clone();
        let http_hlc = Arc::clone(&hlc);
        let http_write_tx = write_tx.clone();
        let http_shutdown_rx = shutdown_tx.subscribe();
        // Pre-bind the listener synchronously so bind failures are caught before we proceed
        let http_addr = format!("{}:{}", http_config.http.bind, http_config.http.port);
        let http_listener = match tokio::net::TcpListener::bind(&http_addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(addr = %http_addr, "failed to bind HTTP server: {e}");
                std::process::exit(1);
            }
        };
        tracing::info!(addr = %http_addr, "HTTP server listening");

        // If TLS is enabled and UI is being served, use HTTPS via axum-server.
        // This enables browsers to connect to https://localhost:<port> without
        // mixed-content or CORS issues (same-origin).
        if http_config.tls.enabled && http_config.ui.enabled {
            let tls_config_clone = http_config.clone();
            tokio::spawn(async move {
                let (cert_path, key_path) = match tls::ensure_certs() {
                    Ok(paths) => paths,
                    Err(e) => {
                        tracing::error!("TLS cert generation failed: {e}");
                        return;
                    }
                };
                let rustls_cfg = match tls::build_rustls_config(cert_path, key_path) {
                    Ok(cfg) => cfg,
                    Err(e) => {
                        tracing::error!("TLS config failed: {e}");
                        return;
                    }
                };

                // Build the same router but via a fresh state (can't move listener across TLS boundary)
                let metrics = if tls_config_clone.metrics.enabled {
                    Some(ForgeMetrics::new())
                } else {
                    None
                };
                let state = AppState {
                    db_path: http_db,
                    events: http_events,
                    hlc: http_hlc,
                    started_at,
                    write_tx: http_write_tx,
                    admin_emails: tls_config_clone.auth.admin_emails.clone(),
                    viewer_emails: tls_config_clone.auth.viewer_emails.clone(),
                    auth_enabled: tls_config_clone.auth.enabled,
                    metrics,
                    rate_limiter: Some(forge_daemon::server::rate_limit::RateLimiter::new(forge_daemon::server::rate_limit::RateLimitConfig::default())),
                };
                let app = build_router(&tls_config_clone, state);

                let rustls_config = axum_server::tls_rustls::RustlsConfig::from_config(rustls_cfg);
                let addr: std::net::SocketAddr = http_addr.parse().unwrap();
                tracing::info!(%addr, "HTTPS server starting (TLS enabled)");
                if let Err(e) = axum_server::bind_rustls(addr, rustls_config)
                    .serve(app.into_make_service_with_connect_info::<std::net::SocketAddr>())
                    .await
                {
                    tracing::error!("HTTPS server failed: {e}");
                }
            });
        } else {
            tokio::spawn(async move {
                if let Err(e) = run_http_server_with_listener(
                    &http_config,
                    http_db,
                    http_events,
                    http_hlc,
                    started_at,
                    http_write_tx,
                    http_shutdown_rx,
                    http_listener,
                )
                .await
                {
                    tracing::error!("HTTP server failed: {e}");
                }
            });
        }
    }

    // Conditionally spawn gRPC server alongside Unix socket when enabled.
    // gRPC bind failure is fatal — if the operator explicitly enabled gRPC, we must serve it.
    if config.grpc.enabled {
        let grpc_db = db_path.clone();
        let grpc_events = events.clone();
        let grpc_hlc = Arc::clone(&hlc);
        let grpc_write_tx = write_tx.clone();
        let grpc_shutdown_rx = shutdown_tx.subscribe();
        // Pre-bind the listener synchronously so bind failures are caught before we proceed
        let grpc_addr = format!("{}:{}", config.grpc.bind, config.grpc.port);
        let grpc_listener = match tokio::net::TcpListener::bind(&grpc_addr).await {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(addr = %grpc_addr, "failed to bind gRPC server: {e}");
                std::process::exit(1);
            }
        };
        tracing::info!(addr = %grpc_addr, "gRPC server listening");
        tokio::spawn(async move {
            if let Err(e) = run_grpc_server(
                grpc_db,
                grpc_events,
                grpc_hlc,
                started_at,
                grpc_write_tx,
                grpc_shutdown_rx,
                grpc_listener,
            )
            .await
            {
                tracing::error!("gRPC server failed: {e}");
            }
        });
    }

    tracing::info!(pid = pid, socket = %socket_path, db = %db_path, "forge-daemon starting");

    // Run the server IMMEDIATELY (no waiting for consolidation).
    // Socket handler opens per-connection read-only SQLite connections for reads
    // and sends writes through the writer actor via mpsc channel.
    if let Err(e) = run_server(
        &socket_path,
        db_path,
        events,
        hlc,
        started_at,
        write_tx,
        shutdown_tx,
    ).await {
        tracing::error!("server failed: {e}");
    }

    // M6: Graceful cleanup after server stops (both success and error paths)
    if let Err(e) = std::fs::remove_file(&socket_path) {
        tracing::warn!("failed to remove socket file: {e}");
    }
    if let Err(e) = std::fs::remove_file(&pid_path) {
        tracing::warn!("failed to remove PID file: {e}");
    }
    tracing::info!("daemon stopped");
    // _pid_file_guard drops here, releasing the advisory lock
}
