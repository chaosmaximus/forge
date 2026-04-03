// workers/extractor.rs — Processes transcript chunks via extraction backend
//
// Receives file paths from the watcher, reads transcripts incrementally,
// extracts memories via the configured LLM backend, and stores them.

use crate::chunk::parse_transcript_incremental;
use crate::config::ForgeConfig;
use crate::db::ops;
use crate::extraction::{self, BackendChoice, ExtractionResult};
use forge_core::types::{Memory, MemoryType};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

/// Receives file paths from the watcher, extracts memories, and stores them in the DB.
///
/// Maintains per-file byte offsets for incremental parsing so each file is only
/// processed from where it left off.
pub async fn run_extractor(
    mut file_rx: mpsc::Receiver<PathBuf>,
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    config: ForgeConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut offsets: HashMap<PathBuf, usize> = HashMap::new();

    eprintln!("[extractor] ready, waiting for files...");

    loop {
        tokio::select! {
            Some(path) = file_rx.recv() => {
                if let Err(e) = process_file(&path, &mut offsets, &state, &config).await {
                    eprintln!("[extractor] error processing {}: {e}", path.display());
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    eprintln!("[extractor] shutdown received");
                    return;
                }
            }
        }
    }
}

/// Process a single transcript file: read, parse incrementally, extract, and store.
async fn process_file(
    path: &PathBuf,
    offsets: &mut HashMap<PathBuf, usize>,
    state: &Arc<Mutex<crate::server::handler::DaemonState>>,
    config: &ForgeConfig,
) -> Result<(), String> {
    // Read the file
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;

    // Get the last offset for this file (or 0 if first time)
    let last_offset = offsets.get(path).copied().unwrap_or(0);

    // Parse incrementally
    let (chunks, new_offset) = parse_transcript_incremental(&content, last_offset);

    if chunks.is_empty() {
        // No new complete lines — still safe to update offset to new_offset
        // because parse_transcript_incremental only advances to the last complete newline.
        offsets.insert(path.clone(), new_offset);
        return Ok(());
    }

    // Combine chunk texts for extraction (limit to last 20 chunks / ~50KB to avoid oversized prompts)
    let recent_chunks: Vec<&_> = chunks.iter().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
    let combined_text: String = recent_chunks
        .iter()
        .map(|c| format!("[{}] {}", c.role, c.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    let combined_text = if combined_text.len() > 50_000 {
        combined_text[combined_text.len() - 50_000..].to_string()
    } else {
        combined_text
    };

    // Detect backend
    let backend = extraction::detect_backend(config).await;

    // Extract memories
    let result = match &backend {
        BackendChoice::ClaudeCli => {
            extraction::claude_cli::extract(&config.extraction.claude.model, &combined_text).await
        }
        BackendChoice::Ollama => {
            extraction::ollama::extract(
                &config.extraction.ollama.endpoint,
                &config.extraction.ollama.model,
                &combined_text,
            )
            .await
        }
        BackendChoice::None(reason) => {
            eprintln!("[extractor] no backend available: {reason}");
            // Don't update offset — chunks will be re-attempted next time
            return Ok(());
        }
    };

    // Process results — only advance offset on successful extraction
    match result {
        ExtractionResult::Success(extracted) => {
            if extracted.is_empty() {
                // Nothing extracted — safe to advance offset (extraction ran successfully)
                offsets.insert(path.clone(), new_offset);
                return Ok(());
            }

            let locked = state.lock().await;
            let mut stored = 0usize;
            let mut failed = 0usize;

            for em in &extracted {
                let memory_type = match em.memory_type.as_str() {
                    "decision" => MemoryType::Decision,
                    "lesson" => MemoryType::Lesson,
                    "pattern" => MemoryType::Pattern,
                    "preference" => MemoryType::Preference,
                    _ => MemoryType::Lesson,
                };

                let memory = Memory::new(memory_type, &em.title, &em.content)
                    .with_confidence(em.confidence)
                    .with_tags(em.tags.clone());

                if let Err(e) = ops::remember(&locked.conn, &memory) {
                    eprintln!("[extractor] failed to store memory '{}': {e}", em.title);
                    failed += 1;
                } else {
                    stored += 1;
                }
            }

            // Only advance offset if ALL memories were stored successfully.
            // If any failed, keep old offset so chunks are re-extracted next time.
            if failed == 0 {
                offsets.insert(path.clone(), new_offset);
            } else {
                eprintln!("[extractor] {} store failures — offset NOT advanced, will retry", failed);
            }

            eprintln!(
                "[extractor] extracted {} memories from {}",
                stored,
                path.display()
            );
            Ok(())
        }
        ExtractionResult::Unavailable(reason) => {
            // Don't update offset — chunks will be re-attempted next time
            eprintln!("[extractor] backend unavailable: {reason}");
            Ok(())
        }
        ExtractionResult::Error(err) => {
            // Don't update offset — chunks will be re-attempted next time
            eprintln!("[extractor] extraction error: {err}");
            Ok(())
        }
    }
}
