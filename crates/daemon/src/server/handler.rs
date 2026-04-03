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

        Ok(DaemonState {
            conn,
            events: crate::events::create_event_bus(),
            started_at: Instant::now(),
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
            let id = memory.id.clone();
            match ops::remember(&state.conn, &memory) {
                Ok(()) => Response::Ok {
                    data: ResponseData::Stored { id },
                },
                Err(e) => Response::Error {
                    message: format!("remember failed: {e}"),
                },
            }
        }

        Request::Recall { query, memory_type, project, limit } => {
            let lim = limit.unwrap_or(10);
            let results =
                hybrid_recall(&state.conn, &query, None, memory_type.as_ref(), project.as_deref(), lim);
            let count = results.len();
            Response::Ok {
                data: ResponseData::Memories { results, count },
            }
        }

        Request::Forget { id } => match ops::forget(&state.conn, &id) {
            Ok(true) => Response::Ok {
                data: ResponseData::Forgotten { id },
            },
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
            Response::Ok {
                data: ResponseData::Doctor {
                    daemon_up: true,
                    db_size_bytes: 0, // would need DB path to check file size
                    memory_count: h.decisions + h.lessons + h.patterns + h.preferences,
                    file_count: files,
                    symbol_count: symbols,
                    edge_count: h.edges,
                    workers: vec![
                        "watcher".into(),
                        "extractor".into(),
                        "embedder".into(),
                        "consolidator".into(),
                        "indexer".into(),
                    ],
                    uptime_secs: state.started_at.elapsed().as_secs(),
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
            match crate::sessions::register_session(&state.conn, &id, &agent, project.as_deref(), cwd.as_deref()) {
                Ok(()) => Response::Ok {
                    data: ResponseData::SessionRegistered { id },
                },
                Err(e) => Response::Error {
                    message: format!("register_session failed: {e}"),
                },
            }
        }

        Request::EndSession { id } => {
            match crate::sessions::end_session(&state.conn, &id) {
                Ok(found) => Response::Ok {
                    data: ResponseData::SessionEnded { id, found },
                },
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
            Response::Ok {
                data: ResponseData::GuardrailsCheck {
                    safe: result.safe,
                    warnings: result.warnings,
                    decisions_affected: result.decisions_affected,
                    callers_count: result.callers_count,
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

        Request::Shutdown => Response::Ok {
            data: ResponseData::Shutdown,
        },
    }
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
                        ..
                    },
            } => {
                assert!(daemon_up);
                assert_eq!(memory_count, 0);
                assert_eq!(file_count, 0);
                assert_eq!(symbol_count, 0);
                assert_eq!(edge_count, 0);
                assert_eq!(workers.len(), 5);
                assert!(workers.contains(&"indexer".to_string()));
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
            Response::Ok { data: ResponseData::GuardrailsCheck { safe, warnings, decisions_affected, callers_count } } => {
                assert!(safe);
                assert!(warnings.is_empty());
                assert!(decisions_affected.is_empty());
                assert_eq!(callers_count, 0);
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
}
