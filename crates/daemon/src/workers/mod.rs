// workers/ — Background worker tasks for the daemon
//
// Workers form the extraction pipeline:
//   watcher     → detects new/modified transcript files (multi-agent)
//   extractor   → parses transcripts via adapters and extracts memories via LLM
//   embedder    → generates vector embeddings for unembedded memories
//   consolidator → periodic dedup, linking, decay
//   indexer     → code index via LSP language servers
//   perception  → monitors environment (git status) and creates ephemeral perceptions
//   disposition → analyzes session history and updates agent disposition traits

pub mod consolidator;
pub mod diagnostics;
pub mod disposition;
pub mod embedder;
pub mod extractor;
pub mod indexer;
pub mod perception;
pub mod reaper;
pub mod watcher;

use crate::adapters;
use crate::config::ForgeConfig;
use crate::server::handler::DaemonState;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

/// Open a read-only SQLite connection for worker use.
/// Workers use this for SELECT queries to avoid contending with the write mutex.
/// Returns None for :memory: databases (tests) or on open failure — caller falls back to state lock.
pub fn open_read_conn(db_path: &str) -> Option<rusqlite::Connection> {
    if db_path == ":memory:" {
        return None; // in-memory DBs can't share across connections
    }
    crate::db::vec::init_sqlite_vec();
    match rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(conn) => {
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
                .ok();
            Some(conn)
        }
        Err(e) => {
            eprintln!("[worker] WARN: failed to open read-only connection: {e} — falling back to state lock");
            None
        }
    }
}

