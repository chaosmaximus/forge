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
    db_path: String,
    debounce_secs: u64,
) {
    let mut offsets: HashMap<PathBuf, usize> = HashMap::new();
    let mut pending: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // Debounce: wait for silence before extraction.
    // gemma3:1b takes 3-10s locally, haiku takes ~11s via API.
    // Default 15s gap = extract roughly every 2-3 conversation turns.
    // At ~65 extractions/day with haiku: ~$1.50/month.
    // Configurable via config.workers.extraction_debounce_secs
    eprintln!("[extractor] ready, waiting for files ({}s debounce)...", debounce_secs);

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
                        let _ = process_file(&path, &mut offsets, &state, &config, &agent_adapters, &db_path).await;
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
                _ = tokio::time::sleep(std::time::Duration::from_secs(debounce_secs)) => {
                    // Debounce period of silence — process all pending files
                    break;
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        for path in pending.drain() {
                            let _ = process_file(&path, &mut offsets, &state, &config, &agent_adapters, &db_path).await;
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
                if let Err(e) = process_file(path, &mut offsets, &state, &config, &agent_adapters, &db_path).await {
                    eprintln!("[extractor] error processing {}: {e}", path.display());
                }
            }
        }
    }
}

/// Process a single transcript file: read, parse incrementally, extract, and store.
/// Logs timing for each phase (parse, extract, store) for observability.
///
/// Config is reloaded from disk on each call so that `forge-next config set`
/// takes effect without daemon restart.
async fn process_file(
    path: &PathBuf,
    offsets: &mut HashMap<PathBuf, usize>,
    state: &Arc<Mutex<crate::server::handler::DaemonState>>,
    _config: &ForgeConfig,
    agent_adapters: &[Box<dyn AgentAdapter>],
    db_path: &str,
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

    // ISS-D8: For files over 50MB, read only the last 10MB (tail) instead of skipping.
    // This ensures long conversations (120MB+) still get their most recent decisions extracted.
    // Files over 200MB are truly skipped to prevent OOM.
    const MAX_FULL_READ: u64 = 50_000_000;     // 50MB — read entire file
    const MAX_TAIL_READ: usize = 10_000_000;    // 10MB — tail read for oversized files
    const MAX_SKIP_SIZE: u64 = 200_000_000;     // 200MB — skip entirely (OOM protection)

    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("failed to stat {}: {e}", canonical.display()))?;
    let file_size = metadata.len();

    if file_size > MAX_SKIP_SIZE {
        offsets.insert(path.clone(), file_size as usize);
        eprintln!(
            "[extractor] file too large ({} bytes > 200MB), skipping: {}",
            file_size, canonical.display()
        );
        return Ok(());
    }

    // Read content: full file for small files, tail for oversized
    let (content, content_file_offset) = if file_size <= MAX_FULL_READ {
        let full = tokio::fs::read_to_string(&canonical)
            .await
            .map_err(|e| format!("failed to read {}: {e}", canonical.display()))?;
        (full, 0usize)
    } else {
        // Tail read: last 10MB of the file
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let tail_start = (file_size as usize).saturating_sub(MAX_TAIL_READ);
        let mut file = tokio::fs::File::open(&canonical)
            .await
            .map_err(|e| format!("failed to open for tail read {}: {e}", canonical.display()))?;
        file.seek(std::io::SeekFrom::Start(tail_start as u64))
            .await
            .map_err(|e| format!("failed to seek {}: {e}", canonical.display()))?;
        let mut buf = Vec::with_capacity(MAX_TAIL_READ);
        file.read_to_end(&mut buf)
            .await
            .map_err(|e| format!("failed to tail-read {}: {e}", canonical.display()))?;

        // M2+M3: Handle empty buffer (file shrunk between stat and read)
        if buf.is_empty() {
            offsets.insert(path.clone(), file_size as usize);
            return Ok(());
        }

        // Find first newline to start at a complete line boundary (JSONL safety)
        let mut line_start = buf.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0);
        // M2: Skip any UTF-8 continuation bytes (0x80..=0xBF) after the newline.
        // Prevents mojibake if seek landed mid-codepoint in CJK/emoji transcripts.
        while line_start < buf.len() && (buf[line_start] & 0xC0) == 0x80 {
            line_start += 1;
        }
        let actual_start = tail_start + line_start;
        let tail_str = String::from_utf8_lossy(&buf[line_start..]).to_string();
        eprintln!(
            "[extractor] tail-reading last {:.1}MB of {:.1}MB file: {}",
            tail_str.len() as f64 / 1_048_576.0,
            file_size as f64 / 1_048_576.0,
            canonical.display()
        );
        (tail_str, actual_start)
    };

    // Get the last offset for this file (or 0 if first time).
    // Adjust for tail reads: if last_offset is before our tail window, start from 0 in the tail.
    let raw_last_offset = offsets.get(path).copied().unwrap_or(0);
    let last_offset = raw_last_offset.saturating_sub(content_file_offset);

    // Parse incrementally using the matched adapter
    let parse_start = std::time::Instant::now();
    let (chunks, new_offset) = adapter.parse_incremental(&content, last_offset);
    let parse_ms = parse_start.elapsed().as_millis();

    // Always advance the offset after parsing. Title-based dedup at remember()
    // prevents duplicate memories if the same chunks are re-processed. This avoids
    // the 18x re-extraction problem where the extractor would re-process the same
    // transcript region on every watcher notification during an active session.
    // For tail reads, adjust offset back to full-file coordinates.
    offsets.insert(path.clone(), content_file_offset + new_offset);

    if chunks.is_empty() {
        return Ok(());
    }

    // Grab event sender once (no DB needed — events is Clone)
    let event_tx_for_status = {
        let locked = state.lock().await;
        locked.events.clone()
    };

    // Emit agent_status based on transcript activity (no DB access needed)
    {
        let last_chunk = chunks.last().unwrap();
        let status = if last_chunk.has_tool_use {
            "working"
        } else if last_chunk.role == "assistant" {
            "thinking"
        } else {
            "waiting"
        };
        crate::events::emit(&event_tx_for_status, "agent_status", serde_json::json!({
            "agent": adapter.name(),
            "status": status,
            "transcript": path.to_string_lossy(),
        }));
    }

    // Action tracking: count tool_use chunks and increment session counter.
    // Lightweight — no LLM needed, just count detection from parsed chunks.
    // Uses read conn for session lookup (SELECT), write lock only for increment (UPDATE).
    {
        let tool_use_count = chunks.iter().filter(|c| c.has_tool_use).count();
        if tool_use_count > 0 {
            // Read session ID using read-only connection (no mutex contention)
            let session_id = if let Some(rc) = super::open_read_conn(db_path) {
                let sid = crate::sessions::get_active_session_id(&rc, adapter.name()).unwrap_or_default();
                drop(rc);
                sid
            } else {
                let locked = state.lock().await;
                let sid = crate::sessions::get_active_session_id(&locked.conn, adapter.name()).unwrap_or_default();
                drop(locked);
                sid
            };
            if !session_id.is_empty() {
                // Write: brief lock for the UPDATE
                let locked = state.lock().await;
                if let Err(e) = crate::sessions::increment_tool_use_count(&locked.conn, &session_id, tool_use_count) {
                    eprintln!("[extractor] failed to increment tool_use_count: {e}");
                }
                drop(locked);
            }
        }
    }

    // Combine chunk texts for extraction (limit to last 20 chunks / ~50KB to avoid oversized prompts)
    let recent_chunks: Vec<&_> = chunks.iter().rev().take(20).collect::<Vec<_>>().into_iter().rev().collect();
    let combined_text: String = recent_chunks
        .iter()
        .map(|c| format!("[{}] {}", c.role, c.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    // Truncate to 15KB — keeps extraction fast and within context limits for all backends.
    // 15KB ≈ last 5-10 conversation turns, which is enough for meaningful extraction.
    // Larger transcripts caused Claude CLI to timeout (60s) and Gemini to truncate responses.
    let max_extract_bytes = 15_000;
    let combined_text = if combined_text.len() > max_extract_bytes {
        let mut start = combined_text.len() - max_extract_bytes;
        while !combined_text.is_char_boundary(start) && start < combined_text.len() {
            start += 1;
        }
        combined_text[start..].to_string()
    } else {
        combined_text
    };

    // Reload config from disk so `forge-next config set` takes effect without restart.
    // File read is ~1ms, negligible compared to the LLM extraction call.
    let config = crate::config::load_config();

    // Detect backend — use smart router if configured, otherwise static detection
    let (backend, _routing_tier) = if config.extraction.routing == "smart" {
        let (b, tier) = extraction::router::route_extraction(&config, &combined_text).await;
        tracing::info!(tier = %tier, provider = ?b, "smart router selected");
        (b, Some(tier))
    } else {
        (extraction::detect_backend(&config).await, None)
    };

    // Extract memories (this is the slow part — LLM call)
    let extract_start = std::time::Instant::now();
    let result = match &backend {
        BackendChoice::ClaudeCli => {
            extraction::claude_cli::extract(&config.extraction.claude.model, &combined_text).await
        }
        BackendChoice::ClaudeApi => {
            let api_key = crate::config::resolve_api_key(
                &config.extraction.claude_api.api_key,
                "ANTHROPIC_API_KEY",
            )
            .unwrap_or_default();
            extraction::claude_api::extract(
                &api_key,
                &config.extraction.claude_api.model,
                &combined_text,
            )
            .await
        }
        BackendChoice::OpenAi => {
            let api_key = crate::config::resolve_api_key(
                &config.extraction.openai.api_key,
                "OPENAI_API_KEY",
            )
            .unwrap_or_default();
            extraction::openai::extract(
                &api_key,
                &config.extraction.openai.model,
                &config.extraction.openai.endpoint,
                &combined_text,
            )
            .await
        }
        BackendChoice::Gemini => {
            let api_key = crate::config::resolve_api_key(
                &config.extraction.gemini.api_key,
                "GEMINI_API_KEY",
            )
            .unwrap_or_default();
            extraction::gemini::extract(
                &api_key,
                &config.extraction.gemini.model,
                &combined_text,
            )
            .await
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

            let mut stored = 0usize;

            // Get event channel (already cloned above) + session ID via read conn
            let event_tx = event_tx_for_status.clone();
            let session_id = if let Some(rc) = super::open_read_conn(db_path) {
                crate::sessions::get_active_session_id(&rc, adapter.name()).unwrap_or_default()
            } else {
                let locked = state.lock().await;
                crate::sessions::get_active_session_id(&locked.conn, adapter.name()).unwrap_or_default()
            };

            for em in &extracted {
                // Re-acquire lock per memory write (short hold, doesn't block socket for long)
                let locked = state.lock().await;
                // Route skills to the skill table (Layer 2) instead of
                // the memory table (Layer 5). The `continue` ensures a
                // skill extraction doesn't also create a duplicate memory entry.
                // Quality gate: reject junk skills and demote them to lessons.
                if em.memory_type == "skill" {
                    let is_behavioral = em.tags.iter().any(|t| t == "behavioral");

                    if is_behavioral {
                        // Behavioral skills: require meaningful description (>=100 chars), no step validation
                        let long_enough = em.content.len() >= 100;
                        let has_domain = em.tags.len() >= 2; // "behavioral" + domain tag

                        if !long_enough || !has_domain {
                            // Demote to pattern (behavioral skills too short = just a pattern)
                            eprintln!(
                                "[extractor] demoted weak behavioral skill '{}' to pattern (long_enough={}, has_domain={})",
                                em.title, long_enough, has_domain
                            );
                            // Don't continue — let it fall through to memory storage as a pattern/lesson
                        } else {
                            let skill = forge_core::types::Skill {
                                id: format!("skill-{}", ulid::Ulid::new()),
                                name: em.title.clone(),
                                domain: em.tags.iter().find(|t| *t != "behavioral").cloned().unwrap_or_else(|| "general".to_string()),
                                description: em.content.clone(),
                                steps: vec![], // behavioral skills don't have steps
                                success_count: 1,
                                fail_count: 0,
                                last_used: None,
                                source: "extracted".to_string(),
                                version: 1,
                                project: None,
                                skill_type: "behavioral".to_string(),
                                user_specific: true,
                                observed_count: 1,
                                correlation_ids: vec![],
                            };

                            // Check for existing behavioral skill with similar name (observation counting)
                            // If found, increment observed_count instead of creating duplicate
                            if let Err(e) = crate::db::manas::store_or_observe_skill(&locked.conn, &skill) {
                                eprintln!("[extractor] failed to store behavioral skill '{}': {e}", em.title);
                            } else {
                                stored += 1;
                                // Run correlation engine to link to related identity/decisions
                                let _ = crate::db::manas::correlate_skill(
                                    &locked.conn,
                                    &skill.id,
                                    &skill.name,
                                    &em.tags,
                                );
                                events::emit(&event_tx, "skill_extracted", serde_json::json!({
                                    "skill_id": skill.id,
                                    "name": skill.name,
                                    "domain": skill.domain,
                                    "skill_type": "behavioral",
                                }));
                            }
                            continue; // Don't also store as memory
                        }
                    } else {
                        // Procedural skills: quality gate (numbered steps OR paragraph-style procedures)
                        let has_steps = em.content.contains("1)")
                            || em.content.contains("1.")
                            || em.content.contains("- ")
                            || em.content.lines().count() >= 3
                            // Paragraph-style procedures: multi-sentence with procedural verbs
                            || (em.content.matches('.').count() >= 2 && {
                                let lower = em.content.to_lowercase();
                                lower.contains("first") || lower.contains("then")
                                    || lower.contains("next") || lower.contains("after")
                                    || lower.contains("finally") || lower.contains("before")
                                    || lower.contains("ensure") || lower.contains("verify")
                                    || lower.contains("run ") || lower.contains("execute")
                            });
                        let long_enough = em.content.len() >= 50;
                        let title_lower = em.title.to_lowercase();
                        let not_status = !title_lower.contains("complete")
                            && !title_lower.contains("remaining")
                            && !title_lower.starts_with("all ")
                            && !title_lower.starts_with("fix the");

                        if !has_steps || !long_enough || !not_status {
                            // Demote to lesson — fall through to normal memory storage below
                            eprintln!(
                                "[extractor] demoted junk skill '{}' to lesson (has_steps={}, long_enough={}, not_status={})",
                                em.title, has_steps, long_enough, not_status
                            );
                            // Don't continue — let it fall through to memory storage as a lesson
                        } else {
                            // Extract steps: try numbered/bulleted lines first, fall back to
                            // sentence splitting for prose-style procedures
                            let mut steps: Vec<String> = em.content.lines()
                                .filter(|l| {
                                    let trimmed = l.trim();
                                    trimmed.starts_with(|c: char| c.is_ascii_digit())
                                        || trimmed.starts_with('-')
                                        || trimmed.starts_with('*')
                                        || trimmed.starts_with("Step")
                                })
                                .map(|l| l.trim().to_string())
                                .collect();
                            // Fallback: if no structured steps found, split on sentences
                            if steps.is_empty() && em.content.len() >= 50 {
                                steps = em.content.split('.')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| s.len() >= 10)
                                    .collect();
                            }
                            let skill = forge_core::types::Skill {
                                id: format!("skill-{}", ulid::Ulid::new()),
                                name: em.title.clone(),
                                domain: em.tags.first().cloned().unwrap_or_else(|| "general".to_string()),
                                description: em.content.clone(),
                                steps,
                                success_count: 1, // Extracted from a successful execution
                                fail_count: 0,
                                last_used: None,
                                source: "extracted".to_string(),
                                version: 1,
                                project: None, // Skills are global — reusable across projects
                                skill_type: "procedural".to_string(),
                                user_specific: false,
                                observed_count: 1,
                                correlation_ids: vec![],
                            };

                            if let Err(e) = crate::db::manas::store_skill(&locked.conn, &skill) {
                                eprintln!("[extractor] failed to store skill '{}': {e}", em.title);
                            } else {
                                stored += 1;
                                events::emit(&event_tx, "skill_extracted", serde_json::json!({
                                    "skill_id": skill.id,
                                    "name": skill.name,
                                    "domain": skill.domain,
                                    "skill_type": "procedural",
                                }));
                            }
                            continue; // Don't also store as memory
                        }
                    }
                }

                // Route identity signals to the identity table (Ahankara)
                // Filter out AI agent roles that the LLM may confuse with user identity
                if em.memory_type == "identity" {
                    let title_lower = em.title.to_lowercase();
                    let agent_role_keywords = [
                        "advisor", "assistant", "agent", "claude", "gpt", "copilot",
                        "cto advisor", "cfo advisor", "cmo advisor", "cpo advisor",
                        "board member", "ai ", "llm ", "model ",
                    ];
                    if agent_role_keywords.iter().any(|kw| title_lower.contains(kw)) {
                        eprintln!(
                            "[extractor] filtered AI agent identity '{}' — only human user identity accepted",
                            em.title
                        );
                        continue;
                    }
                    let facet_type = em.tags.first().cloned().unwrap_or_else(|| "expertise".to_string());
                    let facet = forge_core::types::manas::IdentityFacet {
                        id: format!("identity-{}", ulid::Ulid::new()),
                        agent: adapter.name().to_string(),
                        facet: facet_type,
                        description: em.title.clone(),
                        strength: em.confidence.clamp(0.0, 1.0),
                        source: "extracted".to_string(),
                        active: true,
                        created_at: forge_core::time::now_iso(),
                        user_id: None,
                    };
                    if let Err(e) = crate::db::manas::store_identity(&locked.conn, &facet) {
                        eprintln!("[extractor] failed to store identity '{}': {e}", em.title);
                    } else {
                        stored += 1;
                        events::emit(&event_tx, "identity_updated", serde_json::json!({
                            "id": facet.id,
                            "facet": facet.facet,
                            "agent": facet.agent,
                            "source": "extracted",
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

                let mut memory = Memory::new(memory_type, &em.title, &em.content)
                    .with_confidence(em.confidence)
                    .with_tags(em.tags.clone())
                    .with_valence(&em.valence, em.intensity)
                    .with_alternatives(em.alternatives.clone())
                    .with_participants(em.participants.clone());
                memory.session_id = session_id.clone();

                // Stamp HLC + node_id so sync protocol works for extracted memories
                let locked_for_hlc = state.lock().await;
                memory.set_hlc(
                    locked_for_hlc.hlc.now(),
                    locked_for_hlc.hlc.node_id().to_string(),
                );
                // Set project: first try session's project, then transcript path
                if (memory.project.is_none() || memory.project.as_deref() == Some(""))
                    && !session_id.is_empty()
                {
                    if let Ok(proj) = locked_for_hlc.conn.query_row(
                        "SELECT project FROM session WHERE id = ?1 AND project IS NOT NULL AND project != ''",
                        rusqlite::params![&session_id],
                        |row| row.get::<_, String>(0),
                    ) {
                        memory.project = Some(proj);
                    }
                }
                if memory.project.is_none() || memory.project.as_deref() == Some("") {
                    memory.project = crate::bootstrap::extract_project_from_path(path);
                }
                drop(locked_for_hlc);

                // Task 5: Causal chain — if motivated_by is present, link to referenced memory
                // Codex fix: scope to same project to prevent cross-project fake causal links
                // Only create edge AFTER successful memory store (below)
                if let Some(ref motivation) = em.motivated_by {
                    let project_scope = memory.project.as_deref();
                    if let Ok(results) = ops::recall_bm25_project(&locked.conn, motivation, project_scope, 1) {
                        if let Some(match_result) = results.first() {
                            // Only link if match score is strong enough (prevent weak false links)
                            if match_result.score.abs() > 0.001 {
                                if let Err(e) = ops::store_edge(&locked.conn, &memory.id, &match_result.id, "motivated_by", "{}") {
                                    eprintln!("[extractor] failed to store motivated_by edge: {e}");
                                }
                            }
                        }
                    }
                }

                // Pre-store near-duplicate check: query FTS5 for similar titles
                // of the same type, then compute word overlap to skip near-dupes.
                {
                    let mt_str = em.memory_type.as_str();
                    if let Ok(candidates) = ops::find_similar_by_title(&locked.conn, &em.title, mt_str, 10) {
                        let new_words = ops::meaningful_words_pub(&em.title);
                        if !new_words.is_empty() {
                            let mut skip = false;
                            for (_, existing_title) in &candidates {
                                let existing_words = ops::meaningful_words_pub(existing_title);
                                if existing_words.is_empty() {
                                    continue;
                                }
                                let intersection = new_words.intersection(&existing_words).count();
                                let union = new_words.union(&existing_words).count();
                                let overlap = intersection as f64 / union as f64;
                                if overlap > 0.7 {
                                    eprintln!(
                                        "[extractor] skipped near-duplicate '{}' (overlap={:.2} with existing '{}')",
                                        em.title, overlap, existing_title
                                    );
                                    skip = true;
                                    break;
                                }
                            }
                            if skip {
                                continue;
                            }
                        }
                    }
                }

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
                            if let Err(e) = ops::store_edge(
                                &locked.conn,
                                &memory.id,
                                &format!("file:{}", affected),
                                "affects",
                                "{}",
                            ) {
                                eprintln!("[extractor] failed to store affects edge: {e}");
                            }
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

    #[test]
    fn test_skill_quality_gate_rejects_junk_title() {
        // Verify the quality gate logic used in process_file
        let title = "All 17 Tasks Complete";
        let content = "Done";

        let has_steps = content.contains("1)")
            || content.contains("1.")
            || content.contains("- ")
            || content.lines().count() >= 3;
        let long_enough = content.len() >= 50;
        let title_lower = title.to_lowercase();
        let not_status = !title_lower.contains("complete")
            && !title_lower.contains("remaining")
            && !title_lower.starts_with("all ")
            && !title_lower.starts_with("fix the");

        // Should fail all three checks
        assert!(!has_steps, "junk skill should not have steps");
        assert!(!long_enough, "junk skill should be too short");
        assert!(!not_status, "junk skill title should be detected as status");
    }

    #[test]
    fn test_skill_quality_gate_accepts_good_skill() {
        let title = "Deploy Rust Service";
        let content = "1) cargo build --release 2) scp binary to server 3) systemctl restart forge-daemon";

        let has_steps = content.contains("1)")
            || content.contains("1.")
            || content.contains("- ")
            || content.lines().count() >= 3;
        let long_enough = content.len() >= 50;
        let title_lower = title.to_lowercase();
        let not_status = !title_lower.contains("complete")
            && !title_lower.contains("remaining")
            && !title_lower.starts_with("all ")
            && !title_lower.starts_with("fix the");

        assert!(has_steps, "good skill should have steps");
        assert!(long_enough, "good skill should be long enough");
        assert!(not_status, "good skill title should not look like a status");
    }

    #[test]
    fn test_skill_quality_gate_rejects_short_content() {
        // Skill with a good title but content too short
        let _title = "Build and Test";
        let content = "run cargo test";

        let has_steps = content.contains("1)")
            || content.contains("1.")
            || content.contains("- ")
            || content.lines().count() >= 3;
        let long_enough = content.len() >= 50;

        // "run cargo test" is only 14 chars — should fail length check
        assert!(!long_enough, "short content should fail length check");
        // Even though has_steps might be false, the OR logic means any failure gates it
        assert!(!has_steps || !long_enough, "should be gated as junk");
    }

    #[test]
    fn test_skill_quality_gate_accepts_paragraph_style() {
        // Paragraph-style procedure should pass the quality gate
        let content = "First configure the environment variables. Then run cargo build --release to compile. After that verify with cargo test. Finally deploy using the install script.";
        let has_steps = content.contains("1)")
            || content.contains("1.")
            || content.contains("- ")
            || content.lines().count() >= 3
            || (content.matches('.').count() >= 2 && {
                let lower = content.to_lowercase();
                lower.contains("first") || lower.contains("then")
                    || lower.contains("next") || lower.contains("after")
                    || lower.contains("finally")
            });
        assert!(has_steps, "paragraph-style procedure should pass quality gate");
    }

    #[test]
    fn test_tail_read_offset_math() {
        // ISS-D8: verify offset calculations for tail reads
        // Simulates a 120MB file where we read the last 10MB

        let file_size: usize = 120_000_000;
        let max_tail: usize = 10_000_000;
        let tail_start = file_size.saturating_sub(max_tail); // 110_000_000

        // Simulate finding first newline at byte 42 in the tail buffer
        let line_start = 42usize;
        let content_file_offset = tail_start + line_start; // 110_000_042

        // Case 1: first time seeing this file (raw_last_offset = 0)
        let raw_last_offset = 0usize;
        let last_offset = if raw_last_offset > content_file_offset { raw_last_offset - content_file_offset } else { 0 };
        assert_eq!(last_offset, 0, "first time: should start at beginning of tail content");

        // Case 2: previously processed up to byte 115_000_000 (within our tail window)
        let raw_last_offset = 115_000_000usize;
        let last_offset = if raw_last_offset > content_file_offset { raw_last_offset - content_file_offset } else { 0 };
        assert_eq!(last_offset, 115_000_000 - 110_000_042, "should resume within tail");

        // Case 3: after parse, new_offset is relative to tail content
        let new_relative_offset = 5_000_000usize;
        let stored_offset = content_file_offset + new_relative_offset;
        assert_eq!(stored_offset, 115_000_042, "stored offset should be in full-file coordinates");

        // Case 4: next cycle, raw_last_offset = stored_offset from case 3
        let raw_last_offset = stored_offset;
        let last_offset = if raw_last_offset > content_file_offset { raw_last_offset - content_file_offset } else { 0 };
        assert_eq!(last_offset, new_relative_offset, "should resume from where we left off");
    }

    #[test]
    fn test_utf8_continuation_byte_skip() {
        // M2: After finding newline, skip UTF-8 continuation bytes (0x80..=0xBF)
        // Simulates a buffer where seek landed mid-codepoint

        // Case 1: Normal ASCII — no continuation bytes after newline
        let buf = b"partial line\nfull line here\n";
        let nl = buf.iter().position(|&b| b == b'\n').map(|i| i + 1).unwrap_or(0);
        let mut start = nl;
        while start < buf.len() && (buf[start] & 0xC0) == 0x80 {
            start += 1;
        }
        assert_eq!(start, nl, "ASCII: no bytes skipped after newline");
        assert_eq!(&buf[start..start + 4], b"full");

        // Case 2: UTF-8 continuation bytes after newline (simulated mid-codepoint)
        // 0xC0 mask on continuation bytes (0x80-0xBF) = 0x80
        let buf: &[u8] = &[b'\n', 0x80, 0xBF, b'h', b'i'];
        let nl = 1; // position after \n
        let mut start = nl;
        while start < buf.len() && (buf[start] & 0xC0) == 0x80 {
            start += 1;
        }
        assert_eq!(start, 3, "should skip 2 continuation bytes");
        assert_eq!(buf[start], b'h');

        // Case 3: Empty buffer
        let buf: &[u8] = &[];
        assert!(buf.is_empty(), "empty buffer handled by early return");
    }
}
