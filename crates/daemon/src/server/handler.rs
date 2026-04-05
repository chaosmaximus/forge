use crate::claude_memory;
use crate::db::{ops, schema};
use crate::events::EventSender;
use crate::recall::hybrid_recall;
use forge_core::protocol::*;
use forge_core::types::{Memory, CodeFile, CodeSymbol};
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Instant;

pub struct DaemonState {
    pub conn: Connection,
    pub events: EventSender,
    pub started_at: Instant,
    pub hlc: Arc<crate::sync::Hlc>,
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
        // Enable WAL mode + busy timeout for concurrent multi-connection writes
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        schema::create_schema(&conn)?;

        // v2.0: Ensure default organization and local user exist
        if let Err(e) = crate::db::ops::ensure_defaults(&conn) {
            eprintln!("[daemon] WARN: ensure_defaults failed: {e}");
        }

        // Best-effort: detect and store platform info (OS, arch, shell, etc.)
        if let Err(e) = crate::db::manas::detect_and_store_platform(&conn) {
            eprintln!("[daemon] WARN: failed to detect/store platform info: {e}");
        }

        // Best-effort: detect and store available CLI tools
        let tools_discovered = crate::db::manas::detect_and_store_tools(&conn).unwrap_or(0);

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

        // Auto-cleanup sessions older than 24h that are still ACTIVE (leaked sessions)
        match crate::sessions::cleanup_stale_sessions(&conn) {
            Ok(n) if n > 0 => eprintln!("[daemon] auto-ended {} stale sessions (>24h active)", n),
            Ok(_) => {},
            Err(e) => eprintln!("[daemon] stale session cleanup error: {e}"),
        }

        let node_id = crate::sync::generate_node_id();
        let hlc = crate::sync::Hlc::new(&node_id);

        // Backfill HLC timestamps on existing memories that lack them
        match crate::sync::backfill_hlc(&conn, &hlc) {
            Ok(count) if count > 0 => eprintln!("[daemon] backfilled HLC timestamps on {} existing memories", count),
            Ok(_) => {},
            Err(e) => eprintln!("[daemon] WARN: HLC backfill failed: {e} — sync may be unreliable"),
        }

        // NOTE: Consolidation + project ingestion moved to background task
        // (spawned after socket server starts) to avoid blocking socket startup.
        // See main.rs `spawn_startup_tasks()`.

        let events = crate::events::create_event_bus();

        // Emit tool_discovered event for tools found during startup
        if tools_discovered > 0 {
            crate::events::emit(&events, "tool_discovered", serde_json::json!({
                "count": tools_discovered,
                "source": "startup_scan",
            }));
        }

