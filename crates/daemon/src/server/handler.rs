use crate::claude_memory;
use crate::db::{ops, schema};
use crate::events::EventSender;
use crate::recall::hybrid_recall;
use forge_core::protocol::*;
use forge_core::types::{Memory, CodeFile, CodeSymbol};
use rusqlite::Connection;
use std::time::Instant;

pub struct DaemonState {
    pub conn: Connection,
    pub events: EventSender,
    pub started_at: Instant,
    pub hlc: crate::sync::Hlc,
    /// Channel to send edited file paths to the diagnostics worker.
    /// Set after worker spawn; None before that.
    pub diagnostics_tx: Option<tokio::sync::mpsc::Sender<String>>,
}

impl DaemonState {
    pub fn new(db_path: &str) -> rusqlite::Result<Self> {
        // Must init sqlite-vec extension before opening any connection
        crate::db::vec::init_sqlite_vec();

        let conn = if db_path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(db_path)?
        };
        // Enable WAL mode for better concurrent read/write performance
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        schema::create_schema(&conn)?;

        // Best-effort: detect and store platform info (OS, arch, shell, etc.)
        let _ = crate::db::manas::detect_and_store_platform(&conn);

        // Best-effort: detect and store available CLI tools
        let _ = crate::db::manas::detect_and_store_tools(&conn);

        // Prune low-quality skills (no steps, short descriptions, status-like names)
        match crate::db::manas::prune_junk_skills(&conn) {
            Ok(n) if n > 0 => eprintln!("[daemon] pruned {} junk skills", n),
            Ok(_) => {},
            Err(e) => eprintln!("[daemon] skill pruning error: {e}"),
        }

        // Backfill project on memories that have session_id but no project
        match crate::sessions::backfill_project(&conn) {
            Ok(n) if n > 0 => eprintln!("[daemon] backfilled project on {} memories", n),
            Ok(_) => {},
            Err(e) => eprintln!("[daemon] project backfill error: {e}"),
        }

        let node_id = crate::sync::generate_node_id();
        let hlc = crate::sync::Hlc::new(&node_id);

        // Backfill HLC timestamps on existing memories that lack them
        let backfilled = crate::sync::backfill_hlc(&conn, &hlc).unwrap_or(0);
        if backfilled > 0 {
            eprintln!("[daemon] backfilled HLC timestamps on {} existing memories", backfilled);
        }

        Ok(DaemonState {
            conn,
            events: crate::events::create_event_bus(),
            started_at: Instant::now(),
            hlc,
            diagnostics_tx: None,
        })
    }
}

