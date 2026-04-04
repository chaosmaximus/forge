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

            // Detect active session for this agent to set session provenance
            let session_id = crate::sessions::get_active_session_id(&locked.conn, adapter.name())
                .unwrap_or_default();

            for em in &extracted {
                // Route skills to the skill table (Layer 2) instead of
                // the memory table (Layer 5). The `continue` ensures a
                // skill extraction doesn't also create a duplicate memory entry.
                // Quality gate: reject junk skills and demote them to lessons.
                if em.memory_type == "skill" {
                    let gate = skill_quality_gate(&em.title, &em.content);

                    if !gate.pass {
                        // Demote to lesson — fall through to normal memory storage below
                        eprintln!(
                            "[extractor] demoted junk skill '{}' to lesson (steps={}, len={}, not_status={})",
                            em.title, gate.step_count, em.content.len(), gate.not_status
                        );
                        // Don't continue — let it fall through to memory storage as a lesson
                    } else {
                        let skill = forge_core::types::Skill {
                            id: format!("skill-{}", ulid::Ulid::new()),
                            name: em.title.clone(),
                            domain: em.tags.first().cloned().unwrap_or_else(|| "general".to_string()),
                            description: em.content.clone(),
                            steps: gate.extracted_steps,
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
                }

                // Route identity signals to the identity table (Ahankara)
                if em.memory_type == "identity" {
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
                    .with_valence(&em.valence, em.intensity);
                memory.session_id = session_id.clone();

                // Stamp HLC + node_id so sync protocol works for extracted memories
                let locked_for_hlc = state.lock().await;
                memory.set_hlc(
                    locked_for_hlc.hlc.now(),
                    locked_for_hlc.hlc.node_id().to_string(),
                );
                // Set project from transcript path if not already set
                if memory.project.is_none() || memory.project.as_deref() == Some("") {
                    // Claude Code transcripts: ~/.claude/projects/-Users-name-workspace-project/...
                    // Extract last path component as project name
                    let path_str = path.to_string_lossy();
                    if let Some(projects_idx) = path_str.find("/projects/") {
                        let after = &path_str[projects_idx + 10..];
                        if let Some(slash) = after.find('/') {
                            let project_hash = &after[..slash];
                            // The hash is like "-Users-name-workspace-projectname"
                            // Take the last segment after the last dash-separated word
                            let project_name = project_hash
                                .rsplit('-')
                                .next()
                                .unwrap_or(project_hash);
                            if !project_name.is_empty() {
                                memory.project = Some(project_name.to_string());
                            }
                        }
                    }
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
                                let _ = ops::store_edge(&locked.conn, &memory.id, &match_result.id, "motivated_by", "{}");
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

/// Result of the skill quality gate check.
#[derive(Debug)]
struct SkillQualityGate {
    /// Whether the skill passes quality gates and should be stored.
    pass: bool,
    /// Number of properly structured steps found.
    step_count: usize,
    /// Whether the title is NOT a status/completion message.
    not_status: bool,
    /// Extracted step lines (only meaningful when pass=true).
    extracted_steps: Vec<String>,
}

/// Evaluate whether an extracted skill meets quality standards.
///
/// A skill must have:
/// - At least 200 characters of content
/// - At least 2 properly numbered steps (e.g. "1. ...", "2) ...", "Step 1: ...")
/// - A title that does not indicate task completion or status reporting
fn skill_quality_gate(title: &str, content: &str) -> SkillQualityGate {
    // Extract properly structured steps: numbered ("1.", "2)") or "Step N"
    let extracted_steps: Vec<String> = content
        .lines()
        .filter(|l| {
            let trimmed = l.trim();
            let starts_with_digit = trimmed
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit());
            // Numbered steps: digit followed by '.' or ')'
            (starts_with_digit
                && trimmed
                    .chars()
                    .find(|c| !c.is_ascii_digit())
                    .is_some_and(|c| c == '.' || c == ')'))
                || trimmed.starts_with("Step ")
        })
        .map(|l| l.trim().to_string())
        .collect();

    let step_count = extracted_steps.len();
    let has_steps = step_count >= 2;
    let long_enough = content.len() >= 200;

    let title_lower = title.to_lowercase();
    let not_status = !title_lower.contains("complete")
        && !title_lower.contains("remaining")
        && !title_lower.contains("tasks done")
        && !title_lower.contains("all passing")
        && !title_lower.starts_with("all ")
        && !title_lower.starts_with("fix the")
        && !title_lower.starts_with("completed")
        && !title_lower.starts_with("finished")
        && !title_lower.starts_with("implemented")
        && !title_lower.starts_with("shipped")
        && !title_lower.starts_with("deployed")
        && !title_lower.starts_with("merged")
        && !title_lower.starts_with("resolved");

    let pass = has_steps && long_enough && not_status;

    SkillQualityGate {
        pass,
        step_count,
        not_status,
        extracted_steps,
    }
}

#[cfg(test)]
mod tests {
    use super::skill_quality_gate;
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

    // ── Skill quality gate tests ──────────────────────────────────────

    #[test]
    fn test_skill_quality_gate_accepts_good_skill() {
        // A properly structured skill with numbered steps and 200+ chars
        let title = "Deploy Rust Service to Production";
        let content = "\
1. Build the release binary with `cargo build --release` and verify no warnings
2. Copy the binary to the production server via `scp target/release/forge user@prod:/opt/forge/`
3. SSH into the server and run `sudo systemctl restart forge-daemon`
4. Verify the service is healthy with `curl http://localhost:8080/health`";

        let gate = skill_quality_gate(title, content);
        assert!(gate.pass, "good skill should pass quality gate");
        assert_eq!(gate.step_count, 4, "should detect 4 numbered steps");
        assert!(gate.not_status, "title should not be flagged as status");
        assert_eq!(gate.extracted_steps.len(), 4);
    }

    #[test]
    fn test_skill_quality_gate_accepts_paren_numbered_steps() {
        // Steps using "1)" format
        let title = "Set Up CI Pipeline";
        let content = "\
1) Create a .github/workflows directory in the repository root for CI configuration
2) Write a ci.yml workflow file that runs on push and pull_request events to main
3) Add build, lint, and test jobs with proper caching of dependencies for speed
4) Configure branch protection rules to require CI passing before merge";

        let gate = skill_quality_gate(title, content);
        assert!(gate.pass, "paren-numbered steps should pass");
        assert_eq!(gate.step_count, 4);
    }

    #[test]
    fn test_skill_quality_gate_accepts_step_prefix() {
        // Steps using "Step N" format
        let title = "Database Migration Process";
        let content = "\
Step 1: Back up the current production database using pg_dump to a timestamped file
Step 2: Run the migration script against a staging copy first to verify correctness
Step 3: Apply the migration to production during the maintenance window after backup";

        let gate = skill_quality_gate(title, content);
        assert!(gate.pass, "Step-prefixed content should pass");
        assert!(gate.step_count >= 3, "should detect Step-prefixed lines");
    }

    #[test]
    fn test_skill_quality_gate_rejects_junk_title_complete() {
        let title = "All 17 Tasks Complete";
        let content = "Done. Everything is finished and deployed.";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "status title 'All 17 Tasks Complete' should be rejected");
        assert!(!gate.not_status);
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_completed() {
        let title = "Completed auth module refactor";
        let content = "1. Did the thing one by one carefully\n2. Also did another thing that was needed";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title starting with 'Completed' should be rejected");
        assert!(!gate.not_status);
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_finished() {
        let title = "Finished deploying the new version";
        let content = "1. Step one of the process\n2. Step two of the process";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title starting with 'Finished' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_implemented() {
        let title = "Implemented feature flag system";
        let content = "1. Added flags\n2. Deployed flags";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title starting with 'Implemented' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_shipped() {
        let title = "Shipped v2.0 release";
        let content = "1. Tagged release\n2. Pushed to prod";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title starting with 'Shipped' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_deployed() {
        let title = "Deployed hotfix to production";
        let content = "1. Built hotfix\n2. Deployed it";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title starting with 'Deployed' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_merged() {
        let title = "Merged PR #42 into main";
        let content = "1. Reviewed PR\n2. Merged PR";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title starting with 'Merged' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_resolved() {
        let title = "Resolved all CI failures";
        let content = "1. Fixed lint\n2. Fixed tests";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title starting with 'Resolved' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_tasks_done() {
        let title = "Sprint tasks done for week 14";
        let content = "1. Task A\n2. Task B";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title containing 'tasks done' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_title_all_passing() {
        let title = "Tests all passing now";
        let content = "1. Fixed test A\n2. Fixed test B";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "title containing 'all passing' should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_short_content() {
        // Good title but content under 200 chars
        let title = "Build and Test";
        let content = "1. run cargo test\n2. check output";
        assert!(content.len() < 200);
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "content under 200 chars should be rejected");
    }

    #[test]
    fn test_skill_quality_gate_rejects_no_steps() {
        // Narrative content without numbered steps
        let title = "Debug Authentication";
        let content = "Fixed the bug in auth module. Also updated docs. \
            The issue was that the token validation was skipping expiry checks \
            when the token had been refreshed within the last hour. We patched \
            the validation function and added regression tests to cover the edge case. \
            Everything is working now after the fix was deployed.";
        assert!(content.len() >= 200);
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "content without numbered steps should be rejected");
        assert_eq!(gate.step_count, 0);
    }

    #[test]
    fn test_skill_quality_gate_rejects_single_bullet() {
        // Only one bullet point — not enough steps
        let title = "Cleanup Process";
        let content = "- cleanup files and remove temporary artifacts from the build directory";
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "single bullet with no numbered steps should be rejected");
        assert_eq!(gate.step_count, 0, "bullets without numbers are not counted as steps");
    }

    #[test]
    fn test_skill_quality_gate_rejects_one_numbered_step() {
        // Only one numbered step — minimum is 2
        let title = "Quick Fix";
        let content = "\
1. Run the migration script against the database to update the schema and then \
verify the results are correct by checking the output logs for any error messages \
that might indicate a problem with the migration process or data integrity issues.";
        assert!(content.len() >= 200);
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "only 1 step should be rejected (minimum 2)");
        assert_eq!(gate.step_count, 1);
    }

    #[test]
    fn test_skill_quality_gate_rejects_dashed_lines_as_steps() {
        // Old logic counted "- " as steps — new logic should not
        let title = "Project Setup";
        let content = "\
- Install Node.js and npm on your development machine before starting\n\
- Clone the repository and run npm install to get all dependencies\n\
- Create a .env file with the required environment variables for local dev\n\
- Run npm start to launch the development server on localhost port 3000\n\
These are the basic steps to get started with the project setup workflow.";
        assert!(content.len() >= 200);
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "dashed lines should NOT count as numbered steps");
        assert_eq!(gate.step_count, 0);
    }

    #[test]
    fn test_skill_quality_gate_rejects_three_plain_lines() {
        // Old logic accepted any 3-line content — new logic should not
        let title = "Some Notes";
        let content = "\
The system uses a microservices architecture with three main components that \
communicate via gRPC and REST APIs depending on the use case and performance needs.\n\
Authentication is handled by the auth service which issues JWT tokens with a \
configurable expiry time that defaults to one hour for security purposes.\n\
The database layer uses PostgreSQL with connection pooling via PgBouncer to handle \
the high concurrency requirements of the production workload effectively.";
        assert!(content.len() >= 200);
        assert!(content.lines().count() >= 3);
        let gate = skill_quality_gate(title, content);
        assert!(!gate.pass, "plain multi-line text without numbered steps should be rejected");
    }
}
