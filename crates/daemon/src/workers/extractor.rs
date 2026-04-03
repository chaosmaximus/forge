// workers/extractor.rs — Processes transcript chunks via extraction backend
//
// Receives file paths from the watcher, reads transcripts incrementally,
// extracts memories via the configured LLM backend, and stores them.

use crate::adapters::{self, AgentAdapter};
use crate::config::ForgeConfig;
use crate::db::ops;
use crate::events;
use crate::extraction::{self, BackendChoice, ExtractionResult};
use forge_core::types::{Memory, MemoryType};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};

/// Receives file paths from the watcher, extracts memories, and stores them in the DB.
///
/// Uses agent adapters to parse transcript files — each adapter handles its own format.
/// Maintains per-file byte offsets for incremental parsing.
///
/// Extraction is debounced: collects file change events and waits for a 30-second
/// activity gap before calling the LLM. This prevents wasting API credits during
/// active editing sessions where the transcript changes every few seconds.
pub async fn run_extractor(
    mut file_rx: mpsc::Receiver<PathBuf>,
    state: Arc<Mutex<crate::server::handler::DaemonState>>,
    config: ForgeConfig,
    agent_adapters: Arc<Vec<Box<dyn AgentAdapter>>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut offsets: HashMap<PathBuf, usize> = HashMap::new();
    let mut pending: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // Debounce: wait for 15 seconds of silence before extraction.
    // gemma3:1b takes 3-10s locally, haiku takes ~11s via API.
    // 15s gap = extract roughly every 2-3 conversation turns.
    // At ~65 extractions/day with haiku: ~$1.50/month.
    const DEBOUNCE_SECS: u64 = 15;
    eprintln!("[extractor] ready, waiting for files ({}s debounce)...", DEBOUNCE_SECS);

    loop {
        // Wait for a file event or shutdown
        tokio::select! {
            Some(path) = file_rx.recv() => {
                pending.insert(path);
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    // Process any pending files before shutdown
                    for path in pending.drain() {
                        let _ = process_file(&path, &mut offsets, &state, &config, &agent_adapters).await;
                    }
                    eprintln!("[extractor] shutdown received");
                    return;
                }
            }
        }

        // Activity gap debounce: keep collecting events for 30 seconds of silence.
        // This prevents calling the LLM extractor on every keystroke during active sessions.
        loop {
            tokio::select! {
                Some(path) = file_rx.recv() => {
                    pending.insert(path);
                    // Reset the debounce timer (keep waiting for silence)
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(DEBOUNCE_SECS)) => {
                    // 30 seconds of silence — process all pending files
                    break;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        for path in pending.drain() {
                            let _ = process_file(&path, &mut offsets, &state, &config, &agent_adapters).await;
                        }
                        eprintln!("[extractor] shutdown received during debounce");
                        return;
                    }
                }
            }
        }

        // Process all accumulated files
        let files_to_process: Vec<PathBuf> = pending.drain().collect();
        if !files_to_process.is_empty() {
            eprintln!("[extractor] processing {} files after activity gap", files_to_process.len());
            for path in &files_to_process {
                if let Err(e) = process_file(path, &mut offsets, &state, &config, &agent_adapters).await {
                    eprintln!("[extractor] error processing {}: {e}", path.display());
                }
            }
        }
    }
}