/// Spawn all background workers. Returns join handles for graceful shutdown.
///
/// Detects installed agent adapters and configures the watcher + extractor
/// to handle transcripts from Claude Code, Cline, Codex CLI, etc.
pub fn spawn_workers(
    state: Arc<Mutex<DaemonState>>,
    config: ForgeConfig,
    shutdown_tx: &watch::Sender<bool>,
    db_path: String,
    events: crate::events::EventSender,
    writer_tx: Option<mpsc::Sender<crate::server::WriteCommand>>,
) -> Vec<tokio::task::JoinHandle<()>> {
    // Detect installed agent adapters
    let detected = adapters::detect_adapters();
    let adapter_names: Vec<&str> = detected.iter().map(|a| a.name()).collect();
    eprintln!("[workers] detected adapters: {adapter_names:?}");

    // Build watch configs from adapters
    let watch_configs: Vec<watcher::WatchConfig> = detected
        .iter()
        .flat_map(|a| {
            let ext = a.file_extension().to_string();
            a.watch_dirs()
                .into_iter()
                .map(move |dir| (dir, ext.clone()))
        })
        .collect();

    let agent_adapters = Arc::new(detected);

    let (file_tx, file_rx) = mpsc::channel::<std::path::PathBuf>(100);

    // Plumb a clone of `file_tx` into DaemonState so `Request::ForceExtract`
    // can enqueue files directly into the extractor instead of only counting
    // them. Brief lock; held only for the assignment.
    {
        let state_for_tx = Arc::clone(&state);
        let file_tx_clone = file_tx.clone();
        tokio::spawn(async move {
            let mut locked = state_for_tx.lock().await;
            locked.extractor_tx = Some(file_tx_clone);
        });
    }

    let watcher_shutdown = shutdown_tx.subscribe();
    let extractor_shutdown = shutdown_tx.subscribe();
    let embedder_shutdown = shutdown_tx.subscribe();
    let consolidator_shutdown = shutdown_tx.subscribe();
    let indexer_shutdown = shutdown_tx.subscribe();
    let perception_shutdown = shutdown_tx.subscribe();
    let disposition_shutdown = shutdown_tx.subscribe();
    let diagnostics_shutdown = shutdown_tx.subscribe();

    let extractor_state = Arc::clone(&state);
    let embedder_state = Arc::clone(&state);
    let consolidator_state = Arc::clone(&state);
    let indexer_state = Arc::clone(&state);
    let perception_state = Arc::clone(&state);
    let disposition_state = Arc::clone(&state);
    let diagnostics_state = Arc::clone(&state);

    let extractor_config = config.clone();
    let embedder_config = config.clone();
    let worker_intervals = config.workers.clone();

    // Clone db_path for each worker that uses read-only connections
    let extractor_db_path = db_path.clone();
    let embedder_db_path = db_path.clone();
    let disposition_db_path = db_path.clone();
    let reaper_db_path = db_path.clone();
    let diagnostics_db_path = db_path;

    let watcher_handle = tokio::spawn(async move {
        watcher::run_watcher(file_tx, watch_configs, watcher_shutdown).await;
    });

    let extractor_debounce = worker_intervals.extraction_debounce_secs;
    // Move the writer handle into the extractor (the only consumer today).
    // If future workers need to emit writer commands, clone here instead.
    let extractor_writer_tx = writer_tx;
    let extractor_handle = tokio::spawn(async move {
        extractor::run_extractor(
            file_rx,
            extractor_state,
            extractor_config,
            agent_adapters,
            extractor_shutdown,
            extractor_db_path,
            extractor_debounce,
            extractor_writer_tx,
        )
        .await;
    });

    let embedder_interval = worker_intervals.embedding_interval_secs;
    let embedder_handle = tokio::spawn(async move {
        embedder::run_embedder(
            embedder_state,
            embedder_config,
            embedder_shutdown,
            embedder_db_path,
            embedder_interval,
        )
        .await;
    });

    let consolidator_interval = worker_intervals.consolidation_interval_secs;
    let consolidator_handle = tokio::spawn(async move {
        consolidator::run_consolidator(
            consolidator_state,
            consolidator_shutdown,
            consolidator_interval,
        )
        .await;
    });

    let indexer_interval = worker_intervals.indexer_interval_secs;
    let indexer_handle = tokio::spawn(async move {
        indexer::run_indexer(indexer_state, indexer_shutdown, indexer_interval).await;
    });

    let perception_interval = worker_intervals.perception_interval_secs;
    let perception_handle = tokio::spawn(async move {
        perception::run_perception(perception_state, perception_shutdown, perception_interval)
            .await;
    });

    let disposition_interval = worker_intervals.disposition_interval_secs;
    let disposition_handle = tokio::spawn(async move {
        disposition::run_disposition(
            disposition_state,
            disposition_shutdown,
            disposition_db_path,
            disposition_interval,
        )
        .await;
    });

    // Diagnostics worker — debounced batch analysis
    let (diag_tx, diag_rx) = mpsc::channel::<String>(100);
    {
        // Store the diagnostics sender in DaemonState so PostEditCheck can use it.
        // Spawn a short-lived task to avoid blocking_lock panic in tokio runtime.
        let state_clone = Arc::clone(&state);
        tokio::spawn(async move {
            let mut locked = state_clone.lock().await;
            locked.diagnostics_tx = Some(diag_tx);
        });
    }
    let diagnostics_debounce = worker_intervals.diagnostics_debounce_secs;
    let diagnostics_handle = tokio::spawn(async move {
        diagnostics::run_diagnostics_worker(
            diagnostics_state,
            diag_rx,
            diagnostics_shutdown,
            diagnostics_db_path,
            diagnostics_debounce,
        )
        .await;
    });

    // Session heartbeat reaper
    let reaper_shutdown = shutdown_tx.subscribe();
    let reaper_db = reaper_db_path;
    let reaper_config = config.clone();
    let reaper_events = events;
    let reaper_handle = tokio::spawn(async move {
        reaper::run_session_reaper(reaper_db, reaper_config, reaper_events, reaper_shutdown).await;
    });

    eprintln!("[workers] spawned: watcher, extractor, embedder, consolidator, indexer, perception, disposition, diagnostics, reaper");

    vec![
        watcher_handle,
        extractor_handle,
        embedder_handle,
        consolidator_handle,
        indexer_handle,
        perception_handle,
        disposition_handle,
        diagnostics_handle,
        reaper_handle,
    ]
}