pub fn handle_request(state: &mut DaemonState, request: Request) -> Response {
    match request {
        Request::Remember {
            memory_type,
            title,
            content,
            confidence,
            tags,
            project,
        } => {
            let type_str = format!("{:?}", memory_type);
            let title_clone = title.clone();
            let mut memory = Memory::new(memory_type, title, content);
            if let Some(c) = confidence {
                memory = memory.with_confidence(c);
            }
            if let Some(t) = tags {
                memory = memory.with_tags(t);
            }
            if let Some(p) = project {
                memory = memory.with_project(p);
            }
            // Assign active session ID so CLI-stored memories are linked to a session
            memory.session_id = crate::sessions::get_active_session_id(&state.conn, "cli")
                .unwrap_or_default();
            // Stamp HLC before storing
            memory.set_hlc(state.hlc.now(), state.hlc.node_id().to_string());
            let id = memory.id.clone();
            match ops::remember(&state.conn, &memory) {
                Ok(()) => {
                    crate::events::emit(&state.events, "memory_created", serde_json::json!({
                        "id": id,
                        "memory_type": type_str,
                        "title": title_clone,
                    }));
                    Response::Ok {
                        data: ResponseData::Stored { id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("remember failed: {e}"),
                },
            }
        }

        Request::Recall { query, memory_type, project, limit, layer } => {
            let lim = limit.unwrap_or(10);

            let results = match layer.as_deref() {
                // "experience" → only memory table (hybrid_recall, no manas_recall)
                Some("experience") => {
                    hybrid_recall(&state.conn, &query, None, memory_type.as_ref(), project.as_deref(), lim)
                }
                // "declared" → only declared knowledge
                Some("declared") => {
                    let declared = crate::db::manas::search_declared(&state.conn, &query, project.as_deref())
                        .unwrap_or_default();
                    declared.into_iter().take(lim).map(|d| {
                        MemoryResult {
                            memory: forge_core::types::Memory::new(
                                forge_core::types::MemoryType::Lesson,
                                format!("[declared:{}] {}", d.source, d.id),
                                d.content.chars().take(500).collect::<String>(),
                            ).with_confidence(0.7),
                            score: 0.5,
                            source: "declared".to_string(),
                        }
                    }).collect()
                }
                // "domain_dna" → only domain DNA
                Some("domain_dna") => {
                    let dna_list = crate::db::manas::list_domain_dna(&state.conn, project.as_deref())
                        .unwrap_or_default();
                    let query_lower = query.to_lowercase();
                    dna_list.into_iter()
                        .filter(|dna| dna.pattern.to_lowercase().contains(&query_lower))
                        .take(lim)
                        .map(|dna| {
                            MemoryResult {
                                memory: forge_core::types::Memory::new(
                                    forge_core::types::MemoryType::Pattern,
                                    format!("[dna:{}] {}", dna.aspect, dna.pattern),
                                    format!("Project convention: {} (confidence: {:.0}%)", dna.pattern, dna.confidence * 100.0),
                                ).with_confidence(dna.confidence),
                                score: 0.4,
                                source: "domain_dna".to_string(),
                            }
                        }).collect()
                }
                // "identity" → list identity facets matching query
                Some("identity") => {
                    // Search across all agents via LIKE on facet/description
                    let search = format!("%{}%", query);
                    let facets: Vec<forge_core::types::manas::IdentityFacet> = state.conn.prepare(
                        "SELECT id, agent, facet, description, strength, source, active, created_at
                         FROM identity WHERE active = 1 AND (facet LIKE ?1 OR description LIKE ?1)
                         ORDER BY strength DESC LIMIT ?2"
                    ).and_then(|mut stmt| {
                        stmt.query_map(rusqlite::params![search, lim as i64], |row| {
                            Ok(forge_core::types::manas::IdentityFacet {
                                id: row.get(0)?,
                                agent: row.get(1)?,
                                facet: row.get(2)?,
                                description: row.get(3)?,
                                strength: row.get(4)?,
                                source: row.get(5)?,
                                active: row.get::<_, i32>(6)? != 0,
                                created_at: row.get(7)?,
                            })
                        })?.collect()
                    }).unwrap_or_default();

                    facets.into_iter()
                        .map(|f| {
                            MemoryResult {
                                memory: forge_core::types::Memory::new(
                                    forge_core::types::MemoryType::Preference,
                                    format!("[identity:{}] {}", f.agent, f.facet),
                                    f.description.clone(),
                                ).with_confidence(f.strength),
                                score: 0.6,
                                source: "identity".to_string(),
                            }
                        }).collect()
                }
                // "perception" → list perceptions matching query (project-scoped)
                Some("perception") => {
                    let perceptions = crate::db::manas::list_unconsumed_perceptions(&state.conn, None)
                        .unwrap_or_default();
                    let query_lower = query.to_lowercase();
                    perceptions.into_iter()
                        .filter(|p| {
                            // Codex fix: respect project filter
                            if let Some(ref proj) = project {
                                match &p.project {
                                    Some(pp) if pp != proj => return false,
                                    None => {} // global perceptions are visible
                                    _ => {}
                                }
                            }
                            p.data.to_lowercase().contains(&query_lower)
                        })
                        .take(lim)
                        .map(|p| {
                            let snippet: String = p.data.chars().take(80).collect();
                            MemoryResult {
                                memory: forge_core::types::Memory::new(
                                    forge_core::types::MemoryType::Lesson,
                                    format!("[perception:{:?}] {}", p.kind, snippet),
                                    p.data.clone(),
                                ),
                                score: 0.5,
                                source: "perception".to_string(),
                            }
                        }).collect()
                }
                // "skill" → only skills (Layer 2 — procedural memory)
                Some("skill") => {
                    let skills = crate::db::manas::search_skills(&state.conn, &query, project.as_deref())
                        .unwrap_or_default();
                    skills.into_iter()
                        .take(lim)
                        .map(|s| {
                            MemoryResult {
                                memory: forge_core::types::Memory::new(
                                    forge_core::types::MemoryType::Pattern,
                                    format!("[skill:{}] {}", s.domain, s.name),
                                    s.description,
                                ).with_confidence(
                                    (0.5 + (s.success_count as f64 * 0.1)).min(1.0)
                                ),
                                score: 0.6,
                                source: "skill".to_string(),
                            }
                        }).collect()
                }
                // None or unknown → current behavior (search everything)
                _ => {
                    let mut results =
                        hybrid_recall(&state.conn, &query, None, memory_type.as_ref(), project.as_deref(), lim);
                    // Cross-layer search (only if no type filter)
                    if memory_type.is_none() {
                        let manas_results = crate::recall::manas_recall(&state.conn, &query, project.as_deref(), 3);
                        results.extend(manas_results);
                        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
                        results.truncate(lim);
                    }
                    results
                }
            };

            let count = results.len();
            Response::Ok {
                data: ResponseData::Memories { results, count },
            }
        }

        Request::Forget { id } => match ops::forget(&state.conn, &id) {
            Ok(true) => {
                crate::events::emit(&state.events, "memory_forgotten", serde_json::json!({
                    "id": id,
                }));
                Response::Ok {
                    data: ResponseData::Forgotten { id },
                }
            }
            Ok(false) => Response::Error {
                message: format!("memory not found or already deleted: {id}"),
            },
            Err(e) => Response::Error {
                message: format!("forget failed: {e}"),
            },
        },

        Request::HealthByProject => {
            match ops::health_by_project(&state.conn) {
                Ok(projects) => {
                    let project_data: std::collections::HashMap<String, forge_core::protocol::HealthProjectData> = projects.into_iter()
                        .map(|(k, v)| (k, forge_core::protocol::HealthProjectData {
                            decisions: v.decisions,
                            lessons: v.lessons,
                            patterns: v.patterns,
                            preferences: v.preferences,
                        }))
                        .collect();
                    Response::Ok {
                        data: ResponseData::HealthByProject { projects: project_data },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("health_by_project failed: {e}"),
                },
            }
        }

        Request::Health => match ops::health(&state.conn) {
            Ok(counts) => Response::Ok {
                data: ResponseData::Health {
                    decisions: counts.decisions,
                    lessons: counts.lessons,
                    patterns: counts.patterns,
                    preferences: counts.preferences,
                    edges: counts.edges,
                },
            },
            Err(e) => Response::Error {
                message: format!("health check failed: {e}"),
            },
        },

        Request::Status => {
            let uptime_secs = state.started_at.elapsed().as_secs();
            let memory_count = ops::health(&state.conn)
                .map(|h| h.decisions + h.lessons + h.patterns + h.preferences)
                .unwrap_or(0);
            Response::Ok {
                data: ResponseData::Status {
                    uptime_secs,
                    workers: vec![],
                    memory_count,
                },
            }
        }

        Request::Doctor => {
            let h = match ops::health(&state.conn) {
                Ok(h) => h,
                Err(e) => {
                    return Response::Error {
                        message: format!("doctor: health check failed: {e}"),
                    }
                }
            };
            let files = match ops::count_files(&state.conn) {
                Ok(n) => n,
                Err(e) => {
                    return Response::Error {
                        message: format!("doctor: count_files failed: {e}"),
                    }
                }
            };
            let symbols = match ops::count_symbols(&state.conn) {
                Ok(n) => n,
                Err(e) => {
                    return Response::Error {
                        message: format!("doctor: count_symbols failed: {e}"),
                    }
                }
            };
            let embeddings = crate::db::vec::count_embeddings(&state.conn).unwrap_or(0);
            let mh = crate::db::manas::manas_health(&state.conn).unwrap_or_default();
            Response::Ok {
                data: ResponseData::Doctor {
                    daemon_up: true,
                    db_size_bytes: 0,
                    memory_count: h.decisions + h.lessons + h.patterns + h.preferences,
                    embedding_count: embeddings,
                    file_count: files,
                    symbol_count: symbols,
                    edge_count: h.edges,
                    workers: vec![
                        "watcher".into(),
                        "extractor".into(),
                        "embedder".into(),
                        "consolidator".into(),
                        "indexer".into(),
                        "perception".into(),
                        "disposition".into(),
                        "diagnostics".into(),
                    ],
                    uptime_secs: state.started_at.elapsed().as_secs(),
                    platform_count: mh.platform_entries,
                    tool_count: mh.tools,
                    skill_count: mh.skills,
                    domain_dna_count: mh.domain_dna_entries,
                    perception_count: mh.perceptions_unconsumed,
                    declared_count: mh.declared_entries,
                    identity_count: mh.identity_facets_active,
                    disposition_count: mh.dispositions,
                },
            }
        }

        Request::Export { format: _, since: _ } => {
            let memories = ops::export_memories(&state.conn).unwrap_or_default();
            let files = ops::export_files(&state.conn).unwrap_or_default();
            let symbols = ops::export_symbols(&state.conn).unwrap_or_default();
            let edges = ops::export_edges(&state.conn).unwrap_or_default();

            let memory_results: Vec<MemoryResult> = memories.into_iter().map(|m| MemoryResult {
                memory: m,
                score: 1.0,
                source: "export".into(),
            }).collect();

            let export_edges: Vec<ExportEdge> = edges.into_iter().map(|(from, to, etype, props)| {
                ExportEdge {
                    from_id: from,
                    to_id: to,
                    edge_type: etype,
                    properties: serde_json::from_str(&props).unwrap_or(serde_json::Value::Null),
                }
            }).collect();

            Response::Ok {
                data: ResponseData::Export {
                    memories: memory_results,
                    files,
                    symbols,
                    edges: export_edges,
                },
            }
        }

        Request::Import { data } => {
            // Parse the JSON export payload
            #[derive(serde::Deserialize)]
            struct ExportPayload {
                memories: Option<Vec<serde_json::Value>>,
                files: Option<Vec<CodeFile>>,
                symbols: Option<Vec<CodeSymbol>>,
            }

            let payload: ExportPayload = match serde_json::from_str(&data) {
                Ok(p) => p,
                Err(e) => {
                    return Response::Error {
                        message: format!("import parse error: {e}"),
                    }
                }
            };

            // C-2: Enforce record count limit before importing
            let max_records: usize = 10_000;
            let total_records = payload.memories.as_ref().map_or(0, |v| v.len())
                + payload.files.as_ref().map_or(0, |v| v.len())
                + payload.symbols.as_ref().map_or(0, |v| v.len());
            if total_records > max_records {
                return Response::Error {
                    message: format!("import exceeds {max_records} record limit ({total_records} records)"),
                };
            }

            let mut memories_imported = 0usize;
            let mut files_imported = 0usize;
            let mut symbols_imported = 0usize;
            let mut skipped = 0usize;

            // RAII transaction: auto-rollback on drop if not committed
            let tx = match state.conn.unchecked_transaction() {
                Ok(t) => t,
                Err(e) => {
                    return Response::Error {
                        message: format!("import transaction begin failed: {e}"),
                    };
                }
            };

            // Import memories
            if let Some(mems) = payload.memories {
                for mem_val in mems {
                    if let Ok(mem) = serde_json::from_value::<Memory>(mem_val) {
                        if ops::remember(&tx, &mem).is_ok() {
                            memories_imported += 1;
                        } else {
                            skipped += 1;
                        }
                    } else {
                        skipped += 1;
                    }
                }
            }

            // Import files
            if let Some(files) = payload.files {
                for file in &files {
                    if ops::store_file(&tx, file).is_ok() {
                        files_imported += 1;
                    } else {
                        skipped += 1;
                    }
                }
            }

            // Import symbols
            if let Some(syms) = payload.symbols {
                for sym in &syms {
                    if ops::store_symbol(&tx, sym).is_ok() {
                        symbols_imported += 1;
                    } else {
                        skipped += 1;
                    }
                }
            }

            if let Err(e) = tx.commit() {
                return Response::Error {
                    message: format!("import commit failed: {e}"),
                };
            }

            Response::Ok {
                data: ResponseData::Import {
                    memories_imported,
                    files_imported,
                    symbols_imported,
                    skipped,
                },
            }
        }

        Request::IngestClaude => {
            match claude_memory::ingest_claude_memories(&state.conn) {
                Ok((imported, skipped)) => Response::Ok {
                    data: ResponseData::IngestClaude { imported, skipped },
                },
                Err(e) => Response::Error {
                    message: format!("ingest-claude failed: {e}"),
                },
            }
        }

        Request::IngestDeclared { path, source, project } => {
            match crate::db::manas::ingest_declared_file(
                &state.conn,
                &path,
                &source,
                project.as_deref(),
            ) {
                Ok(ingested) => Response::Ok {
                    data: ResponseData::IngestDeclared {
                        ingested,
                        path,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("ingest-declared failed: {e}"),
                },
            }
        }

        Request::Backfill { path } => {
            // C-1: Validate path is under ~/.claude/ to prevent arbitrary file read
            let home = std::env::var("HOME").unwrap_or_default();
            let allowed_dir = format!("{}/.claude/", home);
            let canonical = match std::fs::canonicalize(&path) {
                Ok(p) => p,
                Err(e) => {
                    return Response::Error {
                        message: format!("invalid path: {e}"),
                    }
                }
            };
            if !canonical.to_string_lossy().starts_with(&allowed_dir) {
                return Response::Error {
                    message: "path must be under ~/.claude/".to_string(),
                };
            }
            // Read the transcript file, parse all chunks from offset 0, store as memories
            match std::fs::read_to_string(&canonical) {
                Ok(content) => {
                    let (chunks, _) = crate::chunk::parse_transcript_incremental(&content, 0);
                    let mut stored = 0usize;
                    for chunk in &chunks {
                        // Store each substantial turn as a memory for later extraction
                        if chunk.content.len() < 50 {
                            continue; // skip trivial turns
                        }
                        let title = if chunk.content.len() > 80 {
                            let mut end = 77;
                            while !chunk.content.is_char_boundary(end) && end > 0 {
                                end -= 1;
                            }
                            format!("{}...", &chunk.content[..end])
                        } else {
                            chunk.content.clone()
                        };
                        let memory = Memory::new(
                            forge_core::types::MemoryType::Lesson,
                            title,
                            format!("[{}] {}", chunk.role, chunk.content),
                        )
                        .with_confidence(0.5)
                        .with_tags(vec!["backfill".to_string(), "transcript".to_string()]);
                        if ops::remember(&state.conn, &memory).is_ok() {
                            stored += 1;
                        }
                    }
                    Response::Ok {
                        data: ResponseData::Backfill {
                            chunks_processed: chunks.len(),
                            memories_stored: stored,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("backfill failed to read {}: {e}", path),
                },
            }
        }

        Request::RegisterSession { id, agent, project, cwd } => {
            let agent_clone = agent.clone();
            match crate::sessions::register_session(&state.conn, &id, &agent, project.as_deref(), cwd.as_deref()) {
                Ok(()) => {
                    crate::events::emit(&state.events, "session_changed", serde_json::json!({
                        "id": id,
                        "agent": agent_clone,
                        "action": "registered",
                    }));
                    Response::Ok {
                        data: ResponseData::SessionRegistered { id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("register_session failed: {e}"),
                },
            }
        }

        Request::EndSession { id } => {
            // Save working set before ending the session
            let _ = crate::sessions::save_working_set(&state.conn, &id);

            match crate::sessions::end_session(&state.conn, &id) {
                Ok(found) => {
                    if found {
                        crate::events::emit(&state.events, "session_changed", serde_json::json!({
                            "id": id,
                            "action": "ended",
                        }));
                    }
                    Response::Ok {
                        data: ResponseData::SessionEnded { id, found },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("end_session failed: {e}"),
                },
            }
        }

        Request::Sessions { active_only } => {
            match crate::sessions::list_sessions(&state.conn, active_only.unwrap_or(true)) {
                Ok(sessions) => {
                    let count = sessions.len();
                    let infos: Vec<forge_core::protocol::SessionInfo> = sessions.into_iter().map(|s| {
                        forge_core::protocol::SessionInfo {
                            id: s.id, agent: s.agent, project: s.project,
                            cwd: s.cwd, started_at: s.started_at,
                            ended_at: s.ended_at, status: s.status,
                        }
                    }).collect();
                    Response::Ok {
                        data: ResponseData::Sessions { sessions: infos, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_sessions failed: {e}"),
                },
            }
        }

        Request::Subscribe { .. } => {
            // Subscribe is handled directly in socket.rs (streaming mode).
            // This arm should never be reached.
            Response::Error {
                message: "subscribe must be handled at the socket layer".to_string(),
            }
        }

        Request::GuardrailsCheck { file, action } => {
            let result = crate::guardrails::check::check_action(&state.conn, &file, &action);

            // Emit guardrail_warning event when check returns unsafe
            if !result.safe {
                crate::events::emit(&state.events, "guardrail_warning", serde_json::json!({
                    "file": file,
                    "safe": false,
                    "warnings": result.warnings.clone(),
                    "decisions_affected": result.decisions_affected.clone(),
                    "callers_count": result.callers_count,
                    "calling_files": result.calling_files.clone(),
                    "relevant_lessons": result.relevant_lessons.clone(),
                    "dangerous_patterns": result.dangerous_patterns.clone(),
                    "applicable_skills": result.applicable_skills.clone(),
                }));
            }

            Response::Ok {
                data: ResponseData::GuardrailsCheck {
                    safe: result.safe,
                    warnings: result.warnings,
                    decisions_affected: result.decisions_affected,
                    callers_count: result.callers_count,
                    calling_files: result.calling_files,
                    relevant_lessons: result.relevant_lessons,
                    dangerous_patterns: result.dangerous_patterns,
                    applicable_skills: result.applicable_skills,
                },
            }
        }

        Request::PreBashCheck { command } => {
            let result = crate::guardrails::check::pre_bash_check(&state.conn, &command);

            // Emit bash_warning event when check returns unsafe
            if !result.safe {
                crate::events::emit(&state.events, "bash_warning", serde_json::json!({
                    "command": command,
                    "safe": false,
                    "warnings": result.warnings.clone(),
                    "relevant_skills": result.relevant_skills.clone(),
                }));
            }

            Response::Ok {
                data: ResponseData::PreBashChecked {
                    safe: result.safe,
                    warnings: result.warnings,
                    relevant_skills: result.relevant_skills,
                },
            }
        }

        Request::PostBashCheck { command, exit_code } => {
            let result = crate::guardrails::check::post_bash_check(&state.conn, &command, exit_code);

            Response::Ok {
                data: ResponseData::PostBashChecked {
                    suggestions: result.suggestions,
                },
            }
        }

        Request::PostEditCheck { file } => {
            let result = crate::guardrails::check::post_edit_check(&state.conn, &file);

            // Emit event if there are any warnings worth surfacing
            if !result.dangerous_patterns.is_empty() || result.callers_count > 0 || !result.decisions_to_review.is_empty() {
                crate::events::emit(&state.events, "post_edit_warning", serde_json::json!({
                    "file": file,
                    "callers": result.callers_count,
                    "warnings": result.dangerous_patterns.len() + result.decisions_to_review.len(),
                }));
            }

            Response::Ok {
                data: ResponseData::PostEditChecked {
                    file: result.file,
                    callers_count: result.callers_count,
                    calling_files: result.calling_files,
                    relevant_lessons: result.relevant_lessons,
                    dangerous_patterns: result.dangerous_patterns,
                    applicable_skills: result.applicable_skills,
                    decisions_to_review: result.decisions_to_review,
                    cached_diagnostics: result.cached_diagnostics,
                },
            }
        }

        Request::BlastRadius { file } => {
            let br = crate::guardrails::blast_radius::analyze_blast_radius(&state.conn, &file);
            let decisions: Vec<forge_core::protocol::BlastRadiusDecision> = br
                .decisions
                .into_iter()
                .map(|(id, title, confidence)| forge_core::protocol::BlastRadiusDecision {
                    id, title, confidence,
                })
                .collect();
            Response::Ok {
                data: ResponseData::BlastRadius {
                    decisions,
                    callers: br.callers,
                    importers: br.importers,
                    files_affected: br.files_affected,
                },
            }
        }

        Request::LspStatus => {
            let project_dir = crate::workers::indexer::find_project_dir();
            let servers = match project_dir {
                Some(ref dir) => crate::lsp::detect::detect_language_servers(dir),
                None => vec![],
            };
            let infos: Vec<forge_core::protocol::LspServerInfo> = servers
                .into_iter()
                .map(|s| forge_core::protocol::LspServerInfo {
                    language: s.language,
                    command: s.command,
                    available: true, // detect_language_servers only returns servers that exist on PATH
                })
                .collect();
            Response::Ok {
                data: ResponseData::LspStatus { servers: infos },
            }
        }

        // ── Manas Layer Handlers ──

        Request::StorePlatform { key, value } => {
            let entry = forge_core::types::manas::PlatformEntry {
                key: key.clone(),
                value,
                detected_at: chrono_now(),
            };
            match crate::db::manas::store_platform(&state.conn, &entry) {
                Ok(()) => Response::Ok {
                    data: ResponseData::PlatformStored { key },
                },
                Err(e) => Response::Error {
                    message: format!("store_platform failed: {e}"),
                },
            }
        }

        Request::ListPlatform => {
            match crate::db::manas::list_platform(&state.conn) {
                Ok(entries) => Response::Ok {
                    data: ResponseData::PlatformList { entries },
                },
                Err(e) => Response::Error {
                    message: format!("list_platform failed: {e}"),
                },
            }
        }

        Request::StoreTool { tool } => {
            let id = tool.id.clone();
            match crate::db::manas::store_tool(&state.conn, &tool) {
                Ok(()) => Response::Ok {
                    data: ResponseData::ToolStored { id },
                },
                Err(e) => Response::Error {
                    message: format!("store_tool failed: {e}"),
                },
            }
        }

        Request::ListTools => {
            match crate::db::manas::list_tools(&state.conn, None) {
                Ok(tools) => {
                    let count = tools.len();
                    Response::Ok {
                        data: ResponseData::ToolList { tools, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_tools failed: {e}"),
                },
            }
        }

        Request::StorePerception { perception } => {
            let id = perception.id.clone();
            let kind_str = format!("{:?}", perception.kind);
            match crate::db::manas::store_perception(&state.conn, &perception) {
                Ok(()) => {
                    crate::events::emit(&state.events, "perception_update", serde_json::json!({
                        "id": id,
                        "kind": kind_str,
                    }));
                    Response::Ok {
                        data: ResponseData::PerceptionStored { id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("store_perception failed: {e}"),
                },
            }
        }

        Request::ListPerceptions { project, limit } => {
            let lim = limit.unwrap_or(20);
            match crate::db::manas::list_unconsumed_perceptions(&state.conn, None) {
                Ok(perceptions) => {
                    // Apply project filter and limit in-memory
                    let filtered: Vec<_> = perceptions.into_iter()
                        .filter(|p| match (&project, &p.project) {
                            (Some(proj), Some(pp)) => pp == proj,
                            (Some(_), None) => false,
                            (None, _) => true,
                        })
                        .take(lim)
                        .collect();
                    let count = filtered.len();
                    Response::Ok {
                        data: ResponseData::PerceptionList { perceptions: filtered, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_perceptions failed: {e}"),
                },
            }
        }

        Request::ConsumePerceptions { ids } => {
            let mut consumed = 0usize;
            for id in &ids {
                match crate::db::manas::consume_perception(&state.conn, id) {
                    Ok(true) => consumed += 1,
                    Ok(false) => {} // already consumed or not found
                    Err(e) => {
                        return Response::Error {
                            message: format!("consume_perception failed for {id}: {e}"),
                        };
                    }
                }
            }
            Response::Ok {
                data: ResponseData::PerceptionsConsumed { count: consumed },
            }
        }

        Request::StoreIdentity { mut facet } => {
            facet.strength = facet.strength.clamp(0.0, 1.0);
            let id = facet.id.clone();
            let facet_name = facet.facet.clone();
            let agent_name = facet.agent.clone();
            match crate::db::manas::store_identity(&state.conn, &facet) {
                Ok(()) => {
                    crate::events::emit(&state.events, "identity_updated", serde_json::json!({
                        "id": id,
                        "facet": facet_name,
                        "agent": agent_name,
                    }));
                    Response::Ok {
                        data: ResponseData::IdentityStored { id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("store_identity failed: {e}"),
                },
            }
        }

        Request::ListIdentity { agent } => {
            match crate::db::manas::list_identity(&state.conn, &agent, true) {
                Ok(facets) => {
                    let count = facets.len();
                    Response::Ok {
                        data: ResponseData::IdentityList { facets, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_identity failed: {e}"),
                },
            }
        }

        Request::DeactivateIdentity { id } => {
            match crate::db::manas::deactivate_identity(&state.conn, &id) {
                Ok(found) => Response::Ok {
                    data: ResponseData::IdentityDeactivated { id, found },
                },
                Err(e) => Response::Error {
                    message: format!("deactivate_identity failed: {e}"),
                },
            }
        }

        Request::ListDisposition { agent } => {
            match crate::db::manas::list_dispositions(&state.conn, &agent) {
                Ok(traits) => {
                    let count = traits.len();
                    Response::Ok {
                        data: ResponseData::DispositionList { traits, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_dispositions failed: {e}"),
                },
            }
        }

        Request::ManasHealth => {
            match crate::db::manas::manas_health(&state.conn) {
                Ok(mh) => Response::Ok {
                    data: ResponseData::ManasHealthData {
                        platform_count: mh.platform_entries,
                        tool_count: mh.tools,
                        skill_count: mh.skills,
                        domain_dna_count: mh.domain_dna_entries,
                        perception_unconsumed: mh.perceptions_unconsumed,
                        declared_count: mh.declared_entries,
                        identity_facets: mh.identity_facets_active,
                        disposition_traits: mh.dispositions,
                        experience_count: mh.experience_count,
                        embedding_count: mh.embedding_count,
                        trait_names: mh.trait_names,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("manas_health failed: {e}"),
                },
            }
        }

        Request::CompileContext { agent, project, static_only } => {
            let agent_name = agent.as_deref().unwrap_or("claude-code");
            let static_prefix = crate::recall::compile_static_prefix(&state.conn, agent_name);

            if static_only.unwrap_or(false) {
                let chars = static_prefix.len();
                Response::Ok {
                    data: ResponseData::CompiledContext {
                        context: static_prefix.clone(),
                        static_prefix,
                        dynamic_suffix: String::new(),
                        layers_used: 4, // platform, identity, disposition, tools
                        chars,
                    },
                }
            } else {
                let dynamic_suffix = crate::recall::compile_dynamic_suffix(
                    &state.conn, agent_name, project.as_deref(), 3000,
                );
                let full = format!(
                    "<forge-context version=\"0.7.0\">\n{}\n{}\n</forge-context>",
                    static_prefix, dynamic_suffix
                );
                let chars = full.len();
                Response::Ok {
                    data: ResponseData::CompiledContext {
                        context: full,
                        static_prefix,
                        dynamic_suffix,
                        layers_used: 9, // platform, identity, disposition, tools, decisions, lessons, skills, perceptions, working-set
                        chars,
                    },
                }
            }
        }

        // ── Sync Operations ──

        Request::SyncExport { project, since } => {
            match crate::sync::sync_export(
                &state.conn,
                project.as_deref(),
                since.as_deref(),
            ) {
                Ok(lines) => {
                    let count = lines.len();
                    let node_id = state.hlc.node_id().to_string();
                    Response::Ok {
                        data: ResponseData::SyncExported { lines, count, node_id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("sync_export failed: {e}"),
                },
            }
        }

        Request::SyncImport { lines } => {
            let local_node_id = state.hlc.node_id().to_string();
            match crate::sync::sync_import(&state.conn, &lines, &local_node_id) {
                Ok(result) => {
                    crate::events::emit(&state.events, "sync_completed", serde_json::json!({
                        "imported": result.imported,
                        "conflicts": result.conflicts,
                        "skipped": result.skipped,
                    }));
                    Response::Ok {
                        data: ResponseData::SyncImported {
                            imported: result.imported,
                            conflicts: result.conflicts,
                            skipped: result.skipped,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("sync_import failed: {e}"),
                },
            }
        }

        Request::SyncConflicts => {
            match crate::sync::list_conflicts(&state.conn) {
                Ok(conflicts) => Response::Ok {
                    data: ResponseData::SyncConflictList { conflicts },
                },
                Err(e) => Response::Error {
                    message: format!("list_conflicts failed: {e}"),
                },
            }
        }

        Request::SyncResolve { keep_id } => {
            let id = keep_id.clone();
            match crate::sync::resolve_conflict(&state.conn, &keep_id) {
                Ok(resolved) => Response::Ok {
                    data: ResponseData::SyncResolved { id, resolved },
                },
                Err(e) => Response::Error {
                    message: format!("resolve_conflict failed: {e}"),
                },
            }
        }

        Request::Verify { file } => {
            match file {
                Some(f) => {
                    // Run checks on a specific file and return its diagnostics
                    let diags = crate::db::diagnostics::get_diagnostics(&state.conn, &f).unwrap_or_default();
                    let errors = diags.iter().filter(|d| d.severity == "error").count();
                    let warnings = diags.iter().filter(|d| d.severity == "warning").count();
                    let diagnostics: Vec<forge_core::protocol::DiagnosticEntry> = diags.iter().map(|d| {
                        forge_core::protocol::DiagnosticEntry {
                            file_path: d.file_path.clone(),
                            severity: d.severity.clone(),
                            message: d.message.clone(),
                            source: d.source.clone(),
                            line: d.line,
                        }
                    }).collect();
                    Response::Ok {
                        data: ResponseData::VerifyResult {
                            files_checked: 1,
                            errors,
                            warnings,
                            diagnostics,
                        },
                    }
                }
                None => {
                    // Return all active diagnostics
                    let diags = crate::db::diagnostics::get_all_active_diagnostics(&state.conn).unwrap_or_default();
                    let errors = diags.iter().filter(|d| d.severity == "error").count();
                    let warnings = diags.iter().filter(|d| d.severity == "warning").count();
                    // Count unique files
                    let files_checked = {
                        let mut files: Vec<&str> = diags.iter().map(|d| d.file_path.as_str()).collect();
                        files.sort();
                        files.dedup();
                        files.len()
                    };
                    let diagnostics: Vec<forge_core::protocol::DiagnosticEntry> = diags.iter().map(|d| {
                        forge_core::protocol::DiagnosticEntry {
                            file_path: d.file_path.clone(),
                            severity: d.severity.clone(),
                            message: d.message.clone(),
                            source: d.source.clone(),
                            line: d.line,
                        }
                    }).collect();
                    Response::Ok {
                        data: ResponseData::VerifyResult {
                            files_checked,
                            errors,
                            warnings,
                            diagnostics,
                        },
                    }
                }
            }
        }

        Request::GetDiagnostics { file } => {
            let diags = crate::db::diagnostics::get_diagnostics(&state.conn, &file).unwrap_or_default();
            let count = diags.len();
            let diagnostics: Vec<forge_core::protocol::DiagnosticEntry> = diags.iter().map(|d| {
                forge_core::protocol::DiagnosticEntry {
                    file_path: d.file_path.clone(),
                    severity: d.severity.clone(),
                    message: d.message.clone(),
                    source: d.source.clone(),
                    line: d.line,
                }
            }).collect();
            Response::Ok {
                data: ResponseData::DiagnosticList {
                    diagnostics,
                    count,
                },
            }
        }

        Request::HlcBackfill => {
            match crate::sync::backfill_hlc(&state.conn, &state.hlc) {
                Ok(count) => {
                    if count > 0 {
                        eprintln!("[daemon] backfilled HLC timestamps on {} existing memories", count);
                    }
                    Response::Ok {
                        data: ResponseData::HlcBackfilled { count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("hlc_backfill failed: {e}"),
                },
            }
        }

        Request::StoreEvaluation { findings, project, session_id } => {
            let mut lessons_created = 0usize;
            let mut diagnostics_created = 0usize;

            for finding in &findings {
                // Determine valence from category
                let valence = match finding.category.as_str() {
                    "good_pattern" => "positive",
                    _ => "negative",
                };
                let intensity = match finding.severity.as_str() {
                    "critical" => 0.95,
                    "high" => 0.8,
                    "medium" => 0.6,
                    "low" => 0.4,
                    _ => 0.3,
                };

                // Store as lesson memory
                let mut memory = Memory::new(
                    forge_core::types::MemoryType::Lesson,
                    finding.description.clone(),
                    format!("[{}] {}: {}", finding.severity, finding.category, finding.description),
                ).with_confidence(intensity)
                 .with_valence(valence, intensity)
                 .with_tags(vec![
                     format!("eval:{}", finding.category),
                     "auto-evaluation".to_string(),
                 ]);

                if let Some(ref p) = project {
                    memory = memory.with_project(p.clone());
                }
                if let Some(ref sid) = session_id {
                    memory.session_id = sid.clone();
                }

                let mem_id = memory.id.clone();

                if let Err(e) = ops::remember(&state.conn, &memory) {
                    eprintln!("[eval-feedback] failed to store lesson: {e}");
                    continue;
                }
                lessons_created += 1;

                // Create "affects" edges to files
                for file in &finding.files {
                    let file_node_id = format!("file:{}", file);
                    if let Err(e) = ops::store_edge(&state.conn, &mem_id, &file_node_id, "affects", "{}") {
                        eprintln!("[eval-feedback] failed to create affects edge: {e}");
                    }
                }

                // For high+ severity: create diagnostic so proactive intelligence warns
                if matches!(finding.severity.as_str(), "critical" | "high") {
                    for file in &finding.files {
                        let diag = crate::db::diagnostics::Diagnostic {
                            id: format!("eval-diag-{}", ulid::Ulid::new()),
                            file_path: file.clone(),
                            severity: finding.severity.clone(),
                            message: finding.description.clone(),
                            source: "forge-evaluator".to_string(),
                            line: None,
                            column: None,
                            created_at: forge_core::time::now_iso(),
                            expires_at: forge_core::time::now_offset(86400), // 24h TTL
                        };
                        if let Err(e) = crate::db::diagnostics::store_diagnostic(&state.conn, &diag) {
                            eprintln!("[eval-feedback] failed to create diagnostic: {e}");
                        } else {
                            diagnostics_created += 1;
                        }
                    }
                }
            }

            if lessons_created > 0 || diagnostics_created > 0 {
                eprintln!("[eval-feedback] stored {} lessons, {} diagnostics from evaluation", lessons_created, diagnostics_created);
            }

            Response::Ok {
                data: ResponseData::EvaluationStored { lessons_created, diagnostics_created },
            }
        }

        Request::Shutdown => Response::Ok {
            data: ResponseData::Shutdown,
        },
    }
}

// Use shared timestamp from forge_core
fn chrono_now() -> String {
    forge_core::time::now_iso()
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::types::MemoryType;

    #[test]
    fn test_remember_and_recall() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Remember a Decision
        let remember_req = Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use JWT".to_string(),
            content: "For auth".to_string(),
            confidence: Some(0.95),
            tags: Some(vec!["auth".to_string()]),
            project: None,
        };
        let response = handle_request(&mut state, remember_req);

        let stored_id = match response {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => {
                assert!(!id.is_empty(), "stored id must be non-empty");
                id
            }
            other => panic!("expected Stored response, got {:?}", other),
        };

        // Recall "JWT auth"
        let recall_req = Request::Recall {
            query: "JWT auth".to_string(),
            memory_type: None,
            project: None,
            limit: None,
            layer: None,
        };
        let response = handle_request(&mut state, recall_req);

        match response {
            Response::Ok {
                data: ResponseData::Memories { results, count },
            } => {
                assert_eq!(count, 1, "should recall exactly 1 memory");
                assert_eq!(results.len(), 1);
                assert!(
                    results[0].memory.title.contains("JWT"),
                    "title should contain 'JWT', got: {}",
                    results[0].memory.title
                );
                assert_eq!(results[0].memory.id, stored_id);
            }
            other => panic!("expected Memories response, got {:?}", other),
        }
    }

    #[test]
    fn test_health() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let response = handle_request(&mut state, Request::Health);

        match response {
            Response::Ok {
                data: ResponseData::Health { decisions, .. },
            } => {
                assert_eq!(decisions, 0, "fresh DB should have 0 decisions");
            }
            other => panic!("expected Health response, got {:?}", other),
        }
    }

    #[test]
    fn test_health_by_project() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store memories in different projects
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Forge arch".into(),
            content: "Rust CLI".into(),
            confidence: None,
            tags: None,
            project: Some("forge".into()),
        });
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Lesson,
            title: "Backend lesson".into(),
            content: "REST".into(),
            confidence: None,
            tags: None,
            project: Some("backend".into()),
        });
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Pattern,
            title: "Global pattern".into(),
            content: "Always test".into(),
            confidence: None,
            tags: None,
            project: None,
        });

        let resp = handle_request(&mut state, Request::HealthByProject);
        match resp {
            Response::Ok { data: ResponseData::HealthByProject { projects } } => {
                assert_eq!(projects.get("forge").unwrap().decisions, 1);
                assert_eq!(projects.get("backend").unwrap().lessons, 1);
                assert_eq!(projects.get("_global").unwrap().patterns, 1);
            }
            other => panic!("expected HealthByProject response, got {:?}", other),
        }
    }

    #[test]
    fn test_doctor() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let resp = handle_request(&mut state, Request::Doctor);
        match resp {
            Response::Ok {
                data:
                    ResponseData::Doctor {
                        daemon_up,
                        memory_count,
                        file_count,
                        symbol_count,
                        edge_count,
                        workers,
                        platform_count,
                        tool_count,
                        skill_count,
                        domain_dna_count,
                        perception_count,
                        declared_count,
                        identity_count,
                        disposition_count,
                        ..
                    },
            } => {
                assert!(daemon_up);
                assert_eq!(memory_count, 0);
                assert_eq!(file_count, 0);
                assert_eq!(symbol_count, 0);
                assert_eq!(edge_count, 0);
                assert_eq!(workers.len(), 8);
                assert!(workers.contains(&"indexer".to_string()));
                // Manas layer counts: detect_and_store_platform and detect_and_store_tools
                // may have stored some entries. The rest should be 0.
                let _ = platform_count; // platform may be non-zero from auto-detect
                let _ = tool_count; // tools may be non-zero from auto-detect
                assert_eq!(skill_count, 0);
                assert_eq!(domain_dna_count, 0);
                assert_eq!(perception_count, 0);
                assert_eq!(declared_count, 0);
                assert_eq!(identity_count, 0);
                assert_eq!(disposition_count, 0);
            }
            _ => panic!("expected Doctor response"),
        }
    }

    #[test]
    fn test_export_empty() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(&mut state, Request::Export { format: None, since: None });
        match resp {
            Response::Ok { data: ResponseData::Export { memories, files, symbols, edges } } => {
                assert!(memories.is_empty());
                assert!(files.is_empty());
                assert!(symbols.is_empty());
                assert!(edges.is_empty());
            }
            _ => panic!("expected Export response"),
        }
    }

    #[test]
    fn test_export_with_data() {
        let mut state = DaemonState::new(":memory:").unwrap();
        // Remember a decision
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use Rust".into(),
            content: "Fast".into(),
            confidence: None,
            tags: None,
            project: None,
        });

        let resp = handle_request(&mut state, Request::Export { format: None, since: None });
        match resp {
            Response::Ok { data: ResponseData::Export { memories, files, symbols, edges } } => {
                assert_eq!(memories.len(), 1);
                assert_eq!(memories[0].memory.title, "Use Rust");
                assert_eq!(memories[0].source, "export");
                assert!((memories[0].score - 1.0).abs() < f64::EPSILON);
                assert!(files.is_empty());
                assert!(symbols.is_empty());
                assert!(edges.is_empty());
            }
            _ => panic!("expected Export response"),
        }
    }

    #[test]
    fn test_import_memories() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // First export is empty
        let resp = handle_request(&mut state, Request::Export { format: None, since: None });
        match &resp {
            Response::Ok { data: ResponseData::Export { memories, .. } } => {
                assert!(memories.is_empty());
            }
            _ => panic!("expected empty Export"),
        }

        // Import a memory via JSON
        let import_data = serde_json::json!({
            "memories": [{
                "id": "test-import-1",
                "memory_type": "decision",
                "title": "Imported decision",
                "content": "From another machine",
                "confidence": 0.85,
                "status": "active",
                "project": null,
                "tags": [],
                "embedding": null,
                "created_at": "2026-04-02 10:00:00",
                "accessed_at": "2026-04-02 10:00:00"
            }],
            "files": [{
                "id": "f-import-1",
                "path": "src/lib.rs",
                "language": "rust",
                "project": "forge",
                "hash": "deadbeef",
                "indexed_at": "2026-04-02"
            }],
            "symbols": [{
                "id": "s-import-1",
                "name": "main",
                "kind": "function",
                "file_path": "src/main.rs",
                "line_start": 1,
                "line_end": 10,
                "signature": "fn main()"
            }]
        });

        let resp = handle_request(&mut state, Request::Import {
            data: import_data.to_string(),
        });
        match resp {
            Response::Ok { data: ResponseData::Import { memories_imported, files_imported, symbols_imported, skipped } } => {
                assert_eq!(memories_imported, 1);
                assert_eq!(files_imported, 1);
                assert_eq!(symbols_imported, 1);
                assert_eq!(skipped, 0);
            }
            _ => panic!("expected Import response"),
        }

        // Verify the imported memory shows up in export
        let resp = handle_request(&mut state, Request::Export { format: None, since: None });
        match resp {
            Response::Ok { data: ResponseData::Export { memories, files, symbols, .. } } => {
                assert_eq!(memories.len(), 1);
                assert_eq!(memories[0].memory.title, "Imported decision");
                assert_eq!(files.len(), 1);
                assert_eq!(files[0].path, "src/lib.rs");
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].name, "main");
            }
            _ => panic!("expected Export response after import"),
        }
    }

    #[test]
    fn test_guardrails_check_safe() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(&mut state, Request::GuardrailsCheck {
            file: "src/lib.rs".into(),
            action: "edit".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::GuardrailsCheck { safe, warnings, decisions_affected, callers_count, calling_files, relevant_lessons, dangerous_patterns, applicable_skills } } => {
                assert!(safe);
                assert!(warnings.is_empty());
                assert!(decisions_affected.is_empty());
                assert_eq!(callers_count, 0);
                assert!(calling_files.is_empty());
                assert!(relevant_lessons.is_empty());
                assert!(dangerous_patterns.is_empty());
                assert!(applicable_skills.is_empty());
            }
            _ => panic!("expected GuardrailsCheck response"),
        }
    }

    #[test]
    fn test_guardrails_check_with_linked_decision() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let resp = handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use JWT".into(),
            content: "Auth".into(),
            confidence: None,
            tags: None,
            project: None,
        });
        let id = match resp {
            Response::Ok { data: ResponseData::Stored { id } } => id,
            _ => panic!("expected Stored"),
        };

        crate::db::ops::store_edge(&state.conn, &id, "file:src/auth.rs", "affects", "{}").unwrap();

        let resp = handle_request(&mut state, Request::GuardrailsCheck {
            file: "src/auth.rs".into(),
            action: "edit".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::GuardrailsCheck { safe, decisions_affected, .. } } => {
                assert!(!safe);
                assert_eq!(decisions_affected.len(), 1);
                assert_eq!(decisions_affected[0], id);
            }
            _ => panic!("expected GuardrailsCheck response"),
        }
    }

    #[test]
    fn test_guardrail_check_emits_warning_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        // Store a decision linked to a file
        let resp = handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use JWT auth".into(),
            content: "Security decision".into(),
            confidence: None,
            tags: None,
            project: None,
        });
        let id = match resp {
            Response::Ok { data: ResponseData::Stored { id } } => id,
            _ => panic!("expected Stored"),
        };

        // Link decision to a file
        crate::db::ops::store_edge(&state.conn, &id, "file:src/auth.rs", "affects", "{}").unwrap();

        // Drain any prior events (e.g. from remember)
        while rx.try_recv().is_ok() {}

        // Fire guardrails check — should be unsafe because decision is linked
        let resp = handle_request(&mut state, Request::GuardrailsCheck {
            file: "src/auth.rs".into(),
            action: "edit".into(),
        });

        // Verify the response itself is still correct
        match &resp {
            Response::Ok { data: ResponseData::GuardrailsCheck { safe, decisions_affected, .. } } => {
                assert!(!safe);
                assert_eq!(decisions_affected.len(), 1);
            }
            _ => panic!("expected GuardrailsCheck response"),
        }

        // Should have emitted a guardrail_warning event
        let event = rx.try_recv().expect("should have emitted guardrail_warning event");
        assert_eq!(event.event, "guardrail_warning");
        assert_eq!(event.data["safe"], false);
        assert_eq!(event.data["file"], "src/auth.rs");
        assert!(event.data["warnings"].is_array());
        assert!(event.data["decisions_affected"].is_array());
        assert_eq!(event.data["decisions_affected"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_guardrail_check_safe_no_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        // Drain any prior events
        while rx.try_recv().is_ok() {}

        // Fire guardrails check on a file with no linked decisions — should be safe
        handle_request(&mut state, Request::GuardrailsCheck {
            file: "src/lib.rs".into(),
            action: "edit".into(),
        });

        // Should NOT have emitted a guardrail_warning event
        assert!(rx.try_recv().is_err(), "should not emit event when check is safe");
    }

    #[test]
    fn test_post_edit_check_clean_file() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(&mut state, Request::PostEditCheck {
            file: "src/lib.rs".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::PostEditChecked {
                file, callers_count, calling_files, relevant_lessons,
                dangerous_patterns, applicable_skills, decisions_to_review,
                cached_diagnostics,
            } } => {
                assert_eq!(file, "src/lib.rs");
                assert_eq!(callers_count, 0);
                assert!(calling_files.is_empty());
                assert!(relevant_lessons.is_empty());
                assert!(dangerous_patterns.is_empty());
                assert!(applicable_skills.is_empty());
                assert!(decisions_to_review.is_empty());
                assert!(cached_diagnostics.is_empty());
            }
            _ => panic!("expected PostEditChecked response"),
        }
    }

    #[test]
    fn test_post_edit_check_with_decision_emits_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        // Store a decision linked to a file
        let resp = handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use JWT".into(),
            content: "JWT tokens".into(),
            confidence: None,
            tags: None,
            project: None,
        });
        let _id = match resp {
            Response::Ok { data: ResponseData::Stored { id } } => id,
            _ => panic!("expected Stored"),
        };
        crate::db::ops::store_edge(&state.conn, &_id, "file:src/auth.rs", "affects", "{}").unwrap();

        // Drain prior events
        while rx.try_recv().is_ok() {}

        let resp = handle_request(&mut state, Request::PostEditCheck {
            file: "src/auth.rs".into(),
        });
        match &resp {
            Response::Ok { data: ResponseData::PostEditChecked {
                decisions_to_review, ..
            } } => {
                assert!(!decisions_to_review.is_empty());
                assert!(decisions_to_review[0].contains("Use JWT"));
            }
            _ => panic!("expected PostEditChecked response"),
        }

        // Should have emitted a post_edit_warning event
        let event = rx.try_recv().expect("should have emitted post_edit_warning event");
        assert_eq!(event.event, "post_edit_warning");
        assert_eq!(event.data["file"], "src/auth.rs");
    }

    #[test]
    fn test_blast_radius_empty() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(&mut state, Request::BlastRadius {
            file: "src/lib.rs".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::BlastRadius { decisions, callers, importers, files_affected } } => {
                assert!(decisions.is_empty());
                assert_eq!(callers, 0);
                assert!(importers.is_empty());
                assert!(files_affected.is_empty());
            }
            _ => panic!("expected BlastRadius response"),
        }
    }

    #[test]
    fn test_register_and_list_sessions() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register two sessions
        let resp1 = handle_request(&mut state, Request::RegisterSession {
            id: "s1".into(),
            agent: "claude-code".into(),
            project: Some("forge".into()),
            cwd: Some("/project".into()),
        });
        match resp1 {
            Response::Ok { data: ResponseData::SessionRegistered { id } } => assert_eq!(id, "s1"),
            other => panic!("expected SessionRegistered, got {:?}", other),
        }

        let resp2 = handle_request(&mut state, Request::RegisterSession {
            id: "s2".into(),
            agent: "cline".into(),
            project: None,
            cwd: None,
        });
        match resp2 {
            Response::Ok { data: ResponseData::SessionRegistered { id } } => assert_eq!(id, "s2"),
            other => panic!("expected SessionRegistered, got {:?}", other),
        }

        // List active sessions — should be 2
        let resp = handle_request(&mut state, Request::Sessions { active_only: Some(true) });
        match resp {
            Response::Ok { data: ResponseData::Sessions { sessions, count } } => {
                assert_eq!(count, 2);
                assert_eq!(sessions.len(), 2);
            }
            other => panic!("expected Sessions, got {:?}", other),
        }
    }

    #[test]
    fn test_end_session_via_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register
        handle_request(&mut state, Request::RegisterSession {
            id: "s1".into(),
            agent: "claude-code".into(),
            project: None,
            cwd: None,
        });

        // End
        let resp = handle_request(&mut state, Request::EndSession { id: "s1".into() });
        match resp {
            Response::Ok { data: ResponseData::SessionEnded { id, found } } => {
                assert_eq!(id, "s1");
                assert!(found);
            }
            other => panic!("expected SessionEnded, got {:?}", other),
        }

        // List active — should be 0
        let resp = handle_request(&mut state, Request::Sessions { active_only: Some(true) });
        match resp {
            Response::Ok { data: ResponseData::Sessions { sessions, count } } => {
                assert_eq!(count, 0);
                assert!(sessions.is_empty());
            }
            other => panic!("expected Sessions, got {:?}", other),
        }
    }

    // ── Manas Handler Tests ──

    #[test]
    fn test_platform_store_and_list() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store a platform entry
        let resp = handle_request(&mut state, Request::StorePlatform {
            key: "os".into(),
            value: "linux".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::PlatformStored { key } } => {
                assert_eq!(key, "os");
            }
            other => panic!("expected PlatformStored, got {:?}", other),
        }

        // Store another
        handle_request(&mut state, Request::StorePlatform {
            key: "arch".into(),
            value: "x86_64".into(),
        });

        // List platform entries
        let resp = handle_request(&mut state, Request::ListPlatform);
        match resp {
            Response::Ok { data: ResponseData::PlatformList { entries } } => {
                // detect_and_store_platform may have added entries, so check ours exist
                let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
                assert!(keys.contains(&"os"), "should contain 'os', got: {:?}", keys);
                assert!(keys.contains(&"arch"), "should contain 'arch', got: {:?}", keys);
                let os_entry = entries.iter().find(|e| e.key == "os").unwrap();
                assert_eq!(os_entry.value, "linux");
            }
            other => panic!("expected PlatformList, got {:?}", other),
        }
    }

    #[test]
    fn test_identity_lifecycle() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store an identity facet
        let facet = forge_core::types::manas::IdentityFacet {
            id: "if-test-1".into(),
            agent: "forge-test".into(),
            facet: "role".into(),
            description: "memory system".into(),
            strength: 0.9,
            source: "declared".into(),
            active: true,
            created_at: "2026-04-03 12:00:00".into(),
        };
        let resp = handle_request(&mut state, Request::StoreIdentity { facet });
        match resp {
            Response::Ok { data: ResponseData::IdentityStored { id } } => {
                assert_eq!(id, "if-test-1");
            }
            other => panic!("expected IdentityStored, got {:?}", other),
        }

        // List identity for the agent
        let resp = handle_request(&mut state, Request::ListIdentity {
            agent: "forge-test".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::IdentityList { facets, count } } => {
                assert_eq!(count, 1);
                assert_eq!(facets.len(), 1);
                assert_eq!(facets[0].facet, "role");
                assert_eq!(facets[0].description, "memory system");
            }
            other => panic!("expected IdentityList, got {:?}", other),
        }

        // Deactivate
        let resp = handle_request(&mut state, Request::DeactivateIdentity {
            id: "if-test-1".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::IdentityDeactivated { id, found } } => {
                assert_eq!(id, "if-test-1");
                assert!(found);
            }
            other => panic!("expected IdentityDeactivated, got {:?}", other),
        }

        // List again — active only, should be empty
        let resp = handle_request(&mut state, Request::ListIdentity {
            agent: "forge-test".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::IdentityList { facets, count } } => {
                assert_eq!(count, 0);
                assert!(facets.is_empty());
            }
            other => panic!("expected IdentityList (empty), got {:?}", other),
        }
    }

    #[test]
    fn test_manas_health_handler() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(&mut state, Request::ManasHealth);
        match resp {
            Response::Ok {
                data: ResponseData::ManasHealthData {
                    tool_count,
                    skill_count,
                    domain_dna_count,
                    perception_unconsumed,
                    declared_count,
                    identity_facets,
                    disposition_traits,
                    experience_count,
                    embedding_count,
                    trait_names,
                    ..
                },
            } => {
                // Fresh DB: non-platform/tool counts should be 0
                // (tool_count may be non-zero from auto-detect at startup)
                let _ = tool_count;
                assert_eq!(skill_count, 0);
                assert_eq!(domain_dna_count, 0);
                assert_eq!(perception_unconsumed, 0);
                assert_eq!(declared_count, 0);
                assert_eq!(identity_facets, 0);
                assert_eq!(disposition_traits, 0);
                assert_eq!(experience_count, 0);
                assert_eq!(embedding_count, 0);
                assert!(trait_names.is_empty());
            }
            other => panic!("expected ManasHealthData, got {:?}", other),
        }
    }

    #[test]
    fn test_hlc_backfill_handler() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Insert a memory with empty hlc_timestamp directly
        state.conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id)
             VALUES ('m-nohlc', 'decision', 'No HLC', 'test', 0.9, 'active', '', '[]', '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '')",
            [],
        ).unwrap();

        let resp = handle_request(&mut state, Request::HlcBackfill);
        match resp {
            Response::Ok { data: ResponseData::HlcBackfilled { count } } => {
                assert_eq!(count, 1, "should backfill 1 memory");
            }
            other => panic!("expected HlcBackfilled, got {:?}", other),
        }

        // Second call should find 0
        let resp = handle_request(&mut state, Request::HlcBackfill);
        match resp {
            Response::Ok { data: ResponseData::HlcBackfilled { count } } => {
                assert_eq!(count, 0, "no more memories to backfill");
            }
            other => panic!("expected HlcBackfilled, got {:?}", other),
        }
    }

    #[test]
    fn test_perception_store_and_consume() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store a perception
        let perception = forge_core::types::manas::Perception {
            id: "p-test-1".into(),
            kind: forge_core::types::manas::PerceptionKind::Error,
            data: "compilation failed".into(),
            severity: forge_core::types::manas::Severity::Error,
            project: Some("forge".into()),
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        };
        let resp = handle_request(&mut state, Request::StorePerception { perception });
        match resp {
            Response::Ok { data: ResponseData::PerceptionStored { id } } => {
                assert_eq!(id, "p-test-1");
            }
            other => panic!("expected PerceptionStored, got {:?}", other),
        }

        // List unconsumed perceptions
        let resp = handle_request(&mut state, Request::ListPerceptions {
            project: None,
            limit: None,
        });
        match resp {
            Response::Ok { data: ResponseData::PerceptionList { perceptions, count } } => {
                assert_eq!(count, 1);
                assert_eq!(perceptions.len(), 1);
                assert_eq!(perceptions[0].data, "compilation failed");
                assert!(!perceptions[0].consumed);
            }
            other => panic!("expected PerceptionList, got {:?}", other),
        }

        // Consume the perception
        let resp = handle_request(&mut state, Request::ConsumePerceptions {
            ids: vec!["p-test-1".into()],
        });
        match resp {
            Response::Ok { data: ResponseData::PerceptionsConsumed { count } } => {
                assert_eq!(count, 1);
            }
            other => panic!("expected PerceptionsConsumed, got {:?}", other),
        }

        // List unconsumed again — should be empty
        let resp = handle_request(&mut state, Request::ListPerceptions {
            project: None,
            limit: None,
        });
        match resp {
            Response::Ok { data: ResponseData::PerceptionList { perceptions, count } } => {
                assert_eq!(count, 0);
                assert!(perceptions.is_empty());
            }
            other => panic!("expected PerceptionList (empty), got {:?}", other),
        }
    }

    #[test]
    fn test_tool_store_and_list() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store a tool
        let tool = forge_core::types::manas::Tool {
            id: "t-test-1".into(),
            name: "cargo".into(),
            kind: forge_core::types::manas::ToolKind::Cli,
            capabilities: vec!["build".into(), "test".into()],
            config: None,
            health: forge_core::types::manas::ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03 12:00:00".into(),
        };
        let resp = handle_request(&mut state, Request::StoreTool { tool });
        match resp {
            Response::Ok { data: ResponseData::ToolStored { id } } => {
                assert_eq!(id, "t-test-1");
            }
            other => panic!("expected ToolStored, got {:?}", other),
        }

        // List tools (includes auto-detected tools from startup + our manually stored one)
        let resp = handle_request(&mut state, Request::ListTools);
        match resp {
            Response::Ok { data: ResponseData::ToolList { tools, count } } => {
                assert!(count >= 1, "should have at least the manually stored tool");
                assert_eq!(tools.len(), count);
                // Verify our manually stored tool is present
                let manual = tools.iter().find(|t| t.id == "t-test-1");
                assert!(manual.is_some(), "manually stored tool should exist");
                let manual = manual.unwrap();
                assert_eq!(manual.name, "cargo");
                assert_eq!(manual.kind, forge_core::types::manas::ToolKind::Cli);
                assert_eq!(manual.capabilities, vec!["build", "test"]);
            }
            other => panic!("expected ToolList, got {:?}", other),
        }
    }

    // ── Event Emission Tests ──

    #[test]
    fn test_remember_emits_memory_created_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use Rust".into(),
            content: "Fast".into(),
            confidence: None,
            tags: None,
            project: None,
        });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "memory_created");
        assert_eq!(event.data["title"], "Use Rust");
        assert_eq!(event.data["memory_type"], "Decision");
    }

    #[test]
    fn test_session_register_emits_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        handle_request(&mut state, Request::RegisterSession {
            id: "s1".into(),
            agent: "claude-code".into(),
            project: None,
            cwd: None,
        });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "session_changed");
        assert_eq!(event.data["action"], "registered");
        assert_eq!(event.data["agent"], "claude-code");
        assert_eq!(event.data["id"], "s1");
    }

    #[test]
    fn test_end_session_emits_event() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register first
        handle_request(&mut state, Request::RegisterSession {
            id: "s1".into(),
            agent: "claude-code".into(),
            project: None,
            cwd: None,
        });

        let mut rx = state.events.subscribe();

        handle_request(&mut state, Request::EndSession { id: "s1".into() });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "session_changed");
        assert_eq!(event.data["action"], "ended");
        assert_eq!(event.data["id"], "s1");
    }

    #[test]
    fn test_forget_emits_memory_forgotten_event() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a memory first
        let resp = handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Temp decision".into(),
            content: "Will be forgotten".into(),
            confidence: None,
            tags: None,
            project: None,
        });
        let id = match resp {
            Response::Ok { data: ResponseData::Stored { id } } => id,
            _ => panic!("expected Stored"),
        };

        let mut rx = state.events.subscribe();

        handle_request(&mut state, Request::Forget { id: id.clone() });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "memory_forgotten");
        assert_eq!(event.data["id"], id);
    }

    #[test]
    fn test_store_perception_emits_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        let perception = forge_core::types::manas::Perception {
            id: "p-ev-1".into(),
            kind: forge_core::types::manas::PerceptionKind::Error,
            data: "test error".into(),
            severity: forge_core::types::manas::Severity::Error,
            project: None,
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        };
        handle_request(&mut state, Request::StorePerception { perception });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "perception_update");
        assert_eq!(event.data["id"], "p-ev-1");
        assert_eq!(event.data["kind"], "Error");
    }

    #[test]
    fn test_store_identity_emits_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        let facet = forge_core::types::manas::IdentityFacet {
            id: "if-ev-1".into(),
            agent: "forge-test".into(),
            facet: "role".into(),
            description: "memory system".into(),
            strength: 0.9,
            source: "declared".into(),
            active: true,
            created_at: "2026-04-03 12:00:00".into(),
        };
        handle_request(&mut state, Request::StoreIdentity { facet });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "identity_updated");
        assert_eq!(event.data["id"], "if-ev-1");
        assert_eq!(event.data["facet"], "role");
        assert_eq!(event.data["agent"], "forge-test");
    }

    // ── Layer-Filtered Recall Tests ──

    #[test]
    fn test_recall_with_layer_filter() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a memory
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use JWT auth".into(),
            content: "For security".into(),
            confidence: None,
            tags: None,
            project: None,
        });

        // Recall with layer=experience should find it
        let resp = handle_request(&mut state, Request::Recall {
            query: "JWT".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: Some("experience".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, .. } } => {
                assert!(count > 0, "should find memory in experience layer");
            }
            other => panic!("expected Memories, got {:?}", other),
        }

        // Recall with layer=declared should NOT find it
        let resp = handle_request(&mut state, Request::Recall {
            query: "JWT".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: Some("declared".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, .. } } => {
                assert_eq!(count, 0, "should not find memory in declared layer");
            }
            other => panic!("expected Memories, got {:?}", other),
        }
    }

    #[test]
    fn test_recall_layer_none_is_default_behavior() {
        let mut state = DaemonState::new(":memory:").unwrap();

        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use Postgres".into(),
            content: "For persistence".into(),
            confidence: None,
            tags: None,
            project: None,
        });

        // layer=None should behave like current (search everything)
        let resp = handle_request(&mut state, Request::Recall {
            query: "Postgres".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: None,
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, .. } } => {
                assert!(count > 0, "layer=None should find memory");
            }
            other => panic!("expected Memories, got {:?}", other),
        }
    }

    #[test]
    fn test_recall_layer_identity() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store an identity facet
        let facet = forge_core::types::manas::IdentityFacet {
            id: "if-recall-1".into(),
            agent: "forge-test".into(),
            facet: "specialty".into(),
            description: "memory system architect".into(),
            strength: 0.95,
            source: "declared".into(),
            active: true,
            created_at: "2026-04-03 12:00:00".into(),
        };
        handle_request(&mut state, Request::StoreIdentity { facet });

        // Recall with layer=identity, query matching description
        let resp = handle_request(&mut state, Request::Recall {
            query: "memory".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: Some("identity".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, results, .. } } => {
                assert!(count > 0, "should find identity facet matching 'memory'");
                assert_eq!(results[0].source, "identity");
            }
            other => panic!("expected Memories, got {:?}", other),
        }

        // Non-matching query
        let resp = handle_request(&mut state, Request::Recall {
            query: "xyzzy_nonexistent".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: Some("identity".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, .. } } => {
                assert_eq!(count, 0, "should not find anything for non-matching query");
            }
            other => panic!("expected Memories, got {:?}", other),
        }
    }

    #[test]
    fn test_recall_layer_perception() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a perception
        let perception = forge_core::types::manas::Perception {
            id: "p-recall-1".into(),
            kind: forge_core::types::manas::PerceptionKind::Error,
            data: "compilation failed in main.rs".into(),
            severity: forge_core::types::manas::Severity::Error,
            project: Some("forge".into()),
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        };
        handle_request(&mut state, Request::StorePerception { perception });

        // Recall with layer=perception
        let resp = handle_request(&mut state, Request::Recall {
            query: "compilation".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: Some("perception".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, results, .. } } => {
                assert!(count > 0, "should find perception matching 'compilation'");
                assert_eq!(results[0].source, "perception");
            }
            other => panic!("expected Memories, got {:?}", other),
        }
    }

    #[test]
    fn test_recall_layer_skill() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a skill directly
        let skill = forge_core::types::Skill {
            id: "s1".into(),
            name: "Deploy Rust".into(),
            domain: "devops".into(),
            description: "cargo build --release then scp".into(),
            steps: vec![],
            success_count: 5,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
        };
        crate::db::manas::store_skill(&state.conn, &skill).unwrap();

        // Recall with layer=skill should find it
        let resp = handle_request(&mut state, Request::Recall {
            query: "deploy".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: Some("skill".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, results, .. } } => {
                assert!(count > 0, "should find skill matching 'deploy'");
                assert_eq!(results[0].source, "skill");
            }
            other => panic!("expected Memories, got {:?}", other),
        }

        // Non-matching query
        let resp = handle_request(&mut state, Request::Recall {
            query: "xyzzy_nonexistent".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: Some("skill".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::Memories { count, .. } } => {
                assert_eq!(count, 0, "should not find anything for non-matching query");
            }
            other => panic!("expected Memories, got {:?}", other),
        }
    }

    #[test]
    fn test_compile_context_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::CompiledContext {
                        context,
                        static_prefix,
                        dynamic_suffix,
                        chars,
                        layers_used,
                    },
            } => {
                assert!(context.contains("<forge-context"), "should contain opening tag");
                assert!(chars > 0, "chars should be > 0");
                assert!(!static_prefix.is_empty(), "static_prefix should not be empty");
                assert!(!dynamic_suffix.is_empty(), "dynamic_suffix should not be empty");
                assert_eq!(layers_used, 9, "full context uses 9 layers");
            }
            other => panic!("expected CompiledContext, got {:?}", other),
        }
    }

    #[test]
    fn test_compile_context_handler_static_only() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: None,
                project: None,
                static_only: Some(true),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::CompiledContext {
                        context,
                        static_prefix,
                        dynamic_suffix,
                        layers_used,
                        ..
                    },
            } => {
                assert!(
                    context.contains("<forge-static>"),
                    "static_only should return static prefix"
                );
                assert!(!context.contains("<forge-dynamic>"), "should not contain dynamic suffix");
                assert_eq!(context, static_prefix, "context should equal static_prefix");
                assert!(dynamic_suffix.is_empty(), "dynamic_suffix should be empty");
                assert_eq!(layers_used, 4, "static only uses 4 layers");
            }
            other => panic!("expected CompiledContext, got {:?}", other),
        }
    }

    #[test]
    fn test_verify_no_file_empty_db() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(&mut state, Request::Verify { file: None });
        match resp {
            Response::Ok { data: ResponseData::VerifyResult {
                files_checked, errors, warnings, diagnostics,
            } } => {
                assert_eq!(files_checked, 0);
                assert_eq!(errors, 0);
                assert_eq!(warnings, 0);
                assert!(diagnostics.is_empty());
            }
            _ => panic!("expected VerifyResult response"),
        }
    }

    #[test]
    fn test_verify_with_file() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a diagnostic
        let d = crate::db::diagnostics::Diagnostic {
            id: "v-1".into(),
            file_path: "src/main.rs".into(),
            severity: "error".into(),
            message: "undefined variable".into(),
            source: "forge-consistency".into(),
            line: Some(10),
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        crate::db::diagnostics::store_diagnostic(&state.conn, &d).unwrap();

        let resp = handle_request(&mut state, Request::Verify {
            file: Some("src/main.rs".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::VerifyResult {
                files_checked, errors, warnings, diagnostics,
            } } => {
                assert_eq!(files_checked, 1);
                assert_eq!(errors, 1);
                assert_eq!(warnings, 0);
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(diagnostics[0].message, "undefined variable");
                assert_eq!(diagnostics[0].source, "forge-consistency");
                assert_eq!(diagnostics[0].line, Some(10));
            }
            _ => panic!("expected VerifyResult response"),
        }
    }

    #[test]
    fn test_verify_all_active_diagnostics() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store diagnostics for two files
        for (id, file, sev) in &[("d1", "src/a.rs", "error"), ("d2", "src/b.rs", "warning")] {
            let d = crate::db::diagnostics::Diagnostic {
                id: id.to_string(),
                file_path: file.to_string(),
                severity: sev.to_string(),
                message: format!("{} in {}", sev, file),
                source: "forge-consistency".into(),
                line: None,
                column: None,
                created_at: forge_core::time::now_iso(),
                expires_at: forge_core::time::now_offset(300),
            };
            crate::db::diagnostics::store_diagnostic(&state.conn, &d).unwrap();
        }

        let resp = handle_request(&mut state, Request::Verify { file: None });
        match resp {
            Response::Ok { data: ResponseData::VerifyResult {
                files_checked, errors, warnings, diagnostics,
            } } => {
                assert_eq!(files_checked, 2);
                assert_eq!(errors, 1);
                assert_eq!(warnings, 1);
                assert_eq!(diagnostics.len(), 2);
            }
            _ => panic!("expected VerifyResult response"),
        }
    }

    #[test]
    fn test_get_diagnostics() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let d = crate::db::diagnostics::Diagnostic {
            id: "gd-1".into(),
            file_path: "src/lib.rs".into(),
            severity: "warning".into(),
            message: "unused import".into(),
            source: "rust-analyzer".into(),
            line: Some(3),
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        crate::db::diagnostics::store_diagnostic(&state.conn, &d).unwrap();

        let resp = handle_request(&mut state, Request::GetDiagnostics {
            file: "src/lib.rs".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::DiagnosticList {
                diagnostics, count,
            } } => {
                assert_eq!(count, 1);
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(diagnostics[0].message, "unused import");
                assert_eq!(diagnostics[0].source, "rust-analyzer");
                assert_eq!(diagnostics[0].line, Some(3));
            }
            _ => panic!("expected DiagnosticList response"),
        }
    }

    #[test]
    fn test_get_diagnostics_empty() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(&mut state, Request::GetDiagnostics {
            file: "nonexistent.rs".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::DiagnosticList {
                diagnostics, count,
            } } => {
                assert_eq!(count, 0);
                assert!(diagnostics.is_empty());
            }
            _ => panic!("expected DiagnosticList response"),
        }
    }

    #[test]
    fn test_post_edit_check_with_cached_diagnostics() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a diagnostic for the file
        let d = crate::db::diagnostics::Diagnostic {
            id: "pe-diag-1".into(),
            file_path: "src/auth.rs".into(),
            severity: "error".into(),
            message: "3 files call validate_token()".into(),
            source: "forge-consistency".into(),
            line: None,
            column: None,
            created_at: forge_core::time::now_iso(),
            expires_at: forge_core::time::now_offset(300),
        };
        crate::db::diagnostics::store_diagnostic(&state.conn, &d).unwrap();

        let resp = handle_request(&mut state, Request::PostEditCheck {
            file: "src/auth.rs".into(),
        });
        match resp {
            Response::Ok { data: ResponseData::PostEditChecked {
                cached_diagnostics, ..
            } } => {
                assert!(!cached_diagnostics.is_empty(), "should include cached diagnostics");
                assert!(cached_diagnostics[0].contains("forge-consistency"));
                assert!(cached_diagnostics[0].contains("3 files call validate_token()"));
            }
            _ => panic!("expected PostEditChecked response"),
        }
    }

    // ── StoreEvaluation tests ──

    #[test]
    fn test_store_evaluation_creates_lessons() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let req = Request::StoreEvaluation {
            findings: vec![forge_core::protocol::EvaluationFinding {
                description: "Missing error handling in auth.rs:42".into(),
                severity: "medium".into(),
                files: vec!["src/auth.rs".into()],
                category: "bug".into(),
            }],
            project: Some("test-project".into()),
            session_id: None,
        };
        let resp = handle_request(&mut state, req);

        match resp {
            Response::Ok { data: ResponseData::EvaluationStored { lessons_created, diagnostics_created } } => {
                assert_eq!(lessons_created, 1, "should create 1 lesson");
                assert_eq!(diagnostics_created, 0, "medium severity should not create diagnostics");
            }
            other => panic!("expected EvaluationStored, got {:?}", other),
        }

        // Verify the lesson is recallable
        let recall_resp = handle_request(&mut state, Request::Recall {
            query: "Missing error handling".into(),
            memory_type: None,
            project: None,
            limit: Some(5),
            layer: None,
        });
        match recall_resp {
            Response::Ok { data: ResponseData::Memories { results, count } } => {
                assert_eq!(count, 1, "should recall exactly 1 lesson");
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].memory.valence, "negative", "bug should have negative valence");
                assert!((results[0].memory.intensity - 0.6).abs() < 0.01, "medium severity should have 0.6 intensity");
            }
            other => panic!("expected Memories, got {:?}", other),
        }
    }

    #[test]
    fn test_store_evaluation_creates_edges() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let req = Request::StoreEvaluation {
            findings: vec![forge_core::protocol::EvaluationFinding {
                description: "SQL injection risk in query builder".into(),
                severity: "high".into(),
                files: vec!["src/db/query.rs".into(), "src/db/ops.rs".into()],
                category: "security".into(),
            }],
            project: None,
            session_id: None,
        };
        handle_request(&mut state, req);

        // Verify edges were created
        let edges = ops::export_edges(&state.conn).unwrap();
        let affects_edges: Vec<_> = edges.iter()
            .filter(|e| e.2 == "affects")
            .collect();
        assert_eq!(affects_edges.len(), 2, "should create 2 affects edges (one per file)");

        // Check edge targets
        let targets: Vec<&String> = affects_edges.iter().map(|e| &e.1).collect();
        assert!(targets.contains(&&"file:src/db/query.rs".to_string()), "should have edge to file:src/db/query.rs");
        assert!(targets.contains(&&"file:src/db/ops.rs".to_string()), "should have edge to file:src/db/ops.rs");
    }

    #[test]
    fn test_store_evaluation_creates_diagnostics_for_high_severity() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let req = Request::StoreEvaluation {
            findings: vec![
                forge_core::protocol::EvaluationFinding {
                    description: "Critical: unvalidated user input".into(),
                    severity: "critical".into(),
                    files: vec!["src/api/handler.rs".into()],
                    category: "security".into(),
                },
                forge_core::protocol::EvaluationFinding {
                    description: "High: missing auth check".into(),
                    severity: "high".into(),
                    files: vec!["src/api/routes.rs".into()],
                    category: "bug".into(),
                },
            ],
            project: None,
            session_id: None,
        };
        let resp = handle_request(&mut state, req);

        match resp {
            Response::Ok { data: ResponseData::EvaluationStored { lessons_created, diagnostics_created } } => {
                assert_eq!(lessons_created, 2, "should create 2 lessons");
                assert_eq!(diagnostics_created, 2, "should create 2 diagnostics (both high+)");
            }
            other => panic!("expected EvaluationStored, got {:?}", other),
        }

        // Verify diagnostics exist and are retrievable
        let diags = crate::db::diagnostics::get_diagnostics(&state.conn, "src/api/handler.rs").unwrap();
        assert_eq!(diags.len(), 1, "should have 1 diagnostic for handler.rs");
        assert_eq!(diags[0].source, "forge-evaluator");
        assert_eq!(diags[0].severity, "critical");
        assert!(diags[0].message.contains("unvalidated user input"));

        let diags2 = crate::db::diagnostics::get_diagnostics(&state.conn, "src/api/routes.rs").unwrap();
        assert_eq!(diags2.len(), 1, "should have 1 diagnostic for routes.rs");
        assert_eq!(diags2[0].severity, "high");
    }

    #[test]
    fn test_store_evaluation_no_diagnostic_for_low_severity() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let req = Request::StoreEvaluation {
            findings: vec![
                forge_core::protocol::EvaluationFinding {
                    description: "Minor style issue: inconsistent naming".into(),
                    severity: "low".into(),
                    files: vec!["src/utils.rs".into()],
                    category: "style".into(),
                },
                forge_core::protocol::EvaluationFinding {
                    description: "Info: consider using const".into(),
                    severity: "info".into(),
                    files: vec!["src/config.rs".into()],
                    category: "style".into(),
                },
            ],
            project: None,
            session_id: None,
        };
        let resp = handle_request(&mut state, req);

        match resp {
            Response::Ok { data: ResponseData::EvaluationStored { lessons_created, diagnostics_created } } => {
                assert_eq!(lessons_created, 2, "should create 2 lessons even for low severity");
                assert_eq!(diagnostics_created, 0, "should NOT create diagnostics for low/info severity");
            }
            other => panic!("expected EvaluationStored, got {:?}", other),
        }

        // Double-check no diagnostics in DB
        let diags = crate::db::diagnostics::get_diagnostics(&state.conn, "src/utils.rs").unwrap();
        assert_eq!(diags.len(), 0, "no diagnostics for low-severity findings");
        let diags2 = crate::db::diagnostics::get_diagnostics(&state.conn, "src/config.rs").unwrap();
        assert_eq!(diags2.len(), 0, "no diagnostics for info-severity findings");
    }

    #[test]
    fn test_store_evaluation_good_pattern_positive_valence() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let req = Request::StoreEvaluation {
            findings: vec![forge_core::protocol::EvaluationFinding {
                description: "Excellent error handling with context propagation".into(),
                severity: "info".into(),
                files: vec!["src/error.rs".into()],
                category: "good_pattern".into(),
            }],
            project: None,
            session_id: None,
        };
        let resp = handle_request(&mut state, req);

        match resp {
            Response::Ok { data: ResponseData::EvaluationStored { lessons_created, .. } } => {
                assert_eq!(lessons_created, 1);
            }
            other => panic!("expected EvaluationStored, got {:?}", other),
        }

        // Verify positive valence
        let recall_resp = handle_request(&mut state, Request::Recall {
            query: "error handling context propagation".into(),
            memory_type: None,
            project: None,
            limit: Some(5),
            layer: None,
        });
        match recall_resp {
            Response::Ok { data: ResponseData::Memories { results, count } } => {
                assert_eq!(count, 1);
                assert_eq!(results[0].memory.valence, "positive", "good_pattern should have positive valence");
            }
            other => panic!("expected Memories, got {:?}", other),
        }
    }
}