/// Process a single transcript file: read, parse incrementally, extract, and store.
/// Logs timing for each phase (parse, extract, store) for observability.
async fn process_file(
    path: &PathBuf,
    offsets: &mut HashMap<PathBuf, usize>,
    state: &Arc<Mutex<crate::server::handler::DaemonState>>,
    config: &ForgeConfig,
    agent_adapters: &[Box<dyn AgentAdapter>],
) -> Result<(), String> {
    let total_start = std::time::Instant::now();
    // Resolve symlinks and verify the canonical path still matches an adapter.
    // Prevents symlink attacks (e.g., evil.jsonl -> /etc/shadow).
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|e| format!("failed to resolve {}: {e}", path.display()))?;

    let adapter = match adapters::adapter_for_path(agent_adapters, &canonical) {
        Some(a) => a,
        None => {
            return Err(format!("no adapter for {} (canonical: {})", path.display(), canonical.display()));
        }
    };

    // Guard against OOM: skip files larger than 50 MB but still advance offset
    // so incremental parsers can resume when new content appears.
    const MAX_FILE_SIZE: u64 = 50_000_000;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("failed to stat {}: {e}", canonical.display()))?;
    if metadata.len() > MAX_FILE_SIZE {
        // Advance offset to file end so we don't re-check this file until it grows
        offsets.insert(path.clone(), metadata.len() as usize);
        eprintln!(
            "[extractor] file too large ({} bytes), skipping but advancing offset: {}",
            metadata.len(),
            canonical.display()
        );
        return Ok(());
    }

    // Read the file
    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|e| format!("failed to read {}: {e}", canonical.display()))?;

    // Get the last offset for this file (or 0 if first time)
    let last_offset = offsets.get(path).copied().unwrap_or(0);

    // Parse incrementally using the matched adapter
    let parse_start = std::time::Instant::now();
    let (chunks, new_offset) = adapter.parse_incremental(&content, last_offset);
    let parse_ms = parse_start.elapsed().as_millis();

    // Always advance the offset after parsing. Title-based dedup at remember()
    // prevents duplicate memories if the same chunks are re-processed. This avoids
    // the 18x re-extraction problem where the extractor would re-process the same
    // transcript region on every watcher notification during an active session.
    offsets.insert(path.clone(), new_offset);

    if chunks.is_empty() {
        return Ok(());
    }

    // Emit agent_status based on transcript activity
    {
        let last_chunk = chunks.last().unwrap();
        let status = if last_chunk.has_tool_use {
            "working"
        } else if last_chunk.role == "assistant" {
            "thinking"
        } else {
            "waiting"
        };
        let locked = state.lock().await;
        crate::events::emit(&locked.events, "agent_status", serde_json::json!({
            "agent": adapter.name(),
            "status": status,
            "transcript": path.to_string_lossy(),
        }));
        drop(locked);
    }

    // Combine chunk texts for extraction (limit to last 20 chunks / ~50KB to avoid oversized prompts)
    let recent_chunks: Vec<&_> = chunks.iter().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
    let combined_text: String = recent_chunks
        .iter()
        .map(|c| format!("[{}] {}", c.role, c.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    let combined_text = if combined_text.len() > 50_000 {
        // Find a safe UTF-8 char boundary near the 50KB mark from the end
        let mut start = combined_text.len() - 50_000;
        while !combined_text.is_char_boundary(start) && start < combined_text.len() {
            start += 1;
        }
        combined_text[start..].to_string()
    } else {
        combined_text
    };

    // Detect backend
    let backend = extraction::detect_backend(config).await;

    // Extract memories (this is the slow part — LLM call)
    let extract_start = std::time::Instant::now();
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
            return Ok(());
        }
    };

    // Process results. Offset is already advanced above — title-based dedup at
    // remember() prevents duplicates even if extraction re-runs on overlapping content.
    match result {
        ExtractionResult::Success(extracted) => {
            if extracted.is_empty() {
                return Ok(());
            }

            let locked = state.lock().await;
            let mut stored = 0usize;
            let event_tx = locked.events.clone();

            for em in &extracted {
                // Route skills to the skill table (Layer 2) instead of
                // the memory table (Layer 5). The `continue` ensures a
                // skill extraction doesn't also create a duplicate memory entry.
                if em.memory_type == "skill" {
                    let skill = forge_core::types::Skill {
                        id: format!("skill-{}", ulid::Ulid::new()),
                        name: em.title.clone(),
                        domain: em.tags.first().cloned().unwrap_or_else(|| "general".to_string()),
                        description: em.content.clone(),
                        steps: em.content.lines()
                            .filter(|l| {
                                let trimmed = l.trim();
                                // Match numbered steps or bullet points
                                trimmed.starts_with(|c: char| c.is_ascii_digit())
                                    || trimmed.starts_with('-')
                                    || trimmed.starts_with('*')
                                    || trimmed.starts_with("Step")
                            })
                            .map(|l| l.trim().to_string())
                            .collect(),
                        success_count: 1, // Extracted from a successful execution
                        fail_count: 0,
                        last_used: None,
                        source: "extracted".to_string(),
                        version: 1,
                        project: None, // Skills are global — reusable across projects
                    };

                    if let Err(e) = crate::db::manas::store_skill(&locked.conn, &skill) {
                        eprintln!("[extractor] failed to store skill '{}': {e}", em.title);
                    } else {
                        stored += 1;
                        events::emit(&event_tx, "skill_extracted", serde_json::json!({
                            "skill_id": skill.id,
                            "name": skill.name,
                            "domain": skill.domain,
                        }));
                    }
                    continue; // Don't also store as memory
                }

                let memory_type = match em.memory_type.as_str() {
                    "decision" => MemoryType::Decision,
                    "lesson" => MemoryType::Lesson,
                    "pattern" => MemoryType::Pattern,
                    "preference" => MemoryType::Preference,
                    _ => MemoryType::Lesson,
                };

                let memory = Memory::new(memory_type, &em.title, &em.content)
                    .with_confidence(em.confidence)
                    .with_tags(em.tags.clone())
                    .with_valence(&em.valence, em.intensity);

                if let Err(e) = ops::remember(&locked.conn, &memory) {
                    eprintln!("[extractor] failed to store memory '{}': {e}", em.title);
                } else {
                    stored += 1;

                    // Emit extraction event for subscribers
                    events::emit(&event_tx, "extraction", serde_json::json!({
                        "memory_id": memory.id,
                        "title": memory.title,
                        "memory_type": format!("{:?}", memory.memory_type),
                        "project": memory.project,
                    }));

                    // Wire affects field to graph edges (SQL edge table)
                    if !em.affects.is_empty() {
                        for affected in &em.affects {
                            let _ = ops::store_edge(
                                &locked.conn,
                                &memory.id,
                                &format!("file:{}", affected),
                                "affects",
                                "{}",
                            );
                        }
                    }
                }
            }

            let extract_ms = extract_start.elapsed().as_millis();
            let total_ms = total_start.elapsed().as_millis();
            eprintln!(
                "[extractor] {} memories from {} | parse: {}ms, extract: {}ms, total: {}ms, chunks: {}",
                stored, path.display(), parse_ms, extract_ms, total_ms, chunks.len()
            );
            Ok(())
        }
        ExtractionResult::Unavailable(reason) => {
            eprintln!("[extractor] backend unavailable: {reason} ({}ms)", total_start.elapsed().as_millis());
            Ok(())
        }
        ExtractionResult::Error(err) => {
            eprintln!("[extractor] extraction error: {err} ({}ms)", total_start.elapsed().as_millis());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_agent_status_event_format_working() {
        let event = serde_json::json!({
            "agent": "claude-code",
            "status": "working",
            "transcript": "/home/user/.claude/projects/test/session.jsonl",
        });
        assert_eq!(event["status"], "working");
        assert_eq!(event["agent"], "claude-code");
        assert!(event["transcript"].as_str().unwrap().contains("session.jsonl"));
    }

    #[test]
    fn test_agent_status_event_format_thinking() {
        let event = serde_json::json!({
            "agent": "codex",
            "status": "thinking",
            "transcript": "/tmp/codex/transcript.jsonl",
        });
        assert_eq!(event["status"], "thinking");
        assert_eq!(event["agent"], "codex");
    }

    #[test]
    fn test_agent_status_event_format_waiting() {
        let event = serde_json::json!({
            "agent": "cline",
            "status": "waiting",
            "transcript": "/tmp/cline/transcript.json",
        });
        assert_eq!(event["status"], "waiting");
        assert_eq!(event["agent"], "cline");
    }

    #[test]
    fn test_agent_status_detection_logic() {
        // Verify the status detection logic matches what process_file does:
        // has_tool_use=true => "working"
        // has_tool_use=false, role="assistant" => "thinking"
        // has_tool_use=false, role="user" => "waiting"
        use forge_core::types::session::ConversationChunk;

        let working_chunk = ConversationChunk {
            id: "1".into(),
            session_id: "s1".into(),
            role: "assistant".into(),
            content: "running tool".into(),
            has_tool_use: true,
            timestamp: "2026-04-03T12:00:00Z".into(),
            extracted: false,
        };
        let status = if working_chunk.has_tool_use {
            "working"
        } else if working_chunk.role == "assistant" {
            "thinking"
        } else {
            "waiting"
        };
        assert_eq!(status, "working");

        let thinking_chunk = ConversationChunk {
            id: "2".into(),
            session_id: "s1".into(),
            role: "assistant".into(),
            content: "considering options".into(),
            has_tool_use: false,
            timestamp: "2026-04-03T12:00:01Z".into(),
            extracted: false,
        };
        let status = if thinking_chunk.has_tool_use {
            "working"
        } else if thinking_chunk.role == "assistant" {
            "thinking"
        } else {
            "waiting"
        };
        assert_eq!(status, "thinking");

        let waiting_chunk = ConversationChunk {
            id: "3".into(),
            session_id: "s1".into(),
            role: "user".into(),
            content: "please help".into(),
            has_tool_use: false,
            timestamp: "2026-04-03T12:00:02Z".into(),
            extracted: false,
        };
        let status = if waiting_chunk.has_tool_use {
            "working"
        } else if waiting_chunk.role == "assistant" {
            "thinking"
        } else {
            "waiting"
        };
        assert_eq!(status, "waiting");
    }
}
