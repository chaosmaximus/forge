use forge_daemon::server::{DaemonState, WriterActor, run_http_server_with_listener, run_server};
use forge_core::{forge_dir, default_socket_path, default_db_path, default_pid_path};
use fs2::FileExt;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Initialize structured JSON logging to stderr BEFORE anything else.
    // stdout is reserved for protocol output (NDJSON).
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("forge_daemon=info")),
        )
        .json()
        .with_target(true)
        .with_writer(std::io::stderr)
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

    // C2: Write PID file with advisory lock to prevent multiple daemon instances
    let pid_path = default_pid_path();
    let pid_file = match std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&pid_path)
    {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("failed to open PID file {pid_path}: {e}");
            std::process::exit(1);
        }
    };

    if pid_file.try_lock_exclusive().is_err() {
        tracing::error!("another forge-daemon is already running (PID file locked)");
        std::process::exit(1);
    }

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

    // Extract shared resources BEFORE wrapping in Arc<Mutex>.
    // These are shared between the socket handler (read path), writer actor,
    // and workers so they all see the same events and HLC.
    let events = worker_state.events.clone();
    let hlc = Arc::clone(&worker_state.hlc);
    let started_at = worker_state.started_at;

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
