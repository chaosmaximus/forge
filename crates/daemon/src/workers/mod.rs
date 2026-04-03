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
pub mod disposition;
pub mod embedder;
pub mod extractor;
pub mod indexer;
pub mod perception;
pub mod watcher;

use crate::adapters;
use crate::config::ForgeConfig;
use crate::server::handler::DaemonState;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

/// Spawn all background workers. Returns join handles for graceful shutdown.
///
/// Detects installed agent adapters and configures the watcher + extractor
/// to handle transcripts from Claude Code, Cline, Codex CLI, etc.
pub fn spawn_workers(
    state: Arc<Mutex<DaemonState>>,
    config: ForgeConfig,
    shutdown_tx: &watch::Sender<bool>,
) -> Vec<tokio::task::JoinHandle<()>> {
    // Detect installed agent adapters
    let detected = adapters::detect_adapters();
    let adapter_names: Vec<&str> = detected.iter().map(|a| a.name()).collect();
    eprintln!("[workers] detected adapters: {:?}", adapter_names);

    // Build watch configs from adapters
    let watch_configs: Vec<watcher::WatchConfig> = detected
        .iter()
        .flat_map(|a| {
            let ext = a.file_extension().to_string();
            a.watch_dirs().into_iter().map(move |dir| (dir, ext.clone()))
        })
        .collect();

    let agent_adapters = Arc::new(detected);

    let (file_tx, file_rx) = mpsc::channel::<std::path::PathBuf>(100);

    let watcher_shutdown = shutdown_tx.subscribe();
    let extractor_shutdown = shutdown_tx.subscribe();
    let embedder_shutdown = shutdown_tx.subscribe();
    let consolidator_shutdown = shutdown_tx.subscribe();
    let indexer_shutdown = shutdown_tx.subscribe();
    let perception_shutdown = shutdown_tx.subscribe();
    let disposition_shutdown = shutdown_tx.subscribe();

    let extractor_state = Arc::clone(&state);
    let embedder_state = Arc::clone(&state);
    let consolidator_state = Arc::clone(&state);
    let indexer_state = Arc::clone(&state);
    let perception_state = Arc::clone(&state);
    let disposition_state = Arc::clone(&state);

    let extractor_config = config.clone();
    let embedder_config = config;

    let watcher_handle = tokio::spawn(async move {
        watcher::run_watcher(file_tx, watch_configs, watcher_shutdown).await;
    });

    let extractor_handle = tokio::spawn(async move {
        extractor::run_extractor(
            file_rx,
            extractor_state,
            extractor_config,
            agent_adapters,
            extractor_shutdown,
        )
        .await;
    });

    let embedder_handle = tokio::spawn(async move {
        embedder::run_embedder(embedder_state, embedder_config, embedder_shutdown).await;
    });

    let consolidator_handle = tokio::spawn(async move {
        consolidator::run_consolidator(consolidator_state, consolidator_shutdown).await;
    });

    let indexer_handle = tokio::spawn(async move {
        indexer::run_indexer(indexer_state, indexer_shutdown).await;
    });

    let perception_handle = tokio::spawn(async move {
        perception::run_perception(perception_state, perception_shutdown).await;
    });

    let disposition_handle = tokio::spawn(async move {
        disposition::run_disposition(disposition_state, disposition_shutdown).await;
    });

    eprintln!("[workers] spawned: watcher, extractor, embedder, consolidator, indexer, perception, disposition");

    vec![
        watcher_handle,
        extractor_handle,
        embedder_handle,
        consolidator_handle,
        indexer_handle,
        perception_handle,
        disposition_handle,
    ]
}