        Ok(DaemonState {
            conn,
            events,
            started_at: Instant::now(),
            hlc: Arc::new(hlc),
            diagnostics_tx: None,
        })
    }

    /// Create a write-capable state that shares resources (event bus, HLC,
    /// started_at) with the primary state. Opens its OWN read-write SQLite
    /// connection to the same db_path. SQLite WAL mode serializes writes
    /// internally, so no application-level mutex is needed between connections.
    ///
    /// Used by the WriterActor so it has an independent write connection that
    /// is never blocked by workers holding the Arc<Mutex<DaemonState>>.
    pub fn new_writer(
        db_path: &str,
        events: EventSender,
        hlc: Arc<crate::sync::Hlc>,
        started_at: Instant,
    ) -> Result<Self, String> {
        // Must init sqlite-vec extension before opening any connection
        crate::db::vec::init_sqlite_vec();

        let conn = if db_path == ":memory:" {
            Connection::open_in_memory()
                .map_err(|e| format!("open in-memory db for writer: {e}"))?
        } else {
            Connection::open(db_path)
                .map_err(|e| format!("open writer db: {e}"))?
        };
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| format!("set WAL mode: {e}"))?;
        // Ensure schema exists on this connection (idempotent)
        schema::create_schema(&conn)
            .map_err(|e| format!("create schema for writer: {e}"))?;
        Ok(Self {
            conn,
            events,
            hlc,
            started_at,
            diagnostics_tx: None,
        })
    }

    /// Create a read-only state for serving read requests on a per-connection
    /// basis. No schema creation, no workers, no platform detection -- just a
    /// read-only SQLite connection for queries.
    ///
    /// Shares the event bus, HLC, and started_at from the write state so that
    /// read handlers (e.g. CompileContext, GuardrailsCheck) can emit events
    /// and Status can report uptime.
    pub fn new_reader(
        db_path: &str,
        events: EventSender,
        hlc: Arc<crate::sync::Hlc>,
        started_at: Instant,
    ) -> Result<Self, String> {
        // Must init sqlite-vec extension before opening any connection
        crate::db::vec::init_sqlite_vec();

        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("open read-only db: {e}"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;").ok();
        Ok(Self {
            conn,
            events,
            hlc,
            started_at,
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
            let is_decision = matches!(memory_type, forge_core::types::MemoryType::Decision);
            let title_clone = title.clone();
            let mut memory = Memory::new(memory_type, title, content);
            if let Some(c) = confidence {
                memory = memory.with_confidence(c);
            }
            if let Some(t) = tags {
                memory = memory.with_tags(t);
            }
            if let Some(ref p) = project {
                memory = memory.with_project(p.clone());
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

                    // Cross-session perception: when a decision is stored and there are
                    // multiple active sessions, create a subtle perception so other sessions
                    // become aware. Only for decisions (important enough to notify).
                    if is_decision {
                        let active_count = crate::sessions::list_sessions(&state.conn, true)
                            .map(|s| s.len())
                            .unwrap_or(0);
                        if active_count > 1 {
                            let perception = forge_core::types::manas::Perception {
                                id: format!("xsession-{}", ulid::Ulid::new()),
                                kind: forge_core::types::manas::PerceptionKind::CrossSessionDecision,
                                data: format!("Another session stored decision: {}", title_clone),
                                severity: forge_core::types::manas::Severity::Info,
                                project: project.clone(),
                                created_at: forge_core::time::now_iso(),
                                expires_at: Some(forge_core::time::now_offset(600)), // 10 min TTL
                                consumed: false,
                            };
                            if let Err(e) = crate::db::manas::store_perception(&state.conn, &perception) {
                                eprintln!("[cross-session] failed to store perception: {e}");
                            }
                        }
                    }

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
                            edges: Vec::new(),
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
                                edges: Vec::new(),
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
                                user_id: None,
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
                                edges: Vec::new(),
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
                                edges: Vec::new(),
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
                                edges: Vec::new(),
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
                edges: Vec::new(),
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

        Request::RegisterSession { id, agent, project, cwd, capabilities, current_task } => {
            let agent_clone = agent.clone();
            let caps_json = capabilities.map(|c| serde_json::to_string(&c).unwrap_or_else(|_| "[]".to_string()));
            match crate::sessions::register_session(&state.conn, &id, &agent, project.as_deref(), cwd.as_deref(), caps_json.as_deref(), current_task.as_deref()) {
                Ok(()) => {
                    // Auto-detect reality from cwd and tag the session
                    if let Some(ref cwd_path) = cwd {
                        use crate::reality::CodeRealityEngine;
                        use forge_core::types::reality_engine::RealityEngine;

                        let engine = CodeRealityEngine;
                        let path = std::path::Path::new(cwd_path);
                        if let Some(detection) = engine.detect(path) {
                            // Check if reality already exists for this path
                            let reality_id = match ops::get_reality_by_path(&state.conn, cwd_path, "default") {
                                Ok(Some(existing)) => Some(existing.id),
                                Ok(None) => {
                                    // Create a new reality
                                    let rid = ulid::Ulid::new().to_string();
                                    let now = chrono_now();
                                    let name = path
                                        .file_name()
                                        .map(|n| n.to_string_lossy().to_string())
                                        .unwrap_or_else(|| detection.domain.clone());
                                    let metadata_str = serde_json::to_string(&detection.metadata)
                                        .unwrap_or_else(|_| "{}".to_string());
                                    let reality = forge_core::types::Reality {
                                        id: rid.clone(),
                                        name,
                                        reality_type: detection.reality_type,
                                        detected_from: Some(detection.detected_from),
                                        project_path: Some(cwd_path.clone()),
                                        domain: Some(detection.domain),
                                        organization_id: "default".to_string(),
                                        owner_type: "user".to_string(),
                                        owner_id: "local".to_string(),
                                        engine_status: "detected".to_string(),
                                        engine_pid: None,
                                        created_at: now.clone(),
                                        last_active: now,
                                        metadata: metadata_str,
                                    };
                                    match ops::store_reality(&state.conn, &reality) {
                                        Ok(()) => Some(rid),
                                        Err(e) => {
                                            eprintln!("[handler] auto-detect: failed to store reality for {}: {e}", cwd_path);
                                            None
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[handler] auto-detect: failed to check reality for {}: {e}", cwd_path);
                                    None
                                }
                            };

                            // Tag the session with the detected reality_id (best-effort)
                            if let Some(ref rid) = reality_id {
                                let _ = state.conn.execute(
                                    "UPDATE session SET reality_id = ?1 WHERE id = ?2",
                                    rusqlite::params![rid, id],
                                );
                            }
                        }
                    }

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
            if let Err(e) = crate::sessions::save_working_set(&state.conn, &id) {
                eprintln!("[handler] failed to save working set for session {}: {e}", id);
            }

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
                        let caps: Vec<String> = serde_json::from_str(&s.capabilities).unwrap_or_default();
                        forge_core::protocol::SessionInfo {
                            id: s.id, agent: s.agent, project: s.project,
                            cwd: s.cwd, started_at: s.started_at,
                            ended_at: s.ended_at, status: s.status,
                            capabilities: caps,
                            current_task: s.current_task,
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

        Request::CleanupSessions { prefix } => {
            match crate::sessions::cleanup_sessions(&state.conn, prefix.as_deref()) {
                Ok(ended) => {
                    eprintln!("[sessions] cleanup: ended {} sessions (prefix: {:?})", ended, prefix);
                    crate::events::emit(&state.events, "session_changed", serde_json::json!({
                        "action": "cleanup",
                        "ended": ended,
                        "prefix": prefix,
                    }));
                    Response::Ok {
                        data: ResponseData::SessionsCleaned { ended },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("cleanup_sessions failed: {e}"),
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
                    cluster_name: br.cluster_name,
                    cluster_files: br.cluster_files,
                    calling_files: br.calling_files,
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
            let tool_name = tool.name.clone();
            match crate::db::manas::store_tool(&state.conn, &tool) {
                Ok(()) => {
                    crate::events::emit(&state.events, "tool_discovered", serde_json::json!({
                        "id": id,
                        "name": tool_name,
                        "source": "manual",
                    }));
                    Response::Ok {
                        data: ResponseData::ToolStored { id },
                    }
                }
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
            let lim = limit.unwrap_or(20).min(100); // Cap at 100
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
            // v2.0: tag identity facets with current user (forge_user.id, not raw OS username)
            if facet.user_id.is_none() {
                let login = std::env::var("USER").unwrap_or_else(|_| "local".into());
                facet.user_id = ops::get_user(&state.conn, &login)
                    .ok()
                    .flatten()
                    .map(|u| u.id)
                    .or_else(|| Some("local".into()));
            }
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
            // Use list_identity_for_user to include user-scoped facets.
            // Default to "local" user (single-user daemon); the fallback path in
            // list_identity_for_user(None, ...) delegates to plain list_identity.
            let user_id = ops::get_user(&state.conn, "local")
                .ok()
                .flatten()
                .map(|u| u.id);
            match crate::db::manas::list_identity_for_user(&state.conn, user_id.as_deref(), &agent, true) {
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

        Request::ManasHealth { project } => {
            match crate::db::manas::manas_health(&state.conn) {
                Ok(mh) => {
                    let is_new = if let Some(ref proj) = project {
                        crate::db::manas::is_new_project(&state.conn, proj)
                            .unwrap_or_else(|e| {
                                eprintln!("[manas_health] is_new_project failed: {e}");
                                false
                            })
                    } else {
                        false
                    };
                    Response::Ok {
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
                            is_new_project: is_new,
                        },
                    }
                },
                Err(e) => Response::Error {
                    message: format!("manas_health failed: {e}"),
                },
            }
        }

        Request::CompileContext { agent, project, static_only, excluded_layers } => {
            let agent_name = agent.as_deref().unwrap_or("claude-code");
            let excluded = excluded_layers.unwrap_or_default();
            let static_prefix = crate::recall::compile_static_prefix(&state.conn, agent_name);

            if static_only.unwrap_or(false) {
                let chars = static_prefix.len();
                // Emit context_compiled event
                crate::events::emit(&state.events, "context_compiled", serde_json::json!({
                    "static_chars": chars,
                    "dynamic_chars": 0,
                    "total_chars": chars,
                    "static_only": true,
                }));
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
                let config = crate::config::load_config();
                let ctx_config = config.context.validated();
                let dynamic_suffix = crate::recall::compile_dynamic_suffix(
                    &state.conn, agent_name, project.as_deref(), &ctx_config, &excluded,
                );
                let full = format!(
                    "<forge-context version=\"0.7.0\">\n{}\n{}\n</forge-context>",
                    static_prefix, dynamic_suffix
                );
                let chars = full.len();
                // Emit context_compiled event
                crate::events::emit(&state.events, "context_compiled", serde_json::json!({
                    "static_chars": static_prefix.len(),
                    "dynamic_chars": dynamic_suffix.len(),
                    "total_chars": chars,
                    "layers_used": 9,
                }));
                // Emit prefetch_loaded event if prefetch hints were generated
                let prefetch_hints = crate::recall::compile_prefetch_hints(
                    &state.conn, agent_name, project.as_deref(), 5,
                );
                if !prefetch_hints.is_empty() {
                    crate::events::emit(&state.events, "prefetch_loaded", serde_json::json!({
                        "hints_count": prefetch_hints.len(),
                        "hints": prefetch_hints,
                    }));
                }
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

        Request::CompileContextTrace { agent, project } => {
            let agent_name = agent.as_deref().unwrap_or("claude-code");
            let trace_config = crate::config::load_config();
            let trace_ctx_config = trace_config.context.validated();
            let trace = crate::recall::compile_context_trace(
                &state.conn, agent_name, project.as_deref(), &trace_ctx_config,
            );
            Response::Ok {
                data: ResponseData::ContextTrace {
                    considered: trace.considered,
                    included: trace.included,
                    excluded: trace.excluded,
                    budget_total: trace.budget_total,
                    budget_used: trace.budget_used,
                    layer_chars: trace.layer_chars,
                },
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
        Request::Bootstrap { project } => {
            let adapters = crate::adapters::detect_adapters();
            let result = crate::bootstrap::run_bootstrap(
                &state.conn,
                &adapters,
                project.as_deref(),
            );
            Response::Ok {
                data: ResponseData::BootstrapComplete {
                    files_processed: result.files_processed,
                    files_skipped: result.files_skipped,
                    memories_extracted: result.memories_extracted,
                    errors: result.errors,
                },
            }
        }
        Request::ForceConsolidate => {
            let consol_config = crate::config::load_config().consolidation.validated();
            let stats = crate::workers::consolidator::run_all_phases(&state.conn, &consol_config);
            Response::Ok {
                data: ResponseData::ConsolidationComplete {
                    exact_dedup: stats.exact_dedup,
                    semantic_dedup: stats.semantic_dedup,
                    linked: stats.linked,
                    faded: stats.faded,
                    promoted: stats.promoted,
                    reconsolidated: stats.reconsolidated,
                    embedding_merged: stats.embedding_merged,
                    strengthened: stats.strengthened,
                    contradictions: stats.contradictions,
                    entities_detected: stats.entities_detected,
                    synthesized: stats.synthesized,
                    gaps_detected: stats.gaps_detected,
                    reweaved: stats.reweaved,
                    scored: stats.scored,
                },
            }
        }

        Request::ForceExtract => {
            let adapters_list = crate::adapters::detect_adapters();
            let all_files = crate::bootstrap::scan_transcripts(&adapters_list);
            let mut files_queued = 0usize;
            for (path, _adapter) in &all_files {
                let hash = match crate::bootstrap::compute_content_hash(path) {
                    Ok(h) => h,
                    Err(_) => continue,
                };
                let (needs_work, _) = crate::bootstrap::needs_processing(&state.conn, path, &hash);
                if needs_work {
                    files_queued += 1;
                }
            }
            eprintln!("[extract] force-extract: {} files need processing", files_queued);
            Response::Ok {
                data: ResponseData::ExtractionTriggered { files_queued },
            }
        }

        Request::ExtractWithProvider { provider, model, text } => {
            let config = crate::config::load_config();
            let model_name = model.unwrap_or_else(|| match provider.as_str() {
                "ollama" => config.extraction.ollama.model.clone(),
                "claude" | "claude_cli" => config.extraction.claude.model.clone(),
                "claude_api" => config.extraction.claude_api.model.clone(),
                "openai" => config.extraction.openai.model.clone(),
                "gemini" => config.extraction.gemini.model.clone(),
                _ => "unknown".into(),
            });

            let start = std::time::Instant::now();

            // Parse text through the extraction output parser to preview what would be extracted.
            // This is a synchronous preview — actual provider-specific extraction happens
            // via the background worker (which is async). This endpoint validates the text
            // and shows what CAN be extracted without making an API call.
            let memories = crate::extraction::parse_extraction_output(&text);
            let latency = start.elapsed().as_millis() as u64;

            Response::Ok {
                data: ResponseData::ExtractionResult {
                    provider: provider.clone(),
                    model: model_name,
                    memories_extracted: memories.len(),
                    tokens_in_estimate: text.len() / 4,
                    tokens_out_estimate: 0,
                    latency_ms: latency,
                },
            }
        }

        Request::GetConfig => {
            let config = crate::config::load_config();
            // SECURITY: never expose API keys — just show if they're set
            let claude_api_key_set = crate::config::resolve_api_key(
                &config.extraction.claude_api.api_key, "ANTHROPIC_API_KEY"
            ).is_some();
            let openai_key_set = crate::config::resolve_api_key(
                &config.extraction.openai.api_key, "OPENAI_API_KEY"
            ).is_some();
            let gemini_key_set = crate::config::resolve_api_key(
                &config.extraction.gemini.api_key, "GEMINI_API_KEY"
            ).is_some();
            Response::Ok {
                data: ResponseData::ConfigData {
                    backend: config.extraction.backend.clone(),
                    ollama_model: config.extraction.ollama.model.clone(),
                    ollama_endpoint: config.extraction.ollama.endpoint.clone(),
                    claude_cli_model: config.extraction.claude.model.clone(),
                    claude_api_model: config.extraction.claude_api.model.clone(),
                    claude_api_key_set,
                    openai_model: config.extraction.openai.model.clone(),
                    openai_endpoint: config.extraction.openai.endpoint.clone(),
                    openai_key_set,
                    gemini_model: config.extraction.gemini.model.clone(),
                    gemini_key_set,
                    embedding_model: config.embedding.model.clone(),
                },
            }
        }

        Request::SetConfig { key, value } => {
            match crate::config::update_config(&key, &value) {
                Ok(()) => {
                    // SECURITY: mask API key values in logs
                    let log_value = if key.contains("api_key") || key.contains("secret") {
                        "****".to_string()
                    } else {
                        value.clone()
                    };
                    eprintln!("[config] updated {} = {}", key, log_value);
                    Response::Ok {
                        data: ResponseData::ConfigUpdated { key, value },
                    }
                }
                Err(e) => {
                    eprintln!("[config] ERROR: failed to update {}: {}", key, e);
                    Response::Error {
                        message: format!("config update failed: {e}"),
                    }
                }
            }
        }

        Request::GetStats { hours } => {
            let h = hours.unwrap_or(24);
            match crate::db::ops::query_stats(&state.conn, h) {
                Ok(stats) => Response::Ok {
                    data: ResponseData::Stats {
                        period_hours: stats.period_hours,
                        extractions: stats.extractions,
                        extraction_errors: stats.extraction_errors,
                        tokens_in: stats.tokens_in,
                        tokens_out: stats.tokens_out,
                        total_cost_usd: stats.total_cost_usd,
                        avg_latency_ms: stats.avg_latency_ms,
                        memories_created: stats.memories_created,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("stats query failed: {e}"),
                },
            }
        }

        Request::GetGraphData { layer, limit } => {
            let max = limit.unwrap_or(50);
            match ops::get_graph_data(&state.conn, layer.as_deref(), max) {
                Ok((nodes, edges)) => {
                    let total_nodes = nodes.len();
                    let total_edges = edges.len();
                    Response::Ok {
                        data: ResponseData::GraphData { nodes, edges, total_nodes, total_edges },
                    }
                }
                Err(e) => {
                    eprintln!("[handler] ERROR: graph query failed: {e}");
                    Response::Error { message: format!("graph query failed: {e}") }
                }
            }
        }

        Request::BatchRecall { queries } => {
            let mut all_results = Vec::new();
            for q in &queries {
                let lim = q.limit.unwrap_or(5);
                let results = hybrid_recall(
                    &state.conn, &q.text, None, q.memory_type.as_ref(), None, lim,
                );
                all_results.push(results);
            }
            Response::Ok {
                data: ResponseData::BatchRecallResults { results: all_results },
            }
        }

        // ── A2A Inter-Session Protocol (FISP) ──

        Request::SessionSend { to, kind, topic, parts, project, timeout_secs, meeting_id } => {
            // A2A permission enforcement
            let config = crate::config::load_config();
            if !config.a2a.enabled {
                return Response::Error { message: "A2A messaging is disabled".into() };
            }

            let from = "api";

            // In controlled mode, check permissions before sending
            if config.a2a.trust == "controlled" {
                // Get sender agent type (from session if available, else "api")
                let from_agent = "api";
                let from_project: Option<String> = None;

                // Get recipient agent type and project
                let (to_agent, to_proj) = if to == "*" {
                    // Broadcast: use wildcard for permission check
                    ("*".to_string(), project.clone())
                } else {
                    // Look up recipient session to get agent type
                    match crate::sessions::get_session(&state.conn, &to) {
                        Ok(Some(session)) => (session.agent.clone(), session.project.clone()),
                        _ => (to.clone(), project.clone()),
                    }
                };

                if !crate::sessions::check_a2a_permission(
                    &state.conn,
                    &config.a2a.trust,
                    from_agent,
                    &to_agent,
                    from_project.as_deref(),
                    to_proj.as_deref(),
                ) {
                    return Response::Error {
                        message: format!("A2A permission denied: {} -> {}", from_agent, to_agent),
                    };
                }
            }

            let parts_json = serde_json::to_string(&parts).unwrap_or_else(|_| "[]".to_string());
            match crate::sessions::send_message(&state.conn, from, &to, &kind, &topic, &parts_json, project.as_deref(), timeout_secs, meeting_id.as_deref()) {
                Ok(id) => {
                    crate::events::emit(&state.events, "session_message", serde_json::json!({
                        "id": &id, "from": from, "to": &to, "kind": &kind, "topic": &topic,
                    }));
                    // Emit message_received event for subscribe filtering
                    let preview: String = parts_json.chars().take(100).collect();
                    crate::events::emit(&state.events, "message_received", serde_json::json!({
                        "to_session": &to,
                        "from_session": from,
                        "topic": &topic,
                        "preview": preview,
                    }));
                    // If this is a meeting response, auto-record it
                    if let Some(ref mid) = meeting_id {
                        let confidence = None; // Could be extracted from parts in future
                        if let Ok(all_responded) = crate::teams::record_meeting_response(
                            &state.conn, mid, from, &parts_json, confidence,
                        ) {
                            crate::events::emit(&state.events, "meeting_response", serde_json::json!({
                                "meeting_id": mid, "session_id": from, "topic": &topic,
                            }));
                            if all_responded {
                                crate::events::emit(&state.events, "meeting_all_responded", serde_json::json!({
                                    "meeting_id": mid,
                                }));
                            }
                        }
                    }
                    Response::Ok { data: ResponseData::MessageSent { id, status: "pending".into() } }
                }
                Err(e) => Response::Error { message: format!("send_message failed: {e}") },
            }
        }

        Request::SessionRespond { message_id, status, parts } => {
            let from = "api";
            let parts_json = serde_json::to_string(&parts).unwrap_or_else(|_| "[]".to_string());
            match crate::sessions::respond_to_message(&state.conn, &message_id, from, &status, &parts_json) {
                Ok(found) => {
                    if !found {
                        eprintln!("[handler] respond_to_message: original message {} not found", message_id);
                    }
                    crate::events::emit(&state.events, "session_message", serde_json::json!({
                        "message_id": &message_id, "status": &status, "action": "responded",
                    }));
                    Response::Ok { data: ResponseData::MessageResponded { id: message_id, status } }
                }
                Err(e) => Response::Error { message: format!("respond_to_message failed: {e}") },
            }
        }

        Request::SessionMessages { session_id, status, limit } => {
            match crate::sessions::list_messages(&state.conn, &session_id, status.as_deref(), limit.unwrap_or(20)) {
                Ok(rows) => {
                    let messages: Vec<forge_core::protocol::SessionMessage> = rows.into_iter().map(|r| {
                        let parts: Vec<forge_core::protocol::request::MessagePart> = serde_json::from_str(&r.parts).unwrap_or_default();
                        forge_core::protocol::SessionMessage {
                            id: r.id, from_session: r.from_session, to_session: r.to_session,
                            kind: r.kind, topic: r.topic, parts, status: r.status,
                            in_reply_to: r.in_reply_to, project: r.project,
                            created_at: r.created_at, delivered_at: r.delivered_at,
                        }
                    }).collect();
                    let count = messages.len();
                    Response::Ok { data: ResponseData::SessionMessageList { messages, count } }
                }
                Err(e) => Response::Error { message: format!("list_messages failed: {e}") },
            }
        }

        Request::SessionAck { message_ids, session_id } => {
            let result = if let Some(sid) = session_id {
                // Scoped ack: only messages addressed to this session
                crate::sessions::ack_messages(&state.conn, &message_ids, &sid)
            } else {
                // Admin/CLI ack: ack regardless of to_session
                crate::sessions::ack_messages_admin(&state.conn, &message_ids)
            };
            match result {
                Ok(count) => Response::Ok { data: ResponseData::MessagesAcked { count } },
                Err(e) => Response::Error { message: format!("ack_messages failed: {e}") },
            }
        }

        Request::ListEntities { project, limit } => {
            let lim = limit.unwrap_or(50).min(200);
            match crate::db::manas::list_entities(&state.conn, project.as_deref(), lim) {
                Ok(entities) => {
                    let count = entities.len();
                    Response::Ok {
                        data: ResponseData::EntityList { entities, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_entities failed: {e}"),
                },
            }
        }

        // ── A2A Permission Management ──

        Request::GrantPermission { from_agent, to_agent, from_project, to_project } => {
            match crate::sessions::grant_a2a_permission(
                &state.conn, &from_agent, &to_agent,
                from_project.as_deref(), to_project.as_deref(),
            ) {
                Ok(id) => Response::Ok { data: ResponseData::PermissionGranted { id } },
                Err(e) => Response::Error { message: format!("grant_permission failed: {e}") },
            }
        }

        Request::RevokePermission { id } => {
            match crate::sessions::revoke_a2a_permission(&state.conn, &id) {
                Ok(found) => Response::Ok { data: ResponseData::PermissionRevoked { id, found } },
                Err(e) => Response::Error { message: format!("revoke_permission failed: {e}") },
            }
        }

        Request::ListPermissions => {
            match crate::sessions::list_a2a_permissions(&state.conn) {
                Ok(permissions) => {
                    let count = permissions.len();
                    Response::Ok { data: ResponseData::PermissionList { permissions, count } }
                }
                Err(e) => Response::Error { message: format!("list_permissions failed: {e}") },
            }
        }

        // ── Scoped Configuration ──

        Request::SetScopedConfig { scope_type, scope_id, key, value, locked, ceiling } => {
            if !ops::validate_scope_type(&scope_type) {
                return Response::Error {
                    message: format!("invalid scope_type '{}': must be one of session, agent, reality, user, team, organization", scope_type),
                };
            }
            match ops::set_scoped_config(&state.conn, &scope_type, &scope_id, &key, &value, locked, ceiling, "user") {
                Ok(()) => Response::Ok {
                    data: ResponseData::ScopedConfigSet { scope_type, scope_id, key },
                },
                Err(e) => Response::Error {
                    message: format!("set_scoped_config failed: {e}"),
                },
            }
        }

        Request::DeleteScopedConfig { scope_type, scope_id, key } => {
            if !ops::validate_scope_type(&scope_type) {
                return Response::Error {
                    message: format!("invalid scope_type '{}': must be one of session, agent, reality, user, team, organization", scope_type),
                };
            }
            match ops::delete_scoped_config(&state.conn, &scope_type, &scope_id, &key) {
                Ok(deleted) => Response::Ok {
                    data: ResponseData::ScopedConfigDeleted { deleted },
                },
                Err(e) => Response::Error {
                    message: format!("delete_scoped_config failed: {e}"),
                },
            }
        }

        Request::ListScopedConfig { scope_type, scope_id } => {
            if !ops::validate_scope_type(&scope_type) {
                return Response::Error {
                    message: format!("invalid scope_type '{}': must be one of session, agent, reality, user, team, organization", scope_type),
                };
            }
            match ops::list_scoped_config(&state.conn, &scope_type, &scope_id) {
                Ok(entries) => Response::Ok {
                    data: ResponseData::ScopedConfigList { entries },
                },
                Err(e) => Response::Error {
                    message: format!("list_scoped_config failed: {e}"),
                },
            }
        }

        Request::GetEffectiveConfig { session_id, agent, reality_id, user_id, team_id, organization_id } => {
            match ops::resolve_effective_config(
                &state.conn,
                session_id.as_deref(),
                agent.as_deref(),
                reality_id.as_deref(),
                user_id.as_deref(),
                team_id.as_deref(),
                organization_id.as_deref(),
            ) {
                Ok(config) => Response::Ok {
                    data: ResponseData::EffectiveConfig { config },
                },
                Err(e) => Response::Error {
                    message: format!("resolve_effective_config failed: {e}"),
                },
            }
        }

        Request::DetectReality { path } => {
            use crate::reality::CodeRealityEngine;
            use forge_core::types::reality_engine::RealityEngine;
            use std::path::Path;

            let engine = CodeRealityEngine;
            let project_path = Path::new(&path);

            match engine.detect(project_path) {
                Some(detection) => {
                    // Check if a reality already exists for this path
                    match ops::get_reality_by_path(&state.conn, &path, "default") {
                        Ok(Some(existing)) => {
                            Response::Ok {
                                data: ResponseData::RealityDetected {
                                    reality_id: existing.id,
                                    name: existing.name,
                                    reality_type: existing.reality_type,
                                    domain: existing.domain.unwrap_or_default(),
                                    detected_from: existing.detected_from.unwrap_or_default(),
                                    confidence: detection.confidence,
                                    is_new: false,
                                    metadata: serde_json::from_str(&existing.metadata)
                                        .unwrap_or_else(|_| serde_json::json!({})),
                                },
                            }
                        }
                        Ok(None) => {
                            // Create a new reality record
                            let reality_id = ulid::Ulid::new().to_string();
                            let now = chrono_now();
                            let name = project_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| detection.domain.clone());
                            let metadata_str = serde_json::to_string(&detection.metadata)
                                .unwrap_or_else(|_| "{}".to_string());

                            let reality = forge_core::types::Reality {
                                id: reality_id.clone(),
                                name: name.clone(),
                                reality_type: detection.reality_type.clone(),
                                detected_from: Some(detection.detected_from.clone()),
                                project_path: Some(path),
                                domain: Some(detection.domain.clone()),
                                organization_id: "default".to_string(),
                                owner_type: "user".to_string(),
                                owner_id: "local".to_string(),
                                engine_status: "detected".to_string(),
                                engine_pid: None,
                                created_at: now.clone(),
                                last_active: now,
                                metadata: metadata_str,
                            };

                            match ops::store_reality(&state.conn, &reality) {
                                Ok(()) => {
                                    crate::events::emit(&state.events, "reality_detected", serde_json::json!({
                                        "reality_id": reality_id,
                                        "domain": detection.domain,
                                        "reality_type": detection.reality_type,
                                    }));
                                    Response::Ok {
                                        data: ResponseData::RealityDetected {
                                            reality_id,
                                            name,
                                            reality_type: detection.reality_type,
                                            domain: detection.domain,
                                            detected_from: detection.detected_from,
                                            confidence: detection.confidence,
                                            is_new: true,
                                            metadata: detection.metadata,
                                        },
                                    }
                                }
                                Err(e) => Response::Error {
                                    message: format!("failed to store reality: {e}"),
                                },
                            }
                        }
                        Err(e) => Response::Error {
                            message: format!("failed to check existing reality: {e}"),
                        },
                    }
                }
                None => Response::Error {
                    message: format!("no reality engine can handle path: {path}"),
                },
            }
        }

        // ── Cross-Engine Queries (v2.0 Wave 3) ──

        Request::CrossEngineQuery { file, reality_id: _reality_id } => {
            // 1. Look up symbols for the file from code_symbol table
            let symbols: Vec<serde_json::Value> = state.conn.prepare(
                "SELECT name, kind, line_start, line_end FROM code_symbol WHERE file_path = ?1"
            ).and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![file], |row| {
                    Ok(serde_json::json!({
                        "name": row.get::<_, String>(0)?,
                        "kind": row.get::<_, String>(1)?,
                        "line_start": row.get::<_, Option<i64>>(2)?,
                        "line_end": row.get::<_, Option<i64>>(3)?,
                    }))
                })?.collect()
            }).unwrap_or_default();

            // 2. Look up callers from edge table (edge_type='calls', to_id contains file path)
            let calling_files: Vec<String> = state.conn.prepare(
                "SELECT DISTINCT from_id FROM edge WHERE edge_type = 'calls' AND to_id = ?1"
            ).and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![file], |row| row.get(0))?.collect()
            }).unwrap_or_default();
            let callers = calling_files.len();

            // 3. Look up cluster from edge table (edge_type='belongs_to_cluster')
            let cluster: Option<String> = state.conn.query_row(
                "SELECT to_id FROM edge WHERE edge_type = 'belongs_to_cluster' AND from_id = ?1 LIMIT 1",
                rusqlite::params![file],
                |row| row.get(0),
            ).ok();

            // 3b. Other files in the same cluster
            let cluster_files: Vec<String> = if let Some(ref cid) = cluster {
                state.conn.prepare(
                    "SELECT from_id FROM edge WHERE edge_type = 'belongs_to_cluster' AND to_id = ?1 AND from_id != ?2"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![cid, file], |row| row.get(0))?.collect()
                }).unwrap_or_default()
            } else {
                vec![]
            };

            // 4. Look up memories that mention this file in content or tags
            let related_memories: Vec<serde_json::Value> = state.conn.prepare(
                "SELECT id, title, memory_type FROM memory WHERE status = 'active' AND (content LIKE '%' || ?1 || '%' OR tags LIKE '%' || ?1 || '%') LIMIT 20"
            ).and_then(|mut stmt| {
                stmt.query_map(rusqlite::params![file], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "title": row.get::<_, String>(1)?,
                        "memory_type": row.get::<_, String>(2)?,
                    }))
                })?.collect()
            }).unwrap_or_default();

            Response::Ok {
                data: ResponseData::CrossEngineResult {
                    file,
                    symbols,
                    callers,
                    calling_files,
                    cluster,
                    cluster_files,
                    related_memories,
                },
            }
        }

        Request::FileMemoryMap { files, reality_id: _ } => {
            let mut mappings = std::collections::HashMap::new();
            for file in &files {
                let memory_count: usize = state.conn.query_row(
                    "SELECT COUNT(*) FROM memory WHERE status = 'active' AND (content LIKE '%' || ?1 || '%' OR tags LIKE '%' || ?1 || '%')",
                    rusqlite::params![file],
                    |row| row.get(0),
                ).unwrap_or(0);

                let decision_count: usize = state.conn.query_row(
                    "SELECT COUNT(*) FROM memory WHERE status = 'active' AND memory_type = 'decision' AND (content LIKE '%' || ?1 || '%' OR tags LIKE '%' || ?1 || '%')",
                    rusqlite::params![file],
                    |row| row.get(0),
                ).unwrap_or(0);

                let entity_names: Vec<String> = state.conn.prepare(
                    "SELECT DISTINCT name FROM entity WHERE description LIKE '%' || ?1 || '%' OR entity_type LIKE '%' || ?1 || '%' LIMIT 10"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![file], |row| row.get(0))?.collect()
                }).unwrap_or_default();

                let last_perception: Option<String> = state.conn.query_row(
                    "SELECT data FROM perception WHERE project IS NOT NULL AND data LIKE '%' || ?1 || '%' ORDER BY created_at DESC LIMIT 1",
                    rusqlite::params![file],
                    |row| row.get(0),
                ).ok();

                mappings.insert(file.clone(), response::FileMemoryInfo {
                    memory_count,
                    decision_count,
                    entity_names,
                    last_perception,
                });
            }

            Response::Ok {
                data: ResponseData::FileMemoryMapResult { mappings },
            }
        }

        Request::CodeSearch { query, kind, limit } => {
            let effective_limit = limit.unwrap_or(20).min(100);
            let pattern = format!("%{}%", query);

            let hits: Vec<serde_json::Value> = if let Some(ref kind_filter) = kind {
                state.conn.prepare(
                    "SELECT id, name, kind, file_path, line_start FROM code_symbol WHERE name LIKE ?1 AND kind = ?2 LIMIT ?3"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![pattern, kind_filter, effective_limit], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, String>(0)?,
                            "name": row.get::<_, String>(1)?,
                            "kind": row.get::<_, String>(2)?,
                            "path": row.get::<_, String>(3)?,
                            "line_start": row.get::<_, Option<i64>>(4)?,
                        }))
                    })?.collect()
                }).unwrap_or_default()
            } else {
                state.conn.prepare(
                    "SELECT id, name, kind, file_path, line_start FROM code_symbol WHERE name LIKE ?1 LIMIT ?2"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![pattern, effective_limit], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, String>(0)?,
                            "name": row.get::<_, String>(1)?,
                            "kind": row.get::<_, String>(2)?,
                            "path": row.get::<_, String>(3)?,
                            "line_start": row.get::<_, Option<i64>>(4)?,
                        }))
                    })?.collect()
                }).unwrap_or_default()
            };

            Response::Ok {
                data: ResponseData::CodeSearchResult { hits },
            }
        }

        Request::ListRealities { organization_id } => {
            let org_id = organization_id.as_deref().unwrap_or("default");
            match ops::list_realities(&state.conn, org_id) {
                Ok(realities) => Response::Ok {
                    data: ResponseData::RealitiesList { realities },
                },
                Err(e) => Response::Error {
                    message: format!("list_realities failed: {e}"),
                },
            }
        }

        Request::ForceIndex => {
            // Re-process already-indexed files: extract import edges + run clustering
            // (LSP-based symbol extraction continues on the background interval)
            let files = ops::list_code_files(&state.conn);
            let import_edges = crate::workers::indexer::extract_and_store_imports(&state.conn, &files);

            // Run clustering for any project that has a reality
            let projects: std::collections::HashSet<String> = files.iter().map(|f| f.project.clone()).collect();
            for project_dir in &projects {
                crate::workers::indexer::run_clustering(&state.conn, project_dir);
            }

            let files_indexed = files.len();
            let symbols_indexed: usize = state.conn
                .query_row("SELECT COUNT(*) FROM code_symbol", [], |r| r.get(0))
                .unwrap_or(0);

            eprintln!("[force-index] processed {} files, {} import edges, {} symbols",
                files_indexed, import_edges, symbols_indexed);

            Response::Ok {
                data: ResponseData::IndexComplete { files_indexed, symbols_indexed },
            }
        }

        // ── Agent Teams: Template CRUD ──

        Request::CreateAgentTemplate {
            name, description, agent_type, organization_id,
            system_context, identity_facets, config_overrides,
            knowledge_domains, decision_style,
        } => {
            let now = chrono_now();
            let template = forge_core::types::team::AgentTemplate {
                id: ulid::Ulid::new().to_string(),
                name: name.clone(),
                description,
                agent_type,
                organization_id: organization_id.unwrap_or_else(|| "default".into()),
                system_context: system_context.unwrap_or_default(),
                identity_facets: identity_facets.unwrap_or_else(|| "[]".into()),
                config_overrides: config_overrides.unwrap_or_else(|| "{}".into()),
                knowledge_domains: knowledge_domains.unwrap_or_else(|| "[]".into()),
                decision_style: decision_style.unwrap_or_else(|| "analytical".into()),
                created_at: now.clone(),
                updated_at: now,
            };
            let id = template.id.clone();
            match crate::teams::create_agent_template(&state.conn, &template) {
                Ok(()) => {
                    crate::events::emit(&state.events, "agent_template_created", serde_json::json!({
                        "id": id, "name": name,
                    }));
                    Response::Ok { data: ResponseData::AgentTemplateCreated { id, name } }
                }
                Err(e) => Response::Error { message: format!("create_agent_template failed: {e}") },
            }
        }

        Request::ListAgentTemplates { organization_id, limit } => {
            let lim = limit.unwrap_or(50).min(200);
            match crate::teams::list_agent_templates(&state.conn, organization_id.as_deref(), lim) {
                Ok(templates) => {
                    let count = templates.len();
                    Response::Ok { data: ResponseData::AgentTemplateList { templates, count } }
                }
                Err(e) => Response::Error { message: format!("list_agent_templates failed: {e}") },
            }
        }

        Request::GetAgentTemplate { id, name } => {
            let result = if let Some(id) = id {
                crate::teams::get_agent_template(&state.conn, &id)
            } else if let Some(name) = name {
                crate::teams::get_agent_template_by_name(&state.conn, &name, "default")
            } else {
                return Response::Error { message: "either id or name required".into() };
            };
            match result {
                Ok(Some(template)) => Response::Ok { data: ResponseData::AgentTemplateData { template } },
                Ok(None) => Response::Error { message: "agent template not found".into() },
                Err(e) => Response::Error { message: format!("get_agent_template failed: {e}") },
            }
        }

        Request::DeleteAgentTemplate { id } => {
            match crate::teams::delete_agent_template(&state.conn, &id) {
                Ok(found) => Response::Ok { data: ResponseData::AgentTemplateDeleted { id, found } },
                Err(e) => Response::Error { message: format!("delete_agent_template failed: {e}") },
            }
        }

        Request::UpdateAgentTemplate {
            id, name, description, system_context,
            identity_facets, config_overrides, knowledge_domains, decision_style,
        } => {
            let update = crate::teams::TemplateUpdate {
                name: name.as_deref(), description: description.as_deref(),
                system_context: system_context.as_deref(),
                identity_facets: identity_facets.as_deref(),
                config_overrides: config_overrides.as_deref(),
                knowledge_domains: knowledge_domains.as_deref(),
                decision_style: decision_style.as_deref(),
            };
            match crate::teams::update_agent_template(&state.conn, &id, &update) {
                Ok(updated) => Response::Ok { data: ResponseData::AgentTemplateUpdated { id, updated } },
                Err(e) => Response::Error { message: format!("update_agent_template failed: {e}") },
            }
        }

        // ── Agent Lifecycle ──

        Request::SpawnAgent { template_name, session_id, project, team } => {
            match crate::teams::spawn_agent(
                &state.conn, &template_name, &session_id,
                project.as_deref(), team.as_deref(),
            ) {
                Ok(()) => {
                    crate::events::emit(&state.events, "agent_spawned", serde_json::json!({
                        "session_id": session_id, "template_name": template_name, "team": team,
                    }));
                    Response::Ok { data: ResponseData::AgentSpawned {
                        session_id, template_name, team,
                    }}
                }
                Err(e) => Response::Error { message: format!("spawn_agent failed: {e}") },
            }
        }

        Request::ListAgents { team, limit } => {
            let lim = limit.unwrap_or(50).min(200);
            match crate::teams::list_agents(&state.conn, team.as_deref(), lim) {
                Ok(agents) => {
                    let count = agents.len();
                    Response::Ok { data: ResponseData::AgentList { agents, count } }
                }
                Err(e) => Response::Error { message: format!("list_agents failed: {e}") },
            }
        }

        Request::UpdateAgentStatus { session_id, status, current_task } => {
            // Get old status for event
            let old_status: String = state.conn.query_row(
                "SELECT COALESCE(agent_status, 'unknown') FROM session WHERE id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            ).unwrap_or_else(|_| "unknown".into());

            match crate::teams::update_agent_status(
                &state.conn, &session_id, &status, current_task.as_deref(),
            ) {
                Ok(_updated) => {
                    crate::events::emit(&state.events, "agent_status_changed", serde_json::json!({
                        "session_id": session_id, "old_status": old_status, "new_status": status,
                        "current_task": current_task,
                    }));
                    Response::Ok { data: ResponseData::AgentStatusUpdated { session_id, status } }
                }
                Err(e) => Response::Error { message: format!("update_agent_status failed: {e}") },
            }
        }

        Request::RetireAgent { session_id } => {
            // Get template name for event
            let template_name: String = state.conn.query_row(
                "SELECT COALESCE(at.name, '') FROM session s
                 LEFT JOIN agent_template at ON at.id = s.template_id
                 WHERE s.id = ?1",
                rusqlite::params![session_id],
                |row| row.get(0),
            ).unwrap_or_default();

            match crate::teams::retire_agent(&state.conn, &session_id) {
                Ok(_retired) => {
                    crate::events::emit(&state.events, "agent_retired", serde_json::json!({
                        "session_id": session_id, "template_name": template_name,
                    }));
                    Response::Ok { data: ResponseData::AgentRetired { session_id } }
                }
                Err(e) => Response::Error { message: format!("retire_agent failed: {e}") },
            }
        }

        // ── Team Enhancements ──

        Request::CreateTeam { name, team_type, purpose, organization_id } => {
            match crate::teams::create_team(
                &state.conn, &name, team_type.as_deref(),
                purpose.as_deref(), organization_id.as_deref(),
            ) {
                Ok(id) => Response::Ok { data: ResponseData::TeamCreated { id, name } },
                Err(e) => Response::Error { message: format!("create_team failed: {e}") },
            }
        }

        Request::ListTeamMembers { team_name } => {
            match crate::teams::list_team_members(&state.conn, &team_name) {
                Ok(members) => {
                    let count = members.len();
                    Response::Ok { data: ResponseData::TeamMemberList { members, count } }
                }
                Err(e) => Response::Error { message: format!("list_team_members failed: {e}") },
            }
        }

        Request::SetTeamOrchestrator { team_name, session_id } => {
            match crate::teams::set_team_orchestrator(&state.conn, &team_name, &session_id) {
                Ok(_set) => Response::Ok { data: ResponseData::TeamOrchestratorSet { team_name, session_id } },
                Err(e) => Response::Error { message: format!("set_team_orchestrator failed: {e}") },
            }
        }

        Request::TeamStatus { team_name } => {
            match crate::teams::team_status(&state.conn, &team_name) {
                Ok(team) => Response::Ok { data: ResponseData::TeamStatusData { team } },
                Err(e) => Response::Error { message: format!("team_status failed: {e}") },
            }
        }

        // ── Meeting Protocol ──

        Request::CreateMeeting { team_id, topic, context, orchestrator_session_id, participant_session_ids } => {
            match crate::teams::create_meeting(
                &state.conn, &team_id, &topic, context.as_deref(),
                &orchestrator_session_id, &participant_session_ids,
            ) {
                Ok((meeting_id, participant_count)) => {
                    crate::events::emit(&state.events, "meeting_created", serde_json::json!({
                        "meeting_id": meeting_id, "team_id": team_id, "topic": topic, "participant_count": participant_count,
                    }));
                    Response::Ok { data: ResponseData::MeetingCreated { meeting_id, participant_count } }
                }
                Err(e) => Response::Error { message: format!("create_meeting failed: {e}") },
            }
        }

        Request::MeetingStatus { meeting_id } => {
            match crate::teams::get_meeting_status(&state.conn, &meeting_id) {
                Ok((meeting, participants)) => {
                    Response::Ok { data: ResponseData::MeetingStatusData { meeting, participants } }
                }
                Err(e) => Response::Error { message: format!("meeting_status failed: {e}") },
            }
        }

        Request::MeetingResponses { meeting_id } => {
            match crate::teams::get_meeting_responses(&state.conn, &meeting_id) {
                Ok(responses) => {
                    let count = responses.len();
                    Response::Ok { data: ResponseData::MeetingResponseList { responses, count } }
                }
                Err(e) => Response::Error { message: format!("meeting_responses failed: {e}") },
            }
        }

        Request::MeetingSynthesize { meeting_id, synthesis } => {
            match crate::teams::synthesize_meeting(&state.conn, &meeting_id, &synthesis) {
                Ok(_updated) => {
                    Response::Ok { data: ResponseData::MeetingSynthesized { meeting_id } }
                }
                Err(e) => Response::Error { message: format!("meeting_synthesize failed: {e}") },
            }
        }

        Request::MeetingDecide { meeting_id, decision } => {
            match crate::teams::decide_meeting(&state.conn, &meeting_id, &decision) {
                Ok((_, decision_memory_id)) => {
                    let summary = if decision.len() > 80 {
                        format!("{}...", &decision[..80])
                    } else {
                        decision
                    };
                    crate::events::emit(&state.events, "meeting_decided", serde_json::json!({
                        "meeting_id": meeting_id, "decision_summary": summary,
                    }));
                    Response::Ok { data: ResponseData::MeetingDecided { meeting_id, decision_memory_id } }
                }
                Err(e) => Response::Error { message: format!("meeting_decide failed: {e}") },
            }
        }

        Request::ListMeetings { team_id, status, limit } => {
            let lim = limit.unwrap_or(50).min(200);
            match crate::teams::list_meetings(&state.conn, team_id.as_deref(), status.as_deref(), lim) {
                Ok(meetings) => {
                    let count = meetings.len();
                    Response::Ok { data: ResponseData::MeetingList { meetings, count } }
                }
                Err(e) => Response::Error { message: format!("list_meetings failed: {e}") },
            }
        }

        Request::MeetingTranscript { meeting_id } => {
            match crate::teams::get_meeting_transcript(&state.conn, &meeting_id) {
                Ok(transcript) => {
                    Response::Ok { data: ResponseData::MeetingTranscriptData { transcript } }
                }
                Err(e) => Response::Error { message: format!("meeting_transcript failed: {e}") },
            }
        }

        Request::RecordMeetingResponse { meeting_id, session_id, response, confidence } => {
            match crate::teams::record_meeting_response(
                &state.conn, &meeting_id, &session_id, &response, confidence,
            ) {
                Ok(all_responded) => {
                    crate::events::emit(&state.events, "meeting_response", serde_json::json!({
                        "meeting_id": &meeting_id, "session_id": &session_id,
                    }));
                    if all_responded {
                        crate::events::emit(&state.events, "meeting_all_responded", serde_json::json!({
                            "meeting_id": &meeting_id,
                        }));
                    }
                    Response::Ok { data: ResponseData::MeetingResponseRecorded { meeting_id, all_responded } }
                }
                Err(e) => Response::Error { message: format!("record_meeting_response failed: {e}") },
            }
        }

        // ── Notification Engine ──

        Request::ListNotifications { status, category, limit } => {
            let lim = limit.unwrap_or(50);
            match crate::notifications::list_notifications(
                &state.conn,
                status.as_deref(),
                category.as_deref(),
                None,
                None,
                lim,
            ) {
                Ok(notifs) => {
                    let count = notifs.len();
                    let vals: Vec<serde_json::Value> = notifs.iter().map(|n| {
                        serde_json::json!({
                            "id": n.id,
                            "category": n.category,
                            "priority": n.priority,
                            "title": n.title,
                            "content": n.content,
                            "source": n.source,
                            "source_id": n.source_id,
                            "target_type": n.target_type,
                            "target_id": n.target_id,
                            "status": n.status,
                            "action_type": n.action_type,
                            "action_payload": n.action_payload,
                            "action_result": n.action_result,
                            "topic": n.topic,
                            "created_at": n.created_at,
                            "metadata": n.metadata,
                        })
                    }).collect();
                    Response::Ok { data: ResponseData::NotificationList { notifications: vals, count } }
                }
                Err(e) => Response::Error { message: format!("list_notifications failed: {e}") },
            }
        }

        Request::AckNotification { id } => {
            match crate::notifications::ack_notification(&state.conn, &id) {
                Ok(_) => Response::Ok { data: ResponseData::NotificationAcked { id } },
                Err(e) => Response::Error { message: format!("ack_notification failed: {e}") },
            }
        }

        Request::DismissNotification { id } => {
            match crate::notifications::dismiss_notification(&state.conn, &id) {
                Ok(_) => Response::Ok { data: ResponseData::NotificationDismissed { id } },
                Err(e) => Response::Error { message: format!("dismiss_notification failed: {e}") },
            }
        }

        Request::ActOnNotification { id, approved } => {
            match crate::notifications::act_on_notification(&state.conn, &id, approved) {
                Ok(result) => Response::Ok { data: ResponseData::NotificationActed { id, result } },
                Err(e) => Response::Error { message: format!("act_on_notification failed: {e}") },
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
                let _ = domain_dna_count; // may be non-zero from auto-detect (Cargo.toml in test dir)
                assert_eq!(perception_count, 0);
                let _ = declared_count; // may be non-zero from auto-detect (CLAUDE.md in test dir)
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
            Response::Ok { data: ResponseData::BlastRadius { decisions, callers, importers, files_affected, cluster_name, cluster_files, calling_files } } => {
                assert!(decisions.is_empty());
                assert_eq!(callers, 0);
                assert!(importers.is_empty());
                assert!(files_affected.is_empty());
                assert!(cluster_name.is_none());
                assert!(cluster_files.is_empty());
                assert!(calling_files.is_empty());
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
            capabilities: None,
            current_task: None,
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
            capabilities: None,
            current_task: None,
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
            capabilities: None,
            current_task: None,
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

    #[test]
    fn test_cleanup_sessions_via_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register 3 sessions: 2 hook-test, 1 real
        for id in &["hook-test-1", "hook-test-2", "real-s1"] {
            handle_request(&mut state, Request::RegisterSession {
                id: id.to_string(),
                agent: "claude-code".into(),
                project: Some("forge".into()),
                cwd: None,
                capabilities: None,
                current_task: None,
            });
        }

        // Cleanup hook-test sessions only
        let resp = handle_request(&mut state, Request::CleanupSessions {
            prefix: Some("hook-test".into()),
        });
        match resp {
            Response::Ok { data: ResponseData::SessionsCleaned { ended } } => {
                assert_eq!(ended, 2, "should end 2 hook-test sessions");
            }
            other => panic!("expected SessionsCleaned, got {:?}", other),
        }

        // Verify: only real session remains
        let resp = handle_request(&mut state, Request::Sessions { active_only: Some(true) });
        match resp {
            Response::Ok { data: ResponseData::Sessions { count, .. } } => {
                assert_eq!(count, 1);
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
            user_id: None,
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

        let resp = handle_request(&mut state, Request::ManasHealth { project: None });
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
                let _ = domain_dna_count; // may be non-zero from auto-detect
                assert_eq!(perception_unconsumed, 0);
                let _ = declared_count; // may be non-zero from auto-detect
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
            capabilities: None,
            current_task: None,
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
            capabilities: None,
            current_task: None,
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
            user_id: None,
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
            user_id: None,
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
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
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
                excluded_layers: None,
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
                excluded_layers: None,
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
    fn test_force_consolidate_handler() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Insert duplicate memories using remember_raw to bypass upsert logic
        let m1 = Memory::new(MemoryType::Decision, "Use JWT auth".to_string(), "For auth tokens".to_string());
        let mut m2 = Memory::new(MemoryType::Decision, "Use JWT auth".to_string(), "For auth tokens".to_string());
        m2.id = format!("dup-{}", m1.id); // different id, same title+type
        ops::remember_raw(&state.conn, &m1).unwrap();
        ops::remember_raw(&state.conn, &m2).unwrap();

        let resp = handle_request(&mut state, Request::ForceConsolidate);
        match resp {
            Response::Ok {
                data:
                    ResponseData::ConsolidationComplete {
                        exact_dedup,
                        ..
                    },
            } => {
                assert!(exact_dedup > 0, "should dedup at least 1 duplicate memory");
            }
            other => panic!("expected ConsolidationComplete, got {:?}", other),
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

    #[test]
    fn test_daemon_state_new_is_fast() {
        // DaemonState::new should complete in <500ms since consolidation
        // and ingestion were moved to background tasks.
        let start = std::time::Instant::now();
        let _state = DaemonState::new(":memory:").expect("DaemonState::new should succeed");
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 1000,
            "DaemonState::new took {}ms — should be <1000ms (consolidation is now background)",
            elapsed.as_millis()
        );
    }

    #[test]
    fn test_force_consolidate_empty_db() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(&mut state, Request::ForceConsolidate);
        match resp {
            Response::Ok {
                data:
                    ResponseData::ConsolidationComplete {
                        exact_dedup,
                        semantic_dedup,
                        linked,
                        faded,
                        promoted,
                        reconsolidated,
                        embedding_merged,
                        strengthened,
                        contradictions,
                        ..
                    },
            } => {
                assert_eq!(exact_dedup, 0);
                assert_eq!(semantic_dedup, 0);
                assert_eq!(linked, 0);
                assert_eq!(faded, 0);
                assert_eq!(promoted, 0);
                assert_eq!(reconsolidated, 0);
                assert_eq!(embedding_merged, 0);
                assert_eq!(strengthened, 0);
                assert_eq!(contradictions, 0);
            }
            other => panic!("expected ConsolidationComplete, got {:?}", other),
        }
    }

    // ── Cortex endpoint tests ──

    #[test]
    fn test_get_graph_data_returns_nodes_and_edges() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store some memories
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use Rust".into(),
            content: "For performance".into(),
            confidence: Some(0.9),
            tags: None,
            project: None,
        });
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Lesson,
            title: "Always test".into(),
            content: "Testing prevents regressions".into(),
            confidence: Some(0.8),
            tags: None,
            project: None,
        });

        let resp = handle_request(&mut state, Request::GetGraphData {
            layer: None,
            limit: Some(50),
        });

        match resp {
            Response::Ok { data: ResponseData::GraphData { nodes, edges: _, total_nodes, total_edges: _ } } => {
                // Should have at least the 2 memory nodes plus platform/tool nodes
                assert!(total_nodes >= 2, "should have at least 2 nodes, got {}", total_nodes);
                // Verify the memory nodes are present
                let memory_nodes: Vec<_> = nodes.iter().filter(|n| n.layer == "experience").collect();
                assert!(memory_nodes.len() >= 2, "should have at least 2 experience nodes");
                for node in &memory_nodes {
                    assert!(!node.id.is_empty());
                    assert!(!node.title.is_empty());
                    assert!(node.confidence > 0.0);
                }
            }
            other => panic!("expected GraphData, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_with_provider_returns_result() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Use text that the extraction parser can parse (valid extraction output JSON)
        let text = r#"[{"type":"decision","title":"Use Rust","content":"Memory safety","confidence":0.9,"tags":["arch"],"affects":[]}]"#;

        let req = Request::ExtractWithProvider {
            provider: "ollama".into(),
            model: Some("qwen3:4b".into()),
            text: text.into(),
        };
        let response = handle_request(&mut state, req);

        match response {
            Response::Ok {
                data:
                    ResponseData::ExtractionResult {
                        provider,
                        model,
                        memories_extracted,
                        tokens_in_estimate,
                        latency_ms,
                        ..
                    },
            } => {
                assert_eq!(provider, "ollama");
                assert_eq!(model, "qwen3:4b");
                assert_eq!(memories_extracted, 1, "should parse 1 memory from valid JSON");
                assert!(tokens_in_estimate > 0, "token estimate should be positive");
                // latency_ms can be 0 for fast parsing — just verify it's a valid number
                assert!(latency_ms < 10_000, "latency should be reasonable");
            }
            other => panic!("expected ExtractionResult, got {:?}", other),
        }
    }

    #[test]
    fn test_get_graph_data_layer_filter() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store a memory (experience layer)
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use Rust".into(),
            content: "For performance".into(),
            confidence: Some(0.9),
            tags: None,
            project: None,
        });

        // Filter by experience layer — should get memory nodes
        let resp = handle_request(&mut state, Request::GetGraphData {
            layer: Some("experience".into()),
            limit: Some(50),
        });
        match resp {
            Response::Ok { data: ResponseData::GraphData { nodes, .. } } => {
                assert!(!nodes.is_empty(), "experience layer should have nodes");
                for node in &nodes {
                    assert_eq!(node.layer, "experience", "all nodes should be experience layer");
                }
            }
            other => panic!("expected GraphData, got {:?}", other),
        }

        // Filter by identity layer — should be empty (no identity facets stored)
        let resp = handle_request(&mut state, Request::GetGraphData {
            layer: Some("identity".into()),
            limit: Some(50),
        });
        match resp {
            Response::Ok { data: ResponseData::GraphData { nodes, .. } } => {
                assert!(nodes.is_empty(), "identity layer should have no nodes when no facets stored");
            }
            other => panic!("expected GraphData, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_with_provider_unknown_provider() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let req = Request::ExtractWithProvider {
            provider: "nonexistent_provider".into(),
            model: None,
            text: "some plain text that is not valid extraction JSON".into(),
        };
        let response = handle_request(&mut state, req);

        match response {
            Response::Ok {
                data:
                    ResponseData::ExtractionResult {
                        provider,
                        model,
                        memories_extracted,
                        ..
                    },
            } => {
                assert_eq!(provider, "nonexistent_provider");
                assert_eq!(model, "unknown", "unknown provider should default model to 'unknown'");
                assert_eq!(memories_extracted, 0, "plain text should not parse as extraction output");
            }
            other => panic!("expected ExtractionResult, got {:?}", other),
        }
    }

    #[test]
    fn test_get_graph_data_position_hints() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Position test".into(),
            content: "Check xyz".into(),
            confidence: Some(0.9),
            tags: None,
            project: None,
        });

        let resp = handle_request(&mut state, Request::GetGraphData {
            layer: Some("experience".into()),
            limit: Some(50),
        });
        match resp {
            Response::Ok { data: ResponseData::GraphData { nodes, .. } } => {
                assert!(!nodes.is_empty());
                for node in &nodes {
                    // x and z should be in [-1.0, 1.0] range
                    assert!(node.x >= -1.0 && node.x <= 1.0, "x={} out of range", node.x);
                    assert!(node.z >= -1.0 && node.z <= 1.0, "z={} out of range", node.z);
                    // y should be the layer height (experience = 3.0-4.0)
                    assert!(node.y >= 0.0, "y={} should be non-negative", node.y);
                }
            }
            other => panic!("expected GraphData, got {:?}", other),
        }
    }

    #[test]
    fn test_batch_recall_returns_per_query() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store some memories
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use Rust for backend".into(),
            content: "Rust gives memory safety".into(),
            confidence: Some(0.9),
            tags: None,
            project: None,
        });
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Lesson,
            title: "TypeScript for frontend".into(),
            content: "React with TypeScript is productive".into(),
            confidence: Some(0.8),
            tags: None,
            project: None,
        });

        let resp = handle_request(&mut state, Request::BatchRecall {
            queries: vec![
                forge_core::protocol::RecallQuery {
                    text: "Rust backend".into(),
                    memory_type: None,
                    limit: Some(5),
                },
                forge_core::protocol::RecallQuery {
                    text: "TypeScript frontend".into(),
                    memory_type: None,
                    limit: Some(5),
                },
                forge_core::protocol::RecallQuery {
                    text: "Python machine learning".into(),
                    memory_type: None,
                    limit: Some(5),
                },
            ],
        });

        match resp {
            Response::Ok { data: ResponseData::BatchRecallResults { results } } => {
                assert_eq!(results.len(), 3, "should have 3 result sets for 3 queries");
                // First query should find the Rust memory
                assert!(!results[0].is_empty(), "Rust query should return results");
                // Second query should find the TypeScript memory
                assert!(!results[1].is_empty(), "TypeScript query should return results");
                // Third query about Python may or may not return results (FTS matching)
            }
            other => panic!("expected BatchRecallResults, got {:?}", other),
        }
    }

    #[test]
    fn test_batch_recall_empty_queries() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(&mut state, Request::BatchRecall {
            queries: vec![],
        });

        match resp {
            Response::Ok { data: ResponseData::BatchRecallResults { results } } => {
                assert!(results.is_empty(), "empty queries should return empty results");
            }
            other => panic!("expected BatchRecallResults, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_with_provider_default_model() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // No model specified — should use config default for the provider
        let req = Request::ExtractWithProvider {
            provider: "claude_api".into(),
            model: None,
            text: "[]".into(),
        };
        let response = handle_request(&mut state, req);

        match response {
            Response::Ok {
                data:
                    ResponseData::ExtractionResult {
                        provider,
                        model,
                        memories_extracted,
                        ..
                    },
            } => {
                assert_eq!(provider, "claude_api");
                // Model should be the config default, not empty
                assert!(!model.is_empty(), "default model should not be empty");
                assert_eq!(memories_extracted, 0, "empty array should yield 0 memories");
            }
            other => panic!("expected ExtractionResult, got {:?}", other),
        }
    }

    #[test]
    fn test_remember_decision_creates_cross_session_perception() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register 2 sessions so cross-session perception triggers
        crate::sessions::register_session(&state.conn, "s1", "claude-code", Some("forge"), None, None, None).unwrap();
        crate::sessions::register_session(&state.conn, "s2", "cline", Some("forge"), None, None, None).unwrap();

        // Store a decision
        let resp = handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use JWT for auth".into(),
            content: "Security decision for API".into(),
            confidence: Some(0.9),
            tags: None,
            project: Some("forge".into()),
        });
        assert!(matches!(resp, Response::Ok { data: ResponseData::Stored { .. } }));

        // Verify cross-session perception was created
        let perceptions = crate::db::manas::list_unconsumed_perceptions(&state.conn, None).unwrap();
        let cross = perceptions.iter().find(|p| {
            p.kind == forge_core::types::manas::PerceptionKind::CrossSessionDecision
        });
        assert!(cross.is_some(), "cross-session perception should exist");
        let cross = cross.unwrap();
        assert!(cross.data.contains("JWT"), "perception should reference the decision");
        assert_eq!(cross.project, Some("forge".into()), "should carry project");
        assert!(cross.expires_at.is_some(), "should have TTL");
    }

    #[test]
    fn test_remember_lesson_no_cross_session_perception() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register 2 sessions
        crate::sessions::register_session(&state.conn, "s1", "claude-code", Some("forge"), None, None, None).unwrap();
        crate::sessions::register_session(&state.conn, "s2", "cline", Some("forge"), None, None, None).unwrap();

        // Store a lesson (NOT a decision)
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Lesson,
            title: "TDD is great".into(),
            content: "Write tests first".into(),
            confidence: None,
            tags: None,
            project: Some("forge".into()),
        });

        // Verify NO cross-session perception was created
        let perceptions = crate::db::manas::list_unconsumed_perceptions(&state.conn, None).unwrap();
        let cross = perceptions.iter().find(|p| {
            p.kind == forge_core::types::manas::PerceptionKind::CrossSessionDecision
        });
        assert!(cross.is_none(), "lessons should not create cross-session perceptions");
    }

    #[test]
    fn test_remember_decision_no_cross_session_when_single_session() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Only 1 session — no cross-session perception needed
        crate::sessions::register_session(&state.conn, "s1", "claude-code", Some("forge"), None, None, None).unwrap();

        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use NDJSON protocol".into(),
            content: "Daemon IPC format".into(),
            confidence: None,
            tags: None,
            project: Some("forge".into()),
        });

        let perceptions = crate::db::manas::list_unconsumed_perceptions(&state.conn, None).unwrap();
        let cross = perceptions.iter().find(|p| {
            p.kind == forge_core::types::manas::PerceptionKind::CrossSessionDecision
        });
        assert!(cross.is_none(), "single session should not create cross-session perception");
    }

    // ── RealityEngine Detection Tests ──

    #[test]
    fn test_detect_reality_rust_project() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let resp = handle_request(&mut state, Request::DetectReality {
            path: dir.path().to_string_lossy().to_string(),
        });
        match resp {
            Response::Ok { data: ResponseData::RealityDetected {
                reality_type, domain, detected_from, confidence, is_new, ..
            } } => {
                assert_eq!(reality_type, "code");
                assert_eq!(domain, "rust");
                assert_eq!(detected_from, "Cargo.toml");
                assert!((confidence - 0.95).abs() < f64::EPSILON);
                assert!(is_new, "first detection should create a new reality");
            }
            other => panic!("expected RealityDetected, got {:?}", other),
        }
    }

    #[test]
    fn test_detect_reality_creates_record() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/test").unwrap();
        let path = dir.path().to_string_lossy().to_string();

        // First call should create
        let resp = handle_request(&mut state, Request::DetectReality {
            path: path.clone(),
        });
        let reality_id = match resp {
            Response::Ok { data: ResponseData::RealityDetected { reality_id, is_new, .. } } => {
                assert!(is_new, "first detection should create new reality");
                reality_id
            }
            other => panic!("expected RealityDetected, got {:?}", other),
        };

        // Verify it's in the DB
        let reality = crate::db::ops::get_reality_by_path(&state.conn, &path, "default")
            .unwrap()
            .expect("reality should exist in DB");
        assert_eq!(reality.id, reality_id);
        assert_eq!(reality.reality_type, "code");
        assert_eq!(reality.domain.as_deref(), Some("go"));
    }

    #[test]
    fn test_detect_reality_reuses_existing() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let path = dir.path().to_string_lossy().to_string();

        // First call creates
        let resp1 = handle_request(&mut state, Request::DetectReality {
            path: path.clone(),
        });
        let id1 = match resp1 {
            Response::Ok { data: ResponseData::RealityDetected { reality_id, is_new, .. } } => {
                assert!(is_new);
                reality_id
            }
            other => panic!("expected RealityDetected, got {:?}", other),
        };

        // Second call reuses
        let resp2 = handle_request(&mut state, Request::DetectReality {
            path: path.clone(),
        });
        let id2 = match resp2 {
            Response::Ok { data: ResponseData::RealityDetected { reality_id, is_new, .. } } => {
                assert!(!is_new, "second detection should reuse existing reality");
                reality_id
            }
            other => panic!("expected RealityDetected, got {:?}", other),
        };

        assert_eq!(id1, id2, "both calls should return the same reality ID");
    }

    #[test]
    fn test_detect_reality_empty_dir_fails() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        // No marker files

        let resp = handle_request(&mut state, Request::DetectReality {
            path: dir.path().to_string_lossy().to_string(),
        });
        match resp {
            Response::Error { message } => {
                assert!(message.contains("no reality engine can handle"), "error: {message}");
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    #[test]
    fn test_register_session_auto_tags_reality() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let cwd_path = dir.path().to_string_lossy().to_string();

        // Register session with cwd pointing to a Rust project
        let resp = handle_request(&mut state, Request::RegisterSession {
            id: "s-reality-test".into(),
            agent: "claude-code".into(),
            project: Some("test-project".into()),
            cwd: Some(cwd_path.clone()),
            capabilities: None,
            current_task: None,
        });
        match resp {
            Response::Ok { data: ResponseData::SessionRegistered { .. } } => {}
            other => panic!("expected SessionRegistered, got {:?}", other),
        }

        // Check that the session now has a reality_id
        let reality_id: Option<String> = state.conn.query_row(
            "SELECT reality_id FROM session WHERE id = ?1",
            rusqlite::params!["s-reality-test"],
            |row| row.get(0),
        ).unwrap();
        assert!(reality_id.is_some(), "session should have reality_id set from auto-detection");

        // Verify the reality record was also created
        let reality = crate::db::ops::get_reality_by_path(&state.conn, &cwd_path, "default")
            .unwrap()
            .expect("reality should exist");
        assert_eq!(reality.reality_type, "code");
        assert_eq!(reality.domain.as_deref(), Some("rust"));
    }

    // ── Cross-Engine Query Tests ──

    #[test]
    fn test_cross_engine_query_basic() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a code file and symbols
        let file = forge_core::types::CodeFile {
            id: "f1".into(),
            path: "src/handler.rs".into(),
            language: "rust".into(),
            project: "forge".into(),
            hash: "abc123".into(),
            indexed_at: "2026-04-05T00:00:00Z".into(),
        };
        crate::db::ops::store_file(&state.conn, &file).unwrap();

        let sym = forge_core::types::CodeSymbol {
            id: "s1".into(),
            name: "handle_request".into(),
            kind: "function".into(),
            file_path: "src/handler.rs".into(),
            line_start: 10,
            line_end: Some(50),
            signature: Some("fn handle_request()".into()),
        };
        crate::db::ops::store_symbol(&state.conn, &sym).unwrap();

        // Add a call edge
        state.conn.execute(
            "INSERT INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from) VALUES ('e1', 'src/main.rs', 'src/handler.rs', 'calls', '{}', '2026-04-05', '2026-04-05')",
            [],
        ).unwrap();

        let resp = handle_request(&mut state, Request::CrossEngineQuery {
            file: "src/handler.rs".into(),
            reality_id: None,
        });

        match resp {
            Response::Ok { data: ResponseData::CrossEngineResult { file, symbols, callers, calling_files, .. } } => {
                assert_eq!(file, "src/handler.rs");
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0]["name"], "handle_request");
                assert_eq!(callers, 1);
                assert_eq!(calling_files, vec!["src/main.rs"]);
            }
            other => panic!("expected CrossEngineResult, got {:?}", other),
        }
    }

    #[test]
    fn test_file_memory_map_basic() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a memory mentioning a file
        handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Handler architecture".into(),
            content: "Use src/handler.rs as the central dispatcher".into(),
            confidence: Some(0.9),
            tags: None,
            project: None,
        });

        let resp = handle_request(&mut state, Request::FileMemoryMap {
            files: vec!["src/handler.rs".into(), "src/nonexistent.rs".into()],
            reality_id: None,
        });

        match resp {
            Response::Ok { data: ResponseData::FileMemoryMapResult { mappings } } => {
                let info = mappings.get("src/handler.rs").expect("should have handler.rs");
                assert!(info.memory_count >= 1, "should find at least 1 memory mentioning handler.rs");
                assert!(info.decision_count >= 1, "should find at least 1 decision");

                let info2 = mappings.get("src/nonexistent.rs").expect("should have nonexistent.rs");
                assert_eq!(info2.memory_count, 0, "nonexistent file should have 0 memories");
            }
            other => panic!("expected FileMemoryMapResult, got {:?}", other),
        }
    }

    #[test]
    fn test_code_search_by_name() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store symbols
        let sym1 = forge_core::types::CodeSymbol {
            id: "s1".into(),
            name: "handle_request".into(),
            kind: "function".into(),
            file_path: "src/handler.rs".into(),
            line_start: 10,
            line_end: Some(50),
            signature: None,
        };
        let sym2 = forge_core::types::CodeSymbol {
            id: "s2".into(),
            name: "handle_response".into(),
            kind: "function".into(),
            file_path: "src/response.rs".into(),
            line_start: 5,
            line_end: Some(20),
            signature: None,
        };
        let sym3 = forge_core::types::CodeSymbol {
            id: "s3".into(),
            name: "DaemonState".into(),
            kind: "class".into(),
            file_path: "src/handler.rs".into(),
            line_start: 1,
            line_end: Some(8),
            signature: None,
        };
        crate::db::ops::store_symbol(&state.conn, &sym1).unwrap();
        crate::db::ops::store_symbol(&state.conn, &sym2).unwrap();
        crate::db::ops::store_symbol(&state.conn, &sym3).unwrap();

        // Search by name pattern
        let resp = handle_request(&mut state, Request::CodeSearch {
            query: "handle".into(),
            kind: None,
            limit: None,
        });
        match resp {
            Response::Ok { data: ResponseData::CodeSearchResult { hits } } => {
                assert_eq!(hits.len(), 2, "should find 2 symbols matching 'handle'");
            }
            other => panic!("expected CodeSearchResult, got {:?}", other),
        }

        // Search with kind filter
        let resp2 = handle_request(&mut state, Request::CodeSearch {
            query: "Daemon".into(),
            kind: Some("class".into()),
            limit: Some(5),
        });
        match resp2 {
            Response::Ok { data: ResponseData::CodeSearchResult { hits } } => {
                assert_eq!(hits.len(), 1, "should find 1 class matching 'Daemon'");
                assert_eq!(hits[0]["name"], "DaemonState");
            }
            other => panic!("expected CodeSearchResult, got {:?}", other),
        }
    }

    #[test]
    fn test_force_index_produces_edges() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Create a temp file with imports
        let tmp = tempfile::tempdir().expect("create temp dir");
        let rs_path = tmp.path().join("main.rs");
        std::fs::write(&rs_path, "use std::io;\nuse crate::db;\nfn main() {}").unwrap();

        // Store the file in code_file table (as if the background indexer already ran)
        let file = CodeFile {
            id: format!("file:{}", rs_path.display()),
            path: rs_path.to_str().unwrap().to_string(),
            language: "rust".to_string(),
            project: tmp.path().to_str().unwrap().to_string(),
            hash: "test:hash".to_string(),
            indexed_at: "2026-01-01T00:00:00Z".to_string(),
        };
        ops::store_file(&state.conn, &file).unwrap();

        // ForceIndex should extract imports from the already-indexed file
        let resp = handle_request(&mut state, Request::ForceIndex);
        match resp {
            Response::Ok { data: ResponseData::IndexComplete { files_indexed, .. } } => {
                assert_eq!(files_indexed, 1, "should report 1 file indexed");
            }
            other => panic!("expected IndexComplete, got {:?}", other),
        }

        // Verify import edges were created
        let edge_count: usize = state.conn
            .query_row("SELECT COUNT(*) FROM edge WHERE edge_type = 'imports'", [], |r| r.get(0))
            .unwrap();
        assert!(edge_count >= 2, "should have at least 2 import edges (std::io and crate::db), got {edge_count}");
    }
}
