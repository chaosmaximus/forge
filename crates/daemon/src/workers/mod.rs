// workers/ — Background worker tasks for the daemon
//
// Three workers form the extraction pipeline:
//   watcher   → detects new/modified .jsonl transcript files
//   extractor → parses transcripts and extracts memories via LLM
//   embedder  → generates vector embeddings for unembedded memories

pub mod consolidator;
pub mod embedder;
pub mod extractor;
pub mod indexer;
pub mod watcher;

use crate::config::ForgeConfig;
use crate::server::handler::DaemonState;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

/// Spawn all background workers. Returns join handles for graceful shutdown.
///
/// Creates:
/// 1. An mpsc channel (capacity 100) for file paths (watcher -> extractor)
/// 2. A watcher task that monitors ~/.claude/projects/ for .jsonl changes
/// 3. An extractor task that processes transcript files and stores memories
/// 4. An embedder task that periodically generates vector embeddings
pub fn spawn_workers(
    state: Arc<Mutex<DaemonState>>,
    config: ForgeConfig,
    shutdown_tx: &watch::Sender<bool>,
) -> Vec<tokio::task::JoinHandle<()>> {
    let (file_tx, file_rx) = mpsc::channel::<std::path::PathBuf>(100);

    let watcher_shutdown = shutdown_tx.subscribe();
    let extractor_shutdown = shutdown_tx.subscribe();
    let embedder_shutdown = shutdown_tx.subscribe();
    let consolidator_shutdown = shutdown_tx.subscribe();

    let extractor_state = Arc::clone(&state);
    let embedder_state = Arc::clone(&state);
    let consolidator_state = Arc::clone(&state);

    let indexer_shutdown = shutdown_tx.subscribe();
    let indexer_state = Arc::clone(&state);

    let extractor_config = config.clone();
    let embedder_config = config;

    let watcher_handle = tokio::spawn(async move {
        watcher::run_watcher(file_tx, watcher_shutdown).await;
    });

    let extractor_handle = tokio::spawn(async move {
        extractor::run_extractor(file_rx, extractor_state, extractor_config, extractor_shutdown)
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

    eprintln!("[workers] spawned: watcher, extractor, embedder, consolidator, indexer");

    vec![watcher_handle, extractor_handle, embedder_handle, consolidator_handle, indexer_handle]
}
