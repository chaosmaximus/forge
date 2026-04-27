use crate::claude_memory;
use crate::db::{ops, schema};
use crate::embed::Embedder;
use crate::events::EventSender;
use crate::recall::hybrid_recall;
use forge_core::protocol::*;
use forge_core::types::{CodeFile, CodeSymbol, Memory};
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Instant;

pub struct DaemonState {
    pub conn: Connection,
    /// Path to the SQLite file backing `conn` (or `:memory:` in tests).
    /// Captured so handlers/actor can spawn background tasks that open their
    /// own write-capable connection (e.g. F23 async force-index in W22).
    pub db_path: String,
    pub events: EventSender,
    pub started_at: Instant,
    pub hlc: Arc<crate::sync::Hlc>,
    /// Channel to send edited file paths to the diagnostics worker.
    /// Set after worker spawn; None before that.
    pub diagnostics_tx: Option<tokio::sync::mpsc::Sender<String>>,
    /// Writer actor channel for fire-and-forget writes from the read-only path.
    /// Set on reader states created by the socket handler; None on writer/test states.
    pub writer_tx: Option<tokio::sync::mpsc::Sender<super::writer::WriteCommand>>,
    /// Shared embedder for the raw storage layer (see docs/benchmarks/plan.md §4.3).
    /// `None` until the daemon has initialized the MiniLM model (lazy — avoids a
    /// ~90 MB download in tests that don't exercise the raw path). `RawIngest`
    /// and `RawSearch` handler arms return a clear error when this is None.
    pub raw_embedder: Option<Arc<dyn Embedder>>,
    /// Shared Prometheus metrics. `Some` on the primary Arc<Mutex<DaemonState>>
    /// created in `main` (so startup consolidation, the periodic consolidator
    /// loop, and the `ForceConsolidate` handler all update the same registry);
    /// `None` on reader/writer states and tests. The same `Arc` is passed to
    /// `AppState` so `/metrics` serves whatever this Arc points at.
    pub metrics: Option<Arc<crate::server::metrics::ForgeMetrics>>,
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
        // P3-4 W1.30 (W23 review MED-4): canonical PRAGMA helper. The
        // `:memory:` case is harmless — apply_runtime_pragmas is idempotent
        // and the WAL PRAGMA is a no-op for in-memory DBs (SQLite ignores
        // it gracefully). Centralizing here unifies the busy_timeout drift
        // (5000 vs 10000) the W23 review surfaced.
        crate::db::apply_runtime_pragmas(&conn)?;
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

        // Best-effort: seed Claude Code builtins (Bash/Read/Edit/etc.) into the
        // tool registry so record_tool_uses_from_transcript has rows to
        // increment. Idempotent via INSERT OR IGNORE; accumulated use_count
        // is preserved across reboots. Closes SESSION-GAPS #54 Layer 2.
        if let Err(e) = crate::db::manas::seed_claude_builtins(&conn) {
            eprintln!("[daemon] WARN: failed to seed Claude builtins: {e}");
        }

        // Prune low-quality skills (no steps, short descriptions, status-like names)
        match crate::db::manas::prune_junk_skills(&conn) {
            Ok(n) if n > 0 => eprintln!("[daemon] pruned {n} junk skills"),
            Ok(_) => {}
            Err(e) => eprintln!("[daemon] skill pruning error: {e}"),
        }

        // Backfill project on memories that have session_id but no project
        match crate::sessions::backfill_project(&conn) {
            Ok(n) if n > 0 => eprintln!("[daemon] backfilled project on {n} memories"),
            Ok(_) => {}
            Err(e) => eprintln!("[daemon] project backfill error: {e}"),
        }

        // Auto-cleanup sessions older than 24h that are still ACTIVE (leaked sessions)
        match crate::sessions::cleanup_stale_sessions(&conn) {
            Ok(n) if n > 0 => eprintln!("[daemon] auto-ended {n} stale sessions (>24h active)"),
            Ok(_) => {}
            Err(e) => eprintln!("[daemon] stale session cleanup error: {e}"),
        }

        let node_id = crate::sync::generate_node_id();
        let hlc = crate::sync::Hlc::new(&node_id);

        // Backfill HLC timestamps on existing memories that lack them
        match crate::sync::backfill_hlc(&conn, &hlc) {
            Ok(count) if count > 0 => {
                eprintln!("[daemon] backfilled HLC timestamps on {count} existing memories")
            }
            Ok(_) => {}
            Err(e) => eprintln!("[daemon] WARN: HLC backfill failed: {e} — sync may be unreliable"),
        }

        // NOTE: Consolidation + project ingestion moved to background task
        // (spawned after socket server starts) to avoid blocking socket startup.
        // See main.rs `spawn_startup_tasks()`.

        let events = crate::events::create_event_bus();

        // Emit tool_discovered event for tools found during startup
        if tools_discovered > 0 {
            crate::events::emit(
                &events,
                "tool_discovered",
                serde_json::json!({
                    "count": tools_discovered,
                    "source": "startup_scan",
                }),
            );
        }

        Ok(DaemonState {
            conn,
            db_path: db_path.to_string(),
            events,
            started_at: Instant::now(),
            hlc: Arc::new(hlc),
            diagnostics_tx: None,
            writer_tx: None,
            raw_embedder: None,
            metrics: None,
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
            Connection::open(db_path).map_err(|e| format!("open writer db: {e}"))?
        };
        // P3-4 W1.30 (W23 review MED-4): canonical PRAGMA helper.
        crate::db::apply_runtime_pragmas(&conn)
            .map_err(|e| format!("apply runtime pragmas: {e}"))?;
        // Ensure schema exists on this connection (idempotent)
        schema::create_schema(&conn).map_err(|e| format!("create schema for writer: {e}"))?;
        Ok(Self {
            conn,
            db_path: db_path.to_string(),
            events,
            hlc,
            started_at,
            diagnostics_tx: None,
            writer_tx: None,
            raw_embedder: None,
            metrics: None,
        })
    }

    /// Create a read-only state for serving read requests on a per-connection
    /// basis. No schema creation, no workers, no platform detection -- just a
    /// read-only SQLite connection for queries.
    ///
    /// Shares the event bus, HLC, and started_at from the write state so that
    /// read handlers (e.g. CompileContext, GuardrailsCheck) can emit events
    /// and Status can report uptime.
    ///
    /// Phase 2A-4d.2.1 #1: `metrics` plumbs the daemon-wide
    /// `Arc<ForgeMetrics>` so the per-request reader can lazy-refresh
    /// the gauge snapshot for `/inspect row_count` (otherwise the
    /// snapshot stays at `refreshed_at_secs == 0` on daemons that
    /// never get a `/metrics` scrape, and `row_count` always returns
    /// `stale: true`). Pass `None` from health probes, skills lookups,
    /// and other paths that never call `Inspect`.
    pub fn new_reader(
        db_path: &str,
        events: EventSender,
        hlc: Arc<crate::sync::Hlc>,
        started_at: Instant,
        writer_tx: Option<tokio::sync::mpsc::Sender<super::writer::WriteCommand>>,
        metrics: Option<Arc<crate::server::metrics::ForgeMetrics>>,
    ) -> Result<Self, String> {
        // Must init sqlite-vec extension before opening any connection
        crate::db::vec::init_sqlite_vec();

        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("open read-only db: {e}"))?;
        // P3-4 W1.30 (W23 review MED-4): inline rather than the helper
        // because `PRAGMA journal_mode=WAL` requires write access; on a
        // read-only handle the SQLite engine returns the existing mode
        // without engaging WAL afresh. busy_timeout IS per-connection
        // and matches the canonical 10s value (`crate::db::BUSY_TIMEOUT_MS`).
        let _ = conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA busy_timeout={};",
            crate::db::BUSY_TIMEOUT_MS
        ));
        Ok(Self {
            conn,
            db_path: db_path.to_string(),
            events,
            hlc,
            started_at,
            diagnostics_tx: None,
            writer_tx,
            raw_embedder: None,
            metrics,
        })
    }
}

/// Extract organization_id from a session. Returns "default" if session not found.
fn get_session_org_id(conn: &Connection, session_id: Option<&str>) -> String {
    if let Some(sid) = session_id {
        conn.query_row(
            "SELECT COALESCE(organization_id, 'default') FROM session WHERE id = ?1",
            rusqlite::params![sid],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "default".to_string())
    } else {
        "default".to_string()
    }
}

/// Send a fire-and-forget TouchMemories command through the writer actor channel.
/// Called after Recall/CompileContext to update access_count and activation_level
/// on the write connection, since the read-only handler connection can't write.
fn send_touch(
    writer_tx: &Option<tokio::sync::mpsc::Sender<super::writer::WriteCommand>>,
    ids: Vec<String>,
    boost: f64,
) {
    if ids.is_empty() {
        return;
    }
    if let Some(tx) = writer_tx {
        // Deduplicate IDs to prevent double-boost (M6 fix)
        let mut unique_ids = ids;
        unique_ids.sort_unstable();
        unique_ids.dedup();

        // try_send is non-blocking — touch is best-effort optimization
        if let Err(e) = tx.try_send(super::writer::WriteCommand::TouchMemories {
            ids: unique_ids,
            boost_amount: boost,
        }) {
            eprintln!("[send_touch] failed to send touch: {e}");
        }
    }
}

/// Resolve the session_id to attribute a proactive-hook RecordInjection to.
///
/// Prefers the explicit value threaded through the Request (new in SP1
/// review-fixup). Falls back to the most recently activated session across
/// any agent — this lets old hook clients (pre-field, deserialized as
/// `None` via `#[serde(default)]`) still record rather than silently
/// dropping the row the way the previous hardcoded `agent="cli"` lookup did
/// on Claude Code sessions (which register as `agent="claude-code"`).
fn resolve_hook_session_id(conn: &rusqlite::Connection, explicit: Option<&str>) -> String {
    if let Some(sid) = explicit.filter(|s| !s.is_empty()) {
        return sid.to_string();
    }
    crate::sessions::get_latest_active_session_id(conn)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Record a proactive-context injection via the writer channel (#45 — SP1 Fix 2).
///
/// No-op when the writer channel is unavailable, session_id is empty, or
/// `proactive_context` produces 0 chars (prevents noise rows for empty-injection
/// hooks — common for PostBashCheck on fresh DBs since bootstrap relevance is
/// 0.1 for all knowledge types, below the 0.3 threshold).
///
/// Mirrors the CompileContext RecordInjection pattern (~handler.rs:2762) but
/// with `context_type = "proactive"` so downstream analytics can split
/// effectiveness by source (proactive hooks vs. SessionStart full context).
fn record_proactive_injection(
    writer_tx: Option<&tokio::sync::mpsc::Sender<super::writer::WriteCommand>>,
    session_id: &str,
    hook_event: &str,
    proactive_context: &[forge_core::protocol::response::ProactiveInjection],
) {
    let Some(tx) = writer_tx else { return };
    if session_id.is_empty() {
        return;
    }
    let chars: usize = proactive_context.iter().map(|i| i.content.len()).sum();
    if chars == 0 {
        return;
    }
    let summary = proactive_context
        .iter()
        .map(|i| format!("{}:{}", i.knowledge_type, i.content.len()))
        .collect::<Vec<_>>()
        .join(",");
    let _ = tx.try_send(super::writer::WriteCommand::RecordInjection {
        session_id: session_id.to_string(),
        hook_event: hook_event.to_string(),
        context_type: "proactive".to_string(),
        content_summary: summary,
        chars_injected: chars,
    });
}

/// Reject ASCII control characters (any codepoint < 0x20) except `\t`.
/// Shared by all Request arms that accept user-supplied identifier strings
/// (session_id, agent, tool_name, etc.) — the policy is a single source of
/// truth so a future expansion (e.g., DEL 0x7F) updates every arm at once.
fn has_control_char(s: &str) -> bool {
    s.chars().any(|c| (c as u32) < 0x20 && c != '\t')
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
            metadata,
            valence,
            intensity,
        } => {
            // W1.39 (W29/W30 strict opt-in): when `memory.require_project
            // = true` is set, the daemon rejects project-less memories
            // instead of accepting them globally. Default-off preserves
            // back-compat. Operators can flip the flag via
            // `forge-next config set memory.require_project=true`.
            let project_is_empty =
                project.as_deref().map(|p| p.trim().is_empty()).unwrap_or(true);
            if project_is_empty {
                let cfg = crate::config::load_config();
                if cfg.memory.require_project {
                    return Response::Error {
                        message: "memory.require_project=true: every Remember must carry an explicit --project (use `forge-next remember ... --project <name>`)".to_string(),
                    };
                }
                // W1.39 audit trail: when running in default
                // (warn-and-proceed) mode, log a one-line warn so
                // operators auditing for project-less writes can find
                // them without enabling the strict gate.
                tracing::warn!(
                    memory_type = ?memory_type,
                    title = %title,
                    "remember: stored memory has no project — set --project to scope it (or `memory.require_project=true` to enforce)"
                );
            }
            let type_str = format!("{memory_type:?}");
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
            // W1.35 (I-9): explicit valence + intensity from
            // `forge-next remember --valence positive --intensity 0.8`.
            // Defaults to "neutral" / 0.5 inside `Memory::new`. Intensity
            // is clamped inside `with_valence`.
            //
            // Wave C+D fix-wave MED-1: daemon-side allowlist. The CLI's
            // typed `ValenceArg` enum closes the parse-time gap, but
            // HTTP/non-CLI clients can still send arbitrary strings;
            // reject any value outside `positive | negative | neutral`
            // here so the wire surface is also enforced. Empty string
            // falls back to default (Memory::new sets "neutral").
            if let Some(v) = valence.filter(|v| !v.is_empty()) {
                if !matches!(v.as_str(), "positive" | "negative" | "neutral") {
                    return Response::Error {
                        message: format!(
                            "invalid valence '{v}' — expected one of: positive, negative, neutral"
                        ),
                    };
                }
                let i = intensity.unwrap_or(0.5);
                memory = memory.with_valence(&v, i);
            } else if let Some(i) = intensity {
                memory = memory.with_valence("neutral", i);
            }
            // Assign active session ID so CLI-stored memories are linked to a session
            memory.session_id =
                crate::sessions::get_active_session_id(&state.conn, "cli").unwrap_or_default();
            // Multi-tenant: derive organization_id from the active session
            let org_id = get_session_org_id(
                &state.conn,
                if memory.session_id.is_empty() {
                    None
                } else {
                    Some(&memory.session_id)
                },
            );
            memory.organization_id = Some(org_id);
            // Stamp HLC before storing
            memory.set_hlc(state.hlc.now(), state.hlc.node_id().to_string());
            let id = memory.id.clone();
            match ops::remember(&state.conn, &memory) {
                Ok(()) => {
                    // Store structured metadata if provided
                    if let Some(ref meta) = metadata {
                        let meta_str = serde_json::to_string(meta).unwrap_or_default();
                        let _ = state.conn.execute(
                            "UPDATE memory SET metadata = ?2 WHERE id = ?1",
                            rusqlite::params![id, meta_str],
                        );
                    }
                    crate::events::emit(
                        &state.events,
                        "memory_created",
                        serde_json::json!({
                            "id": id,
                            "memory_type": type_str,
                            "title": title_clone,
                        }),
                    );

                    // Cross-session perception: when a decision is stored and there are
                    // multiple active sessions, create a subtle perception so other sessions
                    // become aware. Only for decisions (important enough to notify).
                    // Skip test-generated decisions — prevent perception pollution
                    let is_test_decision = title_clone.starts_with("Hook E2E test")
                        || memory.session_id.starts_with("hook-test-")
                        || memory.session_id.starts_with("test-hook-");
                    if is_decision && !is_test_decision {
                        let active_count = crate::sessions::list_sessions(&state.conn, true)
                            .map(|s| s.len())
                            .unwrap_or(0);
                        if active_count > 1 {
                            let perception = forge_core::types::manas::Perception {
                                id: format!("xsession-{}", ulid::Ulid::new()),
                                kind:
                                    forge_core::types::manas::PerceptionKind::CrossSessionDecision,
                                data: format!("Another session stored decision: {title_clone}"),
                                severity: forge_core::types::manas::Severity::Info,
                                project: project.clone(),
                                created_at: forge_core::time::now_iso(),
                                expires_at: Some(forge_core::time::now_offset(600)), // 10 min TTL
                                consumed: false,
                            };
                            if let Err(e) =
                                crate::db::manas::store_perception(&state.conn, &perception)
                            {
                                eprintln!("[cross-session] failed to store perception: {e}");
                            }
                        }
                    }

                    // Store-time healing hint: check if similar active memory exists
                    {
                        let safe_title: String = title_clone
                            .chars()
                            .filter(|c| c.is_alphanumeric() || *c == ' ')
                            .collect();
                        if safe_title.split_whitespace().count() >= 2 {
                            let terms: Vec<&str> = safe_title.split_whitespace().take(5).collect();
                            let fts_query = terms.join(" OR ");
                            let similar: Vec<(String, String)> = state
                                .conn
                                .prepare(
                                    "SELECT m.id, m.title FROM memory m
                                 JOIN memory_fts ON memory_fts.rowid = m.rowid
                                 WHERE memory_fts MATCH ?1
                                   AND m.memory_type = ?2 AND m.status = 'active' AND m.id != ?3
                                 LIMIT 3",
                                )
                                .and_then(|mut stmt| {
                                    stmt.query_map(
                                        rusqlite::params![fts_query, type_str.to_lowercase(), id],
                                        |row| Ok((row.get(0)?, row.get(1)?)),
                                    )?
                                    .collect()
                                })
                                .unwrap_or_default();

                            if !similar.is_empty() {
                                crate::events::emit(
                                    &state.events,
                                    "healing_candidate",
                                    serde_json::json!({
                                        "new_memory_id": id,
                                        "similar_count": similar.len(),
                                        "similar_titles": similar.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(),
                                    }),
                                );
                            }
                        }
                    }

                    // Workspace auto-write: persist decision to team workspace directory
                    if is_decision {
                        let ws_config = crate::config::load_config();
                        if ws_config.workspace.auto_write.decisions
                            && ws_config.workspace.mode != "project"
                        {
                            let org = &ws_config.workspace.org;
                            let team_name = if org.is_empty() {
                                "default"
                            } else {
                                org.as_str()
                            };
                            if let Some(ws_root) = crate::workspace::team_workspace_path(
                                &ws_config.workspace,
                                team_name,
                                org,
                                project.as_deref(),
                            ) {
                                match crate::workspace::write_decision(
                                    &ws_root,
                                    team_name,
                                    &memory.title,
                                    &memory.content,
                                    memory.confidence,
                                    &memory.tags,
                                    &id,
                                ) {
                                    Ok(path) => {
                                        crate::events::emit(
                                            &state.events,
                                            "workspace_decision_written",
                                            serde_json::json!({
                                                "memory_id": id,
                                                "path": path.display().to_string(),
                                            }),
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!("[workspace] auto-write decision failed: {e}");
                                    }
                                }
                            }
                        }
                    }

                    // Create affects edges for file paths mentioned in the memory content/title.
                    // This enables blast-radius to find decisions that reference specific files.
                    if is_decision
                        || matches!(memory.memory_type, forge_core::types::MemoryType::Lesson)
                    {
                        use std::sync::LazyLock;
                        static FILE_PATH_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
                            regex::Regex::new(
                                r"(?:crates|src|lib|app)/[\w/]+\.(?:rs|ts|tsx|js|py|go)",
                            )
                            .unwrap()
                        });
                        // Create affects edges eagerly; orphaned edges cleaned up during vacuum.
                        // File existence not checked here — CWD varies between daemon and project contexts.
                        let mut seen = std::collections::HashSet::new();
                        for text in [&memory.content, &memory.title] {
                            for cap in FILE_PATH_RE.find_iter(text) {
                                let file_target = format!("file:{}", cap.as_str());
                                if seen.insert(file_target.clone()) {
                                    if let Err(e) = ops::store_edge(
                                        &state.conn,
                                        &id,
                                        &file_target,
                                        "affects",
                                        "{}",
                                    ) {
                                        eprintln!("[handler] affects edge error: {e}");
                                    }
                                }
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

        Request::Recall {
            query,
            memory_type,
            project,
            limit,
            layer,
            since,
            include_flipped, // Phase 2A-4a: wired up in T10
            include_globals, // Phase P3-3.11 W29: opt-in for `_global_` rows in project-scoped queries
            query_embedding,
        } => {
            let lim = limit.unwrap_or(10);

            // Phase 2A-4d.3 T3: under bench/test builds honor caller-supplied
            // query_embedding; in production always ignore it (defense against
            // sneaking embeddings through the wire).
            #[cfg(any(test, feature = "bench"))]
            let effective_query_embedding: Option<Vec<f32>> = query_embedding;
            #[cfg(not(any(test, feature = "bench")))]
            let effective_query_embedding: Option<Vec<f32>> = {
                let _ = query_embedding;
                None
            };

            // Phase P3-3.11 W29: pre-resolve once for both hybrid-recall
            // call sites (the `experience` layer branch and the
            // unfiltered fallback). Default is STRICT — globals require
            // an explicit `Recall.include_globals = Some(true)`.
            let include_globals = include_globals.unwrap_or(false);

            // Multi-tenant: extract org_id from the active session for this project
            let _org_id = {
                let active_sid =
                    crate::sessions::get_active_session_id(&state.conn, "cli").unwrap_or_default();
                get_session_org_id(
                    &state.conn,
                    if active_sid.is_empty() {
                        None
                    } else {
                        Some(&active_sid)
                    },
                )
            };
            // TODO: pass _org_id to recall functions when they are updated to accept org_id: Option<&str>

            let preference_half_life_days = crate::config::load_config()
                .recall
                .validated()
                .preference_half_life_days;

            let results = match layer.as_deref() {
                // "experience" → only memory table (hybrid_recall, no manas_recall)
                Some("experience") => {
                    if include_globals {
                        crate::recall::hybrid_recall_with_globals(
                            &state.conn,
                            &query,
                            effective_query_embedding.as_deref(),
                            memory_type.as_ref(),
                            project.as_deref(),
                            lim,
                            include_flipped.unwrap_or(false),
                            preference_half_life_days,
                        )
                    } else {
                        hybrid_recall(
                            &state.conn,
                            &query,
                            effective_query_embedding.as_deref(),
                            memory_type.as_ref(),
                            project.as_deref(),
                            lim,
                            include_flipped.unwrap_or(false),
                            preference_half_life_days,
                        )
                    }
                }
                // "declared" → only declared knowledge
                Some("declared") => {
                    let declared =
                        crate::db::manas::search_declared(&state.conn, &query, project.as_deref())
                            .unwrap_or_default();
                    declared
                        .into_iter()
                        .take(lim)
                        .map(|d| MemoryResult {
                            memory: forge_core::types::Memory::new(
                                forge_core::types::MemoryType::Lesson,
                                format!("[declared:{}] {}", d.source, d.id),
                                d.content.chars().take(500).collect::<String>(),
                            )
                            .with_confidence(0.7),
                            score: 0.5,
                            source: "declared".to_string(),
                            edges: Vec::new(),
                        })
                        .collect()
                }
                // "domain_dna" → only domain DNA
                Some("domain_dna") => {
                    let dna_list =
                        crate::db::manas::list_domain_dna(&state.conn, project.as_deref())
                            .unwrap_or_default();
                    let query_lower = query.to_lowercase();
                    dna_list
                        .into_iter()
                        .filter(|dna| dna.pattern.to_lowercase().contains(&query_lower))
                        .take(lim)
                        .map(|dna| MemoryResult {
                            memory: forge_core::types::Memory::new(
                                forge_core::types::MemoryType::Pattern,
                                format!("[dna:{}] {}", dna.aspect, dna.pattern),
                                format!(
                                    "Project convention: {} (confidence: {:.0}%)",
                                    dna.pattern,
                                    dna.confidence * 100.0
                                ),
                            )
                            .with_confidence(dna.confidence),
                            score: 0.4,
                            source: "domain_dna".to_string(),
                            edges: Vec::new(),
                        })
                        .collect()
                }
                // "identity" → list identity facets matching query
                Some("identity") => {
                    // Search across all agents via LIKE on facet/description
                    let search = format!("%{query}%");
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
                                project: None,
                            })
                        })?.collect()
                    }).unwrap_or_default();

                    facets
                        .into_iter()
                        .map(|f| MemoryResult {
                            memory: forge_core::types::Memory::new(
                                forge_core::types::MemoryType::Preference,
                                format!("[identity:{}] {}", f.agent, f.facet),
                                f.description.clone(),
                            )
                            .with_confidence(f.strength),
                            score: 0.6,
                            source: "identity".to_string(),
                            edges: Vec::new(),
                        })
                        .collect()
                }
                // "perception" → list perceptions matching query (project-scoped)
                Some("perception") => {
                    let perceptions =
                        crate::db::manas::list_unconsumed_perceptions(&state.conn, None, None)
                            .unwrap_or_default();
                    let query_lower = query.to_lowercase();
                    perceptions
                        .into_iter()
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
                        })
                        .collect()
                }
                // "skill" → only skills (Layer 2 — procedural memory)
                Some("skill") => {
                    let skills =
                        crate::db::manas::search_skills(&state.conn, &query, project.as_deref())
                            .unwrap_or_default();
                    skills
                        .into_iter()
                        .take(lim)
                        .map(|s| MemoryResult {
                            memory: forge_core::types::Memory::new(
                                forge_core::types::MemoryType::Pattern,
                                format!("[skill:{}] {}", s.domain, s.name),
                                s.description,
                            )
                            .with_confidence((0.5 + (s.success_count as f64 * 0.1)).min(1.0)),
                            score: 0.6,
                            source: "skill".to_string(),
                            edges: Vec::new(),
                        })
                        .collect()
                }
                // None or unknown → current behavior (search everything)
                _ => {
                    let mut results = if include_globals {
                        crate::recall::hybrid_recall_with_globals(
                            &state.conn,
                            &query,
                            effective_query_embedding.as_deref(),
                            memory_type.as_ref(),
                            project.as_deref(),
                            lim,
                            include_flipped.unwrap_or(false),
                            preference_half_life_days,
                        )
                    } else {
                        hybrid_recall(
                            &state.conn,
                            &query,
                            effective_query_embedding.as_deref(),
                            memory_type.as_ref(),
                            project.as_deref(),
                            lim,
                            include_flipped.unwrap_or(false),
                            preference_half_life_days,
                        )
                    };
                    // Cross-layer search (only if no type filter)
                    if memory_type.is_none() {
                        let manas_results =
                            crate::recall::manas_recall(&state.conn, &query, project.as_deref(), 3);
                        results.extend(manas_results);
                        results.sort_by(|a, b| {
                            b.score
                                .partial_cmp(&a.score)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        results.truncate(lim);
                    }
                    results
                }
            };

            // Temporal filter: only keep memories created at or after the `since` timestamp.
            // ISO timestamps are lexicographically ordered, so string comparison works.
            let mut results = results;
            if let Some(ref since_ts) = since {
                results.retain(|r| r.memory.created_at.as_str() >= since_ts.as_str());
            }

            // Fire-and-forget: send touch/boost through writer channel for read-only path.
            // On the writer path this is redundant (recall already wrote directly), but harmless.
            let touch_ids: Vec<String> = results
                .iter()
                .filter(|r| {
                    r.source != "declared" && r.source != "domain_dna" && r.source != "perception"
                })
                .map(|r| r.memory.id.clone())
                .collect();
            send_touch(&state.writer_tx, touch_ids, 0.3);

            let count = results.len();
            Response::Ok {
                data: ResponseData::Memories { results, count },
            }
        }

        Request::Forget { id } => {
            // Multi-tenant: extract org_id for scoped forget
            let _org_id = {
                // Look up the memory's session to derive org_id
                let mem_session: Option<String> = state
                    .conn
                    .query_row(
                        "SELECT session_id FROM memory WHERE id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                    .ok();
                get_session_org_id(&state.conn, mem_session.as_deref())
            };
            // TODO: pass _org_id to ops::forget when it is updated to accept org_id: Option<&str>
            match ops::forget(&state.conn, &id) {
                Ok(true) => {
                    crate::events::emit(
                        &state.events,
                        "memory_forgotten",
                        serde_json::json!({
                            "id": id,
                        }),
                    );
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
            }
        }

        Request::Supersede { old_id, new_id } => {
            // Derive org scope from the old memory's session (unchanged).
            let supersede_org_id = {
                let mem_session: Option<String> = state
                    .conn
                    .query_row(
                        "SELECT session_id FROM memory WHERE id = ?1",
                        rusqlite::params![old_id],
                        |row| row.get(0),
                    )
                    .ok();
                get_session_org_id(&state.conn, mem_session.as_deref())
            };

            // Pre-fetch old memory to distinguish "old missing/not-active" from
            // "new missing" (preserves the current handler's per-ID error message).
            let old = match ops::fetch_memory_by_id(&state.conn, &old_id) {
                Ok(Some(m)) => m,
                Ok(None) => {
                    return Response::Error {
                        message: format!("old memory not found or already superseded: {old_id}"),
                    }
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("supersede failed: {e}"),
                    }
                }
            };
            if old.status != forge_core::types::memory::MemoryStatus::Active {
                return Response::Error {
                    message: format!("old memory not found or already superseded: {old_id}"),
                };
            }

            // Verify new memory exists (org-scoped).
            // COALESCE(...) keeps the check symmetric with supersede_memory_impl(),
            // which also COALESCEs NULL-org memories into the 'default' bucket.
            let new_exists: bool = state.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM memory WHERE id = ?1 AND status = 'active' AND COALESCE(organization_id, 'default') = ?2)",
                rusqlite::params![&new_id, &supersede_org_id],
                |row| row.get(0),
            ).unwrap_or(false);
            if !new_exists {
                return Response::Error {
                    message: format!("new memory not found: {new_id}"),
                };
            }

            // Wrap the helper call in a transaction so UPDATE + edge INSERT are
            // atomic. Without this, a disk error on the edge INSERT would leave
            // memory.status = 'superseded' with no supersedes edge — inconsistent
            // state. This also matches the pattern T6's FlipPreference handler
            // will use for its 3-statement (INSERT new + UPDATE old + edge) flow.
            let supersede_result: Result<(), ops::OpError> = (|| {
                let tx = state.conn.unchecked_transaction()?;
                ops::supersede_memory_impl(&tx, &old_id, &new_id, Some(&supersede_org_id), None)?;
                tx.commit()?;
                Ok(())
            })();

            match supersede_result {
                Ok(()) => {
                    crate::events::emit(
                        &state.events,
                        "memory_superseded",
                        serde_json::json!({
                            "old_id": old_id,
                            "new_id": new_id,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::Superseded { old_id, new_id },
                    }
                }
                Err(ops::OpError::OldMemoryNotActive { .. }) => Response::Error {
                    message: format!("old memory not found or already superseded: {old_id}"),
                },
                Err(ops::OpError::DbError(e)) => Response::Error {
                    message: format!("supersede failed: {e}"),
                },
            }
        }

        Request::FlipPreference {
            memory_id,
            new_valence,
            new_intensity,
            reason,
        } => {
            // 1. Validate inputs.
            if !matches!(new_valence.as_str(), "positive" | "negative" | "neutral") {
                return Response::Error {
                    message: format!(
                        "new_valence must be positive | negative | neutral (got: {new_valence})"
                    ),
                };
            }
            if !new_intensity.is_finite() || !(0.0..=1.0).contains(&new_intensity) {
                return Response::Error {
                    message: format!(
                        "new_intensity must be finite in [0.0, 1.0] (got: {new_intensity})"
                    ),
                };
            }

            // 2. Load the old preference.
            let old = match ops::fetch_memory_by_id(&state.conn, &memory_id) {
                Ok(Some(m)) => m,
                Ok(None) => {
                    return Response::Error {
                        message: format!("memory_id not found: {memory_id}"),
                    }
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("flip failed: {e}"),
                    }
                }
            };
            if old.memory_type != forge_core::types::memory::MemoryType::Preference {
                let got = format!("{:?}", old.memory_type).to_lowercase();
                return Response::Error {
                    message: format!("memory_type must be preference for flip (got: {got})"),
                };
            }
            if old.status != forge_core::types::memory::MemoryStatus::Active {
                return Response::Error {
                    message: format!("memory already superseded (id: {memory_id})"),
                };
            }

            // 3. Cross-org scope guard.
            // Derive caller_org from the old memory's session (matches Supersede handler pattern).
            let caller_org = {
                let mem_session_opt: Option<String> = state
                    .conn
                    .query_row(
                        "SELECT session_id FROM memory WHERE id = ?1",
                        rusqlite::params![&memory_id],
                        |row| row.get(0),
                    )
                    .ok();
                get_session_org_id(&state.conn, mem_session_opt.as_deref())
            };
            // If old.organization_id is set and differs from caller_org, reject.
            // (caller_org always present — get_session_org_id returns String, defaulting
            // to "default" when no session context exists.)
            if let Some(old_org) = old.organization_id.as_ref() {
                if &caller_org != old_org {
                    return Response::Error {
                        message: "cross-org flip denied".to_string(),
                    };
                }
            }

            // 4. Reject no-op flip (same valence).
            if old.valence == new_valence {
                return Response::Error {
                    message: format!(
                        "no-op flip: memory already has valence {new_valence} (id: {memory_id})"
                    ),
                };
            }

            // 5. Synthesize new memory.
            let now = forge_core::time::now_iso();
            let reason_suffix = reason
                .as_ref()
                .map(|r| format!(" (reason: {r})"))
                .unwrap_or_default();
            let new_id = ulid::Ulid::new().to_string();
            let old_valence = &old.valence;
            let old_content = &old.content;
            let new_content = format!(
                "[flipped from {old_valence} to {new_valence} at {now}]{reason_suffix}: {old_content}"
            );
            // D2 (per master design): inherit confidence with floor 0.5 and cap 1.0.
            // Preserves user's prior conviction while preventing stale-decay propagation.
            let new_confidence = old.confidence.clamp(0.5, 1.0);

            let new_memory = forge_core::types::memory::Memory {
                id: new_id.clone(),
                memory_type: forge_core::types::memory::MemoryType::Preference,
                title: old.title.clone(),
                content: new_content,
                confidence: new_confidence,
                status: forge_core::types::memory::MemoryStatus::Active,
                project: old.project.clone(),
                tags: old.tags.clone(),
                embedding: None,
                created_at: now.clone(),
                accessed_at: now.clone(),
                valence: new_valence.clone(),
                intensity: new_intensity,
                hlc_timestamp: state.hlc.now(),
                node_id: old.node_id.clone(),
                session_id: old.session_id.clone(),
                access_count: 0,
                activation_level: 0.0,
                alternatives: Vec::new(),
                participants: Vec::new(),
                organization_id: old.organization_id.clone(),
                superseded_by: None,
                valence_flipped_at: None,
                reaffirmed_at: None,
            };

            // 6. Atomic transaction: INSERT new + UPDATE+edge via supersede_memory_impl.
            // Same pattern T1 used for Supersede handler (see handler.rs ~773 for prior art).
            let result: Result<(), ops::OpError> = (|| {
                let tx = state.conn.unchecked_transaction()?;
                ops::remember_raw(&tx, &new_memory)?;
                ops::supersede_memory_impl(
                    &tx,
                    &old.id,
                    &new_memory.id,
                    old.organization_id.as_deref(),
                    Some(&now),
                )?;
                tx.commit()?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    // 7. Emit event AFTER commit succeeds.
                    crate::events::emit(
                        &state.events,
                        "preference_flipped",
                        serde_json::json!({
                            "old_id": old.id,
                            "new_id": new_memory.id,
                            "new_valence": new_valence,
                            "new_intensity": new_intensity,
                            "reason": reason.as_deref().unwrap_or(""),
                            "flipped_at": now,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::PreferenceFlipped {
                            old_id: old.id,
                            new_id: new_memory.id,
                            new_valence,
                            new_intensity,
                            flipped_at: now,
                        },
                    }
                }
                Err(ops::OpError::OldMemoryNotActive { .. }) => Response::Error {
                    message: format!("memory already superseded (id: {memory_id})"),
                },
                Err(ops::OpError::DbError(e)) => Response::Error {
                    message: format!("flip transaction failed: {e}"),
                },
            }
        }

        Request::ListFlipped { agent: _, limit } => {
            // Phase 2A-4a: agent is informational this phase (no per-agent memory scope).
            // TODO: derive caller_org_id via get_session_org_id() and pass to list_flipped()
            // for proper multi-tenancy. Matches the deferred wiring in Recall (~line 477).
            let effective_limit = limit.unwrap_or(20);
            match ops::list_flipped(&state.conn, None, effective_limit) {
                Ok(memories) => {
                    let items: Vec<forge_core::protocol::response::FlippedMemory> = memories
                        .into_iter()
                        .map(|m| {
                            // Invariant: list_flipped's SQL filter requires
                            // valence_flipped_at IS NOT NULL, AND supersede_memory_impl's
                            // flip branch sets superseded_by + valence_flipped_at atomically.
                            // Both Options must therefore be Some here.
                            debug_assert!(
                                m.superseded_by.is_some(),
                                "list_flipped SQL guarantees superseded_by is set (atomic with valence_flipped_at via supersede_memory_impl)"
                            );
                            debug_assert!(
                                m.valence_flipped_at.is_some(),
                                "list_flipped SQL guarantees valence_flipped_at is set"
                            );
                            let flipped_to_id = m.superseded_by.clone().unwrap_or_default();
                            let flipped_at = m.valence_flipped_at.clone().unwrap_or_default();
                            forge_core::protocol::response::FlippedMemory {
                                old: m,
                                flipped_to_id,
                                flipped_at,
                            }
                        })
                        .collect();
                    Response::Ok {
                        data: ResponseData::FlippedList { items },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_flipped failed: {e}"),
                },
            }
        }

        // Phase 2A-4b: ReaffirmPreference handler — wired in T9.
        Request::ReaffirmPreference { memory_id } => {
            let now = forge_core::time::now_iso();

            // 1. Derive caller_org from the memory's session — mirrors FlipPreference pattern.
            let caller_org = {
                let mem_session_opt: Option<String> = state
                    .conn
                    .query_row(
                        "SELECT session_id FROM memory WHERE id = ?1",
                        rusqlite::params![&memory_id],
                        |row| row.get(0),
                    )
                    .ok();
                get_session_org_id(&state.conn, mem_session_opt.as_deref())
            };

            // 2. Atomic UPDATE with RETURNING: validates type, status, flipped state, and org scope.
            let updated: Result<String, rusqlite::Error> = state.conn.query_row(
                "UPDATE memory
                   SET reaffirmed_at = ?1
                 WHERE id = ?2
                   AND COALESCE(organization_id, 'default') = ?3
                   AND memory_type = 'preference'
                   AND status = 'active'
                   AND valence_flipped_at IS NULL
                 RETURNING reaffirmed_at",
                rusqlite::params![now, memory_id, caller_org],
                |row| row.get::<_, String>(0),
            );

            match updated {
                Ok(reaffirmed_at) => {
                    crate::events::emit(
                        &state.events,
                        "preference_reaffirmed",
                        serde_json::json!({
                            "memory_id": memory_id,
                            "reaffirmed_at": reaffirmed_at,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::PreferenceReaffirmed {
                            memory_id: memory_id.clone(),
                            reaffirmed_at,
                        },
                    }
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    // Disambiguate failure cause via best-effort diagnostic read.
                    // Scope by org: cross-org memories must surface as "not found"
                    // to prevent existence-probing across organizations.
                    let diag = state.conn.query_row(
                        "SELECT memory_type, status, valence_flipped_at FROM memory
                          WHERE id = ?1
                            AND COALESCE(organization_id, 'default') = ?2",
                        rusqlite::params![&memory_id, &caller_org],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<String>>(2)?,
                            ))
                        },
                    );
                    let msg = match diag {
                        Err(rusqlite::Error::QueryReturnedNoRows) => {
                            // Either truly not found, or belongs to a different org —
                            // never disclose cross-org existence.
                            format!("memory not found: {memory_id}")
                        }
                        Ok((mem_type, _, _)) if mem_type != "preference" => {
                            format!("memory_type must be preference for reaffirm (got: {mem_type})")
                        }
                        Ok((_, _, Some(_))) => {
                            format!("cannot reaffirm flipped memory: {memory_id}")
                        }
                        Ok((_, status, _)) => {
                            format!("memory is not active (status: {status})")
                        }
                        Err(e) => format!("reaffirm failed: {e}"),
                    };
                    Response::Error { message: msg }
                }
                Err(e) => Response::Error {
                    message: format!("reaffirm failed: {e}"),
                },
            }
        }

        // Phase 2A-4c1: RecordToolUse handler — atomic INSERT…SELECT (T5).
        Request::RecordToolUse {
            session_id,
            agent,
            tool_name,
            tool_args,
            tool_result_summary,
            success,
            user_correction_flag,
        } => {
            // Validation — fail-fast before any DB touch. `has_control_char`
            // is defined at module scope so all Request arms share one policy.
            if tool_name.trim().is_empty() {
                return Response::Error {
                    message: "empty_field: tool_name".to_string(),
                };
            }
            if agent.trim().is_empty() {
                return Response::Error {
                    message: "empty_field: agent".to_string(),
                };
            }
            if has_control_char(&session_id) {
                return Response::Error {
                    message: "invalid_field: session_id: control_character".to_string(),
                };
            }
            if has_control_char(&agent) {
                return Response::Error {
                    message: "invalid_field: agent: control_character".to_string(),
                };
            }
            if has_control_char(&tool_name) {
                return Response::Error {
                    message: "invalid_field: tool_name: control_character".to_string(),
                };
            }
            if tool_result_summary.len() > 65536 {
                return Response::Error {
                    message: "payload_too_large: tool_result_summary: 65536".to_string(),
                };
            }
            let id = ulid::Ulid::new().to_string();
            let created_at = forge_core::time::now_iso();
            let canonical = match serde_json::to_string(&tool_args) {
                Ok(s) => s,
                Err(e) => {
                    return Response::Error {
                        message: format!("internal_error: serde_json::to_string failed: {e}"),
                    };
                }
            };
            if canonical.len() > 65536 {
                return Response::Error {
                    message: "payload_too_large: tool_args: 65536".to_string(),
                };
            }

            let rows = state.conn.execute(
                "INSERT INTO session_tool_call
                    (id, session_id, agent, tool_name, tool_args, tool_result_summary,
                     success, user_correction_flag, organization_id, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                        COALESCE(s.organization_id, 'default'), ?9
                 FROM session s
                 WHERE s.id = ?2",
                rusqlite::params![
                    id,
                    session_id,
                    agent,
                    tool_name,
                    canonical,
                    tool_result_summary,
                    success as i64,
                    user_correction_flag as i64,
                    created_at,
                ],
            );

            match rows {
                Ok(1) => {
                    crate::events::emit(
                        &state.events,
                        "tool_use_recorded",
                        serde_json::json!({
                            "id":         id.clone(),
                            "session_id": session_id,
                            "agent":      agent,
                            "tool_name":  tool_name,
                            "success":    success,
                            "created_at": created_at.clone(),
                        }),
                    );
                    Response::Ok {
                        data: forge_core::protocol::ResponseData::ToolCallRecorded {
                            id,
                            created_at,
                        },
                    }
                }
                Ok(0) => Response::Error {
                    message: format!("unknown_session: {session_id}"),
                },
                Ok(n) => Response::Error {
                    message: format!("internal_error: INSERT affected {n} rows (expected 1)"),
                },
                Err(e) => Response::Error {
                    message: format!("internal_error: {e}"),
                },
            }
        }

        // Phase 2A-4c1 T8: ListToolCalls — snapshot-consistent read.
        Request::ListToolCalls {
            session_id,
            agent,
            limit,
        } => {
            // `has_control_char` is defined at module scope — one policy
            // shared across RecordToolUse and ListToolCalls arms.
            if has_control_char(&session_id) {
                return Response::Error {
                    message: "invalid_field: session_id: control_character".to_string(),
                };
            }
            if let Some(ref a) = agent {
                if has_control_char(a) {
                    return Response::Error {
                        message: "invalid_field: agent: control_character".to_string(),
                    };
                }
            }
            let effective_limit: usize = match limit {
                None => 50,
                Some(0) => 50,
                Some(n) if n > 500 => {
                    return Response::Error {
                        message: format!("limit_too_large: requested {n}, max 500"),
                    };
                }
                Some(n) => n,
            };

            // Snapshot transaction: derive target_session_org from the target
            // session row, then list within the same transaction for a
            // consistent read. Per spec §10.2–10.3: this is "target-session
            // org consistency", NOT a cross-caller isolation guarantee — no
            // authenticated caller context exists in this phase.
            let tx = match state.conn.unchecked_transaction() {
                Ok(t) => t,
                Err(e) => {
                    return Response::Error {
                        message: format!("internal_error: {e}"),
                    }
                }
            };

            let target_session_org: String = match tx.query_row(
                "SELECT COALESCE(organization_id, 'default') FROM session WHERE id = ?1",
                rusqlite::params![&session_id],
                |row| row.get::<_, String>(0),
            ) {
                Ok(s) => s,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Response::Error {
                        message: format!("unknown_session: {session_id}"),
                    };
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("internal_error: {e}"),
                    };
                }
            };

            let rows = match crate::db::ops::list_tool_calls(
                &tx,
                &target_session_org,
                &session_id,
                agent.as_deref(),
                effective_limit,
            ) {
                Ok(r) => r,
                Err(e) => {
                    return Response::Error {
                        message: format!("internal_error: {e}"),
                    }
                }
            };

            if let Err(e) = tx.commit() {
                return Response::Error {
                    message: format!("internal_error: {e}"),
                };
            }

            Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls: rows },
            }
        }

        // Phase 2A-4c2 T6: ProbePhase — consolidator phase introspection.
        #[cfg(feature = "bench")]
        Request::ProbePhase { phase_name } => {
            let order = crate::workers::consolidator::PHASE_ORDER;
            match order.iter().position(|(n, _)| *n == phase_name) {
                Some(pos) => {
                    let (_, phase_number) = order[pos];
                    let executed_after: Vec<String> =
                        order[..pos].iter().map(|(n, _)| (*n).to_string()).collect();
                    Response::Ok {
                        data: forge_core::protocol::ResponseData::PhaseProbe {
                            executed_at_phase_index: phase_number,
                            executed_after,
                        },
                    }
                }
                None => Response::Error {
                    message: format!("unknown_phase: {phase_name}"),
                },
            }
        }

        // Phase 2A-4d.3.1 #2: StepDispositionOnce — drive one disposition
        // worker cycle on caller-provided synthetic sessions. Bench-only,
        // mirrors `tick_for_agent` math without touching the session table.
        #[cfg(feature = "bench")]
        Request::StepDispositionOnce {
            agent,
            synthetic_sessions,
        } => match crate::workers::disposition::step_for_bench(
            &state.conn,
            &agent,
            &synthetic_sessions,
        ) {
            Ok(summary) => Response::Ok {
                data: ResponseData::DispositionStep { summary },
            },
            Err(e) => Response::Error {
                message: format!("step_disposition_once failed: {e}"),
            },
        },

        // Phase 2A-4b: ComputeRecencyFactor handler — T12.
        #[cfg(feature = "bench")]
        Request::ComputeRecencyFactor { memory_id } => {
            let fetched = match ops::fetch_memory_by_id(&state.conn, &memory_id) {
                Ok(Some(m)) => m,
                Ok(None) => {
                    return Response::Error {
                        message: format!("memory not found: {memory_id}"),
                    };
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("fetch_memory_by_id failed: {e}"),
                    };
                }
            };
            let half_life = crate::config::load_config()
                .recall
                .validated()
                .preference_half_life_days;
            // Capture a single now_secs used for both days_since_anchor and factor,
            // guaranteeing bit-exact parity between the two derived values.
            let now_secs = ops::current_epoch_secs();
            // Mirror anchor-selection logic from ops::recency_factor.
            let anchor = if fetched.memory_type == forge_core::types::memory::MemoryType::Preference
            {
                match fetched.reaffirmed_at.as_deref() {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => fetched.created_at.clone(),
                }
            } else {
                fetched.created_at.clone()
            };
            let anchor_secs = ops::parse_timestamp_to_epoch(&anchor).unwrap_or(0.0);
            let days_since_anchor = ((now_secs - anchor_secs) / 86400.0).max(0.0);
            // Call the canonical formula — same now_secs → bit-exact.
            let factor = ops::recency_factor(&fetched, half_life, now_secs);
            Response::Ok {
                data: ResponseData::RecencyFactor {
                    memory_id,
                    factor,
                    days_since_anchor,
                    anchor,
                },
            }
        }

        Request::HealthByProject => match ops::health_by_project(&state.conn) {
            Ok(projects) => {
                let project_data: std::collections::HashMap<
                    String,
                    forge_core::protocol::HealthProjectData,
                > = projects
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            forge_core::protocol::HealthProjectData {
                                decisions: v.decisions,
                                lessons: v.lessons,
                                patterns: v.patterns,
                                preferences: v.preferences,
                            },
                        )
                    })
                    .collect();
                Response::Ok {
                    data: ResponseData::HealthByProject {
                        projects: project_data,
                    },
                }
            }
            Err(e) => Response::Error {
                message: format!("health_by_project failed: {e}"),
            },
        },

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

        Request::Version => Response::Ok {
            data: ResponseData::Version {
                version: env!("CARGO_PKG_VERSION").to_string(),
                build_profile: if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                }
                .to_string(),
                target_triple: env!("FORGE_TARGET").to_string(),
                rustc_version: env!("FORGE_RUSTC_VERSION").to_string(),
                git_sha: {
                    let sha = env!("FORGE_GIT_SHA");
                    if sha.is_empty() {
                        None
                    } else {
                        Some(sha.to_string())
                    }
                },
                uptime_secs: state.started_at.elapsed().as_secs(),
            },
        },

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
            let memory_count = h.decisions + h.lessons + h.patterns + h.preferences;
            let uptime_secs = state.started_at.elapsed().as_secs();

            // Compute db_size_bytes from PRAGMA page_count * page_size
            let db_size_bytes: u64 = state
                .conn
                .query_row(
                    "SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            // Build structured health checks
            let mut checks: Vec<forge_core::protocol::HealthCheck> = Vec::new();

            // 1. Daemon running
            checks.push(forge_core::protocol::HealthCheck {
                name: "daemon".into(),
                status: "ok".into(),
                message: format!("running (uptime: {uptime_secs}s)"),
            });

            // 2. Memory count
            checks.push(if memory_count > 0 {
                forge_core::protocol::HealthCheck {
                    name: "memories".into(),
                    status: "ok".into(),
                    message: format!("{memory_count} memories stored"),
                }
            } else {
                forge_core::protocol::HealthCheck {
                    name: "memories".into(),
                    status: "warn".into(),
                    message: "no memories stored — run `forge-next remember` or ingest transcripts"
                        .into(),
                }
            });

            // 3. Embedding count
            checks.push(if embeddings > 0 {
                forge_core::protocol::HealthCheck {
                    name: "embeddings".into(),
                    status: "ok".into(),
                    message: format!("{embeddings} embeddings indexed"),
                }
            } else {
                forge_core::protocol::HealthCheck {
                    name: "embeddings".into(),
                    status: "warn".into(),
                    // P3-4 Phase 10E (F-MED-8): pre-10E this said only
                    // "no embeddings — vector recall will not work" with
                    // no fix hint. Embeddings auto-generate via the
                    // embedder worker on every memory insert; if the
                    // count stays 0 after new memories arrive, the
                    // embedder is stalled (Ollama down, model missing).
                    message: "no embeddings — vector recall will not work \
                              (run `forge-next observe row-count --table memory_vec` to confirm; \
                              embeddings auto-generate via the embedder worker on insert — \
                              if count stays 0, check `forge-next observe phase-summary` \
                              and the embedder worker logs)"
                        .into(),
                }
            });

            // 4. Database size
            checks.push(if db_size_bytes < 500 * 1024 * 1024 {
                forge_core::protocol::HealthCheck {
                    name: "db_size".into(),
                    status: "ok".into(),
                    message: format!("{:.1} MB", db_size_bytes as f64 / (1024.0 * 1024.0)),
                }
            } else {
                forge_core::protocol::HealthCheck {
                    name: "db_size".into(),
                    status: "warn".into(),
                    message: format!(
                        "{:.1} MB — consider running consolidation",
                        db_size_bytes as f64 / (1024.0 * 1024.0)
                    ),
                }
            });

            // P3-4 Wave Z (Z10) per CC voice feedback §2.7: backup hygiene.
            // Operator-created `*.bak` files in ~/.forge/ accumulate from
            // pre-migration safety copies (the CC voice user observed
            // ~1 GB across 5 files in 2 days). Warn when total exceeds
            // 1 GB OR the count exceeds 5 — both heuristic floors that
            // catch the "I forgot to clean these up" footgun without
            // false-positives on a single recent backup.
            let forge_dir = std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".forge"));
            let (backup_count, backup_bytes) = forge_dir
                .as_deref()
                .and_then(|d| {
                    std::fs::read_dir(d).ok().map(|entries| {
                        entries
                            .flatten()
                            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
                            .filter_map(|e| e.metadata().ok().map(|m| m.len()))
                            .fold((0usize, 0u64), |(n, b), size| (n + 1, b + size))
                    })
                })
                .unwrap_or((0, 0));
            const BACKUP_BYTES_WARN: u64 = 1024 * 1024 * 1024; // 1 GB
            const BACKUP_COUNT_WARN: usize = 5;
            if backup_count > 0 {
                let bytes_mb = backup_bytes as f64 / (1024.0 * 1024.0);
                if backup_bytes >= BACKUP_BYTES_WARN || backup_count >= BACKUP_COUNT_WARN {
                    checks.push(forge_core::protocol::HealthCheck {
                        name: "backup_hygiene".into(),
                        status: "warn".into(),
                        message: format!(
                            "{backup_count} *.bak file(s), {bytes_mb:.0} MB in ~/.forge — \
                             move oldest to ~/.forge/backups/ or compress with \
                             `gzip ~/.forge/forge.db.pre-*.bak` (CC voice feedback §2.7)"
                        ),
                    });
                } else {
                    checks.push(forge_core::protocol::HealthCheck {
                        name: "backup_hygiene".into(),
                        status: "ok".into(),
                        message: format!("{backup_count} *.bak file(s), {bytes_mb:.0} MB"),
                    });
                }
            }

            // 5. Extraction backend configured
            let config = crate::config::load_config();
            let backend = &config.extraction.backend;
            checks.push(if backend != "auto" {
                forge_core::protocol::HealthCheck {
                    name: "extraction_backend".into(),
                    status: "ok".into(),
                    message: format!("configured: {backend}"),
                }
            } else {
                // auto means it tries multiple — check if any API key is available
                let has_claude_key = crate::config::resolve_api_key(
                    &config.extraction.claude_api.api_key,
                    "ANTHROPIC_API_KEY",
                )
                .is_some();
                let has_openai_key = crate::config::resolve_api_key(
                    &config.extraction.openai.api_key,
                    "OPENAI_API_KEY",
                )
                .is_some();
                let has_gemini_key = crate::config::resolve_api_key(
                    &config.extraction.gemini.api_key,
                    "GEMINI_API_KEY",
                )
                .is_some();
                if has_claude_key || has_openai_key || has_gemini_key {
                    forge_core::protocol::HealthCheck {
                        name: "extraction_backend".into(),
                        status: "ok".into(),
                        message: "auto (API keys available)".into(),
                    }
                } else {
                    forge_core::protocol::HealthCheck {
                        name: "extraction_backend".into(),
                        status: "warn".into(),
                        message:
                            "auto with no API keys — extraction may fall back to ollama or fail"
                                .into(),
                    }
                }
            });

            // 6. Plugin hooks installed (Claude Code plugin surface check — 2P-1a).
            //    Looks for a plugin hooks.json at the canonical install path(s).
            //    If present, reports OK + event count. If the user has a plugin
            //    install root (`~/.claude/plugins/forge/` or `CLAUDE_PLUGIN_ROOT`)
            //    but no hooks.json inside it, that's a real misconfiguration → warn.
            //    If neither root exists, the daemon is running standalone (e.g.
            //    in-tree development, server install without the Claude Code
            //    plugin) — emit OK with an informational message instead of a
            //    misleading warning. (W25/F2.)
            let hook_paths = [
                std::env::var("HOME").ok().map(|h| {
                    std::path::PathBuf::from(h).join(".claude/plugins/forge/hooks/hooks.json")
                }),
                std::env::var("CLAUDE_PLUGIN_ROOT")
                    .ok()
                    .map(|r| std::path::PathBuf::from(r).join("hooks/hooks.json")),
            ];
            let hook_file = hook_paths.iter().flatten().find(|p| p.exists());
            // F2: detect whether the user has a plugin-install root at all.
            let plugin_install_dirs = [
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".claude/plugins/forge")),
                std::env::var("CLAUDE_PLUGIN_ROOT")
                    .ok()
                    .map(std::path::PathBuf::from),
            ];
            // W1.32 (W28 review LOW-8): use `symlink_metadata` so a broken
            // symlink at `~/.claude/plugins/forge` (target deleted) is still
            // detected as "plugin install present" — `is_dir()` follows the
            // link and silently reports false, which would mis-classify a
            // misconfigured install as standalone. Treat the path as
            // "install root present" if the path resolves to a directory
            // OR exists as a symlink at all (broken-or-not).
            let has_plugin_install = plugin_install_dirs.iter().flatten().any(|p| {
                std::fs::symlink_metadata(p)
                    .map(|m| m.is_dir() || m.file_type().is_symlink())
                    .unwrap_or(false)
            });
            checks.push(match hook_file {
                Some(p) => {
                    let event_count: usize = std::fs::read_to_string(p)
                        .ok()
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
                        .and_then(|v| v.get("hooks").and_then(|h| h.as_object()).map(|o| o.len()))
                        .unwrap_or(0);
                    forge_core::protocol::HealthCheck {
                        name: "hook".into(),
                        status: "ok".into(),
                        message: format!("plugin hooks installed ({event_count} events)"),
                    }
                }
                None if has_plugin_install => forge_core::protocol::HealthCheck {
                    name: "hook".into(),
                    status: "warn".into(),
                    message: "plugin hooks.json not found — install from chaosmaximus/forge marketplace or symlink hooks/hooks.json into ~/.claude/plugins/forge/".into(),
                },
                None => forge_core::protocol::HealthCheck {
                    name: "hook".into(),
                    status: "ok".into(),
                    message: "running outside a Claude Code plugin install (no hooks expected)"
                        .into(),
                },
            });

            let raw_doc_count = crate::db::raw::count_documents(&state.conn).unwrap_or(0);
            let raw_chunk_count = crate::db::raw::count_chunks(&state.conn).unwrap_or(0);
            let active_session_count =
                crate::sessions::count_active_sessions(&state.conn).unwrap_or(0);
            let session_message_count =
                crate::sessions::count_all_messages(&state.conn).unwrap_or(0);

            Response::Ok {
                data: ResponseData::Doctor {
                    daemon_up: true,
                    db_size_bytes,
                    memory_count,
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
                    uptime_secs,
                    platform_count: mh.platform_entries,
                    tool_count: mh.tools,
                    skill_count: mh.skills,
                    domain_dna_count: mh.domain_dna_entries,
                    perception_count: mh.perceptions_unconsumed,
                    declared_count: mh.declared_entries,
                    identity_count: mh.identity_facets_active,
                    disposition_count: mh.dispositions,
                    checks,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    git_sha: {
                        let sha = env!("FORGE_GIT_SHA");
                        if sha.is_empty() {
                            None
                        } else {
                            Some(sha.to_string())
                        }
                    },
                    raw_document_count: raw_doc_count,
                    raw_chunk_count,
                    active_session_count,
                    session_message_count,
                },
            }
        }

        Request::Export {
            format: _,
            since: _,
        } => {
            let memories = ops::export_memories(&state.conn).unwrap_or_default();
            let files = ops::export_files(&state.conn).unwrap_or_default();
            let symbols = ops::export_symbols(&state.conn).unwrap_or_default();
            let edges = ops::export_edges(&state.conn).unwrap_or_default();

            let memory_results: Vec<MemoryResult> = memories
                .into_iter()
                .map(|m| MemoryResult {
                    memory: m,
                    score: 1.0,
                    source: "export".into(),
                    edges: Vec::new(),
                })
                .collect();

            let export_edges: Vec<ExportEdge> = edges
                .into_iter()
                .map(|(from, to, etype, props)| ExportEdge {
                    from_id: from,
                    to_id: to,
                    edge_type: etype,
                    properties: serde_json::from_str(&props).unwrap_or(serde_json::Value::Null),
                })
                .collect();

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
                    message: format!(
                        "import exceeds {max_records} record limit ({total_records} records)"
                    ),
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

        Request::IngestClaude => match claude_memory::ingest_claude_memories(&state.conn) {
            Ok((imported, skipped)) => Response::Ok {
                data: ResponseData::IngestClaude { imported, skipped },
            },
            Err(e) => Response::Error {
                message: format!("ingest-claude failed: {e}"),
            },
        },

        Request::IngestDeclared {
            path,
            source,
            project,
        } => {
            match crate::db::manas::ingest_declared_file(
                &state.conn,
                &path,
                &source,
                project.as_deref(),
            ) {
                Ok(ingested) => Response::Ok {
                    data: ResponseData::IngestDeclared { ingested, path },
                },
                Err(e) => Response::Error {
                    message: format!("ingest-declared failed: {e}"),
                },
            }
        }

        Request::Backfill { path } => {
            // C-1: Validate path is under ~/.claude/ to prevent arbitrary file read
            let home = std::env::var("HOME").unwrap_or_default();
            let allowed_dir = format!("{home}/.claude/");
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
                    message: format!("backfill failed to read {path}: {e}"),
                },
            }
        }

        Request::RegisterSession {
            id,
            agent,
            project,
            cwd,
            capabilities,
            current_task,
            role,
        } => {
            let agent_clone = agent.clone();
            let caps_json = capabilities
                .map(|c| serde_json::to_string(&c).unwrap_or_else(|_| "[]".to_string()));
            match crate::sessions::register_session(
                &state.conn,
                &id,
                &agent,
                project.as_deref(),
                cwd.as_deref(),
                caps_json.as_deref(),
                current_task.as_deref(),
                role.as_deref(),
            ) {
                Ok(()) => {
                    // Auto-detect reality from cwd and tag the session
                    if let Some(ref cwd_path) = cwd {
                        use crate::project::CodeProjectEngine;
                        use forge_core::types::project_engine::ProjectEngine;

                        let engine = CodeProjectEngine;
                        let path = std::path::Path::new(cwd_path);
                        if let Some(detection) = engine.detect(path) {
                            // Check if reality already exists for this path
                            let reality_id = match ops::get_project_by_path(
                                &state.conn,
                                cwd_path,
                                "default",
                            ) {
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
                                    let reality = forge_core::types::Project {
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
                                    match ops::store_project(&state.conn, &reality) {
                                        Ok(()) => Some(rid),
                                        Err(e) => {
                                            eprintln!("[handler] auto-detect: failed to store reality for {cwd_path}: {e}");
                                            None
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[handler] auto-detect: failed to check reality for {cwd_path}: {e}");
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

                    crate::events::emit(
                        &state.events,
                        "session_changed",
                        serde_json::json!({
                            "id": id,
                            "agent": agent_clone,
                            "action": "registered",
                            "project": project,
                            "cwd": cwd,
                        }),
                    );
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
                eprintln!("[handler] failed to save working set for session {id}: {e}");
            }

            // Compile session KPIs before ending
            let session_kpis = crate::sessions::compile_session_kpis(&state.conn, &id);

            match crate::sessions::end_session(&state.conn, &id) {
                Ok(found) => {
                    if found {
                        crate::events::emit(
                            &state.events,
                            "session_changed",
                            serde_json::json!({
                                "id": id,
                                "action": "ended",
                                "kpis": serde_json::to_value(&session_kpis).ok(),
                            }),
                        );
                    }
                    Response::Ok {
                        data: ResponseData::SessionEnded {
                            id,
                            found,
                            session_kpis,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("end_session failed: {e}"),
                },
            }
        }

        Request::SessionUpdate { id, project, cwd } => {
            // P3-4 Wave Z (Z8) — fix misregistered session bindings
            // (e.g. SessionStart fired in a parent dir, user cd'd into
            // a subproject, now wants the right project label without
            // ending+restarting). CC voice feedback §2.6.
            //
            // Existence check first so the caller gets a clear "session
            // not found" message instead of a silent zero-row UPDATE.
            let exists: bool = state
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if !exists {
                return Response::Error {
                    message: format!("session '{id}' not found"),
                };
            }

            let mut fields = Vec::new();
            if let Some(ref p) = project {
                if p.trim().is_empty() {
                    return Response::Error {
                        message: "project must be non-empty".into(),
                    };
                }
                let n = state
                    .conn
                    .execute(
                        "UPDATE session SET project = ?1 WHERE id = ?2",
                        rusqlite::params![p, id],
                    )
                    .unwrap_or(0);
                if n > 0 {
                    fields.push("project".to_string());
                }
            }
            if let Some(ref c) = cwd {
                if c.trim().is_empty() {
                    return Response::Error {
                        message: "cwd must be non-empty".into(),
                    };
                }
                let n = state
                    .conn
                    .execute(
                        "UPDATE session SET cwd = ?1 WHERE id = ?2",
                        rusqlite::params![c, id],
                    )
                    .unwrap_or(0);
                if n > 0 {
                    fields.push("cwd".to_string());
                }
            }

            if fields.is_empty() {
                return Response::Error {
                    message: "session_update: no fields supplied (pass --project and/or --cwd)"
                        .into(),
                };
            }

            crate::events::emit(
                &state.events,
                "session_changed",
                serde_json::json!({
                    "id": id,
                    "action": "updated",
                    "fields": fields,
                }),
            );

            Response::Ok {
                data: ResponseData::SessionUpdated { id, fields },
            }
        }

        Request::SessionHeartbeat { session_id } => {
            match crate::sessions::update_heartbeat(&state.conn, &session_id) {
                Ok(true) => Response::Ok {
                    data: ResponseData::Heartbeat {
                        session_id,
                        status: "ok".into(),
                    },
                },
                Ok(false) => Response::Error {
                    message: "heartbeat rejected".into(),
                },
                Err(e) => Response::Error {
                    message: format!("heartbeat failed: {e}"),
                },
            }
        }

        Request::Sessions { active_only } => {
            match crate::sessions::list_sessions(&state.conn, active_only.unwrap_or(true)) {
                Ok(sessions) => {
                    let count = sessions.len();
                    let infos: Vec<forge_core::protocol::SessionInfo> = sessions
                        .into_iter()
                        .map(|s| {
                            let caps: Vec<String> =
                                serde_json::from_str(&s.capabilities).unwrap_or_default();
                            forge_core::protocol::SessionInfo {
                                id: s.id,
                                agent: s.agent,
                                project: s.project,
                                cwd: s.cwd,
                                started_at: s.started_at,
                                ended_at: s.ended_at,
                                status: s.status,
                                capabilities: caps,
                                current_task: s.current_task,
                            }
                        })
                        .collect();
                    Response::Ok {
                        data: ResponseData::Sessions {
                            sessions: infos,
                            count,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_sessions failed: {e}"),
                },
            }
        }

        Request::CleanupSessions {
            prefix,
            older_than_secs,
            prune_ended,
        } => {
            let mut total_ended = 0usize;
            let mut total_pruned = 0usize;

            // Phase 1: Prefix-based cleanup — only run when prefix is set OR no age filter
            // Without this guard, cleanup_sessions(None) ends ALL active sessions (nuclear)
            if prefix.is_some() || older_than_secs.is_none() {
                match crate::sessions::cleanup_sessions(&state.conn, prefix.as_deref()) {
                    Ok(ended) => {
                        total_ended += ended;
                    }
                    Err(e) => {
                        return Response::Error {
                            message: format!("cleanup_sessions failed: {e}"),
                        };
                    }
                }
            }

            // Phase 2: Age-based cleanup (end active sessions older than threshold)
            // Uses SQLite datetime() for cutoff calculation — no chrono dependency needed
            if let Some(secs) = older_than_secs {
                let age_ended: usize = state.conn.execute(
                    &format!("UPDATE session SET status = 'ended' WHERE status IN ('active', 'idle') AND created_at < datetime('now', '-{secs} seconds')"),
                    [],
                ).unwrap_or(0);
                total_ended += age_ended;

                // Phase 3: Prune (delete) already-ended sessions past age threshold
                if prune_ended {
                    let pruned: usize = state.conn.execute(
                        &format!("DELETE FROM session WHERE status = 'ended' AND created_at < datetime('now', '-{secs} seconds')"),
                        [],
                    ).unwrap_or(0);
                    total_pruned = pruned;
                }
            }

            eprintln!("[sessions] cleanup: ended {total_ended} sessions, pruned {total_pruned} (prefix: {prefix:?}, older_than: {older_than_secs:?}s)");
            crate::events::emit(
                &state.events,
                "session_changed",
                serde_json::json!({
                    "action": "cleanup",
                    "ended": total_ended,
                    "pruned": total_pruned,
                    "prefix": prefix,
                }),
            );
            Response::Ok {
                data: ResponseData::SessionsCleaned {
                    ended: total_ended + total_pruned,
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
                crate::events::emit(
                    &state.events,
                    "guardrail_warning",
                    serde_json::json!({
                        "file": file,
                        "safe": false,
                        "warnings": result.warnings.clone(),
                        "decisions_affected": result.decisions_affected.clone(),
                        "callers_count": result.callers_count,
                        "calling_files": result.calling_files.clone(),
                        "relevant_lessons": result.relevant_lessons.clone(),
                        "dangerous_patterns": result.dangerous_patterns.clone(),
                        "applicable_skills": result.applicable_skills.clone(),
                    }),
                );
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

        Request::PreBashCheck {
            command,
            session_id,
        } => {
            let result = crate::guardrails::check::pre_bash_check(&state.conn, &command);

            // Emit bash_warning event when check returns unsafe
            if !result.safe {
                crate::events::emit(
                    &state.events,
                    "bash_warning",
                    serde_json::json!({
                        "command": command,
                        "safe": false,
                        "warnings": result.warnings.clone(),
                        "relevant_skills": result.relevant_skills.clone(),
                    }),
                );
            }

            // Proactive context injection via Prajna matrix
            let proactive_context = crate::proactive::build_proactive_context(
                &state.conn,
                crate::proactive::HOOK_PRE_BASH,
                None,
            );

            // Record injection for observability (#45) — use explicit session_id
            // from the Request when present; fall back to the most recently active
            // session (any agent) so old hook clients still record.
            let sid = resolve_hook_session_id(&state.conn, session_id.as_deref());
            record_proactive_injection(
                state.writer_tx.as_ref(),
                &sid,
                "PreBashCheck",
                &proactive_context,
            );

            Response::Ok {
                data: ResponseData::PreBashChecked {
                    safe: result.safe,
                    warnings: result.warnings,
                    relevant_skills: result.relevant_skills,
                    proactive_context,
                },
            }
        }

        Request::PostBashCheck {
            command,
            exit_code,
            session_id,
        } => {
            let result =
                crate::guardrails::check::post_bash_check(&state.conn, &command, exit_code);
            let proactive_context = crate::proactive::build_proactive_context(
                &state.conn,
                crate::proactive::HOOK_POST_BASH,
                None,
            );

            // Record injection for observability (#45) — helper no-ops when
            // chars_injected is 0 (common on fresh DBs: PostBashCheck relevance
            // is 0.1 for all knowledge types, below 0.3 threshold).
            let sid = resolve_hook_session_id(&state.conn, session_id.as_deref());
            record_proactive_injection(
                state.writer_tx.as_ref(),
                &sid,
                "PostBashCheck",
                &proactive_context,
            );

            Response::Ok {
                data: ResponseData::PostBashChecked {
                    suggestions: result.suggestions,
                    proactive_context,
                },
            }
        }

        Request::PostEditCheck { file, session_id } => {
            let result = crate::guardrails::check::post_edit_check(&state.conn, &file);

            // Emit event if there are any warnings worth surfacing
            if !result.dangerous_patterns.is_empty()
                || result.callers_count > 0
                || !result.decisions_to_review.is_empty()
            {
                crate::events::emit(
                    &state.events,
                    "post_edit_warning",
                    serde_json::json!({
                        "file": file,
                        "callers": result.callers_count,
                        "warnings": result.dangerous_patterns.len() + result.decisions_to_review.len(),
                    }),
                );
            }

            let proactive_context = crate::proactive::build_proactive_context(
                &state.conn,
                crate::proactive::HOOK_POST_EDIT,
                None,
            );

            // Record injection for observability (#45).
            let sid = resolve_hook_session_id(&state.conn, session_id.as_deref());
            record_proactive_injection(
                state.writer_tx.as_ref(),
                &sid,
                "PostEditCheck",
                &proactive_context,
            );

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
                    proactive_context,
                },
            }
        }

        Request::BlastRadius {
            file,
            project: project_filter,
        } => {
            // Phase 2A-4d.3.1 #3: when context_injection.blast_radius = false,
            // return an empty result. The CLI surface still works for explicit
            // queries; we only suppress passive injection.
            if !crate::config::load_config().context_injection.blast_radius {
                return Response::Ok {
                    data: ResponseData::BlastRadius {
                        decisions: Vec::new(),
                        callers: 0,
                        importers: Vec::new(),
                        files_affected: Vec::new(),
                        cluster_name: None,
                        cluster_files: Vec::new(),
                        warnings: vec![
                            // Phase 2A-4d.3.1 #3 H5 (W5): give the operator
                            // an actionable next step rather than a bare
                            // "suppressed" notice — they shouldn't have to
                            // grep the source to learn how to re-enable.
                            "blast-radius injection is currently disabled. \
                             To re-enable: `forge-next config set context_injection.blast_radius true` \
                             (or set FORGE_CONTEXT_INJECTION_BLAST_RADIUS=true). \
                             This message and the empty result come from the \
                             daemon's gate at `handler::Request::BlastRadius`; \
                             the analysis itself was not run."
                                .to_string(),
                        ],
                        calling_files: Vec::new(),
                    },
                };
            }
            let br = crate::guardrails::blast_radius::analyze_blast_radius(
                &state.conn,
                &file,
                project_filter.as_deref(),
            );
            let decisions: Vec<forge_core::protocol::BlastRadiusDecision> = br
                .decisions
                .into_iter()
                .map(
                    |(id, title, confidence)| forge_core::protocol::BlastRadiusDecision {
                        id,
                        title,
                        confidence,
                    },
                )
                .collect();
            // Warn if file extension is not indexed by the code graph
            let mut warnings = Vec::new();
            let indexed_exts = ["rs", "ts", "tsx", "js", "jsx", "py", "go"];
            if let Some(ext) = std::path::Path::new(&file)
                .extension()
                .and_then(|e| e.to_str())
            {
                if !indexed_exts.contains(&ext) {
                    warnings.push(format!(
                        "Language not indexed — blast-radius unavailable for .{ext} files. Indexed: .rs, .ts, .tsx, .js, .jsx, .py, .go"
                    ));
                }
            }
            // Warn if the code graph appears empty (no indexed files at all)
            let file_count: usize = state
                .conn
                .query_row("SELECT COUNT(*) FROM code_file", [], |row| row.get(0))
                .unwrap_or(0);
            if file_count == 0 {
                warnings.push(
                    "No code graph available. The indexer has not run for this project yet. \
                     Ensure the daemon is running and the project directory is detected. \
                     Check: forge-next doctor"
                        .to_string(),
                );
            } else if decisions.is_empty() && br.callers == 0 && br.importers.is_empty() {
                // Code graph exists but no edges for this specific file.
                // P3-4 W1.2 c2 (I-7): if --project was set, scope the
                // existence-check to that project so the warning text
                // can distinguish "file unknown" from "file indexed
                // under a different project".
                //
                // P3-4 W1.14 (W1.3 review LOW-3): compute file_exists
                // directly; the prior `(bool, &str)` tuple `scope_msg`
                // was populated but never read (the actual scope hint
                // is reconstructed below from `project_filter`).
                let file_exists: bool = match project_filter.as_deref() {
                    Some(proj) => state
                        .conn
                        .query_row(
                            "SELECT COUNT(*) > 0 FROM code_file WHERE path LIKE ?1 AND project = ?2",
                            rusqlite::params![format!("%{}", file), proj],
                            |row| row.get(0),
                        )
                        .unwrap_or(false),
                    None => state
                        .conn
                        .query_row(
                            "SELECT COUNT(*) > 0 FROM code_file WHERE path LIKE ?1",
                            rusqlite::params![format!("%{}", file)],
                            |row| row.get(0),
                        )
                        .unwrap_or(false),
                };
                if !file_exists {
                    let scope_hint = if let Some(proj) = project_filter.as_deref() {
                        format!(" (scoped to project '{proj}')")
                    } else {
                        String::new()
                    };
                    warnings.push(format!(
                        "File '{file}' not found in the code graph{scope_hint}. It may not have been indexed yet."
                    ));
                }
            }
            Response::Ok {
                data: ResponseData::BlastRadius {
                    decisions,
                    callers: br.callers,
                    importers: br.importers,
                    files_affected: br.files_affected,
                    cluster_name: br.cluster_name,
                    cluster_files: br.cluster_files,
                    calling_files: br.calling_files,
                    warnings,
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

        Request::ListPlatform => match crate::db::manas::list_platform(&state.conn) {
            Ok(entries) => Response::Ok {
                data: ResponseData::PlatformList { entries },
            },
            Err(e) => Response::Error {
                message: format!("list_platform failed: {e}"),
            },
        },

        Request::StoreTool { tool } => {
            let id = tool.id.clone();
            let tool_name = tool.name.clone();
            match crate::db::manas::store_tool(&state.conn, &tool) {
                Ok(()) => {
                    crate::events::emit(
                        &state.events,
                        "tool_discovered",
                        serde_json::json!({
                            "id": id,
                            "name": tool_name,
                            "source": "manual",
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::ToolStored { id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("store_tool failed: {e}"),
                },
            }
        }

        Request::ListTools => match crate::db::manas::list_tools(&state.conn, None) {
            Ok(tools) => {
                let count = tools.len();
                Response::Ok {
                    data: ResponseData::ToolList { tools, count },
                }
            }
            Err(e) => Response::Error {
                message: format!("list_tools failed: {e}"),
            },
        },

        Request::StorePerception { perception } => {
            let id = perception.id.clone();
            let kind_str = format!("{:?}", perception.kind);
            match crate::db::manas::store_perception(&state.conn, &perception) {
                Ok(()) => {
                    crate::events::emit(
                        &state.events,
                        "perception_update",
                        serde_json::json!({
                            "id": id,
                            "kind": kind_str,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::PerceptionStored { id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("store_perception failed: {e}"),
                },
            }
        }

        Request::ListPerceptions {
            project,
            limit,
            offset,
        } => {
            let off = offset.unwrap_or(0);
            let lim = limit.unwrap_or(20).min(100); // Cap at 100
            match crate::db::manas::list_unconsumed_perceptions(&state.conn, None, None) {
                Ok(perceptions) => {
                    // Apply project filter, offset, and limit in-memory
                    let filtered: Vec<_> = perceptions
                        .into_iter()
                        .filter(|p| match (&project, &p.project) {
                            (Some(proj), Some(pp)) => pp == proj,
                            (Some(_), None) => false,
                            (None, _) => true,
                        })
                        .skip(off)
                        .take(lim)
                        .collect();
                    let count = filtered.len();
                    Response::Ok {
                        data: ResponseData::PerceptionList {
                            perceptions: filtered,
                            count,
                        },
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
                    crate::events::emit(
                        &state.events,
                        "identity_updated",
                        serde_json::json!({
                            "id": id,
                            "facet": facet_name,
                            "agent": agent_name,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::IdentityStored { id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("store_identity failed: {e}"),
                },
            }
        }

        Request::ListIdentity {
            agent,
            project,
            include_global_identity,
        } => {
            // Use list_identity_for_user to include user-scoped facets.
            // Default to "local" user (single-user daemon); the fallback path in
            // list_identity_for_user(None, ...) delegates to plain list_identity.
            let user_id = ops::get_user(&state.conn, "local")
                .ok()
                .flatten()
                .map(|u| u.id);
            // P3-3.11 W30 (closes F16): when a project is supplied, route
            // through the project-scoped variant; `include_global_identity`
            // (default false) gates the `_global_` sentinel into the
            // result, mirroring the W29 `Recall.include_globals` opt-in.
            let result = match project.as_deref() {
                Some(p) => crate::db::manas::list_identity_for_user_project(
                    &state.conn,
                    user_id.as_deref(),
                    &agent,
                    p,
                    include_global_identity.unwrap_or(false),
                    true,
                ),
                None => crate::db::manas::list_identity_for_user(
                    &state.conn,
                    user_id.as_deref(),
                    &agent,
                    true,
                ),
            };
            match result {
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

        Request::ManasHealth { project } => match crate::db::manas::manas_health(&state.conn) {
            Ok(mh) => {
                let is_new = if let Some(ref proj) = project {
                    crate::db::manas::is_new_project(&state.conn, proj).unwrap_or_else(|e| {
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
            }
            Err(e) => Response::Error {
                message: format!("manas_health failed: {e}"),
            },
        },

        // SessionHeartbeat handled earlier in the match (near EndSession)

        // ── Proactive Context (Prajna) ──
        Request::ContextRefresh { session_id, since } => {
            let since_clause = since.as_deref().unwrap_or("2000-01-01T00:00:00Z");

            // Session-scoped: get project from session for scoping
            let session_project: Option<String> = state
                .conn
                .query_row(
                    "SELECT project FROM session WHERE id = ?1",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .ok()
                .flatten();

            // Notifications scoped to session's target
            let notifications: Vec<String> = state
                .conn
                .prepare(
                    "SELECT title FROM notification WHERE status = 'pending' AND created_at > ?1
                 AND (target_id = ?2 OR target_id IS NULL)
                 ORDER BY created_at DESC LIMIT 3",
                )
                .ok()
                .map(|mut stmt| {
                    stmt.query_map(rusqlite::params![since_clause, session_id], |row| {
                        row.get(0)
                    })
                    .ok()
                    .map(|rows| rows.filter_map(|r| r.ok()).collect())
                    .unwrap_or_default()
                })
                .unwrap_or_default();

            // Anti-pattern perceptions (project-scoped)
            let anti_patterns: Vec<String> = state.conn.prepare(
                "SELECT data FROM perception WHERE kind = 'Warning' AND consumed = 0 AND created_at > ?1
                 AND (project = ?2 OR project IS NULL)
                 ORDER BY created_at DESC LIMIT 3"
            ).ok()
                .map(|mut stmt| stmt.query_map(rusqlite::params![since_clause, session_project], |row| row.get(0))
                    .ok().map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default())
                .unwrap_or_default();

            // Warning perceptions (not anti-pattern — general warnings)
            let warnings: Vec<String> = state.conn.prepare(
                "SELECT data FROM perception WHERE kind = 'Error' AND consumed = 0 AND created_at > ?1
                 AND (project = ?2 OR project IS NULL)
                 ORDER BY created_at DESC LIMIT 3"
            ).ok()
                .map(|mut stmt| stmt.query_map(rusqlite::params![since_clause, session_project], |row| row.get(0))
                    .ok().map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default())
                .unwrap_or_default();

            let messages_pending: usize = state.conn.query_row(
                "SELECT COUNT(*) FROM session_message WHERE to_session = ?1 AND status = 'pending'",
                rusqlite::params![session_id],
                |row| row.get::<_, i64>(0),
            ).unwrap_or(0) as usize;

            // Fetch actual A2A message summaries (top 5, capped at 200 chars each)
            let message_summaries: Vec<String> = {
                let mut summaries = Vec::new();
                if let Ok(mut stmt) = state.conn.prepare(
                    "SELECT from_session, topic, parts FROM session_message \
                     WHERE to_session = ?1 AND status = 'pending' \
                     ORDER BY created_at DESC LIMIT 5",
                ) {
                    if let Ok(rows) = stmt.query_map(rusqlite::params![session_id], |row| {
                        let from: String = row.get(0)?;
                        let topic: String = row.get(1)?;
                        let parts: String = row.get(2)?;
                        Ok((from, topic, parts))
                    }) {
                        for row in rows.flatten() {
                            let (from, topic, parts) = row;
                            // Cap from/topic to prevent oversized summaries
                            let from_cap: String = from.chars().take(40).collect();
                            let topic_cap: String = topic.chars().take(60).collect();
                            // Extract text from parts JSON, cap at 200 chars
                            let text =
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&parts) {
                                    v.as_array()
                                        .and_then(|arr| arr.iter().find(|p| p["kind"] == "text"))
                                        .and_then(|p| p["text"].as_str())
                                        .unwrap_or("")
                                        .chars()
                                        .take(200)
                                        .collect::<String>()
                                } else {
                                    String::new()
                                };
                            summaries.push(format!("[from:{from_cap}] ({topic_cap}) {text}"));
                        }
                    }
                }
                summaries
            };

            // Record injection for observability — route through writer channel
            // since ContextRefresh runs on a read-only connection in production
            let delta_items = notifications.len() + warnings.len() + anti_patterns.len();
            if delta_items > 0 || messages_pending > 0 {
                let summary = format!(
                    "notif={} warn={} ap={} msg={} summaries={}",
                    notifications.len(),
                    warnings.len(),
                    anti_patterns.len(),
                    messages_pending,
                    message_summaries.len()
                );
                let chars_est = notifications
                    .iter()
                    .chain(warnings.iter())
                    .chain(anti_patterns.iter())
                    .chain(message_summaries.iter())
                    .map(|s| s.len())
                    .sum::<usize>();
                if let Some(tx) = &state.writer_tx {
                    let _ = tx.try_send(super::writer::WriteCommand::RecordInjection {
                        session_id: session_id.clone(),
                        hook_event: "UserPromptSubmit".to_string(),
                        context_type: "delta".to_string(),
                        content_summary: summary,
                        chars_injected: chars_est,
                    });
                }
            }

            Response::Ok {
                data: ResponseData::ContextDelta {
                    notifications,
                    warnings,
                    anti_patterns,
                    messages_pending,
                    message_summaries,
                },
            }
        }

        Request::CompletionCheck {
            session_id,
            claimed_done,
        } => {
            // Multi-tenant: scope completion check to the session's organization
            let completion_org_id = get_session_org_id(&state.conn, Some(&session_id));
            if !claimed_done {
                Response::Ok {
                    data: ResponseData::CompletionCheckResult {
                        has_completion_signal: false,
                        relevant_lessons: vec![],
                        severity: "none".into(),
                    },
                }
            } else {
                let lessons: Vec<String> = state.conn.prepare(
                    "SELECT title || ': ' || SUBSTR(content, 1, 150) FROM memory
                     WHERE memory_type IN ('lesson', 'decision') AND status = 'active'
                     AND (organization_id = ?1 OR ?1 IS NULL)
                     AND (tags LIKE '%testing%' OR tags LIKE '%production-readiness%' OR tags LIKE '%anti-pattern%' OR tags LIKE '%uat%' OR tags LIKE '%deployment%')
                     ORDER BY quality_score DESC, confidence DESC LIMIT 3"
                ).ok()
                    .map(|mut stmt| stmt.query_map(rusqlite::params![completion_org_id], |row| row.get(0))
                        .ok().map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default())
                    .unwrap_or_default();

                let severity = if lessons.is_empty() { "none" } else { "high" };
                // Record injection for observability — route through writer channel
                // since CompletionCheck runs on a read-only connection in production
                if !lessons.is_empty() {
                    let chars_est: usize = lessons.iter().map(|s| s.len()).sum();
                    if let Some(tx) = &state.writer_tx {
                        let _ = tx.try_send(super::writer::WriteCommand::RecordInjection {
                            session_id: session_id.clone(),
                            hook_event: "Stop".to_string(),
                            context_type: "completion_lesson".to_string(),
                            content_summary: format!(
                                "{} lessons, severity={}",
                                lessons.len(),
                                severity
                            ),
                            chars_injected: chars_est,
                        });
                    }
                }
                Response::Ok {
                    data: ResponseData::CompletionCheckResult {
                        has_completion_signal: true,
                        relevant_lessons: lessons,
                        severity: severity.into(),
                    },
                }
            }
        }

        Request::TaskCompletionCheck {
            session_id,
            task_subject,
            task_description: _,
        } => {
            // Multi-tenant: scope task completion to the session's organization
            let task_org_id = get_session_org_id(&state.conn, Some(&session_id));
            let subject_lower = task_subject.to_lowercase();
            let is_shipping = subject_lower.contains("ship")
                || subject_lower.contains("deploy")
                || subject_lower.contains("release")
                || subject_lower.contains("production")
                || subject_lower.contains("merge")
                || subject_lower.contains("push");

            let mut warnings = Vec::new();
            let mut checklists = Vec::new();

            if is_shipping {
                let lessons: Vec<String> = state
                    .conn
                    .prepare(
                        "SELECT title FROM memory
                     WHERE memory_type = 'lesson' AND status = 'active'
                     AND (organization_id = ?1 OR ?1 IS NULL)
                     AND (tags LIKE '%uat%' OR tags LIKE '%production%' OR tags LIKE '%deployment%')
                     ORDER BY quality_score DESC LIMIT 3",
                    )
                    .ok()
                    .map(|mut stmt| {
                        stmt.query_map(rusqlite::params![task_org_id], |row| row.get(0))
                            .ok()
                            .map(|rows| rows.filter_map(|r| r.ok()).collect())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();

                if !lessons.is_empty() {
                    warnings.push(format!(
                        "Shipping task detected. {} relevant lesson(s) found.",
                        lessons.len()
                    ));
                    checklists = lessons;
                }
            }

            Response::Ok {
                data: ResponseData::TaskCompletionCheckResult {
                    warnings,
                    checklists,
                },
            }
        }

        Request::ContextStats { session_id } => {
            if let Some(sid) = session_id {
                match crate::db::effectiveness::session_injection_stats(&state.conn, &sid) {
                    Ok(stats) => Response::Ok {
                        data: ResponseData::ContextStatsResult {
                            total_injections: stats.total_injections,
                            total_chars: stats.total_chars,
                            estimated_tokens: stats.estimated_tokens,
                            acknowledged: stats.acknowledged,
                            effectiveness_rate: stats.effectiveness_rate,
                            per_hook: stats
                                .per_hook
                                .iter()
                                .map(|h| (h.hook_event.clone(), h.injections, h.chars))
                                .collect(),
                        },
                    },
                    Err(e) => Response::Error {
                        message: format!("stats error: {e}"),
                    },
                }
            } else {
                match crate::db::effectiveness::global_injection_stats(&state.conn) {
                    Ok(stats) => Response::Ok {
                        data: ResponseData::ContextStatsResult {
                            total_injections: stats.total_injections,
                            total_chars: stats.total_chars,
                            estimated_tokens: stats.estimated_tokens,
                            acknowledged: stats.acknowledged,
                            effectiveness_rate: stats.effectiveness_rate,
                            per_hook: vec![],
                        },
                    },
                    Err(e) => Response::Error {
                        message: format!("stats error: {e}"),
                    },
                }
            }
        }

        Request::CompileContext {
            agent,
            project,
            static_only,
            excluded_layers,
            session_id,
            focus,
            cwd,
            dry_run,
        } => {
            let agent_name = agent.as_deref().unwrap_or("claude-code");
            let excluded = excluded_layers.unwrap_or_default();
            let dry_run = dry_run.unwrap_or(false);

            // P3-4 Wave Z (Z7): if `project` is set but no project record
            // exists for it AND `cwd` was supplied, auto-create the project
            // before rendering. This means cc-voice's first SessionStart
            // sees `<code-structure project="cc-voice" resolution="auto-created">`
            // instead of `resolution="no-match"` — agents get useful
            // boundaries from turn 1 without the user having to remember
            // an explicit `forge-next project init` step.
            //
            // Skipped under dry_run since dry-run intentionally avoids
            // side effects (the user is auditing what would happen, not
            // performing the action).
            //
            // Z-fw1 (Wave Z review HIGH-1+HIGH-2+HIGH-3 fixes):
            //
            // - HIGH-1 was: `let _ = store_project(...)` swallowed a real
            //   storage error so the rendered XML reverted to no-match
            //   without warning. Now the error path emits a tracing::warn
            //   so dogfood logs surface the failure mode.
            // - HIGH-2 was: two concurrent SessionStarts could race the
            //   existence check and produce duplicate project rows (each
            //   gets its own ULID id; the schema has no UNIQUE on
            //   (name, organization_id) — only id is PK). The fix here is
            //   the upsert via `INSERT OR REPLACE INTO project (id, ...)`
            //   only matches on id, so two new ULIDs still produce two
            //   rows. Mitigation: re-fetch by name AFTER store_project;
            //   if a row already exists with our name+org, keep that row's
            //   id (effectively making the racy second writer a no-op).
            //   This is benign-data-wise (both rows have identical
            //   project_path) but keeps the row count tidy.
            // - HIGH-3 was: organization_id hardcoded to "default". Until
            //   multi-org bind-from-session lands (Z12+), the daemon is
            //   single-org in practice; document the assumption with a
            //   TODO so a future cluster JOIN regression test catches the
            //   leak surface early.
            if let (Some(p), Some(c), false) = (project.as_deref(), cwd.as_deref(), dry_run) {
                // TODO(multi-org Z12+): thread organization_id from session
                // context once the multi-org binding work lands.
                const AUTO_CREATE_ORG: &str = "default";

                // P3-4 Wave X / X1.fw1 (HIGH — dogfood data-loss):
                // the schema carries `CREATE UNIQUE INDEX
                // idx_reality_path_unique ON reality(project_path)
                // WHERE project_path IS NOT NULL`. Pre-fix the auto-
                // create branch only checked existence by NAME (`p`),
                // built a Project struct with a fresh ULID id and the
                // supplied path, and ran `INSERT OR REPLACE`. When a
                // row already existed at that path under a DIFFERENT
                // name (the common case: an agent calls
                // `compile-context --project <session-supplied-name>
                // --cwd <existing-project-path>` with a stale or alias
                // name), the PK check passed (new id) but the unique-
                // index check on project_path fired — SQLite's REPLACE
                // semantics REMOVED the conflicting row before
                // inserting ours. Result: the existing project's id,
                // name, and explicitly-set domain were silently wiped.
                //
                // Fix: gate the auto-create on BOTH name absence AND
                // path absence. If a row exists at this path (under
                // any name in this org), the path is already bound;
                // skip auto-create + emit a tracing::warn so operators
                // see the alias mismatch. The renderer then renders
                // resolution="no-match" against the supplied (alien)
                // name, which is the correct behavior — the existing
                // project's row is preserved untouched.
                let existing_by_name = ops::get_project_by_name(&state.conn, p, AUTO_CREATE_ORG)
                    .ok()
                    .flatten();
                let existing_by_path = ops::get_project_by_path(&state.conn, c, AUTO_CREATE_ORG)
                    .ok()
                    .flatten();
                if let Some(ref bound) = existing_by_path {
                    if existing_by_name.is_none() {
                        tracing::warn!(
                            target: "forge::handler",
                            requested_project = p,
                            cwd = c,
                            bound_project = %bound.name,
                            bound_id = %bound.id,
                            "compile_context auto-create skipped: cwd already bound to a different project; <code-structure> will render resolution=\"no-match\" for the requested name. Use `forge-next project rename` (v0.6.1+) or `update-session` to align."
                        );
                    }
                }
                // X1.fw2 (review LOW-5): symmetric warn for the other
                // half of the alias-detection surface — the requested
                // project name is already bound, but to a DIFFERENT
                // path. Pre-fw2 this case skipped silently with no
                // operator breadcrumb.
                if let Some(ref bound) = existing_by_name {
                    if existing_by_path.is_none() && bound.project_path.as_deref() != Some(c) {
                        tracing::warn!(
                            target: "forge::handler",
                            requested_project = p,
                            requested_cwd = c,
                            bound_path = bound.project_path.as_deref().unwrap_or(""),
                            bound_id = %bound.id,
                            "compile_context auto-create skipped: project already bound to a different cwd; <code-structure> will render resolution=\"no-match\" for the requested cwd. Verify the SessionStart hook payload's cwd field."
                        );
                    }
                }
                if existing_by_name.is_none() && existing_by_path.is_none() {
                    use crate::project::CodeProjectEngine;
                    use forge_core::types::project_engine::ProjectEngine;
                    let detected_domain = CodeProjectEngine
                        .detect(std::path::Path::new(c))
                        .map(|d| d.domain)
                        .unwrap_or_else(|| "unknown".into());
                    let now = forge_core::time::now_iso();
                    let project_record = forge_core::types::Project {
                        id: ulid::Ulid::new().to_string(),
                        name: p.to_string(),
                        reality_type: "code".to_string(),
                        detected_from: Some("compile_context_cwd".to_string()),
                        project_path: Some(c.to_string()),
                        domain: Some(detected_domain),
                        organization_id: AUTO_CREATE_ORG.to_string(),
                        owner_type: "user".to_string(),
                        owner_id: AUTO_CREATE_ORG.to_string(),
                        engine_status: "ok".to_string(),
                        engine_pid: None,
                        created_at: now.clone(),
                        last_active: now,
                        metadata: "{}".to_string(),
                    };
                    // P3-4 Wave X (X1) per cc-voice Round 3 §B: pre-X1 this
                    // line called `ops::store_project(&state.conn, ...)`
                    // directly. Production silently failed because
                    // `Request::CompileContext` is in `is_read_only()`
                    // (see crates/daemon/src/server/writer.rs), so
                    // `state.conn` here is a per-request read-only SQLite
                    // handle (`SQLITE_OPEN_READ_ONLY`). The INSERT errored
                    // with "attempt to write a readonly database",
                    // tracing::warn fired (Z-fw1 HIGH-1), and the renderer
                    // correctly fell to `resolution="no-match"`. Z7's
                    // existing tests passed because they construct
                    // `DaemonState::new(":memory:")` which is write-
                    // capable — the read-only routing layer was never
                    // exercised end-to-end.
                    //
                    // Fix: open a fresh write-capable connection from
                    // `state.db_path` and store via that. SQLite WAL
                    // guarantees the renderer's read-only conn sees the
                    // committed row on its next query (auto-commit txn
                    // re-reads the WAL header). Mirrors the `kpi_reaper`
                    // precedent of opening ad-hoc writer connections
                    // from `db_path` for sub-second writes (single
                    // INSERT here — no need for the writer-actor queue).
                    //
                    // The `:memory:` branch preserves the existing Z7
                    // test fixtures: those tests bypass the routing
                    // layer and call `handle_request` directly with a
                    // write-capable `state.conn`, and `:memory:` cannot
                    // be re-opened from a path (each call creates a new
                    // empty in-memory DB).
                    //
                    // X1.fw2 (review MED-1): use the idempotent
                    // `auto_create_reality_if_absent` helper instead of
                    // `store_project`. The helper uses `INSERT OR IGNORE`
                    // so a concurrent peer that already inserted the
                    // same `project_path` (unique-indexed) is silently
                    // respected — the second writer becomes a no-op
                    // instead of triggering REPLACE semantics that
                    // would wipe the peer's row. Closes the residual
                    // race that fw1's name+path existence check left
                    // open.
                    let store_result = if state.db_path == ":memory:" {
                        ops::auto_create_reality_if_absent(&state.conn, &project_record)
                    } else {
                        match rusqlite::Connection::open(&state.db_path) {
                            Ok(wconn) => {
                                // P3-4 W1.30 (W23 review MED-4): canonical helper.
                                let _ = crate::db::apply_runtime_pragmas(&wconn);
                                ops::auto_create_reality_if_absent(&wconn, &project_record)
                            }
                            Err(e) => Err(e),
                        }
                    };
                    match store_result {
                        Err(e) => {
                            // Z-fw1 HIGH-1: surface the failure path.
                            // The rendered <code-structure> tag will
                            // fall through to resolution="no-match" —
                            // at least operators see why instead of
                            // silently reverting.
                            tracing::warn!(
                                target: "forge::handler",
                                project = p,
                                cwd = c,
                                error = %e,
                                "compile_context auto-create failed; <code-structure> will render no-match"
                            );
                        }
                        Ok(0) => {
                            // X1.fw2 (review MED-1): INSERT OR IGNORE
                            // returned "no rows inserted" — a concurrent
                            // peer beat us to it. The on-disk state is
                            // correct (peer's row is there); we just
                            // skip the "newly created" log line so the
                            // operator audit trail is honest.
                            tracing::debug!(
                                target: "forge::handler",
                                project = p,
                                cwd = c,
                                "compile_context auto-create no-op: concurrent peer already inserted"
                            );
                        }
                        Ok(_) => {
                            tracing::info!(
                                target: "forge::handler",
                                project = p,
                                cwd = c,
                                domain = project_record.domain.as_deref().unwrap_or("unknown"),
                                "compile_context auto-created project row"
                            );
                        }
                    }
                }
            }

            // Verify session ownership: if session_id provided, it must be active and match the agent
            let sid = if let Some(ref sid_str) = session_id {
                let session_ok: bool = state.conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1 AND status IN ('active', 'idle'))",
                    rusqlite::params![sid_str],
                    |row| row.get(0),
                ).unwrap_or(false);
                if session_ok {
                    Some(sid_str.as_str())
                } else {
                    None
                }
            } else {
                None
            };
            // Phase 2A-4d.3.1 #3 H6: load config once for the whole arm so
            // compile_static_prefix + compile_dynamic_suffix share a single
            // ContextInjectionConfig instead of each paying for a disk read.
            //
            // W5 H1 (post-review M1 fix): also resolve scoped overrides for
            // context_injection flags (org / team / user / reality / agent
            // / session) when the request carries a session_id. The
            // resolver takes the already-loaded global as a baseline so
            // we don't re-pay the load cost — the H6 invariant ("one
            // config load per request") is preserved.
            let config = crate::config::load_config();
            let inj = crate::config::resolve_context_injection_for_session(
                &state.conn,
                sid,
                Some(agent_name),
                &config.context_injection,
            );
            let static_prefix = crate::recall::compile_static_prefix_with_inj(
                &state.conn,
                agent_name,
                sid,
                project.as_deref(),
                &inj,
            );

            if static_only.unwrap_or(false) {
                let chars = static_prefix.len();
                // Z5: emit context_compiled event only on real runs
                // (dry-run is an audit / preview).
                if !dry_run {
                    crate::events::emit(
                        &state.events,
                        "context_compiled",
                        serde_json::json!({
                            "static_chars": chars,
                            "dynamic_chars": 0,
                            "total_chars": chars,
                            "static_only": true,
                        }),
                    );
                }
                Response::Ok {
                    data: ResponseData::CompiledContext {
                        context: static_prefix.clone(),
                        static_prefix,
                        dynamic_suffix: String::new(),
                        // Phase 2A-4d.3.1 #3 H3 (W5): count actual present
                        // layers given inj flags rather than hard-coding 4.
                        layers_used: crate::recall::count_layers_used(&inj, true),
                        chars,
                    },
                }
            } else {
                let ctx_config = config.context.validated();
                let (dynamic_suffix, ctx_touched_ids) =
                    crate::recall::compile_dynamic_suffix_with_inj(
                        &state.conn,
                        agent_name,
                        project.as_deref(),
                        &ctx_config,
                        &excluded,
                        sid,
                        focus.as_deref(),
                        None, // TODO: wire organization_id from session context (2A-4a T11)
                        &inj,
                    );
                let full = format!(
                    "<forge-context version=\"0.7.0\">\n{static_prefix}\n{dynamic_suffix}\n</forge-context>"
                );
                let chars = full.len();
                // Record injection for observability — route through writer channel
                // since CompileContext runs on a read-only connection.
                // Z5: skipped on dry-run so audits don't pollute the kpi log.
                if !dry_run {
                    if let Some(sid) = session_id.as_deref() {
                        if let Some(tx) = &state.writer_tx {
                            let _ = tx.try_send(super::writer::WriteCommand::RecordInjection {
                                session_id: sid.to_string(),
                                hook_event: "SessionStart".to_string(),
                                context_type: "full_context".to_string(),
                                content_summary: format!(
                                    "static={} dynamic={}",
                                    static_prefix.len(),
                                    dynamic_suffix.len()
                                ),
                                chars_injected: chars,
                            });
                        }
                    }
                }
                // Emit context_compiled event
                // Phase 2A-4d.3.1 #3 H3 (W5): layers_used reflects the
                // actually-present sections given the inj flags, not a
                // hard-coded 9.
                // Z5: skipped on dry-run.
                let layers_used_full = crate::recall::count_layers_used(&inj, false);
                if !dry_run {
                    crate::events::emit(
                        &state.events,
                        "context_compiled",
                        serde_json::json!({
                            "static_chars": static_prefix.len(),
                            "dynamic_chars": dynamic_suffix.len(),
                            "total_chars": chars,
                            "layers_used": layers_used_full,
                        }),
                    );
                    // Touch the exact decisions+lessons that were included in context
                    // compilation. The IDs are returned by compile_dynamic_suffix —
                    // no approximate query needed. Skipped on dry-run so audits don't
                    // bump access counts on memories that were only previewed.
                    send_touch(&state.writer_tx, ctx_touched_ids, 0.1);
                }

                // Emit prefetch_loaded event if prefetch hints were generated
                // (also skipped on dry-run).
                let prefetch_hints = crate::recall::compile_prefetch_hints(
                    &state.conn,
                    agent_name,
                    project.as_deref(),
                    5,
                );
                if !prefetch_hints.is_empty() && !dry_run {
                    crate::events::emit(
                        &state.events,
                        "prefetch_loaded",
                        serde_json::json!({
                            "hints_count": prefetch_hints.len(),
                            "hints": prefetch_hints,
                        }),
                    );
                }
                Response::Ok {
                    data: ResponseData::CompiledContext {
                        context: full,
                        static_prefix,
                        dynamic_suffix,
                        // Phase 2A-4d.3.1 #3 H3 (W5): same dynamic count
                        // already computed above for the event payload.
                        layers_used: layers_used_full,
                        chars,
                    },
                }
            }
        }

        Request::CompileContextTrace {
            agent,
            project,
            session_id,
        } => {
            let agent_name = agent.as_deref().unwrap_or("claude-code");
            // P3-2 W1 (was W5 review M3): mirror `Request::CompileContext`
            // session-ownership check + scoped `context_injection`
            // resolution. With this in place the trace surface honors
            // per-scope overrides exactly like the live compile path.
            let sid = if let Some(ref sid_str) = session_id {
                let session_ok: bool = state
                    .conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1 AND status IN ('active', 'idle'))",
                        rusqlite::params![sid_str],
                        |row| row.get(0),
                    )
                    .unwrap_or(false);
                if session_ok {
                    Some(sid_str.as_str())
                } else {
                    None
                }
            } else {
                None
            };
            let trace_config = crate::config::load_config();
            let trace_ctx_config = trace_config.context.validated();
            let inj = crate::config::resolve_context_injection_for_session(
                &state.conn,
                sid,
                Some(agent_name),
                &trace_config.context_injection,
            );
            let trace = crate::recall::compile_context_trace(
                &state.conn,
                agent_name,
                project.as_deref(),
                &trace_ctx_config,
                &inj,
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
            match crate::sync::sync_export(&state.conn, project.as_deref(), since.as_deref()) {
                Ok(lines) => {
                    let count = lines.len();
                    let node_id = state.hlc.node_id().to_string();
                    Response::Ok {
                        data: ResponseData::SyncExported {
                            lines,
                            count,
                            node_id,
                        },
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
                    crate::events::emit(
                        &state.events,
                        "sync_completed",
                        serde_json::json!({
                            "imported": result.imported,
                            "conflicts": result.conflicts,
                            "skipped": result.skipped,
                        }),
                    );
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

        Request::SyncConflicts => match crate::sync::list_conflicts(&state.conn) {
            Ok(conflicts) => Response::Ok {
                data: ResponseData::SyncConflictList { conflicts },
            },
            Err(e) => Response::Error {
                message: format!("list_conflicts failed: {e}"),
            },
        },

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
                    let diags = crate::db::diagnostics::get_diagnostics(&state.conn, &f)
                        .unwrap_or_default();
                    let errors = diags.iter().filter(|d| d.severity == "error").count();
                    let warnings = diags.iter().filter(|d| d.severity == "warning").count();
                    let diagnostics: Vec<forge_core::protocol::DiagnosticEntry> = diags
                        .iter()
                        .map(|d| forge_core::protocol::DiagnosticEntry {
                            file_path: d.file_path.clone(),
                            severity: d.severity.clone(),
                            message: d.message.clone(),
                            source: d.source.clone(),
                            line: d.line,
                        })
                        .collect();
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
                    let diags = crate::db::diagnostics::get_all_active_diagnostics(&state.conn)
                        .unwrap_or_default();
                    let errors = diags.iter().filter(|d| d.severity == "error").count();
                    let warnings = diags.iter().filter(|d| d.severity == "warning").count();
                    // Count unique files
                    let files_checked = {
                        let mut files: Vec<&str> =
                            diags.iter().map(|d| d.file_path.as_str()).collect();
                        files.sort();
                        files.dedup();
                        files.len()
                    };
                    let diagnostics: Vec<forge_core::protocol::DiagnosticEntry> = diags
                        .iter()
                        .map(|d| forge_core::protocol::DiagnosticEntry {
                            file_path: d.file_path.clone(),
                            severity: d.severity.clone(),
                            message: d.message.clone(),
                            source: d.source.clone(),
                            line: d.line,
                        })
                        .collect();
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
            let diags =
                crate::db::diagnostics::get_diagnostics(&state.conn, &file).unwrap_or_default();
            let count = diags.len();
            let diagnostics: Vec<forge_core::protocol::DiagnosticEntry> = diags
                .iter()
                .map(|d| forge_core::protocol::DiagnosticEntry {
                    file_path: d.file_path.clone(),
                    severity: d.severity.clone(),
                    message: d.message.clone(),
                    source: d.source.clone(),
                    line: d.line,
                })
                .collect();
            Response::Ok {
                data: ResponseData::DiagnosticList { diagnostics, count },
            }
        }

        Request::HlcBackfill => match crate::sync::backfill_hlc(&state.conn, &state.hlc) {
            Ok(count) => {
                if count > 0 {
                    eprintln!("[daemon] backfilled HLC timestamps on {count} existing memories");
                }
                Response::Ok {
                    data: ResponseData::HlcBackfilled { count },
                }
            }
            Err(e) => Response::Error {
                message: format!("hlc_backfill failed: {e}"),
            },
        },

        Request::BackfillProject => {
            match crate::db::ops::backfill_project_from_sessions(&state.conn) {
                Ok((updated, skipped)) => {
                    if updated > 0 {
                        eprintln!("[daemon] backfilled project on {updated} memories ({skipped} still orphaned)");
                    }
                    Response::Ok {
                        data: ResponseData::BackfillProjectResult { updated, skipped },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("backfill_project failed: {e}"),
                },
            }
        }

        Request::CleanupMemory => {
            let garbage = ops::cleanup_garbage_memories(&state.conn).unwrap_or_else(|e| {
                eprintln!("[cleanup] garbage cleanup failed: {e}");
                0
            });
            let projects = ops::normalize_project_names(&state.conn).unwrap_or_else(|e| {
                eprintln!("[cleanup] project normalization failed: {e}");
                0
            });
            let perceptions = crate::db::manas::purge_duplicate_perceptions(&state.conn)
                .unwrap_or_else(|e| {
                    eprintln!("[cleanup] perception purge failed: {e}");
                    0
                });
            let declared =
                crate::db::manas::cleanup_stale_declared(&state.conn).unwrap_or_else(|e| {
                    eprintln!("[cleanup] declared cleanup failed: {e}");
                    0
                });
            let entities = crate::db::manas::cleanup_duplicate_entities(&state.conn)
                .unwrap_or_else(|e| {
                    eprintln!("[cleanup] entity dedup failed: {e}");
                    0
                });
            let dna =
                crate::db::manas::cleanup_duplicate_domain_dna(&state.conn).unwrap_or_else(|e| {
                    eprintln!("[cleanup] domain DNA dedup failed: {e}");
                    0
                });
            eprintln!("[cleanup] garbage={garbage} projects={projects} perceptions={perceptions} declared={declared} entities={entities} dna={dna}");
            Response::Ok {
                data: ResponseData::CleanupMemoryResult {
                    garbage_deleted: garbage,
                    projects_normalized: projects,
                    perceptions_purged: perceptions,
                    declared_cleaned: declared,
                },
            }
        }

        Request::StoreEvaluation {
            findings,
            project,
            session_id,
        } => {
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
                    format!(
                        "[{}] {}: {}",
                        finding.severity, finding.category, finding.description
                    ),
                )
                .with_confidence(intensity)
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
                    let file_node_id = format!("file:{file}");
                    if let Err(e) =
                        ops::store_edge(&state.conn, &mem_id, &file_node_id, "affects", "{}")
                    {
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
                        if let Err(e) = crate::db::diagnostics::store_diagnostic(&state.conn, &diag)
                        {
                            eprintln!("[eval-feedback] failed to create diagnostic: {e}");
                        } else {
                            diagnostics_created += 1;
                        }
                    }
                }
            }

            if lessons_created > 0 || diagnostics_created > 0 {
                eprintln!("[eval-feedback] stored {lessons_created} lessons, {diagnostics_created} diagnostics from evaluation");
            }

            Response::Ok {
                data: ResponseData::EvaluationStored {
                    lessons_created,
                    diagnostics_created,
                },
            }
        }
        Request::Bootstrap { project } => {
            let adapters = crate::adapters::detect_adapters();
            let result =
                crate::bootstrap::run_bootstrap(&state.conn, &adapters, project.as_deref());
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
            // `state.events` is a broadcast::Sender (Arc under the hood);
            // ForceConsolidate is synchronous and doesn't hold a Mutex, so
            // passing a ref to it is straightforward.
            let stats = crate::workers::consolidator::run_all_phases(
                &state.conn,
                &consol_config,
                state.metrics.as_deref(),
                Some(&state.events),
            );
            tracing::info!(
                exact_dedup = stats.exact_dedup,
                semantic_dedup = stats.semantic_dedup,
                linked = stats.linked,
                faded = stats.faded,
                promoted = stats.promoted,
                reconsolidated = stats.reconsolidated,
                embedding_merged = stats.embedding_merged,
                strengthened = stats.strengthened,
                contradictions = stats.contradictions,
                entities_detected = stats.entities_detected,
                synthesized = stats.synthesized,
                gaps_detected = stats.gaps_detected,
                reweaved = stats.reweaved,
                scored = stats.scored,
                skills_inferred = stats.skills_inferred,
                "force_consolidate complete"
            );
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
                    skills_inferred: stats.skills_inferred,
                },
            }
        }

        Request::ForceExtract => {
            let adapters_list = crate::adapters::detect_adapters();
            let all_files = crate::bootstrap::scan_transcripts(&adapters_list);
            let mut files_queued = 0usize;
            let mut files_enqueued = 0usize;
            let mut enqueue_errors = 0usize;
            for (path, _adapter) in &all_files {
                let hash = match crate::bootstrap::compute_content_hash(path) {
                    Ok(h) => h,
                    Err(_) => continue,
                };
                let (needs_work, _) = crate::bootstrap::needs_processing(&state.conn, path, &hash);
                if !needs_work {
                    continue;
                }
                files_queued += 1;
                if let Some(tx) = crate::extractor_queue::GLOBAL_EXTRACTOR_TX.get() {
                    match tx.try_send(path.clone()) {
                        Ok(()) => files_enqueued += 1,
                        Err(_) => enqueue_errors += 1,
                    }
                }
            }
            eprintln!(
                "[extract] force-extract: {files_queued} files need processing, \
                 {files_enqueued} enqueued, {enqueue_errors} drop(full/closed)"
            );
            Response::Ok {
                data: ResponseData::ExtractionTriggered { files_queued },
            }
        }

        Request::ExtractWithProvider {
            provider,
            model,
            text,
        } => {
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
                &config.extraction.claude_api.api_key,
                "ANTHROPIC_API_KEY",
            )
            .is_some();
            let openai_key_set =
                crate::config::resolve_api_key(&config.extraction.openai.api_key, "OPENAI_API_KEY")
                    .is_some();
            let gemini_key_set =
                crate::config::resolve_api_key(&config.extraction.gemini.api_key, "GEMINI_API_KEY")
                    .is_some();
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
                    eprintln!("[config] updated {key} = {log_value}");
                    Response::Ok {
                        data: ResponseData::ConfigUpdated { key, value },
                    }
                }
                Err(e) => {
                    eprintln!("[config] ERROR: failed to update {key}: {e}");
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
                        data: ResponseData::GraphData {
                            nodes,
                            edges,
                            total_nodes,
                            total_edges,
                        },
                    }
                }
                Err(e) => {
                    eprintln!("[handler] ERROR: graph query failed: {e}");
                    Response::Error {
                        message: format!("graph query failed: {e}"),
                    }
                }
            }
        }

        Request::BatchRecall { queries } => {
            let batch_half_life = crate::config::load_config()
                .recall
                .validated()
                .preference_half_life_days;
            let mut all_results = Vec::new();
            let mut all_touch_ids = Vec::new();
            for q in &queries {
                let lim = q.limit.unwrap_or(5);
                // BatchRecall does not expose include_flipped — always exclude flipped prefs.
                let results = hybrid_recall(
                    &state.conn,
                    &q.text,
                    None,
                    q.memory_type.as_ref(),
                    None,
                    lim,
                    false,
                    batch_half_life,
                );
                for r in &results {
                    all_touch_ids.push(r.memory.id.clone());
                }
                all_results.push(results);
            }
            send_touch(&state.writer_tx, all_touch_ids, 0.3);
            Response::Ok {
                data: ResponseData::BatchRecallResults {
                    results: all_results,
                },
            }
        }

        // ── A2A Inter-Session Protocol (FISP) ──
        Request::SessionSend {
            to,
            kind,
            topic,
            parts,
            project,
            timeout_secs,
            meeting_id,
            from_session,
        } => {
            // A2A permission enforcement
            let config = crate::config::load_config();
            if !config.a2a.enabled {
                return Response::Error {
                    message: "A2A messaging is disabled".into(),
                };
            }

            let from = from_session.as_deref().unwrap_or("api");

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
                        message: format!("A2A permission denied: {from_agent} -> {to_agent}"),
                    };
                }
            }

            // Rate limit: max 50 messages per minute per sender
            let recent_sent: i64 = state.conn.query_row(
                "SELECT COUNT(*) FROM session_message WHERE from_session = ?1 AND created_at > datetime('now', '-60 seconds')",
                rusqlite::params![from],
                |row| row.get(0),
            ).unwrap_or(0);

            if recent_sent >= 50 {
                return Response::Error {
                    message: "rate limit exceeded: max 50 messages per minute".to_string(),
                };
            }

            // Queue depth limit: max 100 pending messages per recipient
            if to != "*" {
                let pending_count: i64 = state.conn.query_row(
                    "SELECT COUNT(*) FROM session_message WHERE to_session = ?1 AND status = 'pending'",
                    rusqlite::params![to],
                    |row| row.get(0),
                ).unwrap_or(0);

                if pending_count >= 100 {
                    return Response::Error {
                        message: "recipient queue full: max 100 pending messages".to_string(),
                    };
                }
            }

            let parts_json = serde_json::to_string(&parts).unwrap_or_else(|_| "[]".to_string());
            match crate::sessions::send_message(
                &state.conn,
                from,
                &to,
                &kind,
                &topic,
                &parts_json,
                project.as_deref(),
                timeout_secs,
                meeting_id.as_deref(),
            ) {
                Ok(id) => {
                    crate::events::emit(
                        &state.events,
                        "session_message",
                        serde_json::json!({
                            "id": &id, "from": from, "to": &to, "kind": &kind, "topic": &topic,
                        }),
                    );
                    // Emit message_received event for subscribe filtering
                    let preview: String = parts_json.chars().take(100).collect();
                    crate::events::emit(
                        &state.events,
                        "message_received",
                        serde_json::json!({
                            "to_session": &to,
                            "from_session": from,
                            "topic": &topic,
                            "preview": preview,
                        }),
                    );
                    // If this is a meeting response, auto-record it
                    if let Some(ref mid) = meeting_id {
                        let confidence = None; // Could be extracted from parts in future
                        if let Ok(all_responded) = crate::teams::record_meeting_response(
                            &state.conn,
                            mid,
                            from,
                            &parts_json,
                            confidence,
                        ) {
                            crate::events::emit(
                                &state.events,
                                "meeting_response",
                                serde_json::json!({
                                    "meeting_id": mid, "session_id": from, "topic": &topic,
                                }),
                            );
                            if all_responded {
                                crate::events::emit(
                                    &state.events,
                                    "meeting_all_responded",
                                    serde_json::json!({
                                        "meeting_id": mid,
                                    }),
                                );
                            }
                        }
                    }
                    Response::Ok {
                        data: ResponseData::MessageSent {
                            id,
                            status: "pending".into(),
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("send_message failed: {e}"),
                },
            }
        }

        Request::SessionRespond {
            message_id,
            status,
            parts,
            from_session,
        } => {
            // P3-4 W1.13 (W23 review HIGH-2): use the caller's session_id
            // as `from_session` when provided; fall back to the legacy
            // "api" sentinel for backward compat with pre-W1.13 callers.
            let from = from_session.as_deref().unwrap_or("api");
            let parts_json = serde_json::to_string(&parts).unwrap_or_else(|_| "[]".to_string());
            match crate::sessions::respond_to_message(
                &state.conn,
                &message_id,
                from,
                &status,
                &parts_json,
            ) {
                Ok(found) => {
                    if !found {
                        eprintln!(
                            "[handler] respond_to_message: original message {message_id} not found"
                        );
                    }
                    crate::events::emit(
                        &state.events,
                        "session_message",
                        serde_json::json!({
                            "message_id": &message_id, "status": &status, "action": "responded",
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::MessageResponded {
                            id: message_id,
                            status,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("respond_to_message failed: {e}"),
                },
            }
        }

        Request::SessionMessages {
            session_id,
            status,
            limit,
            offset,
        } => {
            match crate::sessions::list_messages(
                &state.conn,
                &session_id,
                status.as_deref(),
                limit.unwrap_or(20),
                offset,
            ) {
                Ok(rows) => {
                    let messages: Vec<forge_core::protocol::SessionMessage> = rows
                        .into_iter()
                        .map(|r| {
                            // If parts deserialization fails (e.g. a future
                            // schema change adds a non-Option field without
                            // serde(default)), log the failure loudly rather
                            // than silently returning an empty Vec — empty
                            // parts would silently break Forge-Persist's
                            // verify_matches FISP hash round-trip and
                            // collapse consistency_rate to 0.0 with no test
                            // signal. Caught by adversarial review of cycle
                            // (j1) (CRITICAL 90/100).
                            let parts: Vec<forge_core::protocol::request::MessagePart> =
                                match serde_json::from_str(&r.parts) {
                                    Ok(p) => p,
                                    Err(e) => {
                                        tracing::error!(
                                            message_id = %r.id,
                                            error = %e,
                                            "session_messages: failed to deserialize stored parts JSON; returning empty parts vec — this will break Forge-Persist FISP consistency_rate"
                                        );
                                        Vec::new()
                                    }
                                };
                            forge_core::protocol::SessionMessage {
                                id: r.id,
                                from_session: r.from_session,
                                to_session: r.to_session,
                                kind: r.kind,
                                topic: r.topic,
                                parts,
                                status: r.status,
                                in_reply_to: r.in_reply_to,
                                project: r.project,
                                created_at: r.created_at,
                                delivered_at: r.delivered_at,
                            }
                        })
                        .collect();
                    let count = messages.len();
                    Response::Ok {
                        data: ResponseData::SessionMessageList { messages, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_messages failed: {e}"),
                },
            }
        }

        Request::SessionMessageRead { id, caller_session } => {
            // W27 (F12+F14): single-message lookup by exact ID or unambiguous
            // prefix. Replaces the prior client-side "fetch 100 then filter"
            // pattern in `forge-next message-read` which silently failed for
            // truncated IDs and for messages outside the most-recent batch.
            //
            // P3-4 W1.13 (W28 review HIGH-1): when caller_session is set, the
            // daemon scopes the lookup to messages the caller is a participant
            // in. None preserves the W27 default-open semantics for backward
            // compat (single-tenant Unix-socket deployments without a stable
            // caller-id concept).
            match crate::sessions::read_message_by_id_or_prefix(
                &state.conn,
                &id,
                caller_session.as_deref(),
            ) {
                Ok(Some(r)) => {
                    let parts: Vec<forge_core::protocol::request::MessagePart> =
                        serde_json::from_str(&r.parts).unwrap_or_else(|e| {
                            tracing::error!(
                                message_id = %r.id,
                                error = %e,
                                "session_message_read: failed to deserialize stored parts JSON"
                            );
                            Vec::new()
                        });
                    Response::Ok {
                        data: ResponseData::SessionMessageItem {
                            message: forge_core::protocol::SessionMessage {
                                id: r.id,
                                from_session: r.from_session,
                                to_session: r.to_session,
                                kind: r.kind,
                                topic: r.topic,
                                parts,
                                status: r.status,
                                in_reply_to: r.in_reply_to,
                                project: r.project,
                                created_at: r.created_at,
                                delivered_at: r.delivered_at,
                            },
                        },
                    }
                }
                Ok(None) => Response::Error {
                    message: format!("message not found: {id}"),
                },
                // W1.32 (W28 LOW-3): the helper now returns typed errors —
                // each variant renders without the `session_message_read failed:`
                // implementation prefix that previously muddied the actionable
                // hint.
                Err(crate::sessions::MessageReadError::InvalidChars(s)) => Response::Error {
                    message: format!(
                        "invalid characters in message ID '{s}' — expected Crockford base32 (ULID format)"
                    ),
                },
                Err(crate::sessions::MessageReadError::Ambiguous { prefix, count }) => Response::Error {
                    message: format!(
                        "ambiguous message ID prefix '{prefix}' — type more characters (matches at least {count} rows)"
                    ),
                },
                Err(crate::sessions::MessageReadError::Sql(e)) => Response::Error {
                    message: format!("session_message_read failed: {e}"),
                },
            }
        }

        Request::SessionAck {
            message_ids,
            session_id,
        } => {
            // Try acking as session messages first
            let msg_result = if let Some(sid) = &session_id {
                crate::sessions::ack_messages(&state.conn, &message_ids, sid)
            } else {
                crate::sessions::ack_messages_admin(&state.conn, &message_ids)
            };
            // H2 fix: Don't swallow DB errors — only fall through to notifications on Ok(0).
            // Design: notification fallback ONLY fires when msg_count==0. Mixed batches
            // (some message IDs + some notification IDs) will NOT ack the notifications.
            // This is intentional — callers should use separate ack calls per entity type.
            let msg_count = match msg_result {
                Ok(count) => count,
                Err(e) => {
                    return Response::Error {
                        message: format!("ack_messages failed: {e}"),
                    };
                }
            };

            // Unified ack: if no messages matched, try acking as notifications.
            // This fixes the protocol gap where `ack` on a notification ID silently fails.
            // H1 fix: check Ok(true) not just is_ok() — Ok(false) means ID not found.
            let notif_count = if msg_count == 0 {
                let mut count = 0usize;
                for id in &message_ids {
                    match crate::notifications::ack_notification(&state.conn, id) {
                        Ok(true) => count += 1,
                        Ok(false) => {} // ID not in notification table either
                        Err(e) => eprintln!("[ack] notification ack error for {id}: {e}"),
                    }
                }
                count
            } else {
                0
            };

            let total = msg_count + notif_count;
            Response::Ok {
                data: ResponseData::MessagesAcked { count: total },
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
        Request::GrantPermission {
            from_agent,
            to_agent,
            from_project,
            to_project,
        } => {
            match crate::sessions::grant_a2a_permission(
                &state.conn,
                &from_agent,
                &to_agent,
                from_project.as_deref(),
                to_project.as_deref(),
            ) {
                Ok(id) => Response::Ok {
                    data: ResponseData::PermissionGranted { id },
                },
                Err(e) => Response::Error {
                    message: format!("grant_permission failed: {e}"),
                },
            }
        }

        Request::RevokePermission { id } => {
            match crate::sessions::revoke_a2a_permission(&state.conn, &id) {
                Ok(found) => Response::Ok {
                    data: ResponseData::PermissionRevoked { id, found },
                },
                Err(e) => Response::Error {
                    message: format!("revoke_permission failed: {e}"),
                },
            }
        }

        Request::ListPermissions => match crate::sessions::list_a2a_permissions(&state.conn) {
            Ok(permissions) => {
                let count = permissions.len();
                Response::Ok {
                    data: ResponseData::PermissionList { permissions, count },
                }
            }
            Err(e) => Response::Error {
                message: format!("list_permissions failed: {e}"),
            },
        },

        // ── Scoped Configuration ──
        Request::SetScopedConfig {
            scope_type,
            scope_id,
            key,
            value,
            locked,
            ceiling,
        } => {
            if !ops::validate_scope_type(&scope_type) {
                return Response::Error {
                    message: format!("invalid scope_type '{scope_type}': must be one of session, agent, reality, user, team, organization"),
                };
            }
            match ops::set_scoped_config(
                &state.conn,
                &scope_type,
                &scope_id,
                &key,
                &value,
                locked,
                ceiling,
                "user",
            ) {
                Ok(()) => Response::Ok {
                    data: ResponseData::ScopedConfigSet {
                        scope_type,
                        scope_id,
                        key,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("set_scoped_config failed: {e}"),
                },
            }
        }

        Request::DeleteScopedConfig {
            scope_type,
            scope_id,
            key,
        } => {
            if !ops::validate_scope_type(&scope_type) {
                return Response::Error {
                    message: format!("invalid scope_type '{scope_type}': must be one of session, agent, reality, user, team, organization"),
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

        Request::ListScopedConfig {
            scope_type,
            scope_id,
        } => {
            if !ops::validate_scope_type(&scope_type) {
                return Response::Error {
                    message: format!("invalid scope_type '{scope_type}': must be one of session, agent, reality, user, team, organization"),
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

        Request::GetEffectiveConfig {
            session_id,
            agent,
            reality_id,
            user_id,
            team_id,
            organization_id,
        } => {
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

        Request::ProjectDetect { path } => {
            // P3-4 Wave Z (Z3): renamed from DetectReality. Same behavior:
            // run domain detection, upsert a project record, return the
            // detection metadata. CC voice feedback §1.2/§2.4.
            //
            // P3-4 Wave Y (Y2) per cc-voice Round 2 §B: when the engine
            // can't classify the path (no Cargo.toml / package.json /
            // etc.), fall back to a synthetic detection with
            // `domain="unknown"` and `confidence=0.0` instead of erroring
            // out. This matches what `project init` already accepts
            // (`Domain: unknown`) and lets cc-voice-shaped projects
            // (one .md file, no language markers) bind cleanly. The
            // auto-create code path inside CompileContext already does
            // this; ProjectDetect was the asymmetric outlier.
            use crate::project::CodeProjectEngine;
            use forge_core::types::project_engine::{DetectionResult, ProjectEngine};
            use std::path::Path;

            let engine = CodeProjectEngine;
            let project_path = Path::new(&path);

            let detection = engine
                .detect(project_path)
                .unwrap_or_else(|| DetectionResult {
                    confidence: 0.0,
                    detected_from: "fallback_no_engine_match".to_string(),
                    domain: "unknown".to_string(),
                    reality_type: "code".to_string(),
                    metadata: serde_json::json!({"language": "unknown"}),
                });

            // Check if a project already exists for this path.
            match ops::get_project_by_path(&state.conn, &path, "default") {
                Ok(Some(existing)) => Response::Ok {
                    data: ResponseData::ProjectDetected {
                        id: existing.id,
                        name: existing.name,
                        engine: existing.reality_type.clone(),
                        domain: existing.domain.unwrap_or_default(),
                        detected_from: existing.detected_from.unwrap_or_default(),
                        confidence: detection.confidence,
                        is_new: false,
                        metadata: serde_json::from_str(&existing.metadata)
                            .unwrap_or_else(|_| serde_json::json!({})),
                    },
                },
                Ok(None) => {
                    // Create a new project record (synthetic-detection
                    // fallback gives us domain=unknown for code-less dirs).
                    let project_id = ulid::Ulid::new().to_string();
                    let now = chrono_now();
                    let name = project_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| detection.domain.clone());
                    let metadata_str = serde_json::to_string(&detection.metadata)
                        .unwrap_or_else(|_| "{}".to_string());

                    let project = forge_core::types::Project {
                        id: project_id.clone(),
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

                    match ops::store_project(&state.conn, &project) {
                        Ok(()) => {
                            crate::events::emit(
                                &state.events,
                                "project_detected",
                                serde_json::json!({
                                    "project_id": project_id,
                                    "domain": detection.domain,
                                    "engine": detection.reality_type,
                                }),
                            );
                            Response::Ok {
                                data: ResponseData::ProjectDetected {
                                    id: project_id,
                                    name,
                                    engine: detection.reality_type.clone(),
                                    domain: detection.domain,
                                    detected_from: detection.detected_from,
                                    confidence: detection.confidence,
                                    is_new: true,
                                    metadata: detection.metadata,
                                },
                            }
                        }
                        Err(e) => Response::Error {
                            message: format!("failed to store project: {e}"),
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("failed to check existing project: {e}"),
                },
            }
        }

        // ── Cross-Engine Queries (v2.0 Wave 3) ──
        Request::CrossEngineQuery {
            file,
            reality_id: _reality_id,
        } => {
            // 1. Look up symbols for the file from code_symbol table
            let symbols: Vec<serde_json::Value> = state
                .conn
                .prepare(
                    "SELECT name, kind, line_start, line_end FROM code_symbol WHERE file_path = ?1",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![file], |row| {
                        Ok(serde_json::json!({
                            "name": row.get::<_, String>(0)?,
                            "kind": row.get::<_, String>(1)?,
                            "line_start": row.get::<_, Option<i64>>(2)?,
                            "line_end": row.get::<_, Option<i64>>(3)?,
                        }))
                    })?
                    .collect()
                })
                .unwrap_or_default();

            // 2. Look up callers from edge table (edge_type='calls', to_id contains file path)
            let calling_files: Vec<String> = state
                .conn
                .prepare(
                    "SELECT DISTINCT from_id FROM edge WHERE edge_type = 'calls' AND to_id = ?1",
                )
                .and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![file], |row| row.get(0))?
                        .collect()
                })
                .unwrap_or_default();
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

        Request::FileMemoryMap {
            files,
            reality_id: _,
        } => {
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

                mappings.insert(
                    file.clone(),
                    response::FileMemoryInfo {
                        memory_count,
                        decision_count,
                        entity_names,
                        last_perception,
                    },
                );
            }

            Response::Ok {
                data: ResponseData::FileMemoryMapResult { mappings },
            }
        }

        Request::CodeSearch {
            query,
            kind,
            limit,
            project,
        } => {
            let effective_limit = limit.unwrap_or(20).min(100);
            let pattern = format!("%{query}%");

            // P3-4 W1.2 c2 (I-7): when --project is set, JOIN code_file
            // and filter on code_file.project. Without the JOIN every
            // indexed reality's symbols leak through (live-verified
            // during W1 dogfood — DhruviShah's IPython sysroot returned
            // 50 hits for `find-symbol main` from a forge-only session).
            let hits: Vec<serde_json::Value> = if let Some(ref proj) = project {
                let sql = match kind {
                    Some(_) => {
                        "SELECT s.id, s.name, s.kind, s.file_path, s.line_start
                                FROM code_symbol s
                                JOIN code_file f ON s.file_path = f.path
                                WHERE s.name LIKE ?1 AND s.kind = ?2 AND f.project = ?3 LIMIT ?4"
                    }
                    None => {
                        "SELECT s.id, s.name, s.kind, s.file_path, s.line_start
                             FROM code_symbol s
                             JOIN code_file f ON s.file_path = f.path
                             WHERE s.name LIKE ?1 AND f.project = ?2 LIMIT ?3"
                    }
                };
                state
                    .conn
                    .prepare(sql)
                    .and_then(|mut stmt| {
                        let map_row = |row: &rusqlite::Row<'_>| {
                            // P3-4 W1.24 (W1.3 LOW-5): JSON key is
                            // `file_path` to match what the CLI
                            // consumer reads (`hit.get("file_path")`
                            // in commands/system.rs). Pre-W1.24 this
                            // emitted `path` and the CLI silently
                            // rendered `?` for every hit's location
                            // (the json_macro_silent_drift trap).
                            Ok(serde_json::json!({
                                "id": row.get::<_, String>(0)?,
                                "name": row.get::<_, String>(1)?,
                                "kind": row.get::<_, String>(2)?,
                                "file_path": row.get::<_, String>(3)?,
                                "line_start": row.get::<_, Option<i64>>(4)?,
                            }))
                        };
                        match kind.as_deref() {
                            Some(k) => stmt
                                .query_map(
                                    rusqlite::params![pattern, k, proj, effective_limit],
                                    map_row,
                                )?
                                .collect(),
                            None => stmt
                                .query_map(
                                    rusqlite::params![pattern, proj, effective_limit],
                                    map_row,
                                )?
                                .collect(),
                        }
                    })
                    .unwrap_or_default()
            } else if let Some(ref kind_filter) = kind {
                state.conn.prepare(
                    "SELECT id, name, kind, file_path, line_start FROM code_symbol WHERE name LIKE ?1 AND kind = ?2 LIMIT ?3"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![pattern, kind_filter, effective_limit], |row| {
                        // P3-4 W1.24 — see same-block comment above.
                        Ok(serde_json::json!({
                            "id": row.get::<_, String>(0)?,
                            "name": row.get::<_, String>(1)?,
                            "kind": row.get::<_, String>(2)?,
                            "file_path": row.get::<_, String>(3)?,
                            "line_start": row.get::<_, Option<i64>>(4)?,
                        }))
                    })?.collect()
                }).unwrap_or_default()
            } else {
                state.conn.prepare(
                    "SELECT id, name, kind, file_path, line_start FROM code_symbol WHERE name LIKE ?1 LIMIT ?2"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![pattern, effective_limit], |row| {
                        // P3-4 W1.24 — see same-block comment above.
                        Ok(serde_json::json!({
                            "id": row.get::<_, String>(0)?,
                            "name": row.get::<_, String>(1)?,
                            "kind": row.get::<_, String>(2)?,
                            "file_path": row.get::<_, String>(3)?,
                            "line_start": row.get::<_, Option<i64>>(4)?,
                        }))
                    })?.collect()
                }).unwrap_or_default()
            };

            Response::Ok {
                data: ResponseData::CodeSearchResult { hits },
            }
        }

        Request::ProjectList { organization_id } => {
            let org_id = organization_id.as_deref().unwrap_or("default");
            match ops::list_projects(&state.conn, org_id) {
                Ok(projects) => Response::Ok {
                    data: ResponseData::ProjectList { projects },
                },
                Err(e) => Response::Error {
                    message: format!("project_list failed: {e}"),
                },
            }
        }

        Request::ProjectInit {
            name,
            path,
            domain,
            organization_id,
        } => {
            // P3-4 Wave Z (Z3) — explicit project creation. CC voice
            // feedback §2.4: lets users create a project record before
            // any code exists, so `compile-context --project <name>`
            // can bind cleanly from turn 1 of the agent's session
            // (instead of triggering the auto-create path on first
            // SessionStart, which still leaves a brief window of
            // resolution=no-match output).
            //
            // Resolves the path: explicit `--path` argument wins;
            // otherwise tries the canonicalized current working
            // directory of the daemon (best-effort fallback for
            // sessions whose CWD wasn't passed through the wire).
            let org_id = organization_id.as_deref().unwrap_or("default");
            let project_path = match path {
                Some(p) => match std::fs::canonicalize(&p) {
                    Ok(canonical) => canonical.to_string_lossy().to_string(),
                    Err(e) => {
                        return Response::Error {
                            message: format!("cannot resolve path '{p}': {e}"),
                        };
                    }
                },
                None => match std::env::current_dir() {
                    Ok(cwd) => cwd.to_string_lossy().to_string(),
                    Err(e) => {
                        return Response::Error {
                            message: format!("no --path supplied and daemon CWD unreadable: {e}"),
                        };
                    }
                },
            };

            // Auto-detect domain from path contents when not supplied —
            // reuse the CodeProjectEngine.detect() machinery already
            // wired for ProjectDetect. Falls back to "unknown" when the
            // path has no recognizable signature (e.g. an empty dir).
            let detected_domain = domain.unwrap_or_else(|| {
                use crate::project::CodeProjectEngine;
                use forge_core::types::project_engine::ProjectEngine;
                CodeProjectEngine
                    .detect(std::path::Path::new(&project_path))
                    .map(|d| d.domain)
                    .unwrap_or_else(|| "unknown".into())
            });

            // Check for existing project with same (name, organization_id).
            // ProjectInit is idempotent — returns is_new=false on rerun.
            //
            // P3-4 Wave Y (Y5) per cc-voice Round 2 §E: pre-Y5 we
            // overwrote the existing row with whatever the new args
            // said, even when the response status line said "already
            // existed". Saying one thing and doing another is the
            // "data-integrity" issue cc-voice flagged: a defensive
            // `project init <name> --domain code` after a previous
            // `project init <name>` (no domain) silently mutated the
            // row from `unknown` → `code`. Now we don't touch the
            // existing row at all on rerun — log a warn if the user
            // tried to change something so the divergence isn't
            // silent. Use `project update` (when it lands, MED-3) for
            // explicit mutation.
            let existing = ops::get_project_by_name(&state.conn, &name, org_id)
                .ok()
                .flatten();

            if let Some(r) = existing {
                let existing_domain = r.domain.clone().unwrap_or_default();
                let existing_path = r.project_path.clone().unwrap_or_default();
                if existing_domain != detected_domain {
                    tracing::warn!(
                        target: "forge::handler",
                        project = %name,
                        existing_domain = %existing_domain,
                        requested_domain = %detected_domain,
                        "project_init: refused to overwrite existing project; use `project update --domain X` (MED-3, not yet implemented) for explicit mutation"
                    );
                }
                if existing_path != project_path {
                    tracing::warn!(
                        target: "forge::handler",
                        project = %name,
                        existing_path = %existing_path,
                        requested_path = %project_path,
                        "project_init: refused to overwrite existing project path; use `project update --path X` for relocate"
                    );
                }
                return Response::Ok {
                    data: ResponseData::ProjectInitialized {
                        id: r.id,
                        name,
                        path: existing_path,
                        domain: existing_domain,
                        is_new: false,
                    },
                };
            }

            let id = ulid::Ulid::new().to_string();
            let now = forge_core::time::now_iso();
            let project = forge_core::types::Project {
                id: id.clone(),
                name: name.clone(),
                reality_type: "code".to_string(),
                detected_from: Some("project_init".to_string()),
                project_path: Some(project_path.clone()),
                domain: Some(detected_domain.clone()),
                organization_id: org_id.to_string(),
                owner_type: "user".to_string(),
                owner_id: org_id.to_string(),
                engine_status: "ok".to_string(),
                engine_pid: None,
                created_at: now.clone(),
                last_active: now,
                metadata: "{}".to_string(),
            };

            match ops::store_project(&state.conn, &project) {
                Ok(()) => Response::Ok {
                    data: ResponseData::ProjectInitialized {
                        id,
                        name,
                        path: project_path,
                        domain: detected_domain,
                        is_new: true,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("project_init failed: {e}"),
                },
            }
        }

        Request::ProjectShow {
            name,
            organization_id,
        } => {
            let org_id = organization_id.as_deref().unwrap_or("default");
            let project = match ops::get_project_by_name(&state.conn, &name, org_id) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    return Response::Error {
                        message: format!("project '{name}' not found"),
                    };
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("project_show failed: {e}"),
                    };
                }
            };

            let files_indexed: usize = state
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM code_file WHERE project = ?1",
                    rusqlite::params![project.name],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let symbols_indexed: usize = state
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM code_symbol s
                     JOIN code_file f ON s.file_path = f.path
                     WHERE f.project = ?1",
                    rusqlite::params![project.name],
                    |r| r.get(0),
                )
                .unwrap_or(0);

            Response::Ok {
                data: ResponseData::ProjectInfo {
                    id: project.id,
                    name: project.name,
                    path: project.project_path.unwrap_or_default(),
                    domain: project.domain.unwrap_or_else(|| "unknown".into()),
                    engine: project.reality_type,
                    last_active: project.last_active,
                    files_indexed,
                    symbols_indexed,
                },
            }
        }

        Request::ForceIndex { path } => {
            if let Some(ref dir) = path {
                // Index a specific directory (ISSUE-17: multi-project support)
                // Security: ForceIndex is admin-only (RBAC gated in rbac.rs).
                // No workspace boundary check — intentional for single-user daemon.
                // For multi-tenant: add workspace boundary enforcement.
                let canonical = match std::fs::canonicalize(dir) {
                    Ok(p) => p.to_string_lossy().to_string(),
                    Err(e) => {
                        return Response::Error {
                            message: format!("cannot resolve path '{dir}': {e}"),
                        };
                    }
                };
                if !std::path::Path::new(&canonical).is_dir() {
                    return Response::Error {
                        message: format!("'{dir}' is not a directory"),
                    };
                }

                let (files_indexed, symbols_indexed) =
                    crate::workers::indexer::index_directory_sync(&state.conn, &canonical);

                eprintln!("[force-index] indexed {files_indexed} files, {symbols_indexed} symbols from {canonical}");

                Response::Ok {
                    data: ResponseData::IndexComplete {
                        files_indexed,
                        symbols_indexed,
                        // Synchronous force-index handler — counts are real,
                        // not background-dispatch placeholders.
                        dispatched: false,
                    },
                }
            } else {
                // Re-process already-indexed files: extract import edges + run clustering
                // (LSP-based symbol extraction continues on the background interval)
                let files = ops::list_code_files(&state.conn);
                let import_edges =
                    crate::workers::indexer::extract_and_store_imports(&state.conn, &files);

                // Run clustering for any project that has a reality
                let projects: std::collections::HashSet<String> =
                    files.iter().map(|f| f.project.clone()).collect();
                for project_dir in &projects {
                    crate::workers::indexer::run_clustering(&state.conn, project_dir);
                }

                let files_indexed = files.len();
                let symbols_indexed: usize = state
                    .conn
                    .query_row("SELECT COUNT(*) FROM code_symbol", [], |r| r.get(0))
                    .unwrap_or(0);

                eprintln!("[force-index] processed {files_indexed} files, {import_edges} import edges, {symbols_indexed} symbols");

                Response::Ok {
                    data: ResponseData::IndexComplete {
                        files_indexed,
                        symbols_indexed,
                        // Synchronous force-index handler — counts are real,
                        // not background-dispatch placeholders.
                        dispatched: false,
                    },
                }
            }
        }

        // ── Contradictions ──
        Request::ListContradictions { status, limit } => {
            let lim = limit.unwrap_or(50);
            // Query contradiction edges joined with memory titles
            let sql = "SELECT e.id, e.from_id, e.to_id, e.properties, e.created_at,
                               m1.title, m1.valence, m2.title, m2.valence
                        FROM edge e
                        LEFT JOIN memory m1 ON e.from_id = m1.id
                        LEFT JOIN memory m2 ON e.to_id = m2.id
                        WHERE e.edge_type = 'contradicts'
                        ORDER BY e.created_at DESC
                        LIMIT ?1";
            let mut stmt = match state.conn.prepare(sql) {
                Ok(s) => s,
                Err(e) => {
                    return Response::Error {
                        message: format!("list_contradictions: {e}"),
                    }
                }
            };
            let rows: Vec<forge_core::protocol::response::ContradictionInfo> = stmt
                .query_map(rusqlite::params![lim], |row| {
                    let id: String = row.get(0)?;
                    let from_id: String = row.get(1)?;
                    let to_id: String = row.get(2)?;
                    let props: String = row.get(3)?;
                    let created_at: String = row.get(4)?;
                    let title_a: String = row.get::<_, Option<String>>(5)?.unwrap_or_default();
                    let valence_a: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
                    let title_b: String = row.get::<_, Option<String>>(7)?.unwrap_or_default();
                    let valence_b: String = row.get::<_, Option<String>>(8)?.unwrap_or_default();
                    let shared_tags: usize = serde_json::from_str::<serde_json::Value>(&props)
                        .ok()
                        .and_then(|v| v["shared_tags"].as_u64())
                        .unwrap_or(0) as usize;
                    // Check if resolved (supersede edge exists from either memory)
                    let resolved = false; // will be enriched below
                    Ok(forge_core::protocol::response::ContradictionInfo {
                        id,
                        memory_a_id: from_id,
                        memory_a_title: title_a,
                        memory_a_valence: valence_a,
                        memory_b_id: to_id,
                        memory_b_title: title_b,
                        memory_b_valence: valence_b,
                        shared_tags,
                        resolved,
                        created_at,
                    })
                })
                .ok()
                .map(|r| r.flatten().collect())
                .unwrap_or_default();

            // Enrich with resolution status: check if either memory has been superseded
            let mut enriched: Vec<forge_core::protocol::response::ContradictionInfo> = rows.into_iter().map(|mut c| {
                let has_supersede: bool = state.conn.query_row(
                    "SELECT COUNT(*) > 0 FROM edge WHERE edge_type = 'supersedes' AND (from_id = ?1 OR from_id = ?2)",
                    rusqlite::params![c.memory_a_id, c.memory_b_id],
                    |r| r.get(0),
                ).unwrap_or(false);
                c.resolved = has_supersede;
                c
            }).collect();

            // Filter by status if requested
            if let Some(ref s) = status {
                match s.as_str() {
                    "unresolved" => enriched.retain(|c| !c.resolved),
                    "resolved" => enriched.retain(|c| c.resolved),
                    _ => {}
                }
            }

            let count = enriched.len();
            Response::Ok {
                data: ResponseData::Contradictions {
                    contradictions: enriched,
                    count,
                },
            }
        }

        Request::ResolveContradiction {
            contradiction_id,
            resolution,
        } => {
            // Find the contradiction edge
            let edge = state.conn.query_row(
                "SELECT from_id, to_id FROM edge WHERE id = ?1 AND edge_type = 'contradicts'",
                rusqlite::params![contradiction_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            );
            let (from_id, to_id) = match edge {
                Ok(pair) => pair,
                Err(_) => {
                    return Response::Error {
                        message: format!("contradiction '{contradiction_id}' not found"),
                    }
                }
            };

            // Apply resolution
            match resolution.as_str() {
                "a" => {
                    // Memory A wins — supersede B
                    let _ = state.conn.execute(
                        "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                        rusqlite::params![to_id],
                    );
                    let _ = ops::store_edge(&state.conn, &from_id, &to_id, "supersedes", "{}");
                }
                "b" => {
                    // Memory B wins — supersede A
                    let _ = state.conn.execute(
                        "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                        rusqlite::params![from_id],
                    );
                    let _ = ops::store_edge(&state.conn, &to_id, &from_id, "supersedes", "{}");
                }
                _ => {
                    return Response::Error {
                        message: format!("invalid resolution '{resolution}': expected 'a' or 'b'"),
                    };
                }
            }

            // Remove the contradiction diagnostic
            let diag_id = contradiction_id.replace("edge-contradiction-", "contradiction-");
            let _ = state.conn.execute(
                "DELETE FROM diagnostic WHERE id = ?1",
                rusqlite::params![diag_id],
            );

            Response::Ok {
                data: ResponseData::ContradictionResolved {
                    contradiction_id,
                    resolution,
                },
            }
        }

        // ── Agent Teams: Template CRUD ──
        Request::CreateAgentTemplate {
            name,
            description,
            agent_type,
            organization_id,
            system_context,
            identity_facets,
            config_overrides,
            knowledge_domains,
            decision_style,
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
                    crate::events::emit(
                        &state.events,
                        "agent_template_created",
                        serde_json::json!({
                            "id": id, "name": name,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::AgentTemplateCreated { id, name },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("create_agent_template failed: {e}"),
                },
            }
        }

        Request::ListAgentTemplates {
            organization_id,
            limit,
        } => {
            let lim = limit.unwrap_or(50).min(200);
            match crate::teams::list_agent_templates(&state.conn, organization_id.as_deref(), lim) {
                Ok(templates) => {
                    let count = templates.len();
                    Response::Ok {
                        data: ResponseData::AgentTemplateList { templates, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_agent_templates failed: {e}"),
                },
            }
        }

        Request::GetAgentTemplate { id, name } => {
            let result = if let Some(id) = id {
                crate::teams::get_agent_template(&state.conn, &id)
            } else if let Some(name) = name {
                crate::teams::get_agent_template_by_name(&state.conn, &name, "default")
            } else {
                return Response::Error {
                    message: "either id or name required".into(),
                };
            };
            match result {
                Ok(Some(template)) => Response::Ok {
                    data: ResponseData::AgentTemplateData { template },
                },
                Ok(None) => Response::Error {
                    message: "agent template not found".into(),
                },
                Err(e) => Response::Error {
                    message: format!("get_agent_template failed: {e}"),
                },
            }
        }

        Request::DeleteAgentTemplate { id } => {
            match crate::teams::delete_agent_template(&state.conn, &id) {
                Ok(found) => Response::Ok {
                    data: ResponseData::AgentTemplateDeleted { id, found },
                },
                Err(e) => Response::Error {
                    message: format!("delete_agent_template failed: {e}"),
                },
            }
        }

        Request::UpdateAgentTemplate {
            id,
            name,
            description,
            system_context,
            identity_facets,
            config_overrides,
            knowledge_domains,
            decision_style,
        } => {
            let update = crate::teams::TemplateUpdate {
                name: name.as_deref(),
                description: description.as_deref(),
                system_context: system_context.as_deref(),
                identity_facets: identity_facets.as_deref(),
                config_overrides: config_overrides.as_deref(),
                knowledge_domains: knowledge_domains.as_deref(),
                decision_style: decision_style.as_deref(),
            };
            match crate::teams::update_agent_template(&state.conn, &id, &update) {
                Ok(updated) => Response::Ok {
                    data: ResponseData::AgentTemplateUpdated { id, updated },
                },
                Err(e) => Response::Error {
                    message: format!("update_agent_template failed: {e}"),
                },
            }
        }

        // ── Agent Lifecycle ──
        Request::SpawnAgent {
            template_name,
            session_id,
            project,
            team,
        } => {
            match crate::teams::spawn_agent(
                &state.conn,
                &template_name,
                &session_id,
                project.as_deref(),
                team.as_deref(),
            ) {
                Ok(()) => {
                    crate::events::emit(
                        &state.events,
                        "agent_spawned",
                        serde_json::json!({
                            "session_id": session_id, "template_name": template_name, "team": team,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::AgentSpawned {
                            session_id,
                            template_name,
                            team,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("spawn_agent failed: {e}"),
                },
            }
        }

        Request::ListAgents { team, limit } => {
            let lim = limit.unwrap_or(50).min(200);
            match crate::teams::list_agents(&state.conn, team.as_deref(), lim) {
                Ok(agents) => {
                    let count = agents.len();
                    Response::Ok {
                        data: ResponseData::AgentList { agents, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_agents failed: {e}"),
                },
            }
        }

        Request::UpdateAgentStatus {
            session_id,
            status,
            current_task,
        } => {
            // Validate status against allowed values
            const VALID_AGENT_STATUSES: &[&str] = &["active", "idle", "busy", "error", "retired"];
            if !VALID_AGENT_STATUSES.contains(&status.as_str()) {
                return Response::Error {
                    message: format!(
                        "invalid agent status '{status}': must be one of {VALID_AGENT_STATUSES:?}"
                    ),
                };
            }

            // Get old status for event
            let old_status: String = state
                .conn
                .query_row(
                    "SELECT COALESCE(agent_status, 'unknown') FROM session WHERE id = ?1",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "unknown".into());

            match crate::teams::update_agent_status(
                &state.conn,
                &session_id,
                &status,
                current_task.as_deref(),
            ) {
                Ok(_updated) => {
                    let now = forge_core::time::now_iso();
                    let mut event_data = serde_json::json!({
                        "session_id": session_id, "old_status": old_status, "new_status": status,
                        "current_task": current_task, "timestamp": now,
                    });
                    // Add completed_at when agent transitions to a terminal/idle state
                    if (status == "retired" || status == "idle")
                        && (old_status == "busy" || old_status == "active")
                    {
                        event_data["completed_at"] = serde_json::Value::String(now.clone());
                    }
                    crate::events::emit(&state.events, "agent_status_changed", event_data);
                    Response::Ok {
                        data: ResponseData::AgentStatusUpdated { session_id, status },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("update_agent_status failed: {e}"),
                },
            }
        }

        Request::RetireAgent { session_id } => {
            // Get template name for event
            let template_name: String = state
                .conn
                .query_row(
                    "SELECT COALESCE(at.name, '') FROM session s
                 LEFT JOIN agent_template at ON at.id = s.template_id
                 WHERE s.id = ?1",
                    rusqlite::params![session_id],
                    |row| row.get(0),
                )
                .unwrap_or_default();

            match crate::teams::retire_agent(&state.conn, &session_id) {
                Ok(_retired) => {
                    crate::events::emit(
                        &state.events,
                        "agent_retired",
                        serde_json::json!({
                            "session_id": session_id, "template_name": template_name,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::AgentRetired { session_id },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("retire_agent failed: {e}"),
                },
            }
        }

        // ── Team Enhancements ──
        Request::CreateTeam {
            name,
            team_type,
            purpose,
            organization_id,
            parent_team_id,
        } => {
            match crate::teams::create_team(
                &state.conn,
                &name,
                team_type.as_deref(),
                purpose.as_deref(),
                organization_id.as_deref(),
                parent_team_id.as_deref(),
            ) {
                Ok(id) => Response::Ok {
                    data: ResponseData::TeamCreated { id, name },
                },
                Err(e) => Response::Error {
                    message: format!("create_team failed: {e}"),
                },
            }
        }

        Request::ListTeamMembers { team_name } => {
            match crate::teams::list_team_members(&state.conn, &team_name) {
                Ok(members) => {
                    let count = members.len();
                    Response::Ok {
                        data: ResponseData::TeamMemberList { members, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_team_members failed: {e}"),
                },
            }
        }

        Request::SetTeamOrchestrator {
            team_name,
            session_id,
        } => match crate::teams::set_team_orchestrator(&state.conn, &team_name, &session_id) {
            Ok(_set) => Response::Ok {
                data: ResponseData::TeamOrchestratorSet {
                    team_name,
                    session_id,
                },
            },
            Err(e) => Response::Error {
                message: format!("set_team_orchestrator failed: {e}"),
            },
        },

        Request::TeamStatus { team_name, team_id } => {
            let resolved_name = if let Some(ref tid) = team_id {
                state
                    .conn
                    .query_row(
                        "SELECT name FROM team WHERE id = ?1",
                        rusqlite::params![tid],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap_or(team_name)
            } else {
                team_name
            };
            match crate::teams::team_status(&state.conn, &resolved_name) {
                Ok(team) => Response::Ok {
                    data: ResponseData::TeamStatusData { team },
                },
                Err(e) => Response::Error {
                    message: format!("team_status failed: {e}"),
                },
            }
        }

        // ── Team Orchestration ──
        Request::RunTeam {
            team_name,
            template_names,
            topology,
            goal,
            project,
        } => {
            // W1.32 (W28 review LOW-7): warn on unknown `--project`. A typo
            // silently scopes the team's work to a non-existent project and
            // health-by-project never surfaces it. We log instead of
            // rejecting because (a) the underlying register_session is
            // permissive by design (sessions can write to projects that
            // auto-create downstream) and (b) the strict-reject path is
            // tracked separately as the optional `memory.require_project`
            // config (W29/W30 backlog). The warn is the cheapest signal
            // to operators that something is off.
            if let Some(p) = project.as_deref().filter(|s| !s.is_empty()) {
                match crate::db::ops::get_project_by_name(&state.conn, p, "default") {
                    Ok(Some(_)) => {}
                    Ok(None) => tracing::warn!(
                        project = %p,
                        team = %team_name,
                        "run_team: --project is not in the known projects table — typo? (use `forge-next health` to list known projects)"
                    ),
                    Err(e) => tracing::warn!(
                        project = %p,
                        team = %team_name,
                        error = %e,
                        "run_team: project-existence check failed; proceeding"
                    ),
                }
            }
            match crate::teams::run_team(
                &state.conn,
                &team_name,
                &template_names,
                topology.as_deref(),
                goal.as_deref(),
                project.as_deref(),
            ) {
                Ok((name, agents_spawned, session_ids)) => {
                    // Emit individual agent_spawned events for each agent
                    for (i, sid) in session_ids.iter().enumerate() {
                        let tpl = template_names
                            .get(i)
                            .map(|s| s.as_str())
                            .unwrap_or("unknown");
                        crate::events::emit(
                            &state.events,
                            "agent_spawned",
                            serde_json::json!({
                                "session_id": sid, "template_name": tpl, "team": team_name,
                            }),
                        );
                    }
                    // Emit team_started event
                    crate::events::emit(
                        &state.events,
                        "team_started",
                        serde_json::json!({
                            "team_name": name,
                            "members": session_ids,
                            "template_names": template_names,
                            "topology": topology.as_deref().unwrap_or("mesh"),
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::RunTeamResult {
                            team_name: name,
                            agents_spawned,
                            session_ids,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("run_team failed: {e}"),
                },
            }
        }

        Request::StopTeam { team_name } => match crate::teams::stop_team(&state.conn, &team_name) {
            // W1.32 (W28 review LOW-4): the helper now returns the
            // `(retired, errors)` tuple so the CLI can surface
            // "all retires failed" distinctly from "team had no agents".
            Ok((agents_retired, retire_errors)) => {
                crate::events::emit(
                    &state.events,
                    "team_stopped",
                    serde_json::json!({
                        "team_name": team_name,
                        "agents_retired": agents_retired,
                        "retire_errors": retire_errors,
                    }),
                );
                Response::Ok {
                    data: ResponseData::TeamStopped {
                        team_name,
                        agents_retired,
                        retire_errors,
                    },
                }
            }
            Err(e) => Response::Error {
                message: format!("stop_team failed: {e}"),
            },
        },

        Request::ListTeamTemplates => match crate::teams::list_team_templates(&state.conn) {
            Ok(templates) => {
                let count = templates.len();
                Response::Ok {
                    data: ResponseData::TeamTemplateList { templates, count },
                }
            }
            Err(e) => Response::Error {
                message: format!("list_team_templates failed: {e}"),
            },
        },

        // ── Meeting Protocol ──
        Request::CreateMeeting {
            team_id,
            topic,
            context,
            orchestrator_session_id,
            participant_session_ids,
            goal,
        } => {
            match crate::teams::create_meeting(
                &state.conn,
                &team_id,
                &topic,
                context.as_deref(),
                &orchestrator_session_id,
                &participant_session_ids,
                goal.as_deref(),
            ) {
                Ok((meeting_id, participant_count)) => {
                    // Gap 9: meeting_started event with full details for app sidebar
                    crate::events::emit(
                        &state.events,
                        "meeting_started",
                        serde_json::json!({
                            "meeting_id": meeting_id,
                            "team_id": team_id,
                            "topic": topic,
                            "orchestrator": orchestrator_session_id,
                            "participants": participant_session_ids,
                            "participant_count": participant_count,
                            "status": "collecting",
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::MeetingCreated {
                            meeting_id,
                            participant_count,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("create_meeting failed: {e}"),
                },
            }
        }

        Request::MeetingStatus { meeting_id } => {
            match crate::teams::get_meeting_status(&state.conn, &meeting_id) {
                Ok((meeting, participants)) => Response::Ok {
                    data: ResponseData::MeetingStatusData {
                        meeting,
                        participants,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("meeting_status failed: {e}"),
                },
            }
        }

        Request::MeetingResponses { meeting_id } => {
            match crate::teams::get_meeting_responses(&state.conn, &meeting_id) {
                Ok(responses) => {
                    let count = responses.len();
                    Response::Ok {
                        data: ResponseData::MeetingResponseList { responses, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("meeting_responses failed: {e}"),
                },
            }
        }

        Request::MeetingSynthesize {
            meeting_id,
            synthesis,
        } => match crate::teams::synthesize_meeting(&state.conn, &meeting_id, &synthesis) {
            Ok(_updated) => Response::Ok {
                data: ResponseData::MeetingSynthesized { meeting_id },
            },
            Err(e) => Response::Error {
                message: format!("meeting_synthesize failed: {e}"),
            },
        },

        Request::MeetingDecide {
            meeting_id,
            decision,
        } => {
            match crate::teams::decide_meeting(&state.conn, &meeting_id, &decision) {
                Ok((_, decision_memory_id)) => {
                    // Gap 9: meeting_completed event with topic + decisions for app sidebar
                    let topic: String = state
                        .conn
                        .query_row(
                            "SELECT topic FROM meeting WHERE id = ?1",
                            rusqlite::params![meeting_id],
                            |row| row.get(0),
                        )
                        .unwrap_or_else(|_| "unknown".to_string());
                    crate::events::emit(
                        &state.events,
                        "meeting_completed",
                        serde_json::json!({
                            "meeting_id": meeting_id,
                            "topic": topic,
                            "decision": decision,
                            "decision_memory_id": decision_memory_id,
                            "status": "decided",
                        }),
                    );

                    // Workspace auto-write: persist meeting minutes to team workspace
                    {
                        let ws_config = crate::config::load_config();
                        if ws_config.workspace.auto_write.meetings
                            && ws_config.workspace.mode != "project"
                        {
                            // Fetch team_id from the meeting
                            let team_id_str: String = state
                                .conn
                                .query_row(
                                    "SELECT team_id FROM meeting WHERE id = ?1",
                                    rusqlite::params![meeting_id],
                                    |row| row.get(0),
                                )
                                .unwrap_or_else(|_| "default".to_string());

                            let org = &ws_config.workspace.org;
                            let team_name = if team_id_str.is_empty() {
                                if org.is_empty() {
                                    "default"
                                } else {
                                    org.as_str()
                                }
                            } else {
                                &team_id_str
                            };

                            // Fetch participants and their contributions
                            let participants: Vec<String> = state.conn.prepare(
                                "SELECT COALESCE(template_name, session_id) FROM meeting_participant WHERE meeting_id = ?1"
                            ).and_then(|mut stmt| {
                                stmt.query_map(rusqlite::params![meeting_id], |row| row.get(0))?.collect()
                            }).unwrap_or_default();

                            let contributions: Vec<(String, String)> = state.conn.prepare(
                                "SELECT COALESCE(template_name, session_id), COALESCE(response, '') FROM meeting_participant WHERE meeting_id = ?1"
                            ).and_then(|mut stmt| {
                                stmt.query_map(rusqlite::params![meeting_id], |row| {
                                    Ok((row.get(0)?, row.get(1)?))
                                })?.collect()
                            }).unwrap_or_default();

                            if let Some(ws_root) = crate::workspace::team_workspace_path(
                                &ws_config.workspace,
                                team_name,
                                org,
                                None, // meetings are org-level, no project_dir needed for centralized
                            ) {
                                match crate::workspace::write_meeting_minutes(
                                    &ws_root,
                                    team_name,
                                    &topic,
                                    &participants,
                                    &contributions,
                                    &decision,
                                    &meeting_id,
                                ) {
                                    Ok(path) => {
                                        crate::events::emit(
                                            &state.events,
                                            "workspace_meeting_written",
                                            serde_json::json!({
                                                "meeting_id": meeting_id,
                                                "path": path.display().to_string(),
                                            }),
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[workspace] auto-write meeting minutes failed: {e}"
                                        );
                                    }
                                }
                            }
                        }
                    }

                    Response::Ok {
                        data: ResponseData::MeetingDecided {
                            meeting_id,
                            decision_memory_id,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("meeting_decide failed: {e}"),
                },
            }
        }

        Request::ListMeetings {
            team_id,
            status,
            limit,
        } => {
            let lim = limit.unwrap_or(50).min(200);
            match crate::teams::list_meetings(
                &state.conn,
                team_id.as_deref(),
                status.as_deref(),
                lim,
            ) {
                Ok(meetings) => {
                    let count = meetings.len();
                    Response::Ok {
                        data: ResponseData::MeetingList { meetings, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_meetings failed: {e}"),
                },
            }
        }

        Request::MeetingTranscript { meeting_id } => {
            match crate::teams::get_meeting_transcript(&state.conn, &meeting_id) {
                Ok(transcript) => Response::Ok {
                    data: ResponseData::MeetingTranscriptData { transcript },
                },
                Err(e) => Response::Error {
                    message: format!("meeting_transcript failed: {e}"),
                },
            }
        }

        Request::RecordMeetingResponse {
            meeting_id,
            session_id,
            response,
            confidence,
        } => {
            match crate::teams::record_meeting_response(
                &state.conn,
                &meeting_id,
                &session_id,
                &response,
                confidence,
            ) {
                Ok(all_responded) => {
                    crate::events::emit(
                        &state.events,
                        "meeting_response",
                        serde_json::json!({
                            "meeting_id": &meeting_id, "session_id": &session_id,
                        }),
                    );
                    if all_responded {
                        crate::events::emit(
                            &state.events,
                            "meeting_all_responded",
                            serde_json::json!({
                                "meeting_id": &meeting_id,
                            }),
                        );
                    }
                    Response::Ok {
                        data: ResponseData::MeetingResponseRecorded {
                            meeting_id,
                            all_responded,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("record_meeting_response failed: {e}"),
                },
            }
        }

        // ── FISP Consensus / Voting ──
        Request::MeetingVote {
            meeting_id,
            session_id,
            choice,
        } => {
            match crate::teams::record_vote(&state.conn, &meeting_id, &session_id, &choice) {
                Ok(recorded_choice) => {
                    crate::events::emit(
                        &state.events,
                        "meeting_vote",
                        serde_json::json!({
                            "meeting_id": &meeting_id, "session_id": &session_id, "choice": &recorded_choice,
                        }),
                    );

                    // Auto-resolve if quorum is met
                    if let Ok(Some(outcome)) =
                        crate::teams::check_and_resolve_vote(&state.conn, &meeting_id)
                    {
                        let topic: String = state
                            .conn
                            .query_row(
                                "SELECT topic FROM meeting WHERE id = ?1",
                                rusqlite::params![meeting_id],
                                |row| row.get(0),
                            )
                            .unwrap_or_else(|_| "unknown".to_string());
                        crate::events::emit(
                            &state.events,
                            "meeting_decided",
                            serde_json::json!({
                                "meeting_id": &meeting_id,
                                "topic": topic,
                                "outcome": outcome,
                                "status": "decided",
                            }),
                        );
                    }

                    Response::Ok {
                        data: ResponseData::MeetingVoteRecorded {
                            meeting_id,
                            choice: recorded_choice,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("meeting_vote failed: {e}"),
                },
            }
        }

        Request::MeetingResult { meeting_id } => {
            match crate::teams::get_vote_results(&state.conn, &meeting_id) {
                Ok(results) => Response::Ok {
                    data: ResponseData::MeetingResultData {
                        meeting_id,
                        outcome: results.outcome,
                        votes: results.votes,
                        quorum_met: results.quorum_met,
                        total_votes: results.total_votes,
                        required_votes: results.required_votes,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("meeting_result failed: {e}"),
                },
            }
        }

        // ── Notification Engine ──
        Request::ListNotifications {
            status,
            category,
            limit,
        } => {
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
                    let vals: Vec<serde_json::Value> = notifs
                        .iter()
                        .map(|n| {
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
                        })
                        .collect();
                    Response::Ok {
                        data: ResponseData::NotificationList {
                            notifications: vals,
                            count,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_notifications failed: {e}"),
                },
            }
        }

        Request::AckNotification { id } => {
            match crate::notifications::ack_notification(&state.conn, &id) {
                Ok(_) => Response::Ok {
                    data: ResponseData::NotificationAcked { id },
                },
                Err(e) => Response::Error {
                    message: format!("ack_notification failed: {e}"),
                },
            }
        }

        Request::DismissNotification { id } => {
            match crate::notifications::dismiss_notification(&state.conn, &id) {
                Ok(_) => Response::Ok {
                    data: ResponseData::NotificationDismissed { id },
                },
                Err(e) => Response::Error {
                    message: format!("dismiss_notification failed: {e}"),
                },
            }
        }

        Request::ActOnNotification { id, approved } => {
            match crate::notifications::act_on_notification(&state.conn, &id, approved) {
                Ok(result) => Response::Ok {
                    data: ResponseData::NotificationActed { id, result },
                },
                Err(e) => Response::Error {
                    message: format!("act_on_notification failed: {e}"),
                },
            }
        }

        // ── Organization Hierarchy ──
        Request::CreateOrganization { name, description } => {
            match crate::org::create_organization(&state.conn, &name, description.as_deref()) {
                Ok(id) => Response::Ok {
                    data: ResponseData::OrganizationCreated { id },
                },
                Err(e) => Response::Error {
                    message: format!("create_organization: {e}"),
                },
            }
        }
        Request::ListOrganizations => match crate::org::list_organizations(&state.conn) {
            Ok(orgs) => Response::Ok {
                data: ResponseData::OrganizationList {
                    organizations: orgs,
                },
            },
            Err(e) => Response::Error {
                message: format!("list_organizations: {e}"),
            },
        },
        Request::TeamSend {
            team_name,
            kind,
            topic,
            parts,
            from_session,
            recursive,
        } => {
            let from = from_session.as_deref().unwrap_or("system");

            // Enforce team topology before routing messages
            if let Ok((topology, orchestrator)) =
                crate::teams::get_team_topology(&state.conn, &team_name)
            {
                if topology == "star" {
                    // In star topology, only the orchestrator (or "system") can send to team members.
                    // Non-orchestrator members must route through the orchestrator.
                    if from != "system" {
                        if let Some(ref orch_id) = orchestrator {
                            if from != orch_id.as_str() {
                                return Response::Error {
                                    message: format!(
                                        "star topology: only the orchestrator ({orch_id}) can send to team members, not {from}"
                                    ),
                                };
                            }
                        }
                        // If no orchestrator is set, allow messages (degrade gracefully)
                    }
                }
                // mesh: any-to-any (default, no restriction)
                // chain: not yet enforced (follow-up)
            }

            match crate::org::team_session_ids(&state.conn, &team_name, recursive) {
                Ok(session_ids) => {
                    let parts_json =
                        serde_json::to_string(&parts).unwrap_or_else(|_| "[]".to_string());
                    let mut sent = 0usize;
                    for sid in &session_ids {
                        if crate::sessions::send_message(
                            &state.conn,
                            from,
                            sid,
                            &kind,
                            &topic,
                            &parts_json,
                            None,
                            None,
                            None,
                        )
                        .is_ok()
                        {
                            sent += 1;
                        }
                    }
                    crate::events::emit(
                        &state.events,
                        "team_message_sent",
                        serde_json::json!({
                            "team": team_name, "recipients": sent, "recursive": recursive, "topic": topic,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::TeamSent {
                            messages_sent: sent,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("team_send: {e}"),
                },
            }
        }
        Request::TeamTree { organization_id } => {
            let org = organization_id.as_deref().unwrap_or("default");
            match crate::org::team_tree(&state.conn, org) {
                Ok(tree) => Response::Ok {
                    data: ResponseData::TeamTreeData { tree },
                },
                Err(e) => Response::Error {
                    message: format!("team_tree: {e}"),
                },
            }
        }
        Request::CreateOrgFromTemplate {
            template_name,
            org_name,
        } => match crate::org::create_org_from_template(&state.conn, &template_name, &org_name) {
            Ok((org_id, teams_created)) => {
                crate::events::emit(
                    &state.events,
                    "org_created_from_template",
                    serde_json::json!({
                        "org_id": org_id, "template": template_name, "teams_created": teams_created,
                    }),
                );
                Response::Ok {
                    data: ResponseData::OrgFromTemplateCreated {
                        org_id,
                        teams_created,
                    },
                }
            }
            Err(e) => Response::Error {
                message: format!("create_org_from_template: {e}"),
            },
        },

        // ── Memory Self-Healing ──
        Request::HealingStatus => {
            let total_superseded: i64 = state
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM healing_log WHERE action = 'auto_superseded'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let total_faded: i64 = state
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM healing_log WHERE action = 'auto_faded'",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let last_cycle: Option<String> = state
                .conn
                .query_row("SELECT MAX(created_at) FROM healing_log", [], |r| r.get(0))
                .ok()
                .flatten();
            let stale: i64 = state.conn.query_row(
                "SELECT COUNT(*) FROM memory WHERE status = 'active' AND COALESCE(quality_score, 0.5) < 0.2 AND access_count = 0
                 AND created_at < datetime('now', '-7 days')", [], |r| r.get(0),
            ).unwrap_or(0);
            Response::Ok {
                data: ResponseData::HealingStatusResult {
                    total_healed: (total_superseded + total_faded) as usize,
                    auto_superseded: total_superseded as usize,
                    auto_faded: total_faded as usize,
                    last_cycle_at: last_cycle,
                    stale_candidates: stale as usize,
                },
            }
        }

        Request::HealingRun => {
            let config = crate::config::load_config().healing;
            let topic_stats =
                crate::workers::consolidator::heal_topic_supersedes(&state.conn, &config);
            let faded = crate::workers::consolidator::heal_session_staleness(&state.conn, &config);
            let quality =
                crate::workers::consolidator::apply_quality_pressure(&state.conn, &config);
            Response::Ok {
                data: ResponseData::HealingRunResult {
                    topic_superseded: topic_stats.topic_superseded,
                    session_faded: faded,
                    quality_adjusted: quality,
                },
            }
        }

        Request::HealingLog { limit, action } => {
            let lim = limit.unwrap_or(20);
            let entries: Vec<serde_json::Value> = if let Some(ref act) = action {
                state.conn.prepare(
                    "SELECT id, action, old_memory_id, new_memory_id, similarity_score, overlap_score, reason, created_at
                     FROM healing_log WHERE action = ?1 ORDER BY created_at DESC LIMIT ?2"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![act, lim as i64], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, String>(0)?,
                            "action": row.get::<_, String>(1)?,
                            "old_memory_id": row.get::<_, String>(2)?,
                            "new_memory_id": row.get::<_, Option<String>>(3)?,
                            "similarity": row.get::<_, Option<f64>>(4)?,
                            "overlap": row.get::<_, Option<f64>>(5)?,
                            "reason": row.get::<_, String>(6)?,
                            "created_at": row.get::<_, String>(7)?,
                        }))
                    })?.collect()
                }).unwrap_or_default()
            } else {
                state.conn.prepare(
                    "SELECT id, action, old_memory_id, new_memory_id, similarity_score, overlap_score, reason, created_at
                     FROM healing_log ORDER BY created_at DESC LIMIT ?1"
                ).and_then(|mut stmt| {
                    stmt.query_map(rusqlite::params![lim as i64], |row| {
                        Ok(serde_json::json!({
                            "id": row.get::<_, String>(0)?,
                            "action": row.get::<_, String>(1)?,
                            "old_memory_id": row.get::<_, String>(2)?,
                            "new_memory_id": row.get::<_, Option<String>>(3)?,
                            "similarity": row.get::<_, Option<f64>>(4)?,
                            "overlap": row.get::<_, Option<f64>>(5)?,
                            "reason": row.get::<_, String>(6)?,
                            "created_at": row.get::<_, String>(7)?,
                        }))
                    })?.collect()
                }).unwrap_or_default()
            };
            let count = entries.len();
            Response::Ok {
                data: ResponseData::HealingLogResult { entries, count },
            }
        }

        // ── Workspace ──
        Request::WorkspaceInit {
            org_name,
            template: _,
        } => {
            let config = crate::config::load_config().workspace;
            // Get team names from the organization's teams in the DB
            let team_names: Vec<String> = {
                // Find the org ID
                let org_id: Option<String> = state
                    .conn
                    .query_row(
                        "SELECT id FROM organization WHERE name = ?1 LIMIT 1",
                        rusqlite::params![&org_name],
                        |row| row.get(0),
                    )
                    .ok();
                if let Some(oid) = &org_id {
                    let mut stmt = state.conn.prepare(
                        "SELECT name FROM team WHERE organization_id = ?1 AND status = 'active'"
                    ).unwrap();
                    stmt.query_map(rusqlite::params![oid], |row| row.get::<_, String>(0))
                        .unwrap()
                        .filter_map(|r| r.ok())
                        .collect()
                } else {
                    Vec::new()
                }
            };

            // Determine project_dir from current working directory env or a sensible default
            let project_dir = std::env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(String::from));

            match crate::workspace::init_org_workspace(
                &config,
                &org_name,
                &team_names,
                project_dir.as_deref(),
            ) {
                Ok(path) => {
                    let teams_created = team_names.len();
                    crate::events::emit(
                        &state.events,
                        "workspace_initialized",
                        serde_json::json!({
                            "org": org_name, "path": path.display().to_string(), "teams": teams_created,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::WorkspaceInitialized {
                            path: path.display().to_string(),
                            teams_created,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("workspace_init: {e}"),
                },
            }
        }

        Request::WorkspaceStatus => {
            let config = crate::config::load_config().workspace;
            // List team names from DB
            let team_names: Vec<String> = {
                match state
                    .conn
                    .prepare("SELECT DISTINCT name FROM team WHERE status = 'active' ORDER BY name")
                {
                    Ok(mut stmt) => stmt
                        .query_map([], |row| row.get::<_, String>(0))
                        .ok()
                        .map(|rows| rows.filter_map(|r| r.ok()).collect())
                        .unwrap_or_default(),
                    Err(_) => vec![],
                }
            };

            Response::Ok {
                data: ResponseData::WorkspaceStatusData {
                    mode: config.mode.clone(),
                    org: config.org.clone(),
                    root: config.root.clone(),
                    teams: team_names,
                },
            }
        }

        Request::SetCurrentTask { session_id, task } => {
            match state.conn.execute(
                "UPDATE session SET current_task = ?1 WHERE id = ?2 AND status IN ('active', 'idle')",
                rusqlite::params![task, session_id],
            ) {
                Ok(n) if n > 0 => {
                    crate::events::emit(
                        &state.events,
                        "session_changed",
                        serde_json::json!({
                            "action": "task_updated",
                            "id": session_id,
                            "current_task": task,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::CurrentTaskSet { session_id, task },
                    }
                }
                Ok(_) => Response::Error {
                    message: format!("session '{session_id}' not found or not active"),
                },
                Err(e) => Response::Error {
                    message: format!("set_current_task failed: {e}"),
                },
            }
        }

        Request::LicenseStatus => {
            let config = crate::config::load_config();
            Response::Ok {
                data: ResponseData::LicenseStatusResult {
                    tier: config.license.tier.clone(),
                    has_key: !config.license.key.is_empty(),
                },
            }
        }

        Request::SetLicense { tier, key } => {
            let valid_tiers = ["free", "pro", "team", "enterprise"];
            if !valid_tiers.contains(&tier.as_str()) {
                return Response::Error {
                    message: format!(
                        "invalid tier '{}' — must be one of: {}",
                        tier,
                        valid_tiers.join(", ")
                    ),
                };
            }
            if let Err(e) = crate::config::update_config("license.tier", &tier) {
                return Response::Error {
                    message: format!("failed to set tier: {e}"),
                };
            }
            if !key.is_empty() {
                if let Err(e) = crate::config::update_config("license.key", &key) {
                    return Response::Error {
                        message: format!("failed to set key: {e}"),
                    };
                }
            }
            Response::Ok {
                data: ResponseData::LicenseSet { tier },
            }
        }

        // ── Skills Registry ──
        Request::SkillsList {
            category,
            search,
            limit,
        } => {
            let lim = limit.unwrap_or(100);
            match crate::skills::list_skills(
                &state.conn,
                category.as_deref(),
                search.as_deref(),
                lim,
            ) {
                Ok(entries) => {
                    let count = entries.len();
                    let skills: Vec<serde_json::Value> = entries
                        .into_iter()
                        .map(|e| serde_json::to_value(e).unwrap_or_default())
                        .collect();
                    Response::Ok {
                        data: ResponseData::SkillsList { skills, count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("skills_list: {e}"),
                },
            }
        }

        Request::SkillsInstall { name, project } => {
            match crate::skills::install_skill(&state.conn, &name, &project) {
                Ok(()) => Response::Ok {
                    data: ResponseData::SkillInstalled { name, project },
                },
                Err(e) => Response::Error {
                    message: format!("skills_install: {e}"),
                },
            }
        }

        Request::SkillsUninstall { name, project } => {
            match crate::skills::uninstall_skill(&state.conn, &name, &project) {
                Ok(()) => Response::Ok {
                    data: ResponseData::SkillUninstalled { name, project },
                },
                Err(e) => Response::Error {
                    message: format!("skills_uninstall: {e}"),
                },
            }
        }

        Request::SkillsInfo { name } => {
            let ws_root = crate::workers::indexer::find_project_dir_from_db(&state.conn)
                .or_else(crate::workers::indexer::find_project_dir)
                .or_else(|| {
                    std::env::current_dir()
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                });
            match crate::skills::skill_info(&state.conn, &name, ws_root.as_deref()) {
                Ok(entry) => {
                    let skill = entry.map(|e| serde_json::to_value(e).unwrap_or_default());
                    Response::Ok {
                        data: ResponseData::SkillInfo { skill },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("skills_info: {e}"),
                },
            }
        }

        Request::SkillsRefresh => {
            let config = crate::config::load_config();
            let skills_dir = if config.skills_directory.is_empty() {
                // Use project directory from active session CWD, fallback to env/transcript heuristic
                let project_dir = crate::workers::indexer::find_project_dir_from_db(&state.conn)
                    .or_else(crate::workers::indexer::find_project_dir)
                    .unwrap_or_else(|| {
                        std::env::current_dir()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    });
                std::path::PathBuf::from(&project_dir).join("skills")
            } else {
                std::path::PathBuf::from(&config.skills_directory)
            };
            match crate::skills::refresh_skills(&state.conn, &skills_dir) {
                Ok(count) => {
                    crate::events::emit(
                        &state.events,
                        "skills_indexed",
                        serde_json::json!({
                            "count": count,
                            "source": "refresh",
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::SkillsRefreshed { count },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("skills_refresh: {e}"),
                },
            }
        }

        Request::RoutingStats => {
            match crate::extraction::router::query_routing_stats(&state.conn) {
                Ok(stats) => Response::Ok {
                    data: ResponseData::RoutingStats {
                        total_routed: stats.total_routed,
                        tiers: stats
                            .tiers
                            .iter()
                            .map(|t| forge_core::protocol::response::RoutingTierStats {
                                tier: t.tier.clone(),
                                count: t.count,
                                successes: t.successes,
                                tokens_saved: t.tokens_saved,
                            })
                            .collect(),
                        total_tokens_saved: stats.total_tokens_saved,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("routing stats query failed: {e}"),
                },
            }
        }

        Request::VacuumDb => {
            // Phase 1: Purge faded memories older than 30 days
            let faded_purged = ops::purge_faded_memories(&state.conn, 30).unwrap_or_else(|e| {
                eprintln!("[vacuum] purge_faded_memories failed: {e}");
                0
            });

            // Phase 2: Remove orphan code entries (files no longer on disk)
            let (orphan_files, orphan_symbols) = ops::cleanup_orphan_code_entries(&state.conn)
                .unwrap_or_else(|e| {
                    eprintln!("[vacuum] cleanup_orphan_code_entries failed: {e}");
                    (0, 0)
                });

            // Phase 2b: Remove orphaned affects edges (file:* targets that no longer exist)
            let orphan_edges =
                ops::cleanup_orphaned_affects_edges(&state.conn).unwrap_or_else(|e| {
                    eprintln!("[vacuum] cleanup_orphaned_affects_edges failed: {e}");
                    0
                });
            if orphan_edges > 0 {
                eprintln!("[vacuum] removed {orphan_edges} orphaned affects edges");
            }

            // Phase 3: Get DB size before, VACUUM, get DB size after
            let db_path: Option<String> = state
                .conn
                .query_row("PRAGMA database_list", [], |row| row.get::<_, String>(2))
                .ok()
                .filter(|p| !p.is_empty());

            let size_before = db_path
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);

            let _ = state.conn.execute_batch("VACUUM;");

            let size_after = db_path
                .as_ref()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .unwrap_or(0);

            let freed = size_before.saturating_sub(size_after);

            eprintln!("[vacuum] faded={faded_purged} orphan_files={orphan_files} orphan_symbols={orphan_symbols} orphan_edges={orphan_edges} freed={freed}");

            Response::Ok {
                data: ResponseData::Vacuumed {
                    faded_purged,
                    orphan_files_removed: orphan_files,
                    orphan_symbols_removed: orphan_symbols,
                    orphan_edges_removed: orphan_edges,
                    freed_bytes: freed,
                },
            }
        }

        Request::BackfillAffects => {
            // Scan all decision/lesson memories and create affects edges for file paths in content/title
            use std::sync::LazyLock;
            static FILE_PATH_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
                regex::Regex::new(r"(?:crates|src|lib|app)/[\w/]+\.(?:rs|ts|tsx|js|py|go)").unwrap()
            });

            let rows: Vec<(String, String, String)> = match state.conn.prepare(
                "SELECT id, title, content FROM memory WHERE memory_type IN ('decision', 'lesson') AND status = 'active'"
            ) {
                Ok(mut stmt) => stmt.query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                }).ok().map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default(),
                Err(e) => return Response::Error { message: format!("backfill query failed: {e}") },
            };

            let memories_scanned = rows.len();
            let mut edges_created = 0usize;

            // Note: edge table has no foreign keys — no need to toggle PRAGMA
            let mut seen_global = std::collections::HashSet::new();
            for (mem_id, title, content) in &rows {
                for text in [title, content] {
                    for cap in FILE_PATH_RE.find_iter(text) {
                        let file_target = format!("file:{}", cap.as_str());
                        let edge_key = format!("{mem_id}→{file_target}");
                        if seen_global.insert(edge_key) {
                            // Check if edge already exists
                            let exists: bool = state.conn.query_row(
                                "SELECT COUNT(*) > 0 FROM edge WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'affects'",
                                rusqlite::params![mem_id, file_target],
                                |row| row.get(0),
                            ).unwrap_or(false);
                            if !exists
                                && ops::store_edge(
                                    &state.conn,
                                    mem_id,
                                    &file_target,
                                    "affects",
                                    "{}",
                                )
                                .is_ok()
                            {
                                edges_created += 1;
                            }
                        }
                    }
                }
            }

            Response::Ok {
                data: ResponseData::BackfillAffectsResult {
                    memories_scanned,
                    edges_created,
                },
            }
        }

        Request::FindSymbol {
            name,
            file,
            project,
        } => {
            if name.trim().is_empty() {
                return Response::Ok {
                    data: ResponseData::SymbolResults { symbols: vec![] },
                };
            }
            let name_pattern = format!("%{name}%");
            // P3-4 W1.2 c2 (I-7): when --project is set, JOIN code_file
            // and add `f.project = ?` to the WHERE clause. The file
            // filter remains a substring-match (LIKE) for parity with
            // pre-W1.2 behavior; project is exact match because the
            // indexer writes a normalized name (basename of project
            // dir, never a path).
            let (sql, params_vec): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match (file.as_deref(), project.as_deref()) {
                (Some(f), Some(p)) => {
                    let file_pattern = format!("%{f}%");
                    (
                        "SELECT s.name, s.kind, s.file_path, s.line_start, s.signature
                         FROM code_symbol s
                         JOIN code_file cf ON s.file_path = cf.path
                         WHERE s.name LIKE ?1 AND s.file_path LIKE ?2 AND cf.project = ?3
                         ORDER BY s.file_path, s.line_start LIMIT 50",
                        vec![
                            Box::new(name_pattern) as Box<dyn rusqlite::types::ToSql>,
                            Box::new(file_pattern),
                            Box::new(p.to_string()),
                        ],
                    )
                }
                (Some(f), None) => {
                    let file_pattern = format!("%{f}%");
                    (
                        "SELECT name, kind, file_path, line_start, signature FROM code_symbol WHERE name LIKE ?1 AND file_path LIKE ?2 ORDER BY file_path, line_start LIMIT 50",
                        vec![Box::new(name_pattern) as Box<dyn rusqlite::types::ToSql>, Box::new(file_pattern)],
                    )
                }
                (None, Some(p)) => (
                    "SELECT s.name, s.kind, s.file_path, s.line_start, s.signature
                     FROM code_symbol s
                     JOIN code_file cf ON s.file_path = cf.path
                     WHERE s.name LIKE ?1 AND cf.project = ?2
                     ORDER BY s.file_path, s.line_start LIMIT 50",
                    vec![
                        Box::new(name_pattern) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(p.to_string()),
                    ],
                ),
                (None, None) => (
                    "SELECT name, kind, file_path, line_start, signature FROM code_symbol WHERE name LIKE ?1 ORDER BY file_path, line_start LIMIT 50",
                    vec![Box::new(name_pattern) as Box<dyn rusqlite::types::ToSql>],
                ),
            };
            match state.conn.prepare(sql) {
                Ok(mut stmt) => {
                    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                        params_vec.iter().map(|p| p.as_ref()).collect();
                    let symbols: Vec<forge_core::protocol::response::SymbolInfo> = stmt
                        .query_map(params_refs.as_slice(), |row| {
                            Ok(forge_core::protocol::response::SymbolInfo {
                                name: row.get(0)?,
                                kind: row.get(1)?,
                                file: row.get(2)?,
                                line: row.get::<_, Option<u32>>(3)?.unwrap_or(0),
                                parent: row.get(4)?,
                            })
                        })
                        .ok()
                        .map(|rows| rows.filter_map(|r| r.ok()).collect())
                        .unwrap_or_default();
                    Response::Ok {
                        data: ResponseData::SymbolResults { symbols },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("find_symbol query failed: {e}"),
                },
            }
        }

        Request::GetSymbolsOverview { file } => {
            let file_pattern = format!("%{file}%");
            match state.conn.prepare(
                "SELECT name, kind, file_path, line_start, signature FROM code_symbol WHERE file_path LIKE ?1 ORDER BY line_start LIMIT 200"
            ) {
                Ok(mut stmt) => {
                    let symbols: Vec<forge_core::protocol::response::SymbolInfo> = stmt.query_map(rusqlite::params![file_pattern], |row| {
                        Ok(forge_core::protocol::response::SymbolInfo {
                            name: row.get(0)?,
                            kind: row.get(1)?,
                            file: row.get(2)?,
                            line: row.get::<_, Option<u32>>(3)?.unwrap_or(0),
                            parent: row.get(4)?,
                        })
                    }).ok().map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default();
                    Response::Ok { data: ResponseData::SymbolResults { symbols } }
                }
                Err(e) => Response::Error { message: format!("get_symbols_overview query failed: {e}") },
            }
        }

        // ── HUD Configuration ──
        Request::GetHudConfig {
            user_id,
            team_id,
            organization_id,
            project,
        } => {
            match crate::hud_config::get_merged_hud_config(
                &state.conn,
                organization_id.as_deref().or(Some("default")),
                team_id.as_deref(),
                project.as_deref(),
                user_id.as_deref(),
            ) {
                Ok(entries) => {
                    let result: Vec<forge_core::protocol::response::HudConfigEntry> = entries
                        .into_iter()
                        .map(|e| forge_core::protocol::response::HudConfigEntry {
                            key: e.key,
                            value: e.value,
                            scope_type: e.scope_type,
                            scope_id: e.scope_id,
                            locked: e.locked,
                        })
                        .collect();
                    Response::Ok {
                        data: ResponseData::HudConfigResult { entries: result },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("get_hud_config failed: {e}"),
                },
            }
        }

        Request::SetHudConfig {
            scope_type,
            scope_id,
            key,
            value,
            locked,
        } => {
            // Validate the key/value
            if let Err(msg) = crate::hud_config::validate_hud_config(&key, &value) {
                return Response::Error { message: msg };
            }

            // Check if the key is locked at a higher scope
            if let Ok(Some((lock_scope, lock_id))) =
                crate::hud_config::check_lock(&state.conn, &key, &scope_type, None, None, None)
            {
                return Response::Error {
                    message: format!("{key} is locked at {lock_scope} scope ({lock_id})"),
                };
            }

            // Delegate to existing SetScopedConfig logic
            let id = ulid::Ulid::new().to_string();
            let now = forge_core::time::now_iso();
            match state.conn.execute(
                "INSERT OR REPLACE INTO config_scope (id, scope_type, scope_id, key, value, locked, set_by, set_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'user', ?7)",
                rusqlite::params![id, scope_type, scope_id, key, value, locked, now],
            ) {
                Ok(_) => {
                    crate::events::emit(&state.events, "hud_config_changed", serde_json::json!({
                        "key": &key, "scope_type": &scope_type, "scope_id": &scope_id,
                    }));
                    Response::Ok { data: ResponseData::HudConfigSet { key, scope_type, scope_id } }
                }
                Err(e) => Response::Error { message: format!("set_hud_config failed: {e}") },
            }
        }

        Request::ExportHudConfig {
            scope_type,
            scope_id,
        } => match crate::hud_config::export_as_toml(&state.conn, &scope_type, &scope_id) {
            Ok(toml) => Response::Ok {
                data: ResponseData::HudConfigExport { toml },
            },
            Err(e) => Response::Error {
                message: format!("export_hud_config failed: {e}"),
            },
        },

        Request::Shutdown => Response::Ok {
            data: ResponseData::Shutdown,
        },

        // ── Budget Tracking ──
        Request::RecordAgentCost {
            session_id,
            amount,
            description,
        } => {
            match crate::teams::record_agent_cost(&state.conn, &session_id, amount, &description) {
                Ok((total_spent, budget_limit, exceeded)) => Response::Ok {
                    data: ResponseData::CostRecorded {
                        session_id,
                        total_spent,
                        budget_limit,
                        exceeded,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("record_agent_cost: {e}"),
                },
            }
        }

        Request::BudgetStatus { session_id } => {
            match crate::teams::budget_status(&state.conn, session_id.as_deref()) {
                Ok(entries) => Response::Ok {
                    data: ResponseData::BudgetStatusResult { entries },
                },
                Err(e) => Response::Error {
                    message: format!("budget_status: {e}"),
                },
            }
        }

        // ── Raw layer (benchmark parity + verbatim retrieval) ──
        Request::RawIngest {
            text,
            project,
            session_id,
            source,
            timestamp,
            metadata,
        } => {
            let Some(embedder) = state
                .raw_embedder
                .clone()
                .or_else(crate::embed::global_embedder)
            else {
                return Response::Error {
                    message: "raw_ingest: embedder not initialized — daemon must load the MiniLM model before raw layer requests can be served".to_string(),
                };
            };
            let metadata_string = metadata.map(|v| v.to_string());
            match crate::raw::ingest_text(
                &state.conn,
                &embedder,
                crate::raw::IngestParams {
                    text: &text,
                    source: &source,
                    project: project.as_deref(),
                    session_id: session_id.as_deref(),
                    timestamp: timestamp.as_deref(),
                    metadata_json: metadata_string.as_deref(),
                },
            ) {
                Ok(report) => Response::Ok {
                    data: ResponseData::RawIngest {
                        document_id: report.document_id,
                        chunk_count: report.chunk_count,
                        total_chars: report.total_chars,
                    },
                },
                Err(e) => Response::Error {
                    message: format!("raw_ingest: {e}"),
                },
            }
        }

        Request::RawSearch {
            query,
            project,
            session_id,
            k,
            max_distance,
        } => {
            let Some(embedder) = state
                .raw_embedder
                .clone()
                .or_else(crate::embed::global_embedder)
            else {
                return Response::Error {
                    message: "raw_search: embedder not initialized — daemon must load the MiniLM model before raw layer requests can be served".to_string(),
                };
            };
            let dim = embedder.dim();
            match crate::raw::search(
                &state.conn,
                &embedder,
                &query,
                project.as_deref(),
                session_id.as_deref(),
                k,
                max_distance,
            ) {
                Ok(hits) => {
                    let response_hits: Vec<forge_core::protocol::RawSearchHit> = hits
                        .into_iter()
                        .map(|h| forge_core::protocol::RawSearchHit {
                            chunk_id: h.chunk_id,
                            document_id: h.document_id,
                            chunk_index: h.chunk_index,
                            text: h.text,
                            project: h.project,
                            session_id: h.session_id,
                            source: h.source,
                            timestamp: h.timestamp,
                            distance: h.distance,
                        })
                        .collect();
                    Response::Ok {
                        data: ResponseData::RawSearch {
                            hits: response_hits,
                            query_embedding_dim: dim,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("raw_search: {e}"),
                },
            }
        }

        Request::RawDocumentsList { source, limit } => {
            // Default row cap: 10_000. Large enough for any realistic
            // Forge-Persist workload, small enough to keep a single
            // pathological caller from fan-out-reading the whole table.
            const RAW_DOCUMENTS_LIST_DEFAULT_LIMIT: usize = 10_000;
            let effective_limit = limit.unwrap_or(RAW_DOCUMENTS_LIST_DEFAULT_LIMIT);
            match crate::db::raw::list_documents_by_source(&state.conn, &source, effective_limit) {
                Ok(docs) => {
                    let documents = docs
                        .into_iter()
                        .map(|d| forge_core::protocol::RawDocumentInfo {
                            id: d.id,
                            source: d.source,
                            text: d.text,
                            timestamp: d.timestamp,
                        })
                        .collect();
                    Response::Ok {
                        data: ResponseData::RawDocumentsList { documents },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("raw_documents_list: {e}"),
                },
            }
        }
        // Phase 2A-4d.2: Observability API. Shape handlers live in
        // server/inspect.rs. `/inspect row_count` reads the atomic
        // GaugeSnapshot by cloning it once (tiny struct, cheap); if the
        // daemon was constructed without metrics (shouldn't happen in
        // production but does in tests) we pass None and row_count returns
        // stale+empty.
        //
        // T9/Q9 fix: if the snapshot is empty (refreshed_at_secs == 0, i.e.
        // no one has scraped /metrics yet) and the caller is asking for
        // row_count, lazy-refresh the snapshot right here. Otherwise
        // `/inspect row_count` would return stale:true forever on daemons
        // without a Prometheus scraper. Non-row_count shapes don't use the
        // snapshot, so no refresh needed.
        Request::Inspect {
            shape,
            window,
            filter,
            group_by,
        } => {
            if matches!(shape, forge_core::protocol::InspectShape::RowCount) {
                if let Some(metrics_arc) = state.metrics.as_ref() {
                    let needs_refresh = metrics_arc.snapshot.read().refreshed_at_secs == 0;
                    if needs_refresh {
                        crate::server::metrics::refresh_gauges_from_conn(metrics_arc, &state.conn);
                    }
                }
            }
            let snap_owned = state.metrics.as_ref().map(|m| m.snapshot.read().clone());
            let snap_ref = snap_owned.as_ref();
            crate::server::inspect::run_inspect(
                &state.conn,
                shape,
                window,
                filter,
                group_by,
                snap_ref,
            )
        }
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
    fn w1_35_w28_i9_remember_threads_valence_and_intensity_to_memory() {
        // P3-4 W1.35 (I-9): `--valence` and `--intensity` end-to-end —
        // a Remember request with explicit valence + intensity must
        // persist them onto the stored memory row, replacing the
        // `Memory::new` defaults of "neutral" / 0.5.
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let remember_req = Request::Remember {
            memory_type: MemoryType::Preference,
            title: "Prefer rust-analyzer over rls".to_string(),
            content: "rust-analyzer is the supported LSP".to_string(),
            confidence: Some(0.9),
            tags: None,
            project: Some("forge".to_string()),
            metadata: None,
            valence: Some("positive".to_string()),
            intensity: Some(0.85),
        };
        let response = handle_request(&mut state, remember_req);
        let id = match response {
            Response::Ok { data: ResponseData::Stored { id } } => id,
            other => panic!("expected Stored, got {other:?}"),
        };

        let (valence, intensity): (String, f64) = state
            .conn
            .query_row(
                "SELECT valence, intensity FROM memory WHERE id = ?1",
                rusqlite::params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(valence, "positive");
        assert!(
            (intensity - 0.85).abs() < 1e-6,
            "intensity must round-trip exactly, got {intensity}"
        );

        // Intensity-only (no valence) keeps the default "neutral" valence.
        let req2 = Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Intensity only".to_string(),
            content: "no valence flag".to_string(),
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
            valence: None,
            intensity: Some(0.7),
        };
        let r2 = handle_request(&mut state, req2);
        let id2 = match r2 {
            Response::Ok { data: ResponseData::Stored { id } } => id,
            other => panic!("expected Stored, got {other:?}"),
        };
        let (v2, i2): (String, f64) = state
            .conn
            .query_row(
                "SELECT valence, intensity FROM memory WHERE id = ?1",
                rusqlite::params![id2],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(v2, "neutral");
        assert!((i2 - 0.7).abs() < 1e-6);

        // Empty `valence: Some("")` is treated as None — falls back to
        // default "neutral" / 0.5 from Memory::new.
        let req3 = Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Empty valence string".to_string(),
            content: "should not override".to_string(),
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
            valence: Some(String::new()),
            intensity: None,
        };
        let r3 = handle_request(&mut state, req3);
        let id3 = match r3 {
            Response::Ok { data: ResponseData::Stored { id } } => id,
            other => panic!("expected Stored, got {other:?}"),
        };
        let (v3, i3): (String, f64) = state
            .conn
            .query_row(
                "SELECT valence, intensity FROM memory WHERE id = ?1",
                rusqlite::params![id3],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(v3, "neutral", "empty valence string falls back to default");
        // Memory::new sets intensity = 0.0 by default; --valence "" + no
        // --intensity must NOT call with_valence at all.
        assert!((i3 - 0.0).abs() < 1e-6, "intensity defaults to 0.0 (Memory::new)");
    }

    #[test]
    fn wave_c_d_fix_med1_remember_rejects_invalid_valence_string() {
        // Wave C+D fix-wave MED-1: HTTP/non-CLI clients can send
        // arbitrary valence strings. The handler must reject any value
        // outside {positive, negative, neutral} so a typo can't silently
        // poison the contradiction-detection path.
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        for bad in ["positiv", "POSITIVE", "happy", "????", "neg"] {
            let req = Request::Remember {
                memory_type: MemoryType::Preference,
                title: format!("invalid valence test: {bad}"),
                content: "should be rejected".to_string(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: Some(bad.to_string()),
                intensity: Some(0.5),
            };
            let resp = handle_request(&mut state, req);
            match resp {
                Response::Error { message } => {
                    assert!(
                        message.contains("invalid valence") && message.contains(bad),
                        "expected reject for '{bad}', got '{message}'"
                    );
                }
                other => panic!("expected Error reject for '{bad}', got {other:?}"),
            }
        }
        // Sanity: each of the three accepted strings still works.
        for good in ["positive", "negative", "neutral"] {
            let req = Request::Remember {
                memory_type: MemoryType::Preference,
                title: format!("valid valence: {good}"),
                content: "should be accepted".to_string(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: Some(good.to_string()),
                intensity: Some(0.5),
            };
            let resp = handle_request(&mut state, req);
            assert!(
                matches!(resp, Response::Ok { .. }),
                "valence '{good}' must be accepted; got {resp:?}"
            );
        }
    }

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
            metadata: None,
            valence: None,
            intensity: None,
        };
        let response = handle_request(&mut state, remember_req);

        let stored_id = match response {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => {
                assert!(!id.is_empty(), "stored id must be non-empty");
                id
            }
            other => panic!("expected Stored response, got {other:?}"),
        };

        // Recall "JWT auth"
        let recall_req = Request::Recall {
            query: "JWT auth".to_string(),
            memory_type: None,
            project: None,
            limit: None,
            layer: None,
            since: None,
            include_flipped: None,
            include_globals: None,
            query_embedding: None,
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
            other => panic!("expected Memories response, got {other:?}"),
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
            other => panic!("expected Health response, got {other:?}"),
        }
    }

    #[test]
    fn test_health_by_project() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store memories in different projects
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Forge arch".into(),
                content: "Rust CLI".into(),
                confidence: None,
                tags: None,
                project: Some("forge".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "Backend lesson".into(),
                content: "REST".into(),
                confidence: None,
                tags: None,
                project: Some("backend".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Pattern,
                title: "Global pattern".into(),
                content: "Always test".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let resp = handle_request(&mut state, Request::HealthByProject);
        match resp {
            Response::Ok {
                data: ResponseData::HealthByProject { projects },
            } => {
                assert_eq!(projects.get("forge").unwrap().decisions, 1);
                assert_eq!(projects.get("backend").unwrap().lessons, 1);
                // Phase P3-3.11 W29: globals carry the explicit '_global_' sentinel.
                assert_eq!(projects.get("_global_").unwrap().patterns, 1);
            }
            other => panic!("expected HealthByProject response, got {other:?}"),
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
    fn test_doctor_includes_version_and_raw_stats() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let resp = handle_request(&mut state, Request::Doctor);
        match resp {
            Response::Ok {
                data:
                    ResponseData::Doctor {
                        version,
                        raw_document_count,
                        raw_chunk_count,
                        active_session_count,
                        session_message_count,
                        ..
                    },
            } => {
                assert!(!version.is_empty(), "doctor should include version");
                assert_eq!(raw_document_count, 0);
                assert_eq!(raw_chunk_count, 0);
                assert_eq!(active_session_count, 0);
                assert_eq!(session_message_count, 0);
            }
            other => panic!("expected Doctor response, got: {other:?}"),
        }
    }

    #[test]
    fn test_export_empty() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::Export {
                format: None,
                since: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::Export {
                        memories,
                        files,
                        symbols,
                        edges,
                    },
            } => {
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
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use Rust".into(),
                content: "Fast".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let resp = handle_request(
            &mut state,
            Request::Export {
                format: None,
                since: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::Export {
                        memories,
                        files,
                        symbols,
                        edges,
                    },
            } => {
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
        let resp = handle_request(
            &mut state,
            Request::Export {
                format: None,
                since: None,
            },
        );
        match &resp {
            Response::Ok {
                data: ResponseData::Export { memories, .. },
            } => {
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

        let resp = handle_request(
            &mut state,
            Request::Import {
                data: import_data.to_string(),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::Import {
                        memories_imported,
                        files_imported,
                        symbols_imported,
                        skipped,
                    },
            } => {
                assert_eq!(memories_imported, 1);
                assert_eq!(files_imported, 1);
                assert_eq!(symbols_imported, 1);
                assert_eq!(skipped, 0);
            }
            _ => panic!("expected Import response"),
        }

        // Verify the imported memory shows up in export
        let resp = handle_request(
            &mut state,
            Request::Export {
                format: None,
                since: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::Export {
                        memories,
                        files,
                        symbols,
                        ..
                    },
            } => {
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
        let resp = handle_request(
            &mut state,
            Request::GuardrailsCheck {
                file: "src/lib.rs".into(),
                action: "edit".into(),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::GuardrailsCheck {
                        safe,
                        warnings,
                        decisions_affected,
                        callers_count,
                        calling_files,
                        relevant_lessons,
                        dangerous_patterns,
                        applicable_skills,
                    },
            } => {
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

        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use JWT".into(),
                content: "Auth".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let id = match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            _ => panic!("expected Stored"),
        };

        crate::db::ops::store_edge(&state.conn, &id, "file:src/auth.rs", "affects", "{}").unwrap();

        let resp = handle_request(
            &mut state,
            Request::GuardrailsCheck {
                file: "src/auth.rs".into(),
                action: "edit".into(),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::GuardrailsCheck {
                        safe,
                        decisions_affected,
                        ..
                    },
            } => {
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
        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use JWT auth".into(),
                content: "Security decision".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let id = match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            _ => panic!("expected Stored"),
        };

        // Link decision to a file
        crate::db::ops::store_edge(&state.conn, &id, "file:src/auth.rs", "affects", "{}").unwrap();

        // Drain any prior events (e.g. from remember)
        while rx.try_recv().is_ok() {}

        // Fire guardrails check — should be unsafe because decision is linked
        let resp = handle_request(
            &mut state,
            Request::GuardrailsCheck {
                file: "src/auth.rs".into(),
                action: "edit".into(),
            },
        );

        // Verify the response itself is still correct
        match &resp {
            Response::Ok {
                data:
                    ResponseData::GuardrailsCheck {
                        safe,
                        decisions_affected,
                        ..
                    },
            } => {
                assert!(!safe);
                assert_eq!(decisions_affected.len(), 1);
            }
            _ => panic!("expected GuardrailsCheck response"),
        }

        // Should have emitted a guardrail_warning event
        let event = rx
            .try_recv()
            .expect("should have emitted guardrail_warning event");
        assert_eq!(event.event, "guardrail_warning");
        assert_eq!(event.data["safe"], false);
        assert_eq!(event.data["file"], "src/auth.rs");
        assert!(event.data["warnings"].is_array());
        assert!(event.data["decisions_affected"].is_array());
        assert_eq!(
            event.data["decisions_affected"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn test_guardrail_check_safe_no_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        // Drain any prior events
        while rx.try_recv().is_ok() {}

        // Fire guardrails check on a file with no linked decisions — should be safe
        handle_request(
            &mut state,
            Request::GuardrailsCheck {
                file: "src/lib.rs".into(),
                action: "edit".into(),
            },
        );

        // Should NOT have emitted a guardrail_warning event
        assert!(
            rx.try_recv().is_err(),
            "should not emit event when check is safe"
        );
    }

    #[test]
    fn test_post_edit_check_clean_file() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::PostEditCheck {
                file: "src/lib.rs".into(),
                session_id: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::PostEditChecked {
                        file,
                        callers_count,
                        calling_files,
                        relevant_lessons,
                        dangerous_patterns,
                        applicable_skills,
                        decisions_to_review,
                        cached_diagnostics,
                        ..
                    },
            } => {
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
        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use JWT".into(),
                content: "JWT tokens".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let _id = match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            _ => panic!("expected Stored"),
        };
        crate::db::ops::store_edge(&state.conn, &_id, "file:src/auth.rs", "affects", "{}").unwrap();

        // Drain prior events
        while rx.try_recv().is_ok() {}

        let resp = handle_request(
            &mut state,
            Request::PostEditCheck {
                file: "src/auth.rs".into(),
                session_id: None,
            },
        );
        match &resp {
            Response::Ok {
                data:
                    ResponseData::PostEditChecked {
                        decisions_to_review,
                        ..
                    },
            } => {
                assert!(!decisions_to_review.is_empty());
                assert!(decisions_to_review[0].contains("Use JWT"));
            }
            _ => panic!("expected PostEditChecked response"),
        }

        // Should have emitted a post_edit_warning event
        let event = rx
            .try_recv()
            .expect("should have emitted post_edit_warning event");
        assert_eq!(event.event, "post_edit_warning");
        assert_eq!(event.data["file"], "src/auth.rs");
    }

    #[test]
    fn test_blast_radius_empty() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::BlastRadius {
                file: "src/lib.rs".into(),
                project: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::BlastRadius {
                        decisions,
                        callers,
                        importers,
                        files_affected,
                        cluster_name,
                        cluster_files,
                        calling_files,
                        warnings: _,
                    },
            } => {
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
    fn test_remember_decision_creates_affects_edges() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store a decision that mentions file paths in its content
        let resp = handle_request(&mut state, Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Refactor crates/daemon/src/server/handler.rs".into(),
            content: "The handler in crates/daemon/src/server/handler.rs should be split. Also affects src/db/ops.rs for the edge storage.".into(),
            confidence: Some(0.9),
            tags: None,
            project: None,
            metadata: None,
            valence: None,
            intensity: None,
        });
        let id = match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            other => panic!("expected Stored, got {other:?}"),
        };

        // Check that affects edges were created
        let edge_count: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE from_id = ?1 AND edge_type = 'affects'",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            edge_count >= 2,
            "expected at least 2 affects edges (handler.rs + ops.rs), got {edge_count}",
        );

        // Verify blast-radius now finds this decision
        let resp = handle_request(
            &mut state,
            Request::BlastRadius {
                file: "crates/daemon/src/server/handler.rs".into(),
                project: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::BlastRadius { decisions, .. },
            } => {
                assert!(
                    !decisions.is_empty(),
                    "blast-radius should find the decision that affects handler.rs",
                );
                assert!(
                    decisions.iter().any(|d| d.title.contains("Refactor")),
                    "decision title should contain 'Refactor', got: {decisions:?}",
                );
            }
            other => panic!("expected BlastRadius response, got {other:?}"),
        }
    }

    #[test]
    fn test_remember_lesson_creates_affects_edges() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "Lesson about src/auth/mod.rs".into(),
                content: "The auth module needs better error handling.".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let id = match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            other => panic!("expected Stored, got {other:?}"),
        };

        // Check that an affects edge was created from the title
        let edge_count: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE from_id = ?1 AND edge_type = 'affects'",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            edge_count >= 1,
            "expected at least 1 affects edge from title file path, got {edge_count}",
        );
    }

    #[test]
    fn test_remember_pattern_no_affects_edges() {
        // Pattern memories should NOT create affects edges
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Pattern,
                title: "Pattern about src/db/ops.rs".into(),
                content: "Always use transactions in src/db/ops.rs".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let id = match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            other => panic!("expected Stored, got {other:?}"),
        };

        let edge_count: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE from_id = ?1 AND edge_type = 'affects'",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            edge_count, 0,
            "Pattern memories should not create affects edges, got {edge_count}",
        );
    }

    #[test]
    fn test_register_and_list_sessions() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register two sessions
        let resp1 = handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s1".into(),
                agent: "claude-code".into(),
                project: Some("forge".into()),
                cwd: Some("/project".into()),
                capabilities: None,
                current_task: None,
            role: None,
            },
        );
        match resp1 {
            Response::Ok {
                data: ResponseData::SessionRegistered { id },
            } => assert_eq!(id, "s1"),
            other => panic!("expected SessionRegistered, got {other:?}"),
        }

        let resp2 = handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s2".into(),
                agent: "cline".into(),
                project: None,
                cwd: None,
                capabilities: None,
                current_task: None,
            role: None,
            },
        );
        match resp2 {
            Response::Ok {
                data: ResponseData::SessionRegistered { id },
            } => assert_eq!(id, "s2"),
            other => panic!("expected SessionRegistered, got {other:?}"),
        }

        // List active sessions — should be 2
        let resp = handle_request(
            &mut state,
            Request::Sessions {
                active_only: Some(true),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Sessions { sessions, count },
            } => {
                assert_eq!(count, 2);
                assert_eq!(sessions.len(), 2);
            }
            other => panic!("expected Sessions, got {other:?}"),
        }
    }

    #[test]
    fn test_end_session_via_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register
        handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s1".into(),
                agent: "claude-code".into(),
                project: None,
                cwd: None,
                capabilities: None,
                current_task: None,
            role: None,
            },
        );

        // End
        let resp = handle_request(&mut state, Request::EndSession { id: "s1".into() });
        match resp {
            Response::Ok {
                data: ResponseData::SessionEnded { id, found, .. },
            } => {
                assert_eq!(id, "s1");
                assert!(found);
            }
            other => panic!("expected SessionEnded, got {other:?}"),
        }

        // List active — should be 0
        let resp = handle_request(
            &mut state,
            Request::Sessions {
                active_only: Some(true),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Sessions { sessions, count },
            } => {
                assert_eq!(count, 0);
                assert!(sessions.is_empty());
            }
            other => panic!("expected Sessions, got {other:?}"),
        }
    }

    #[test]
    fn test_cleanup_sessions_via_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register 3 sessions: 2 hook-test, 1 real
        for id in &["hook-test-1", "hook-test-2", "real-s1"] {
            handle_request(
                &mut state,
                Request::RegisterSession {
                    id: id.to_string(),
                    agent: "claude-code".into(),
                    project: Some("forge".into()),
                    cwd: None,
                    capabilities: None,
                    current_task: None,
                role: None,
                },
            );
        }

        // Cleanup hook-test sessions only
        let resp = handle_request(
            &mut state,
            Request::CleanupSessions {
                prefix: Some("hook-test".into()),
                older_than_secs: None,
                prune_ended: false,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::SessionsCleaned { ended },
            } => {
                assert_eq!(ended, 2, "should end 2 hook-test sessions");
            }
            other => panic!("expected SessionsCleaned, got {other:?}"),
        }

        // Verify: only real session remains
        let resp = handle_request(
            &mut state,
            Request::Sessions {
                active_only: Some(true),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Sessions { count, .. },
            } => {
                assert_eq!(count, 1);
            }
            other => panic!("expected Sessions, got {other:?}"),
        }
    }

    // ── Manas Handler Tests ──

    #[test]
    fn test_platform_store_and_list() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store a platform entry
        let resp = handle_request(
            &mut state,
            Request::StorePlatform {
                key: "os".into(),
                value: "linux".into(),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::PlatformStored { key },
            } => {
                assert_eq!(key, "os");
            }
            other => panic!("expected PlatformStored, got {other:?}"),
        }

        // Store another
        handle_request(
            &mut state,
            Request::StorePlatform {
                key: "arch".into(),
                value: "x86_64".into(),
            },
        );

        // List platform entries
        let resp = handle_request(&mut state, Request::ListPlatform);
        match resp {
            Response::Ok {
                data: ResponseData::PlatformList { entries },
            } => {
                // detect_and_store_platform may have added entries, so check ours exist
                let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
                assert!(keys.contains(&"os"), "should contain 'os', got: {keys:?}");
                assert!(
                    keys.contains(&"arch"),
                    "should contain 'arch', got: {keys:?}"
                );
                let os_entry = entries.iter().find(|e| e.key == "os").unwrap();
                assert_eq!(os_entry.value, "linux");
            }
            other => panic!("expected PlatformList, got {other:?}"),
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
            project: None,
        };
        let resp = handle_request(&mut state, Request::StoreIdentity { facet });
        match resp {
            Response::Ok {
                data: ResponseData::IdentityStored { id },
            } => {
                assert_eq!(id, "if-test-1");
            }
            other => panic!("expected IdentityStored, got {other:?}"),
        }

        // List identity for the agent
        let resp = handle_request(
            &mut state,
            Request::ListIdentity {
                agent: "forge-test".into(),
                project: None,
                include_global_identity: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::IdentityList { facets, count },
            } => {
                assert_eq!(count, 1);
                assert_eq!(facets.len(), 1);
                assert_eq!(facets[0].facet, "role");
                assert_eq!(facets[0].description, "memory system");
            }
            other => panic!("expected IdentityList, got {other:?}"),
        }

        // Deactivate
        let resp = handle_request(
            &mut state,
            Request::DeactivateIdentity {
                id: "if-test-1".into(),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::IdentityDeactivated { id, found },
            } => {
                assert_eq!(id, "if-test-1");
                assert!(found);
            }
            other => panic!("expected IdentityDeactivated, got {other:?}"),
        }

        // List again — active only, should be empty
        let resp = handle_request(
            &mut state,
            Request::ListIdentity {
                agent: "forge-test".into(),
                project: None,
                include_global_identity: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::IdentityList { facets, count },
            } => {
                assert_eq!(count, 0);
                assert!(facets.is_empty());
            }
            other => panic!("expected IdentityList (empty), got {other:?}"),
        }
    }

    #[test]
    fn test_manas_health_handler() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(&mut state, Request::ManasHealth { project: None });
        match resp {
            Response::Ok {
                data:
                    ResponseData::ManasHealthData {
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
            other => panic!("expected ManasHealthData, got {other:?}"),
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
            Response::Ok {
                data: ResponseData::HlcBackfilled { count },
            } => {
                assert_eq!(count, 1, "should backfill 1 memory");
            }
            other => panic!("expected HlcBackfilled, got {other:?}"),
        }

        // Second call should find 0
        let resp = handle_request(&mut state, Request::HlcBackfill);
        match resp {
            Response::Ok {
                data: ResponseData::HlcBackfilled { count },
            } => {
                assert_eq!(count, 0, "no more memories to backfill");
            }
            other => panic!("expected HlcBackfilled, got {other:?}"),
        }
    }

    #[test]
    fn test_backfill_project_handler() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Insert a session with a known project
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, project, started_at, status)
             VALUES ('sess-1', 'claude-code', 'forge', '2026-01-01', 'active')",
                [],
            )
            .unwrap();

        // Insert a memory with session_id but no project
        state.conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id)
             VALUES ('m-orphan1', 'decision', 'Use Rust', 'Rust is fast', 0.9, 'active', '', '[]', '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '', 'sess-1')",
            [],
        ).unwrap();

        // Insert a memory with no session_id and no project (truly orphaned)
        state.conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id)
             VALUES ('m-orphan2', 'lesson', 'Test often', 'Testing saves time', 0.8, 'active', '', '[]', '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '', '')",
            [],
        ).unwrap();

        // Insert a memory that already has a project (should not be touched)
        state.conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, hlc_timestamp, node_id, session_id)
             VALUES ('m-has-proj', 'decision', 'Use SQLite', 'SQLite is great', 0.9, 'active', 'forge', '[]', '2026-01-01', '2026-01-01', 'neutral', 0.0, '', '', '')",
            [],
        ).unwrap();

        let resp = handle_request(&mut state, Request::BackfillProject);
        match resp {
            Response::Ok {
                data: ResponseData::BackfillProjectResult { updated, skipped },
            } => {
                // Phase 1: m-orphan1 updated from session (has session_id='sess-1' -> project='forge')
                // Phase 2: m-orphan2 also updated because only one project ('forge') in DB
                assert_eq!(
                    updated, 2,
                    "should backfill 2 memories (1 from session, 1 from single-project fallback)"
                );
                assert_eq!(skipped, 0, "no memories should remain orphaned");
            }
            other => panic!("expected BackfillProjectResult, got {other:?}"),
        }

        // Verify m-orphan1 now has the project
        let project: String = state
            .conn
            .query_row(
                "SELECT COALESCE(project, '') FROM memory WHERE id = 'm-orphan1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project, "forge", "orphan1 should now have project=forge");

        // Verify m-has-proj is unchanged
        let project2: String = state
            .conn
            .query_row(
                "SELECT COALESCE(project, '') FROM memory WHERE id = 'm-has-proj'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(project2, "forge", "existing project should be untouched");

        // Running again should find 0 updated (orphan1 already backfilled, orphan2 still unresolvable)
        let resp = handle_request(&mut state, Request::BackfillProject);
        match resp {
            Response::Ok {
                data: ResponseData::BackfillProjectResult { updated, skipped },
            } => {
                assert_eq!(updated, 0, "no more memories to backfill");
                assert_eq!(skipped, 0, "all memories should have project now");
            }
            other => panic!("expected BackfillProjectResult, got {other:?}"),
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
            Response::Ok {
                data: ResponseData::PerceptionStored { id },
            } => {
                assert_eq!(id, "p-test-1");
            }
            other => panic!("expected PerceptionStored, got {other:?}"),
        }

        // List unconsumed perceptions
        let resp = handle_request(
            &mut state,
            Request::ListPerceptions {
                project: None,
                limit: None,
                offset: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::PerceptionList { perceptions, count },
            } => {
                assert_eq!(count, 1);
                assert_eq!(perceptions.len(), 1);
                assert_eq!(perceptions[0].data, "compilation failed");
                assert!(!perceptions[0].consumed);
            }
            other => panic!("expected PerceptionList, got {other:?}"),
        }

        // Consume the perception
        let resp = handle_request(
            &mut state,
            Request::ConsumePerceptions {
                ids: vec!["p-test-1".into()],
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::PerceptionsConsumed { count },
            } => {
                assert_eq!(count, 1);
            }
            other => panic!("expected PerceptionsConsumed, got {other:?}"),
        }

        // List unconsumed again — should be empty
        let resp = handle_request(
            &mut state,
            Request::ListPerceptions {
                project: None,
                limit: None,
                offset: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::PerceptionList { perceptions, count },
            } => {
                assert_eq!(count, 0);
                assert!(perceptions.is_empty());
            }
            other => panic!("expected PerceptionList (empty), got {other:?}"),
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
            Response::Ok {
                data: ResponseData::ToolStored { id },
            } => {
                assert_eq!(id, "t-test-1");
            }
            other => panic!("expected ToolStored, got {other:?}"),
        }

        // List tools (includes auto-detected tools from startup + our manually stored one)
        let resp = handle_request(&mut state, Request::ListTools);
        match resp {
            Response::Ok {
                data: ResponseData::ToolList { tools, count },
            } => {
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
            other => panic!("expected ToolList, got {other:?}"),
        }
    }

    // ── Event Emission Tests ──

    #[test]
    fn test_remember_emits_memory_created_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use Rust".into(),
                content: "Fast".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "memory_created");
        assert_eq!(event.data["title"], "Use Rust");
        assert_eq!(event.data["memory_type"], "Decision");
    }

    #[test]
    fn test_session_register_emits_event() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s1".into(),
                agent: "claude-code".into(),
                project: None,
                cwd: None,
                capabilities: None,
                current_task: None,
            role: None,
            },
        );

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
        handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s1".into(),
                agent: "claude-code".into(),
                project: None,
                cwd: None,
                capabilities: None,
                current_task: None,
            role: None,
            },
        );

        let mut rx = state.events.subscribe();

        handle_request(&mut state, Request::EndSession { id: "s1".into() });

        let event = rx.try_recv().unwrap();
        assert_eq!(event.event, "session_changed");
        assert_eq!(event.data["action"], "ended");
        assert_eq!(event.data["id"], "s1");
    }

    #[test]
    fn test_supersede_marks_old_and_creates_edge() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store two decisions
        let resp1 = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Old auth approach".into(),
                content: "Use session cookies".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let old_id = match resp1 {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            _ => panic!("expected Stored"),
        };

        let resp2 = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "New auth approach".into(),
                content: "Use JWT tokens".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let new_id = match resp2 {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
            _ => panic!("expected Stored"),
        };

        // Supersede
        let resp = handle_request(
            &mut state,
            Request::Supersede {
                old_id: old_id.clone(),
                new_id: new_id.clone(),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::Superseded {
                        old_id: oid,
                        new_id: nid,
                    },
            } => {
                assert_eq!(oid, old_id);
                assert_eq!(nid, new_id);
            }
            other => panic!("expected Superseded, got: {other:?}"),
        }

        // Verify old memory is superseded
        let status: String = state
            .conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![old_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "superseded");

        // Verify superseded_by column
        let by: String = state
            .conn
            .query_row(
                "SELECT COALESCE(superseded_by, '') FROM memory WHERE id = ?1",
                rusqlite::params![old_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(by, new_id);

        // Verify edge was created
        let edge_count: i64 = state.conn.query_row(
            "SELECT COUNT(*) FROM edge WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'supersedes'",
            rusqlite::params![new_id, old_id],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(edge_count, 1);

        // Old memory should NOT appear in compile-context
        let ctx_resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: None,
                dry_run: None,
            },
        );
        match ctx_resp {
            Response::Ok {
                data: ResponseData::CompiledContext { context, .. },
            } => {
                assert!(
                    !context.contains("Old auth approach"),
                    "superseded memory should not appear in context"
                );
                assert!(
                    context.contains("New auth approach"),
                    "new memory should appear in context"
                );
            }
            _ => panic!("expected CompiledContext"),
        }
    }

    #[test]
    fn test_forget_emits_memory_forgotten_event() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a memory first
        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Temp decision".into(),
                content: "Will be forgotten".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let id = match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id,
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
            project: None,
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
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use JWT auth".into(),
                content: "For security".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Recall with layer=experience should find it
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "JWT".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: Some("experience".into()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert!(count > 0, "should find memory in experience layer");
            }
            other => panic!("expected Memories, got {other:?}"),
        }

        // Recall with layer=declared should NOT find it
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "JWT".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: Some("declared".into()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert_eq!(count, 0, "should not find memory in declared layer");
            }
            other => panic!("expected Memories, got {other:?}"),
        }
    }

    #[test]
    fn test_recall_layer_none_is_default_behavior() {
        let mut state = DaemonState::new(":memory:").unwrap();

        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use Postgres".into(),
                content: "For persistence".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // layer=None should behave like current (search everything)
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "Postgres".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: None,
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert!(count > 0, "layer=None should find memory");
            }
            other => panic!("expected Memories, got {other:?}"),
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
            project: None,
        };
        handle_request(&mut state, Request::StoreIdentity { facet });

        // Recall with layer=identity, query matching description
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "memory".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: Some("identity".into()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, results, .. },
            } => {
                assert!(count > 0, "should find identity facet matching 'memory'");
                assert_eq!(results[0].source, "identity");
            }
            other => panic!("expected Memories, got {other:?}"),
        }

        // Non-matching query
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "xyzzy_nonexistent".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: Some("identity".into()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert_eq!(count, 0, "should not find anything for non-matching query");
            }
            other => panic!("expected Memories, got {other:?}"),
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
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "compilation".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: Some("perception".into()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, results, .. },
            } => {
                assert!(count > 0, "should find perception matching 'compilation'");
                assert_eq!(results[0].source, "perception");
            }
            other => panic!("expected Memories, got {other:?}"),
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
            ..Default::default()
        };
        crate::db::manas::store_skill(&state.conn, &skill).unwrap();

        // Recall with layer=skill should find it
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "deploy".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: Some("skill".into()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, results, .. },
            } => {
                assert!(count > 0, "should find skill matching 'deploy'");
                assert_eq!(results[0].source, "skill");
            }
            other => panic!("expected Memories, got {other:?}"),
        }

        // Non-matching query
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "xyzzy_nonexistent".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: Some("skill".into()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert_eq!(count, 0, "should not find anything for non-matching query");
            }
            other => panic!("expected Memories, got {other:?}"),
        }
    }

    #[test]
    fn p3_4_z8_session_update_fixes_misregistered_project() {
        // P3-4 Wave Z (Z8) — fix a session whose project label was set
        // by SessionStart in the wrong dir. CC voice feedback §2.6.
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register a session with the WRONG project label.
        let resp = handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s1".into(),
                agent: "claude-code".into(),
                project: Some("parent-dir".into()),
                cwd: Some("/tmp/parent-dir".into()),
                capabilities: None,
                current_task: None,
            role: None,
            },
        );
        assert!(matches!(resp, Response::Ok { .. }));

        // Now update it to the correct subproject.
        let resp = handle_request(
            &mut state,
            Request::SessionUpdate {
                id: "s1".into(),
                project: Some("cc-voice".into()),
                cwd: Some("/tmp/parent-dir/cc-voice".into()),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::SessionUpdated { id, fields },
            } => {
                assert_eq!(id, "s1");
                assert!(fields.contains(&"project".to_string()));
                assert!(fields.contains(&"cwd".to_string()));
            }
            other => panic!("expected SessionUpdated, got {other:?}"),
        }

        // Verify the row was rewritten.
        let row: (String, String) = state
            .conn
            .query_row(
                "SELECT project, cwd FROM session WHERE id = 's1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row.0, "cc-voice");
        assert_eq!(row.1, "/tmp/parent-dir/cc-voice");
    }

    #[test]
    fn p3_4_z8_session_update_unknown_session_errors_clearly() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::SessionUpdate {
                id: "ghost".into(),
                project: Some("x".into()),
                cwd: None,
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(
                    message.contains("ghost"),
                    "error must name the missing session: {message}"
                );
                assert!(message.contains("not found"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn p3_4_z8_session_update_no_fields_errors() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let _ = handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s2".into(),
                agent: "claude-code".into(),
                project: Some("forge".into()),
                cwd: None,
                capabilities: None,
                current_task: None,
            role: None,
            },
        );
        let resp = handle_request(
            &mut state,
            Request::SessionUpdate {
                id: "s2".into(),
                project: None,
                cwd: None,
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(message.contains("no fields supplied"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn p3_4_z7_compile_context_cwd_auto_creates_project() {
        // P3-4 Wave Z (Z7) — when `compile-context --project cc-voice
        // --cwd /path/to/cc-voice` is called and no project record exists
        // for cc-voice, the daemon auto-creates one before rendering. This
        // means cc-voice's first SessionStart sees the cleanly-scoped
        // <code-structure project="cc-voice" resolution="auto-created"
        // domain="rust" files="0" symbols="0"/> right out of the gate.
        // CC voice feedback §1.2 fix #2.
        let mut state = DaemonState::new(":memory:").unwrap();

        // Set up a tempdir that looks like a Rust project so domain
        // detection has something to work with.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"cc-voice\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let cwd = dir.path().to_string_lossy().to_string();

        // Pre-condition: project "cc-voice" does not exist
        let pre = crate::db::ops::get_project_by_name(&state.conn, "cc-voice", "default").unwrap();
        assert!(
            pre.is_none(),
            "cc-voice should not exist before compile-context"
        );

        // Call compile-context with --project cc-voice --cwd <tmpdir>
        let resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: Some("claude-code".into()),
                project: Some("cc-voice".into()),
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: Some(cwd.clone()),
                dry_run: None,
            },
        );

        match resp {
            Response::Ok { .. } => { /* good */ }
            other => panic!("expected Ok, got {other:?}"),
        }

        // Post-condition: project "cc-voice" was auto-created
        let post = crate::db::ops::get_project_by_name(&state.conn, "cc-voice", "default")
            .unwrap()
            .expect("cc-voice project should be auto-created");
        assert_eq!(post.name, "cc-voice");
        assert_eq!(post.project_path.as_deref(), Some(cwd.as_str()));
        assert_eq!(post.domain.as_deref(), Some("rust"));
        assert_eq!(post.detected_from.as_deref(), Some("compile_context_cwd"));
    }

    #[test]
    fn p3_4_x1_compile_context_cwd_auto_creates_under_readonly_routing() {
        // P3-4 Wave X (X1) per cc-voice Round 3 §B — Z7's auto-create at
        // handler.rs:~3423 silently failed in production because
        // Request::CompileContext is in is_read_only(), so the per-request
        // DaemonState's `state.conn` was a read-only SQLite handle and the
        // INSERT errored. The Z7 unit test (above) used
        // DaemonState::new(":memory:") which is write-capable — the
        // routing layer was never exercised end-to-end. This test pins
        // the production path: a tempfile DB + DaemonState::new_reader,
        // simulating exactly what http.rs / socket.rs build for read-only
        // requests. Without the X1 fix, the assertion below would fail
        // because the row never lands.
        //
        // cc-voice's suggested test (verbatim) lives in their Round 3 §B:
        // assert_eq!(count_after, count_before + 1).
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("forge.db").to_string_lossy().to_string();

        // Seed the schema via a write-capable state, then drop it so the
        // next read-only opener doesn't race the schema-create writer.
        {
            let _writer = DaemonState::new(&db_path).unwrap();
        }

        // A code-bearing cwd (Cargo.toml present → engine detects "rust").
        let rust_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            rust_dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let rust_cwd = rust_dir.path().to_string_lossy().to_string();

        // A code-less cwd (no markers → engine returns None → Y2 synthesis
        // falls back to domain="unknown").
        let empty_dir = tempfile::tempdir().unwrap();
        let empty_cwd = empty_dir.path().to_string_lossy().to_string();

        // Build a read-only DaemonState mirroring http.rs:184/socket.rs:190.
        let events = crate::events::create_event_bus();
        let hlc = std::sync::Arc::new(crate::sync::Hlc::new("x1-test-node"));
        let started_at = std::time::Instant::now();
        let (write_tx, _write_rx) = tokio::sync::mpsc::channel(8);

        // Case 1 — fresh project name + Cargo.toml-bearing cwd.
        let mut reader = DaemonState::new_reader(
            &db_path,
            events.clone(),
            hlc.clone(),
            started_at,
            Some(write_tx.clone()),
            None,
        )
        .unwrap();
        let count_before: i64 = reader
            .conn
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        let resp = handle_request(
            &mut reader,
            Request::CompileContext {
                agent: Some("claude-code".into()),
                project: Some("x1-fresh-rust".into()),
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: Some(rust_cwd.clone()),
                dry_run: None,
            },
        );
        assert!(
            matches!(resp, Response::Ok { .. }),
            "expected Ok, got {resp:?}"
        );
        // Open a fresh reader to bypass any per-connection page cache and
        // assert the row landed (the renderer's read-only conn would have
        // seen it via the WAL, but a fresh open is the cleanest assertion).
        let probe = rusqlite::Connection::open(&db_path).unwrap();
        let count_after: i64 = probe
            .query_row("SELECT COUNT(*) FROM project", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count_after,
            count_before + 1,
            "Y2/X1 promised auto-create on code-bearing cwd but no row landed"
        );
        let row: (String, Option<String>, Option<String>) = probe
            .query_row(
                "SELECT name, domain, detected_from FROM project
                 WHERE name = 'x1-fresh-rust' AND organization_id = 'default'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, "x1-fresh-rust");
        assert_eq!(row.1.as_deref(), Some("rust"));
        assert_eq!(row.2.as_deref(), Some("compile_context_cwd"));

        // Case 2 — fresh project name + code-less cwd.
        let mut reader2 =
            DaemonState::new_reader(&db_path, events, hlc, started_at, Some(write_tx), None)
                .unwrap();
        let resp = handle_request(
            &mut reader2,
            Request::CompileContext {
                agent: Some("claude-code".into()),
                project: Some("x1-fresh-empty".into()),
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: Some(empty_cwd.clone()),
                dry_run: None,
            },
        );
        assert!(
            matches!(resp, Response::Ok { .. }),
            "expected Ok, got {resp:?}"
        );
        let probe2 = rusqlite::Connection::open(&db_path).unwrap();
        let row2: (String, Option<String>, Option<String>) = probe2
            .query_row(
                "SELECT name, domain, detected_from FROM project
                 WHERE name = 'x1-fresh-empty' AND organization_id = 'default'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row2.0, "x1-fresh-empty");
        assert_eq!(
            row2.1.as_deref(),
            Some("unknown"),
            "code-less cwd must auto-create with domain=\"unknown\""
        );
        assert_eq!(row2.2.as_deref(), Some("compile_context_cwd"));
    }

    #[test]
    fn p3_4_x1_fw1_compile_context_cwd_does_not_wipe_existing_row_at_same_path() {
        // P3-4 Wave X / X1.fw1 (HIGH — dogfood data-loss). The schema
        // carries `CREATE UNIQUE INDEX idx_reality_path_unique ON
        // reality(project_path) WHERE project_path IS NOT NULL`. Pre-fw1
        // the auto-create branch built a Project with a fresh ULID and
        // ran `INSERT OR REPLACE`; when a different-named row already
        // existed at the same path, SQLite REPLACE semantics removed
        // it before inserting the new row — silent data loss of the
        // user's `forge-next project init forge --domain rust` setup.
        //
        // This test reproduces the dogfood incident: pre-existing
        // `forge` row at /path/to/code, then call
        // compile-context --project test-fresh --cwd /path/to/code.
        // Without the fw1 fix, the post-condition would observe
        // `forge` deleted and replaced by `test-fresh`. With the fix,
        // the auto-create branch is skipped, the existing `forge`
        // row survives untouched, and a tracing::warn is emitted.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("forge.db").to_string_lossy().to_string();

        // Prepare a tempdir that looks like a Rust project.
        let code_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            code_dir.path().join("Cargo.toml"),
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let code_cwd = code_dir.path().to_string_lossy().to_string();

        // Seed: the user explicitly registered `forge` at this path
        // with domain="rust". X1.fw2 (review LOW-4): use a real ULID
        // here so a future invariant-tightening migration that
        // enforces Crockford base32 doesn't silently break this
        // regression test.
        let pre_existing_id = ulid::Ulid::new().to_string();
        {
            let writer = DaemonState::new(&db_path).unwrap();
            let now = forge_core::time::now_iso();
            crate::db::ops::store_project(
                &writer.conn,
                &forge_core::types::Project {
                    id: pre_existing_id.clone(),
                    name: "forge".to_string(),
                    reality_type: "code".to_string(),
                    detected_from: Some("user_init".to_string()),
                    project_path: Some(code_cwd.clone()),
                    domain: Some("rust".to_string()),
                    organization_id: "default".to_string(),
                    owner_type: "user".to_string(),
                    owner_id: "default".to_string(),
                    engine_status: "ok".to_string(),
                    engine_pid: None,
                    created_at: now.clone(),
                    last_active: now,
                    metadata: "{}".to_string(),
                },
            )
            .unwrap();
            // Sanity: row exists.
            let pre = crate::db::ops::get_project_by_name(&writer.conn, "forge", "default")
                .unwrap()
                .unwrap();
            assert_eq!(pre.id, pre_existing_id);
            // Drop the writer so the read-only opener doesn't race.
            drop(writer);
        }

        // Now an agent calls compile-context with a DIFFERENT project
        // name pointing at the same path. Pre-fw1 this would silently
        // delete the `forge` row.
        let events = crate::events::create_event_bus();
        let hlc = std::sync::Arc::new(crate::sync::Hlc::new("x1-fw1-test"));
        let (write_tx, _write_rx) = tokio::sync::mpsc::channel(8);
        let mut reader = DaemonState::new_reader(
            &db_path,
            events,
            hlc,
            std::time::Instant::now(),
            Some(write_tx),
            None,
        )
        .unwrap();
        let resp = handle_request(
            &mut reader,
            Request::CompileContext {
                agent: Some("claude-code".into()),
                project: Some("test-fresh-collision".into()),
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: Some(code_cwd.clone()),
                dry_run: None,
            },
        );
        assert!(
            matches!(resp, Response::Ok { .. }),
            "expected Ok, got {resp:?}"
        );

        // Post-condition: the `forge` row is INTACT (id, name, domain
        // all preserved); the auto-create did NOT fire for the colliding
        // alias name.
        let probe = rusqlite::Connection::open(&db_path).unwrap();
        let row: (String, String, Option<String>) = probe
            .query_row(
                "SELECT id, name, domain FROM project
                 WHERE project_path = ?1 AND organization_id = 'default'",
                rusqlite::params![code_cwd],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            row.0, pre_existing_id,
            "the pre-existing row's id must survive — got {:?}",
            row
        );
        assert_eq!(row.1, "forge", "the pre-existing row's name must survive");
        assert_eq!(
            row.2.as_deref(),
            Some("rust"),
            "the pre-existing row's explicit domain must survive"
        );
        // And the colliding-alias name must NOT have been written.
        use rusqlite::OptionalExtension;
        let no_alias: Option<String> = probe
            .query_row(
                "SELECT id FROM project WHERE name = 'test-fresh-collision' AND organization_id = 'default'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert!(
            no_alias.is_none(),
            "X1.fw1 must NOT auto-create when the cwd is already bound to a different project; got id={no_alias:?}"
        );
    }

    #[test]
    fn p3_4_y5_project_init_is_idempotent_does_not_overwrite_existing_domain() {
        // P3-4 Wave Y (Y5) per cc-voice Round 2 §E: pre-Y5,
        // running `project init cc-voice --path X` then
        // `project init cc-voice --path X --domain code` mutated
        // the row's domain from `unknown` → `code` while saying
        // "already existed". Status line lied about what happened.
        // Y5 makes ProjectInit truly idempotent: existing rows are
        // returned as-is regardless of what the rerun args say.
        let mut state = DaemonState::new(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();

        // First call — explicit domain="unknown".
        let resp = handle_request(
            &mut state,
            Request::ProjectInit {
                name: "cc-voice".to_string(),
                path: Some(path.clone()),
                domain: Some("unknown".to_string()),
                organization_id: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::ProjectInitialized { is_new, domain, .. },
            } => {
                assert!(is_new, "first init must report is_new=true");
                assert_eq!(domain, "unknown");
            }
            other => panic!("expected ProjectInitialized, got {other:?}"),
        }

        // Second call — same name, ATTEMPTING to change domain to "code".
        // Y5 must reject the mutation silently (existing row stays
        // domain=unknown) but still return is_new=false.
        let resp = handle_request(
            &mut state,
            Request::ProjectInit {
                name: "cc-voice".to_string(),
                path: Some(path.clone()),
                domain: Some("code".to_string()),
                organization_id: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::ProjectInitialized {
                        is_new,
                        domain,
                        name,
                        ..
                    },
            } => {
                assert!(!is_new, "second init must report is_new=false");
                assert_eq!(
                    domain, "unknown",
                    "Y5 contract: existing row's domain must NOT be overwritten by rerun args; got domain={domain}"
                );
                assert_eq!(name, "cc-voice");
            }
            other => panic!("expected ProjectInitialized, got {other:?}"),
        }

        // Verify against the DB directly — the reality row's domain
        // must still be "unknown".
        let post = crate::db::ops::get_project_by_name(&state.conn, "cc-voice", "default")
            .unwrap()
            .expect("cc-voice exists");
        assert_eq!(
            post.domain.as_deref(),
            Some("unknown"),
            "DB-side: domain must NOT have been overwritten"
        );
    }

    #[test]
    fn p3_4_y2_project_detect_falls_back_to_unknown_domain_for_codeless_dirs() {
        // P3-4 Wave Y (Y2) per cc-voice Round 2 §B: pre-Y2,
        // `project detect` errored out with "no reality engine can
        // handle path: ..." for any directory without a recognized
        // language marker (Cargo.toml / package.json / pyproject.toml /
        // ...). cc-voice's actual project (one .md file) hit this every
        // time. Y2 makes ProjectDetect synthesise a fallback detection
        // with `domain="unknown"` and `confidence=0.0` so the user can
        // still bind the path. Matches what `project init` already
        // accepts.
        let mut state = DaemonState::new(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        // Write a .md file so the dir exists with content but no code
        // markers. Mirror of cc-voice's actual layout.
        std::fs::write(dir.path().join("README.md"), "# cc-voice\n").unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let resp = handle_request(&mut state, Request::ProjectDetect { path: path.clone() });
        match resp {
            Response::Ok {
                data:
                    ResponseData::ProjectDetected {
                        domain,
                        confidence,
                        is_new,
                        engine,
                        ..
                    },
            } => {
                assert_eq!(
                    domain, "unknown",
                    "code-less dir must fall back to domain=unknown"
                );
                assert_eq!(
                    confidence, 0.0,
                    "fallback detection must report confidence=0.0 so callers can distinguish it from a real engine match"
                );
                assert!(is_new, "first detect on this path should create a row");
                assert_eq!(engine, "code");
            }
            other => panic!(
                "expected ProjectDetected with domain=unknown, got {other:?}; \
                 pre-Y2 this would have errored with 'no reality engine can handle path'"
            ),
        }

        // Re-running on the same path returns the existing row.
        let resp = handle_request(&mut state, Request::ProjectDetect { path });
        match resp {
            Response::Ok {
                data: ResponseData::ProjectDetected { is_new, .. },
            } => assert!(!is_new, "second detect should NOT create a duplicate row"),
            other => panic!("expected ProjectDetected, got {other:?}"),
        }
    }

    #[test]
    fn p3_4_z7_compile_context_cwd_skipped_when_dry_run() {
        // Z5/Z7 — dry-run intentionally skips the auto-create side
        // effect. A user previewing what `compile-context --project
        // cc-voice --cwd /tmp/foo --dry-run` *would* do shouldn't end up
        // with a real cc-voice project record persisted just from
        // running the audit.
        let mut state = DaemonState::new(":memory:").unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"").unwrap();

        let _ = handle_request(
            &mut state,
            Request::CompileContext {
                agent: Some("claude-code".into()),
                project: Some("dry-cc-voice".into()),
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: Some(dir.path().to_string_lossy().to_string()),
                dry_run: Some(true),
            },
        );

        let post =
            crate::db::ops::get_project_by_name(&state.conn, "dry-cc-voice", "default").unwrap();
        assert!(
            post.is_none(),
            "dry-run must NOT create the project record; got: {post:?}"
        );
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
                session_id: None,
                focus: None,
                cwd: None,
                dry_run: None,
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
                assert!(
                    context.contains("<forge-context"),
                    "should contain opening tag"
                );
                assert!(chars > 0, "chars should be > 0");
                assert!(
                    !static_prefix.is_empty(),
                    "static_prefix should not be empty"
                );
                assert!(
                    !dynamic_suffix.is_empty(),
                    "dynamic_suffix should not be empty"
                );
                assert_eq!(layers_used, 9, "full context uses 9 layers");
            }
            other => panic!("expected CompiledContext, got {other:?}"),
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
                session_id: None,
                focus: None,
                cwd: None,
                dry_run: None,
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
                assert!(
                    !context.contains("<forge-dynamic>"),
                    "should not contain dynamic suffix"
                );
                assert_eq!(context, static_prefix, "context should equal static_prefix");
                assert!(dynamic_suffix.is_empty(), "dynamic_suffix should be empty");
                assert_eq!(layers_used, 4, "static only uses 4 layers");
            }
            other => panic!("expected CompiledContext, got {other:?}"),
        }
    }

    #[test]
    fn test_verify_no_file_empty_db() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(&mut state, Request::Verify { file: None });
        match resp {
            Response::Ok {
                data:
                    ResponseData::VerifyResult {
                        files_checked,
                        errors,
                        warnings,
                        diagnostics,
                    },
            } => {
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

        let resp = handle_request(
            &mut state,
            Request::Verify {
                file: Some("src/main.rs".into()),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::VerifyResult {
                        files_checked,
                        errors,
                        warnings,
                        diagnostics,
                    },
            } => {
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
                message: format!("{sev} in {file}"),
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
            Response::Ok {
                data:
                    ResponseData::VerifyResult {
                        files_checked,
                        errors,
                        warnings,
                        diagnostics,
                    },
            } => {
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

        let resp = handle_request(
            &mut state,
            Request::GetDiagnostics {
                file: "src/lib.rs".into(),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::DiagnosticList { diagnostics, count },
            } => {
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
        let resp = handle_request(
            &mut state,
            Request::GetDiagnostics {
                file: "nonexistent.rs".into(),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::DiagnosticList { diagnostics, count },
            } => {
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

        let resp = handle_request(
            &mut state,
            Request::PostEditCheck {
                file: "src/auth.rs".into(),
                session_id: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::PostEditChecked {
                        cached_diagnostics, ..
                    },
            } => {
                assert!(
                    !cached_diagnostics.is_empty(),
                    "should include cached diagnostics"
                );
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
            Response::Ok {
                data:
                    ResponseData::EvaluationStored {
                        lessons_created,
                        diagnostics_created,
                    },
            } => {
                assert_eq!(lessons_created, 1, "should create 1 lesson");
                assert_eq!(
                    diagnostics_created, 0,
                    "medium severity should not create diagnostics"
                );
            }
            other => panic!("expected EvaluationStored, got {other:?}"),
        }

        // Verify the lesson is recallable
        let recall_resp = handle_request(
            &mut state,
            Request::Recall {
                query: "Missing error handling".into(),
                memory_type: None,
                project: None,
                limit: Some(5),
                layer: None,
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match recall_resp {
            Response::Ok {
                data: ResponseData::Memories { results, count },
            } => {
                assert_eq!(count, 1, "should recall exactly 1 lesson");
                assert_eq!(results.len(), 1);
                assert_eq!(
                    results[0].memory.valence, "negative",
                    "bug should have negative valence"
                );
                assert!(
                    (results[0].memory.intensity - 0.6).abs() < 0.01,
                    "medium severity should have 0.6 intensity"
                );
            }
            other => panic!("expected Memories, got {other:?}"),
        }
    }

    #[test]
    fn test_force_consolidate_handler() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Insert duplicate memories using remember_raw to bypass upsert logic
        let m1 = Memory::new(
            MemoryType::Decision,
            "Use JWT auth".to_string(),
            "For auth tokens".to_string(),
        );
        let mut m2 = Memory::new(
            MemoryType::Decision,
            "Use JWT auth".to_string(),
            "For auth tokens".to_string(),
        );
        m2.id = format!("dup-{}", m1.id); // different id, same title+type
        ops::remember_raw(&state.conn, &m1).unwrap();
        ops::remember_raw(&state.conn, &m2).unwrap();

        let resp = handle_request(&mut state, Request::ForceConsolidate);
        match resp {
            Response::Ok {
                data: ResponseData::ConsolidationComplete { exact_dedup, .. },
            } => {
                assert!(exact_dedup > 0, "should dedup at least 1 duplicate memory");
            }
            other => panic!("expected ConsolidationComplete, got {other:?}"),
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
        let affects_edges: Vec<_> = edges.iter().filter(|e| e.2 == "affects").collect();
        assert_eq!(
            affects_edges.len(),
            2,
            "should create 2 affects edges (one per file)"
        );

        // Check edge targets
        let targets: Vec<&String> = affects_edges.iter().map(|e| &e.1).collect();
        assert!(
            targets.contains(&&"file:src/db/query.rs".to_string()),
            "should have edge to file:src/db/query.rs"
        );
        assert!(
            targets.contains(&&"file:src/db/ops.rs".to_string()),
            "should have edge to file:src/db/ops.rs"
        );
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
            Response::Ok {
                data:
                    ResponseData::EvaluationStored {
                        lessons_created,
                        diagnostics_created,
                    },
            } => {
                assert_eq!(lessons_created, 2, "should create 2 lessons");
                assert_eq!(
                    diagnostics_created, 2,
                    "should create 2 diagnostics (both high+)"
                );
            }
            other => panic!("expected EvaluationStored, got {other:?}"),
        }

        // Verify diagnostics exist and are retrievable
        let diags =
            crate::db::diagnostics::get_diagnostics(&state.conn, "src/api/handler.rs").unwrap();
        assert_eq!(diags.len(), 1, "should have 1 diagnostic for handler.rs");
        assert_eq!(diags[0].source, "forge-evaluator");
        assert_eq!(diags[0].severity, "critical");
        assert!(diags[0].message.contains("unvalidated user input"));

        let diags2 =
            crate::db::diagnostics::get_diagnostics(&state.conn, "src/api/routes.rs").unwrap();
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
            Response::Ok {
                data:
                    ResponseData::EvaluationStored {
                        lessons_created,
                        diagnostics_created,
                    },
            } => {
                assert_eq!(
                    lessons_created, 2,
                    "should create 2 lessons even for low severity"
                );
                assert_eq!(
                    diagnostics_created, 0,
                    "should NOT create diagnostics for low/info severity"
                );
            }
            other => panic!("expected EvaluationStored, got {other:?}"),
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
            Response::Ok {
                data:
                    ResponseData::EvaluationStored {
                        lessons_created, ..
                    },
            } => {
                assert_eq!(lessons_created, 1);
            }
            other => panic!("expected EvaluationStored, got {other:?}"),
        }

        // Verify positive valence
        let recall_resp = handle_request(
            &mut state,
            Request::Recall {
                query: "error handling context propagation".into(),
                memory_type: None,
                project: None,
                limit: Some(5),
                layer: None,
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match recall_resp {
            Response::Ok {
                data: ResponseData::Memories { results, count },
            } => {
                assert_eq!(count, 1);
                assert_eq!(
                    results[0].memory.valence, "positive",
                    "good_pattern should have positive valence"
                );
            }
            other => panic!("expected Memories, got {other:?}"),
        }
    }

    #[test]
    fn test_daemon_state_new_is_fast() {
        // DaemonState::new should complete quickly since consolidation
        // and ingestion were moved to background tasks. Threshold is
        // 10s to absorb cargo's parallel-test scheduler contention on
        // loaded CI runners (1500+ siblings). Catches the regression
        // class "consolidation re-introduced into the cold path" — a
        // synchronous consolidation pass takes minutes, not seconds.
        let start = std::time::Instant::now();
        let _state = DaemonState::new(":memory:").expect("DaemonState::new should succeed");
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 10_000,
            "DaemonState::new took {}ms — should be <10000ms (consolidation is now background)",
            elapsed.as_millis()
        );
    }

    // ── Prajna E2E Tests ──

    #[test]
    fn test_context_refresh_empty_delta() {
        let mut state = DaemonState::new(":memory:").unwrap();
        crate::sessions::register_session(&state.conn, "s1", "claude-code", None, None, None, None, None)
            .unwrap();
        let resp = handle_request(
            &mut state,
            Request::ContextRefresh {
                session_id: "s1".into(),
                since: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::ContextDelta {
                        notifications,
                        warnings,
                        anti_patterns,
                        messages_pending,
                        message_summaries,
                    },
            } => {
                assert!(notifications.is_empty());
                assert!(warnings.is_empty());
                assert!(anti_patterns.is_empty());
                assert_eq!(messages_pending, 0);
                assert!(message_summaries.is_empty());
            }
            other => panic!("expected ContextDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_completion_check_no_signal() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::CompletionCheck {
                session_id: "s1".into(),
                claimed_done: false,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::CompletionCheckResult {
                        has_completion_signal,
                        severity,
                        ..
                    },
            } => {
                assert!(!has_completion_signal);
                assert_eq!(severity, "none");
            }
            other => panic!("expected CompletionCheckResult, got {other:?}"),
        }
    }

    #[test]
    fn test_completion_check_with_lessons() {
        let mut state = DaemonState::new(":memory:").unwrap();
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "Unit tests insufficient for daemon upgrades".into(),
                content: "Before calling code production-ready: rebuild, live smoke test".into(),
                tags: Some(vec![
                    "testing".into(),
                    "production-readiness".into(),
                    "anti-pattern".into(),
                ]),
                confidence: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let resp = handle_request(
            &mut state,
            Request::CompletionCheck {
                session_id: "s1".into(),
                claimed_done: true,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::CompletionCheckResult {
                        has_completion_signal,
                        relevant_lessons,
                        severity,
                    },
            } => {
                assert!(has_completion_signal);
                assert!(!relevant_lessons.is_empty(), "should surface UAT lesson");
                assert_eq!(severity, "high");
            }
            other => panic!("expected CompletionCheckResult, got {other:?}"),
        }
    }

    #[test]
    fn test_completion_check_surfaces_deployment_tagged_lesson() {
        let mut state = DaemonState::new(":memory:").unwrap();
        // Store a lesson tagged ONLY with "deployment" — no other completion tags
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "Verify rollback plan before deployment".into(),
                content: "Every deployment needs a rollback strategy".into(),
                tags: Some(vec!["deployment".into()]),
                confidence: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let resp = handle_request(
            &mut state,
            Request::CompletionCheck {
                session_id: "s1".into(),
                claimed_done: true,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::CompletionCheckResult {
                        relevant_lessons,
                        severity,
                        ..
                    },
            } => {
                assert!(
                    !relevant_lessons.is_empty(),
                    "deployment-tagged lesson must be surfaced by CompletionCheck"
                );
                assert_eq!(severity, "high");
            }
            other => panic!("expected CompletionCheckResult, got {other:?}"),
        }
    }

    #[test]
    fn test_task_completion_shipping_task() {
        let mut state = DaemonState::new(":memory:").unwrap();
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "Run live UAT before deploy".into(),
                content: "Every deploy needs verification".into(),
                tags: Some(vec!["uat".into(), "production".into()]),
                confidence: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        let resp = handle_request(
            &mut state,
            Request::TaskCompletionCheck {
                session_id: "s1".into(),
                task_subject: "deploy to production".into(),
                task_description: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::TaskCompletionCheckResult {
                        warnings,
                        checklists,
                    },
            } => {
                assert!(!warnings.is_empty(), "should warn about shipping");
                assert!(!checklists.is_empty(), "should include UAT checklist");
            }
            other => panic!("expected TaskCompletionCheckResult, got {other:?}"),
        }
    }

    #[test]
    fn test_task_completion_non_shipping() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::TaskCompletionCheck {
                session_id: "s1".into(),
                task_subject: "add unit test for parser".into(),
                task_description: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::TaskCompletionCheckResult {
                        warnings,
                        checklists,
                    },
            } => {
                assert!(warnings.is_empty());
                assert!(checklists.is_empty());
            }
            other => panic!("expected TaskCompletionCheckResult, got {other:?}"),
        }
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
                        skills_inferred,
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
                assert_eq!(
                    skills_inferred, 0,
                    "2P-1b §15: Phase 23 count exposed in response"
                );
            }
            other => panic!("expected ConsolidationComplete, got {other:?}"),
        }
    }

    // ── Cortex endpoint tests ──

    #[test]
    fn test_get_graph_data_returns_nodes_and_edges() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store some memories
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use Rust".into(),
                content: "For performance".into(),
                confidence: Some(0.9),
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "Always test".into(),
                content: "Testing prevents regressions".into(),
                confidence: Some(0.8),
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let resp = handle_request(
            &mut state,
            Request::GetGraphData {
                layer: None,
                limit: Some(50),
            },
        );

        match resp {
            Response::Ok {
                data:
                    ResponseData::GraphData {
                        nodes,
                        edges: _,
                        total_nodes,
                        total_edges: _,
                    },
            } => {
                // Should have at least the 2 memory nodes plus platform/tool nodes
                assert!(
                    total_nodes >= 2,
                    "should have at least 2 nodes, got {total_nodes}"
                );
                // Verify the memory nodes are present
                let memory_nodes: Vec<_> =
                    nodes.iter().filter(|n| n.layer == "experience").collect();
                assert!(
                    memory_nodes.len() >= 2,
                    "should have at least 2 experience nodes"
                );
                for node in &memory_nodes {
                    assert!(!node.id.is_empty());
                    assert!(!node.title.is_empty());
                    assert!(node.confidence > 0.0);
                }
            }
            other => panic!("expected GraphData, got {other:?}"),
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
                assert_eq!(
                    memories_extracted, 1,
                    "should parse 1 memory from valid JSON"
                );
                assert!(tokens_in_estimate > 0, "token estimate should be positive");
                // latency_ms can be 0 for fast parsing — just verify it's a valid number
                assert!(latency_ms < 10_000, "latency should be reasonable");
            }
            other => panic!("expected ExtractionResult, got {other:?}"),
        }
    }

    #[test]
    fn test_get_graph_data_layer_filter() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store a memory (experience layer)
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use Rust".into(),
                content: "For performance".into(),
                confidence: Some(0.9),
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Filter by experience layer — should get memory nodes
        let resp = handle_request(
            &mut state,
            Request::GetGraphData {
                layer: Some("experience".into()),
                limit: Some(50),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::GraphData { nodes, .. },
            } => {
                assert!(!nodes.is_empty(), "experience layer should have nodes");
                for node in &nodes {
                    assert_eq!(
                        node.layer, "experience",
                        "all nodes should be experience layer"
                    );
                }
            }
            other => panic!("expected GraphData, got {other:?}"),
        }

        // Filter by identity layer — should be empty (no identity facets stored)
        let resp = handle_request(
            &mut state,
            Request::GetGraphData {
                layer: Some("identity".into()),
                limit: Some(50),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::GraphData { nodes, .. },
            } => {
                assert!(
                    nodes.is_empty(),
                    "identity layer should have no nodes when no facets stored"
                );
            }
            other => panic!("expected GraphData, got {other:?}"),
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
                assert_eq!(
                    model, "unknown",
                    "unknown provider should default model to 'unknown'"
                );
                assert_eq!(
                    memories_extracted, 0,
                    "plain text should not parse as extraction output"
                );
            }
            other => panic!("expected ExtractionResult, got {other:?}"),
        }
    }

    #[test]
    fn test_get_graph_data_position_hints() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Position test".into(),
                content: "Check xyz".into(),
                confidence: Some(0.9),
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let resp = handle_request(
            &mut state,
            Request::GetGraphData {
                layer: Some("experience".into()),
                limit: Some(50),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::GraphData { nodes, .. },
            } => {
                assert!(!nodes.is_empty());
                for node in &nodes {
                    // x and z should be in [-1.0, 1.0] range
                    assert!(node.x >= -1.0 && node.x <= 1.0, "x={} out of range", node.x);
                    assert!(node.z >= -1.0 && node.z <= 1.0, "z={} out of range", node.z);
                    // y should be the layer height (experience = 3.0-4.0)
                    assert!(node.y >= 0.0, "y={} should be non-negative", node.y);
                }
            }
            other => panic!("expected GraphData, got {other:?}"),
        }
    }

    #[test]
    fn test_batch_recall_returns_per_query() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store some memories
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use Rust for backend".into(),
                content: "Rust gives memory safety".into(),
                confidence: Some(0.9),
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "TypeScript for frontend".into(),
                content: "React with TypeScript is productive".into(),
                confidence: Some(0.8),
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let resp = handle_request(
            &mut state,
            Request::BatchRecall {
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
            },
        );

        match resp {
            Response::Ok {
                data: ResponseData::BatchRecallResults { results },
            } => {
                assert_eq!(results.len(), 3, "should have 3 result sets for 3 queries");
                // First query should find the Rust memory
                assert!(!results[0].is_empty(), "Rust query should return results");
                // Second query should find the TypeScript memory
                assert!(
                    !results[1].is_empty(),
                    "TypeScript query should return results"
                );
                // Third query about Python may or may not return results (FTS matching)
            }
            other => panic!("expected BatchRecallResults, got {other:?}"),
        }
    }

    #[test]
    fn test_batch_recall_empty_queries() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(&mut state, Request::BatchRecall { queries: vec![] });

        match resp {
            Response::Ok {
                data: ResponseData::BatchRecallResults { results },
            } => {
                assert!(
                    results.is_empty(),
                    "empty queries should return empty results"
                );
            }
            other => panic!("expected BatchRecallResults, got {other:?}"),
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
            other => panic!("expected ExtractionResult, got {other:?}"),
        }
    }

    #[test]
    fn test_remember_decision_creates_cross_session_perception() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register 2 sessions so cross-session perception triggers
        crate::sessions::register_session(
            &state.conn,
            "s1",
            "claude-code",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        crate::sessions::register_session(
            &state.conn,
            "s2",
            "cline",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Store a decision
        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use JWT for auth".into(),
                content: "Security decision for API".into(),
                confidence: Some(0.9),
                tags: None,
                project: Some("forge".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );
        assert!(matches!(
            resp,
            Response::Ok {
                data: ResponseData::Stored { .. }
            }
        ));

        // Verify cross-session perception was created
        let perceptions =
            crate::db::manas::list_unconsumed_perceptions(&state.conn, None, None).unwrap();
        let cross = perceptions
            .iter()
            .find(|p| p.kind == forge_core::types::manas::PerceptionKind::CrossSessionDecision);
        assert!(cross.is_some(), "cross-session perception should exist");
        let cross = cross.unwrap();
        assert!(
            cross.data.contains("JWT"),
            "perception should reference the decision"
        );
        assert_eq!(cross.project, Some("forge".into()), "should carry project");
        assert!(cross.expires_at.is_some(), "should have TTL");
    }

    #[test]
    fn test_remember_lesson_no_cross_session_perception() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register 2 sessions
        crate::sessions::register_session(
            &state.conn,
            "s1",
            "claude-code",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        crate::sessions::register_session(
            &state.conn,
            "s2",
            "cline",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Store a lesson (NOT a decision)
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "TDD is great".into(),
                content: "Write tests first".into(),
                confidence: None,
                tags: None,
                project: Some("forge".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Verify NO cross-session perception was created
        let perceptions =
            crate::db::manas::list_unconsumed_perceptions(&state.conn, None, None).unwrap();
        let cross = perceptions
            .iter()
            .find(|p| p.kind == forge_core::types::manas::PerceptionKind::CrossSessionDecision);
        assert!(
            cross.is_none(),
            "lessons should not create cross-session perceptions"
        );
    }

    #[test]
    fn test_remember_decision_no_cross_session_when_single_session() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Only 1 session — no cross-session perception needed
        crate::sessions::register_session(
            &state.conn,
            "s1",
            "claude-code",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use NDJSON protocol".into(),
                content: "Daemon IPC format".into(),
                confidence: None,
                tags: None,
                project: Some("forge".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let perceptions =
            crate::db::manas::list_unconsumed_perceptions(&state.conn, None, None).unwrap();
        let cross = perceptions
            .iter()
            .find(|p| p.kind == forge_core::types::manas::PerceptionKind::CrossSessionDecision);
        assert!(
            cross.is_none(),
            "single session should not create cross-session perception"
        );
    }

    // ── ProjectEngine Detection Tests ──

    #[test]
    fn test_detect_reality_rust_project() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"test\"").unwrap();

        let resp = handle_request(
            &mut state,
            Request::ProjectDetect {
                path: dir.path().to_string_lossy().to_string(),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::ProjectDetected {
                        engine,
                        domain,
                        detected_from,
                        confidence,
                        is_new,
                        ..
                    },
            } => {
                assert_eq!(engine, "code");
                assert_eq!(domain, "rust");
                assert_eq!(detected_from, "Cargo.toml");
                assert!((confidence - 0.95).abs() < f64::EPSILON);
                assert!(is_new, "first detection should create a new reality");
            }
            other => panic!("expected ProjectDetected, got {other:?}"),
        }
    }

    #[test]
    fn test_detect_reality_creates_record() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/test").unwrap();
        let path = dir.path().to_string_lossy().to_string();

        // First call should create
        let resp = handle_request(&mut state, Request::ProjectDetect { path: path.clone() });
        let reality_id = match resp {
            Response::Ok {
                data: ResponseData::ProjectDetected { id, is_new, .. },
            } => {
                assert!(is_new, "first detection should create new reality");
                id
            }
            other => panic!("expected ProjectDetected, got {other:?}"),
        };

        // Verify it's in the DB
        let reality = crate::db::ops::get_project_by_path(&state.conn, &path, "default")
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
        let resp1 = handle_request(&mut state, Request::ProjectDetect { path: path.clone() });
        let id1 = match resp1 {
            Response::Ok {
                data: ResponseData::ProjectDetected { id, is_new, .. },
            } => {
                assert!(is_new);
                id
            }
            other => panic!("expected ProjectDetected, got {other:?}"),
        };

        // Second call reuses
        let resp2 = handle_request(&mut state, Request::ProjectDetect { path: path.clone() });
        let id2 = match resp2 {
            Response::Ok {
                data: ResponseData::ProjectDetected { id, is_new, .. },
            } => {
                assert!(!is_new, "second detection should reuse existing reality");
                id
            }
            other => panic!("expected ProjectDetected, got {other:?}"),
        };

        assert_eq!(id1, id2, "both calls should return the same reality ID");
    }

    #[test]
    fn test_detect_reality_empty_dir_returns_unknown() {
        // Pre-Wave Y this test asserted Error on empty dirs — the
        // historical contract was "no reality engine can handle <path>".
        // Wave Y / Y2 (per cc-voice Round 2 §B) changed `Request::ProjectDetect`
        // to fall back to a synthetic detection (`domain="unknown"`,
        // `confidence=0.0`, `detected_from="fallback_no_engine_match"`)
        // so code-less directories can still bind cleanly. The previous
        // assertion shape was missed when Y2 landed; this test now pins
        // the post-Y2 contract that empty dirs return ProjectDetected
        // with the synthetic fallback.
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        // No marker files — Y2 fallback should fire.

        let resp = handle_request(
            &mut state,
            Request::ProjectDetect {
                path: dir.path().to_string_lossy().to_string(),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::ProjectDetected {
                        domain,
                        detected_from,
                        confidence,
                        ..
                    },
            } => {
                assert_eq!(domain, "unknown", "empty dir → domain=\"unknown\"");
                assert_eq!(
                    detected_from, "fallback_no_engine_match",
                    "empty dir → detected_from=\"fallback_no_engine_match\""
                );
                assert!(
                    (confidence - 0.0).abs() < f64::EPSILON,
                    "synthetic detection emits confidence=0.0, got {confidence}"
                );
            }
            other => panic!("expected ProjectDetected with synthetic fallback, got {other:?}"),
        }
    }

    #[test]
    fn test_register_session_auto_tags_reality() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let cwd_path = dir.path().to_string_lossy().to_string();

        // Register session with cwd pointing to a Rust project
        let resp = handle_request(
            &mut state,
            Request::RegisterSession {
                id: "s-reality-test".into(),
                agent: "claude-code".into(),
                project: Some("test-project".into()),
                cwd: Some(cwd_path.clone()),
                capabilities: None,
                current_task: None,
            role: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::SessionRegistered { .. },
            } => {}
            other => panic!("expected SessionRegistered, got {other:?}"),
        }

        // Check that the session now has a id
        let reality_id: Option<String> = state
            .conn
            .query_row(
                "SELECT reality_id FROM session WHERE id = ?1",
                rusqlite::params!["s-reality-test"],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            reality_id.is_some(),
            "session should have reality_id set from auto-detection"
        );

        // Verify the reality record was also created
        let reality = crate::db::ops::get_project_by_path(&state.conn, &cwd_path, "default")
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

        let resp = handle_request(
            &mut state,
            Request::CrossEngineQuery {
                file: "src/handler.rs".into(),
                reality_id: None,
            },
        );

        match resp {
            Response::Ok {
                data:
                    ResponseData::CrossEngineResult {
                        file,
                        symbols,
                        callers,
                        calling_files,
                        ..
                    },
            } => {
                assert_eq!(file, "src/handler.rs");
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0]["name"], "handle_request");
                assert_eq!(callers, 1);
                assert_eq!(calling_files, vec!["src/main.rs"]);
            }
            other => panic!("expected CrossEngineResult, got {other:?}"),
        }
    }

    #[test]
    fn test_file_memory_map_basic() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a memory mentioning a file
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Handler architecture".into(),
                content: "Use src/handler.rs as the central dispatcher".into(),
                confidence: Some(0.9),
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let resp = handle_request(
            &mut state,
            Request::FileMemoryMap {
                files: vec!["src/handler.rs".into(), "src/nonexistent.rs".into()],
                reality_id: None,
            },
        );

        match resp {
            Response::Ok {
                data: ResponseData::FileMemoryMapResult { mappings },
            } => {
                let info = mappings
                    .get("src/handler.rs")
                    .expect("should have handler.rs");
                assert!(
                    info.memory_count >= 1,
                    "should find at least 1 memory mentioning handler.rs"
                );
                assert!(info.decision_count >= 1, "should find at least 1 decision");

                let info2 = mappings
                    .get("src/nonexistent.rs")
                    .expect("should have nonexistent.rs");
                assert_eq!(
                    info2.memory_count, 0,
                    "nonexistent file should have 0 memories"
                );
            }
            other => panic!("expected FileMemoryMapResult, got {other:?}"),
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
        let resp = handle_request(
            &mut state,
            Request::CodeSearch {
                query: "handle".into(),
                kind: None,
                limit: None,
                project: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::CodeSearchResult { hits },
            } => {
                assert_eq!(hits.len(), 2, "should find 2 symbols matching 'handle'");
            }
            other => panic!("expected CodeSearchResult, got {other:?}"),
        }

        // Search with kind filter
        let resp2 = handle_request(
            &mut state,
            Request::CodeSearch {
                query: "Daemon".into(),
                kind: Some("class".into()),
                limit: Some(5),
                project: None,
            },
        );
        match resp2 {
            Response::Ok {
                data: ResponseData::CodeSearchResult { hits },
            } => {
                assert_eq!(hits.len(), 1, "should find 1 class matching 'Daemon'");
                assert_eq!(hits[0]["name"], "DaemonState");
            }
            other => panic!("expected CodeSearchResult, got {other:?}"),
        }
    }

    #[test]
    fn p3_4_w1_24_code_search_emits_file_path_key() {
        // W1.3 LOW-5 contract test: the daemon's CodeSearchResult JSON
        // must use the key `file_path` (not `path`). Pre-W1.24 the
        // daemon emitted `path` and the CLI consumer read `file_path`,
        // so every hit's location rendered as `?` in user output —
        // the silent serde_json macro drift trap captured by the
        // `feedback_json_macro_silent_drift` auto-memory.
        //
        // Exercise all three branches in the CodeSearch handler
        // (project-scoped, kind-filter, no-filter) since each builds
        // its own `serde_json::json!({...})` literal — a regression
        // could land in any one of them independently.
        let mut state = DaemonState::new(":memory:").unwrap();

        let sym = forge_core::types::CodeSymbol {
            id: "s1".into(),
            name: "find_target".into(),
            kind: "function".into(),
            file_path: "src/lib.rs".into(),
            line_start: 42,
            line_end: Some(50),
            signature: None,
        };
        crate::db::ops::store_symbol(&state.conn, &sym).unwrap();

        let assert_file_path_key = |hits: &[serde_json::Value], branch: &str| {
            assert!(
                !hits.is_empty(),
                "{branch}: should produce at least one hit"
            );
            for hit in hits {
                assert!(
                    hit.get("file_path").is_some(),
                    "{branch}: hit must carry `file_path` key, got {hit:?}"
                );
                assert!(
                    hit.get("path").is_none(),
                    "{branch}: hit must NOT carry legacy `path` key, got {hit:?}"
                );
                assert_eq!(
                    hit["file_path"], "src/lib.rs",
                    "{branch}: file_path value must match stored row"
                );
            }
        };

        // Branch 1: no project, no kind.
        let resp = handle_request(
            &mut state,
            Request::CodeSearch {
                query: "find".into(),
                kind: None,
                limit: None,
                project: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::CodeSearchResult { hits },
            } => assert_file_path_key(&hits, "no-filter"),
            other => panic!("expected CodeSearchResult, got {other:?}"),
        }

        // Branch 2: kind filter, no project.
        let resp = handle_request(
            &mut state,
            Request::CodeSearch {
                query: "find".into(),
                kind: Some("function".into()),
                limit: None,
                project: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::CodeSearchResult { hits },
            } => assert_file_path_key(&hits, "kind-filter"),
            other => panic!("expected CodeSearchResult, got {other:?}"),
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
        let resp = handle_request(&mut state, Request::ForceIndex { path: None });
        match resp {
            Response::Ok {
                data: ResponseData::IndexComplete { files_indexed, .. },
            } => {
                assert_eq!(files_indexed, 1, "should report 1 file indexed");
            }
            other => panic!("expected IndexComplete, got {other:?}"),
        }

        // Verify import edges were created
        let edge_count: usize = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE edge_type = 'imports'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            edge_count >= 2,
            "should have at least 2 import edges (std::io and crate::db), got {edge_count}"
        );
    }

    #[test]
    fn test_create_organization_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::CreateOrganization {
                name: "TestOrg".into(),
                description: Some("A test".into()),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::OrganizationCreated { id },
            } => assert!(!id.is_empty()),
            other => panic!("expected OrganizationCreated, got {other:?}"),
        }
    }

    #[test]
    fn test_team_send_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let now = forge_core::time::now_iso();
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at, team_type) VALUES ('t1', 'eng', 'default', 'system', 'active', ?1, 'human')",
            rusqlite::params![now],
        ).unwrap();
        crate::sessions::register_session(&state.conn, "s1", "claude-code", None, None, None, None, None)
            .unwrap();
        state
            .conn
            .execute("UPDATE session SET team_id = 't1' WHERE id = 's1'", [])
            .unwrap();

        let resp = handle_request(
            &mut state,
            Request::TeamSend {
                team_name: "eng".into(),
                kind: "notification".into(),
                topic: "test".into(),
                parts: vec![],
                from_session: Some("system".into()),
                recursive: false,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamSent { messages_sent },
            } => assert_eq!(messages_sent, 1),
            other => panic!("expected TeamSent, got {other:?}"),
        }
    }

    #[test]
    fn test_list_organizations_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();
        // Create two additional organizations
        handle_request(
            &mut state,
            Request::CreateOrganization {
                name: "OrgAlpha".into(),
                description: Some("First".into()),
            },
        );
        handle_request(
            &mut state,
            Request::CreateOrganization {
                name: "OrgBeta".into(),
                description: None,
            },
        );

        let resp = handle_request(&mut state, Request::ListOrganizations);
        match resp {
            Response::Ok {
                data: ResponseData::OrganizationList { organizations },
            } => {
                // "default" + OrgAlpha + OrgBeta = at least 3
                assert!(
                    organizations.len() >= 3,
                    "expected at least 3 orgs (including default), got {}",
                    organizations.len()
                );
                let names: Vec<&str> = organizations
                    .iter()
                    .filter_map(|o| o["name"].as_str())
                    .collect();
                assert!(names.contains(&"Default"), "should contain default org");
                assert!(names.contains(&"OrgAlpha"), "should contain OrgAlpha");
                assert!(names.contains(&"OrgBeta"), "should contain OrgBeta");
            }
            other => panic!("expected OrganizationList, got {other:?}"),
        }
    }

    #[test]
    fn test_team_tree_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let now = forge_core::time::now_iso();
        let org_id = "test-org-tree";
        state.conn.execute(
            "INSERT INTO organization (id, name, created_at, updated_at) VALUES (?1, 'TreeTestOrg', ?2, ?3)",
            rusqlite::params![org_id, now, now],
        ).unwrap();
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at, team_type) VALUES ('tt1', 'engineering', ?1, 'system', 'active', ?2, 'human')",
            rusqlite::params![org_id, now],
        ).unwrap();
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, parent_team_id, created_by, status, created_at, team_type) VALUES ('tt2', 'backend', ?1, 'tt1', 'system', 'active', ?2, 'human')",
            rusqlite::params![org_id, now],
        ).unwrap();

        let resp = handle_request(
            &mut state,
            Request::TeamTree {
                organization_id: Some(org_id.to_string()),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamTreeData { tree },
            } => {
                assert_eq!(tree.len(), 1, "should have 1 root node");
                assert_eq!(tree[0]["name"], "engineering");
                let children = tree[0]["children"].as_array().unwrap();
                assert_eq!(children.len(), 1, "engineering should have 1 child");
                assert_eq!(children[0]["name"], "backend");
            }
            other => panic!("expected TeamTreeData, got {other:?}"),
        }
    }

    #[test]
    fn test_team_tree_by_name() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let now = forge_core::time::now_iso();
        let org_id = "test-org-byname";
        state.conn.execute(
            "INSERT INTO organization (id, name, created_at, updated_at) VALUES (?1, 'ByNameOrg', ?2, ?3)",
            rusqlite::params![org_id, now, now],
        ).unwrap();
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at, team_type) VALUES ('bn1', 'platform', ?1, 'system', 'active', ?2, 'human')",
            rusqlite::params![org_id, now],
        ).unwrap();
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, parent_team_id, created_by, status, created_at, team_type) VALUES ('bn2', 'infra', ?1, 'bn1', 'system', 'active', ?2, 'human')",
            rusqlite::params![org_id, now],
        ).unwrap();

        // Pass org NAME instead of ID — should resolve correctly
        let resp = handle_request(
            &mut state,
            Request::TeamTree {
                organization_id: Some("ByNameOrg".to_string()),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamTreeData { tree },
            } => {
                assert_eq!(tree.len(), 1, "should have 1 root node");
                assert_eq!(tree[0]["name"], "platform");
                let children = tree[0]["children"].as_array().unwrap();
                assert_eq!(children.len(), 1, "platform should have 1 child");
                assert_eq!(children[0]["name"], "infra");
            }
            other => panic!("expected TeamTreeData, got {other:?}"),
        }
    }

    #[test]
    fn test_org_from_template_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::CreateOrgFromTemplate {
                template_name: "startup".into(),
                org_name: "TemplateStartup".into(),
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::OrgFromTemplateCreated {
                        org_id,
                        teams_created,
                    },
            } => {
                assert!(!org_id.is_empty(), "org_id should not be empty");
                assert_eq!(teams_created, 12, "startup template creates 12 teams");
            }
            other => panic!("expected OrgFromTemplateCreated, got {other:?}"),
        }
    }

    #[test]
    fn test_org_from_template_unknown() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::CreateOrgFromTemplate {
                template_name: "nonexistent".into(),
                org_name: "BadOrg".into(),
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(
                    message.contains("nonexistent"),
                    "error should mention the unknown template, got: {message}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_team_send_recursive_handler() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let now = forge_core::time::now_iso();
        let org_id = "test-org-recursive";
        state.conn.execute(
            "INSERT INTO organization (id, name, created_at, updated_at) VALUES (?1, 'RecursiveOrg', ?2, ?3)",
            rusqlite::params![org_id, now, now],
        ).unwrap();
        // Parent team
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at, team_type) VALUES ('tp', 'parentteam', ?1, 'system', 'active', ?2, 'human')",
            rusqlite::params![org_id, now],
        ).unwrap();
        // Child team
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, parent_team_id, created_by, status, created_at, team_type) VALUES ('tc', 'childteam', ?1, 'tp', 'system', 'active', ?2, 'human')",
            rusqlite::params![org_id, now],
        ).unwrap();

        // Register sessions and assign to teams
        crate::sessions::register_session(
            &state.conn,
            "s-parent",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        state
            .conn
            .execute(
                "UPDATE session SET team_id = 'tp' WHERE id = 's-parent'",
                [],
            )
            .unwrap();
        crate::sessions::register_session(
            &state.conn,
            "s-child",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        state
            .conn
            .execute("UPDATE session SET team_id = 'tc' WHERE id = 's-child'", [])
            .unwrap();

        // Non-recursive: only parent team session
        let resp = handle_request(
            &mut state,
            Request::TeamSend {
                team_name: "parentteam".into(),
                kind: "notification".into(),
                topic: "test".into(),
                parts: vec![],
                from_session: Some("system".into()),
                recursive: false,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamSent { messages_sent },
            } => {
                assert_eq!(
                    messages_sent, 1,
                    "non-recursive should send to 1 session (parent team only)"
                );
            }
            other => panic!("expected TeamSent, got {other:?}"),
        }

        // Recursive: parent + child team sessions
        let resp = handle_request(
            &mut state,
            Request::TeamSend {
                team_name: "parentteam".into(),
                kind: "notification".into(),
                topic: "test-recursive".into(),
                parts: vec![],
                from_session: Some("system".into()),
                recursive: true,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamSent { messages_sent },
            } => {
                assert_eq!(
                    messages_sent, 2,
                    "recursive should send to 2 sessions (parent + child)"
                );
            }
            other => panic!("expected TeamSent, got {other:?}"),
        }
    }

    #[test]
    fn test_team_send_empty_team() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let now = forge_core::time::now_iso();
        // Create team with no sessions
        state.conn.execute(
            "INSERT INTO team (id, name, organization_id, created_by, status, created_at, team_type) VALUES ('tempty', 'emptyteam', 'default', 'system', 'active', ?1, 'human')",
            rusqlite::params![now],
        ).unwrap();

        let resp = handle_request(
            &mut state,
            Request::TeamSend {
                team_name: "emptyteam".into(),
                kind: "notification".into(),
                topic: "hello".into(),
                parts: vec![],
                from_session: Some("system".into()),
                recursive: false,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamSent { messages_sent },
            } => {
                assert_eq!(messages_sent, 0, "empty team should have 0 messages sent");
            }
            other => panic!("expected TeamSent, got {other:?}"),
        }
    }

    #[test]
    fn test_remember_emits_healing_candidate_event() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store first decision
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use PostgreSQL for database storage".into(),
                content: "Relational DB for main data".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let mut rx = state.events.subscribe();

        // Store similar decision — should trigger healing_candidate event
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Use MySQL for database storage".into(),
                content: "Relational DB for main data".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Check for healing_candidate event
        let mut found = false;
        while let Ok(event) = rx.try_recv() {
            if event.event == "healing_candidate" {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "should emit healing_candidate event when similar memory exists"
        );
    }

    // ── Temporal Filter (--since) Tests ──

    #[test]
    fn test_recall_with_since_filters_old_memories() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a memory (will get current timestamp)
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Old decision from last month".into(),
                content: "Ancient stuff about architecture".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Backdate it to March 2026
        state.conn.execute(
            "UPDATE memory SET created_at = '2026-03-01 12:00:00' WHERE title LIKE '%Old decision%'",
            [],
        ).unwrap();

        // Store a recent memory (gets current timestamp)
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Decision,
                title: "Recent decision from today".into(),
                content: "Fresh stuff about architecture".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Recall WITH since filter — should only get the recent memory
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "architecture".into(),
                memory_type: Some(MemoryType::Decision),
                project: None,
                limit: Some(10),
                layer: None,
                since: Some("2026-04-01 00:00:00".into()),
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { results, count },
            } => {
                assert!(count > 0, "should find at least the recent memory");
                for r in &results {
                    assert!(
                        r.memory.created_at.as_str() >= "2026-04-01",
                        "all results should be after since date, got: {}",
                        r.memory.created_at
                    );
                }
                assert!(
                    !results
                        .iter()
                        .any(|r| r.memory.title.contains("Old decision")),
                    "old memory should be filtered out by since"
                );
            }
            other => panic!("expected Memories response, got {other:?}"),
        }

        // Recall WITHOUT since filter — should get both
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "architecture".into(),
                memory_type: Some(MemoryType::Decision),
                project: None,
                limit: Some(10),
                layer: None,
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert!(
                    count >= 2,
                    "without since filter, should find both memories, got {count}"
                );
            }
            other => panic!("expected Memories response, got {other:?}"),
        }
    }

    #[test]
    fn test_recall_since_none_returns_all() {
        let mut state = DaemonState::new(":memory:").unwrap();

        handle_request(
            &mut state,
            Request::Remember {
                memory_type: MemoryType::Lesson,
                title: "Lesson about testing".into(),
                content: "Always test temporal filters".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // since: None should not filter anything
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "testing".into(),
                memory_type: None,
                project: None,
                limit: Some(10),
                layer: None,
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert!(count > 0, "since=None should not filter anything");
            }
            other => panic!("expected Memories response, got {other:?}"),
        }
    }

    #[test]
    fn test_list_perceptions_with_offset() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Store 5 perceptions
        for i in 0..5 {
            let perception = forge_core::types::manas::Perception {
                id: format!("p-off-{i}"),
                kind: forge_core::types::manas::PerceptionKind::Error,
                data: format!("error {i}"),
                severity: forge_core::types::manas::Severity::Warning,
                project: Some("forge".into()),
                created_at: "2026-04-06 12:00:00".into(),
                expires_at: None,
                consumed: false,
            };
            handle_request(&mut state, Request::StorePerception { perception });
        }

        // List with offset=2, limit=2 — should skip first 2, return next 2
        let resp = handle_request(
            &mut state,
            Request::ListPerceptions {
                project: None,
                limit: Some(2),
                offset: Some(2),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::PerceptionList { perceptions, count },
            } => {
                assert_eq!(count, 2, "should return exactly 2 perceptions after offset");
                assert_eq!(perceptions.len(), 2);
            }
            other => panic!("expected PerceptionList, got {other:?}"),
        }

        // List with offset=4, limit=10 — should return only 1 (5 total, skip 4)
        let resp = handle_request(
            &mut state,
            Request::ListPerceptions {
                project: None,
                limit: Some(10),
                offset: Some(4),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::PerceptionList { perceptions, count },
            } => {
                assert_eq!(count, 1, "should return 1 perception (offset 4 of 5)");
                assert_eq!(perceptions.len(), 1);
            }
            other => panic!("expected PerceptionList, got {other:?}"),
        }

        // List with offset=0, no change from default behavior
        let resp = handle_request(
            &mut state,
            Request::ListPerceptions {
                project: None,
                limit: Some(10),
                offset: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::PerceptionList {
                        perceptions: _,
                        count,
                    },
            } => {
                assert_eq!(count, 5, "offset=None should return all 5");
            }
            other => panic!("expected PerceptionList, got {other:?}"),
        }
    }

    #[test]
    fn test_team_status_by_id() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Create organization first (teams require org)
        let resp = handle_request(
            &mut state,
            Request::CreateOrganization {
                name: "test-org".into(),
                description: None,
            },
        );
        let org_id = match resp {
            Response::Ok {
                data: ResponseData::OrganizationCreated { id },
            } => id,
            other => panic!("expected OrganizationCreated, got {other:?}"),
        };

        // Create a team via handler
        let resp = handle_request(
            &mut state,
            Request::CreateTeam {
                name: "engineering".into(),
                team_type: Some("agent".into()),
                purpose: Some("Build stuff".into()),
                organization_id: Some(org_id.clone()),
            parent_team_id: None,
            },
        );
        let team_id = match resp {
            Response::Ok {
                data: ResponseData::TeamCreated { id, .. },
            } => id,
            other => panic!("expected TeamCreated, got {other:?}"),
        };

        // Look up by team_id with a wrong team_name — should resolve from ID
        let resp = handle_request(
            &mut state,
            Request::TeamStatus {
                team_name: "nonexistent".into(),
                team_id: Some(team_id.clone()),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamStatusData { team },
            } => {
                let name = team.get("name").and_then(|v| v.as_str()).unwrap_or("");
                assert_eq!(name, "engineering", "should resolve name from team_id");
            }
            other => panic!("expected TeamStatusData, got {other:?}"),
        }

        // Look up by name only (team_id=None) — normal behavior
        let resp = handle_request(
            &mut state,
            Request::TeamStatus {
                team_name: "engineering".into(),
                team_id: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamStatusData { team },
            } => {
                let name = team.get("name").and_then(|v| v.as_str()).unwrap_or("");
                assert_eq!(name, "engineering", "should find by name directly");
            }
            other => panic!("expected TeamStatusData, got {other:?}"),
        }

        // Look up with invalid team_id — should fall back to team_name
        let resp = handle_request(
            &mut state,
            Request::TeamStatus {
                team_name: "engineering".into(),
                team_id: Some("nonexistent-id".into()),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamStatusData { team },
            } => {
                let name = team.get("name").and_then(|v| v.as_str()).unwrap_or("");
                assert_eq!(
                    name, "engineering",
                    "invalid team_id should fall back to team_name"
                );
            }
            other => panic!("expected TeamStatusData, got {other:?}"),
        }
    }

    #[test]
    fn test_remember_decision_auto_write_skipped_in_project_mode() {
        // Default config has workspace.mode = "project", so auto-write should NOT happen.
        // This test verifies the remember handler works correctly without workspace side effects.
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();

        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "Use PostgreSQL for storage".into(),
                content: "PostgreSQL chosen for ACID compliance".into(),
                confidence: Some(0.9),
                tags: Some(vec!["database".into()]),
                project: Some("forge".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Memory should be stored successfully
        match &resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => {
                assert!(!id.is_empty(), "stored decision should have a non-empty id");
            }
            other => panic!("expected Stored, got {other:?}"),
        }

        // Verify no workspace_decision_written event was emitted
        // (because default mode is "project")
        let mut found_workspace_event = false;
        while let Ok(evt) = rx.try_recv() {
            if evt.event == "workspace_decision_written" {
                found_workspace_event = true;
            }
        }
        assert!(
            !found_workspace_event,
            "workspace_decision_written event should NOT be emitted in project mode"
        );
    }

    #[test]
    fn test_remember_decision_auto_write_with_team_mode() {
        // Test that when workspace mode is "team", write_decision is called.
        // We simulate this by directly calling write_decision after a remember,
        // since we can't easily override load_config() in a unit test.
        let mut state = DaemonState::new(":memory:").unwrap();

        // Store a decision
        let resp = handle_request(
            &mut state,
            Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "Use gRPC for internal APIs".into(),
                content: "gRPC chosen for type safety and performance".into(),
                confidence: Some(0.85),
                tags: Some(vec!["api".into(), "protocol".into()]),
                project: Some("forge".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        let id = match &resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => id.clone(),
            other => panic!("expected Stored, got {other:?}"),
        };

        // Simulate what the auto-write code path does in team mode
        let tmp = tempfile::tempdir().unwrap();
        let ws_root = tmp.path().to_path_buf();
        let result = crate::workspace::write_decision(
            &ws_root,
            "backend",
            "Use gRPC for internal APIs",
            "gRPC chosen for type safety and performance",
            0.85,
            &["api".to_string(), "protocol".to_string()],
            &id,
        );

        assert!(result.is_ok(), "write_decision should succeed");
        let path = result.unwrap();
        assert!(path.exists(), "decision file should exist on disk");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(
            content.contains("Use gRPC for internal APIs"),
            "decision file should contain title"
        );
        assert!(
            content.contains(&id),
            "decision file should contain memory id"
        );
    }

    #[test]
    fn test_recall_sends_touch_via_writer_tx() {
        // Wave 1: Verify that Recall handler sends touch IDs through the writer channel.
        // We set up a real mpsc channel and verify the TouchMemories command is received.
        let mut state = DaemonState::new(":memory:").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<super::super::writer::WriteCommand>(10);
        state.writer_tx = Some(tx);

        // Store a memory
        handle_request(
            &mut state,
            Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "test touch decision".into(),
                content: "testing that recall updates access_count".into(),
                confidence: None,
                tags: None,
                project: Some("test".into()),
                metadata: None,
                valence: None,
                intensity: None,
            },
        );

        // Recall it — this should send a TouchMemories command through the channel
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "test touch decision".into(),
                memory_type: None,
                project: Some("test".into()),
                limit: Some(5),
                layer: None,
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );

        // Verify recall returned results
        match resp {
            Response::Ok {
                data: ResponseData::Memories { count, .. },
            } => {
                assert!(count > 0, "should find the decision");
            }
            other => panic!("expected Memories, got {other:?}"),
        }

        // Verify a TouchMemories command was sent through the channel
        match rx.try_recv() {
            Ok(super::super::writer::WriteCommand::TouchMemories { ids, boost_amount }) => {
                assert!(!ids.is_empty(), "touch IDs should not be empty");
                assert!(
                    (boost_amount - 0.3).abs() < f64::EPSILON,
                    "boost should be 0.3"
                );
            }
            Ok(other) => panic!(
                "expected TouchMemories, got {:?}",
                std::mem::discriminant(&other)
            ),
            Err(e) => panic!("expected TouchMemories command in channel, got error: {e}"),
        }
    }

    #[test]
    fn test_cleanup_memory_handler() {
        // Wave 3: Verify CleanupMemory handler deletes garbage and normalizes projects.
        let mut state = DaemonState::new(":memory:").unwrap();

        // Insert a garbage memory (quality 0.0, access 0, older than 7 days)
        state.conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, quality_score, access_count)
             VALUES ('garbage-1', 'decision', 'garbage', 'bad content', 0.5, 'active', 'test', '[]', datetime('now', '-30 days'), datetime('now', '-30 days'), 0.0, 0)",
            [],
        ).unwrap();

        // Insert a good memory
        state.conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, quality_score, access_count)
             VALUES ('good-1', 'decision', 'good decision', 'quality content', 0.9, 'active', 'test', '[]', datetime('now'), datetime('now'), 0.8, 5)",
            [],
        ).unwrap();

        let resp = handle_request(&mut state, Request::CleanupMemory);
        match resp {
            Response::Ok {
                data:
                    ResponseData::CleanupMemoryResult {
                        garbage_deleted, ..
                    },
            } => {
                assert_eq!(garbage_deleted, 1, "should delete 1 garbage memory");
            }
            other => panic!("expected CleanupMemoryResult, got {other:?}"),
        }

        // Verify garbage is soft-deleted, good one is untouched
        let deleted: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE id = 'garbage-1' AND deleted_at IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(deleted, 1, "garbage memory should be soft-deleted");

        let active: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE id = 'good-1' AND deleted_at IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(active, 1, "good memory should remain active");
    }

    #[test]
    fn test_send_touch_with_none_writer_tx() {
        // Wave 1: send_touch with None writer_tx should be a no-op (no panic)
        send_touch(&None, vec!["id1".to_string(), "id2".to_string()], 0.3);
        // Should not panic — that's the test
    }

    #[test]
    fn test_send_touch_with_empty_ids() {
        // Wave 1: send_touch with empty IDs should be a no-op
        let (tx, _rx) = tokio::sync::mpsc::channel(10);
        send_touch(&Some(tx), vec![], 0.3);
        // Should not send anything — channel should be empty
    }

    // ── Fix 3: Context effectiveness via writer channel ──

    #[test]
    fn test_compile_context_sends_record_injection_via_writer() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<super::super::writer::WriteCommand>(10);
        state.writer_tx = Some(tx);

        // Register a session so session_id validation passes
        handle_request(
            &mut state,
            Request::RegisterSession {
                id: "test-session-1".into(),
                agent: "claude-code".into(),
                project: Some("forge".into()),
                cwd: None,
                capabilities: None,
                current_task: None,
            role: None,
            },
        );

        // CompileContext with session_id should send RecordInjection through the writer
        let resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: Some("claude-code".into()),
                project: Some("forge".into()),
                static_only: None,
                excluded_layers: None,
                session_id: Some("test-session-1".into()),
                focus: None,
                cwd: None,
                dry_run: None,
            },
        );

        // Verify the response is successful
        match &resp {
            Response::Ok {
                data: ResponseData::CompiledContext { chars, .. },
            } => {
                assert!(*chars > 0, "compiled context should have content");
            }
            other => panic!("expected CompiledContext, got {other:?}"),
        }

        // Drain TouchMemories first (CompileContext sends touch before injection)
        let mut found_injection = false;
        while let Ok(cmd) = rx.try_recv() {
            if let super::super::writer::WriteCommand::RecordInjection {
                session_id,
                hook_event,
                context_type,
                chars_injected,
                ..
            } = cmd
            {
                assert_eq!(session_id, "test-session-1");
                assert_eq!(hook_event, "SessionStart");
                assert_eq!(context_type, "full_context");
                assert!(chars_injected > 0, "chars_injected should be > 0");
                found_injection = true;
            }
        }
        assert!(
            found_injection,
            "CompileContext with session_id should send RecordInjection via writer channel"
        );
    }

    #[test]
    fn test_star_topology_blocks_non_orchestrator() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Create a team
        let resp = handle_request(
            &mut state,
            Request::CreateTeam {
                name: "star-team".into(),
                team_type: Some("agent".into()),
                purpose: Some("test star topology".into()),
                organization_id: None,
            parent_team_id: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamCreated { .. },
            } => {}
            other => panic!("expected TeamCreated, got {other:?}"),
        }

        // Set topology to star
        state
            .conn
            .execute(
                "UPDATE team SET topology = 'star' WHERE name = 'star-team'",
                [],
            )
            .unwrap();

        // Register orchestrator and member sessions
        crate::sessions::register_session(
            &state.conn,
            "orch-1",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        crate::sessions::register_session(
            &state.conn,
            "member-1",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        crate::sessions::register_session(
            &state.conn,
            "member-2",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Assign sessions to team
        let team_id: String = state
            .conn
            .query_row("SELECT id FROM team WHERE name = 'star-team'", [], |r| {
                r.get(0)
            })
            .unwrap();
        state
            .conn
            .execute(
                "UPDATE session SET team_id = ?1 WHERE id = 'orch-1'",
                rusqlite::params![team_id],
            )
            .unwrap();
        state
            .conn
            .execute(
                "UPDATE session SET team_id = ?1 WHERE id = 'member-1'",
                rusqlite::params![team_id],
            )
            .unwrap();
        state
            .conn
            .execute(
                "UPDATE session SET team_id = ?1 WHERE id = 'member-2'",
                rusqlite::params![team_id],
            )
            .unwrap();

        // Set orchestrator
        let resp = handle_request(
            &mut state,
            Request::SetTeamOrchestrator {
                team_name: "star-team".into(),
                session_id: "orch-1".into(),
            },
        );
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok, got {other:?}"),
        }

        // Non-orchestrator member should be BLOCKED from sending
        let resp = handle_request(
            &mut state,
            Request::TeamSend {
                team_name: "star-team".into(),
                kind: "notification".into(),
                topic: "test".into(),
                parts: vec![],
                from_session: Some("member-1".into()),
                recursive: false,
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(
                    message.contains("star topology"),
                    "error should mention star topology: {message}"
                );
                assert!(
                    message.contains("orch-1"),
                    "error should mention orchestrator: {message}"
                );
            }
            other => panic!("expected Error for non-orchestrator in star topology, got {other:?}"),
        }

        // Orchestrator SHOULD be able to send
        let resp = handle_request(
            &mut state,
            Request::TeamSend {
                team_name: "star-team".into(),
                kind: "notification".into(),
                topic: "test".into(),
                parts: vec![],
                from_session: Some("orch-1".into()),
                recursive: false,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamSent { messages_sent },
            } => {
                assert!(
                    messages_sent > 0,
                    "orchestrator should be able to send in star topology"
                );
            }
            other => panic!("expected TeamSent for orchestrator, got {other:?}"),
        }

        // "system" should also be allowed (graceful degradation)
        let resp = handle_request(
            &mut state,
            Request::TeamSend {
                team_name: "star-team".into(),
                kind: "notification".into(),
                topic: "sys-test".into(),
                parts: vec![],
                from_session: Some("system".into()),
                recursive: false,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::TeamSent { .. },
            } => {}
            other => panic!("expected TeamSent for system sender, got {other:?}"),
        }
    }

    // ── Skills Handler Tests ──

    #[test]
    fn test_skills_list_empty() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::SkillsList {
                category: None,
                search: None,
                limit: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::SkillsList { skills, count },
            } => {
                assert_eq!(count, 0, "should return 0 skills on fresh DB");
                assert!(skills.is_empty());
            }
            other => panic!("expected SkillsList, got {other:?}"),
        }
    }

    #[test]
    fn test_skills_install_nonexistent() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Installing a non-existent skill should return an error
        let resp = handle_request(
            &mut state,
            Request::SkillsInstall {
                name: "nonexistent-skill".into(),
                project: "forge".into(),
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(
                    message.contains("not found"),
                    "should report skill not found: {message}"
                );
            }
            other => panic!("expected Error for nonexistent skill, got {other:?}"),
        }
    }

    #[test]
    fn test_skills_list_with_search() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // List with search filter should return empty (no skills match)
        let resp = handle_request(
            &mut state,
            Request::SkillsList {
                category: None,
                search: Some("nonexistent".into()),
                limit: Some(10),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::SkillsList { count, .. },
            } => {
                assert_eq!(count, 0, "search for nonexistent should return 0");
            }
            other => panic!("expected SkillsList, got {other:?}"),
        }
    }

    #[test]
    fn test_skills_info_not_found() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::SkillsInfo {
                name: "nonexistent-skill".into(),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::SkillInfo { skill },
            } => {
                assert!(skill.is_none(), "nonexistent skill should return None");
            }
            other => panic!("expected SkillInfo, got {other:?}"),
        }
    }

    #[test]
    fn test_cleanup_sessions_prefix_preserves_others() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Register sessions with different prefixes
        for id in &["temp-1", "temp-2", "keep-1"] {
            handle_request(
                &mut state,
                Request::RegisterSession {
                    id: id.to_string(),
                    agent: "claude-code".into(),
                    project: Some("forge".into()),
                    cwd: None,
                    capabilities: None,
                    current_task: None,
                role: None,
                },
            );
        }

        // Cleanup only "temp-" prefix sessions
        let resp = handle_request(
            &mut state,
            Request::CleanupSessions {
                prefix: Some("temp".into()),
                older_than_secs: None,
                prune_ended: false,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::SessionsCleaned { ended },
            } => {
                assert_eq!(ended, 2, "should end 2 temp- sessions, got {ended}");
            }
            other => panic!("expected SessionsCleaned, got {other:?}"),
        }

        // "keep-1" should still be active
        let resp = handle_request(
            &mut state,
            Request::Sessions {
                active_only: Some(true),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Sessions { count, .. },
            } => {
                assert_eq!(count, 1, "keep-1 should survive prefix cleanup");
            }
            other => panic!("expected Sessions, got {other:?}"),
        }
    }

    // ── Contradiction Handler Tests ──

    #[test]
    fn test_list_contradictions_empty() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::ListContradictions {
                status: None,
                limit: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Contradictions { count, .. },
            } => {
                assert_eq!(count, 0, "empty DB should have 0 contradictions");
            }
            other => panic!("expected Contradictions, got {other:?}"),
        }
    }

    #[test]
    fn test_list_contradictions_with_data() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Create two memories with opposite valence + shared tags → contradiction
        let mut m1 = Memory::new(
            MemoryType::Decision,
            "Use JWT for auth",
            "JWT is stateless and scalable",
        );
        m1.valence = "positive".into();
        m1.intensity = 0.8;
        m1.tags = vec!["auth".into(), "security".into(), "api".into()];
        ops::remember(&state.conn, &m1).unwrap();

        let mut m2 = Memory::new(
            MemoryType::Decision,
            "Avoid JWT for auth",
            "JWT tokens can't be revoked",
        );
        m2.valence = "negative".into();
        m2.intensity = 0.8;
        m2.tags = vec!["auth".into(), "security".into(), "session".into()];
        ops::remember(&state.conn, &m2).unwrap();

        // Run contradiction detection
        let found = ops::detect_contradictions(&state.conn).unwrap();
        assert!(
            found >= 1,
            "should detect at least 1 contradiction, got {found}"
        );

        // List contradictions
        let resp = handle_request(
            &mut state,
            Request::ListContradictions {
                status: None,
                limit: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::Contradictions {
                        contradictions,
                        count,
                    },
            } => {
                assert!(
                    count >= 1,
                    "should list at least 1 contradiction, got {count}"
                );
                let c = &contradictions[0];
                assert!(!c.memory_a_title.is_empty());
                assert!(!c.memory_b_title.is_empty());
                assert!(c.shared_tags >= 2, "should have 2+ shared tags");
                assert!(!c.resolved, "should be unresolved initially");
            }
            other => panic!("expected Contradictions, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_contradiction() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Create contradicting memories
        let mut m1 = Memory::new(MemoryType::Decision, "Use REST API", "REST is standard");
        m1.valence = "positive".into();
        m1.intensity = 0.9;
        m1.tags = vec!["api".into(), "architecture".into()];
        ops::remember(&state.conn, &m1).unwrap();

        let mut m2 = Memory::new(MemoryType::Decision, "Avoid REST API", "GraphQL is better");
        m2.valence = "negative".into();
        m2.intensity = 0.9;
        m2.tags = vec!["api".into(), "architecture".into()];
        ops::remember(&state.conn, &m2).unwrap();

        // Detect contradiction
        ops::detect_contradictions(&state.conn).unwrap();

        // Get the contradiction ID
        let resp = handle_request(
            &mut state,
            Request::ListContradictions {
                status: None,
                limit: None,
            },
        );
        let contradiction_id = match resp {
            Response::Ok {
                data: ResponseData::Contradictions { contradictions, .. },
            } => {
                assert!(!contradictions.is_empty());
                contradictions[0].id.clone()
            }
            other => panic!("expected Contradictions, got {other:?}"),
        };

        // Resolve: memory A wins
        let resp = handle_request(
            &mut state,
            Request::ResolveContradiction {
                contradiction_id: contradiction_id.clone(),
                resolution: "a".into(),
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::ContradictionResolved { resolution, .. },
            } => {
                assert_eq!(resolution, "a");
            }
            other => panic!("expected ContradictionResolved, got {other:?}"),
        }

        // Verify: contradiction should now be resolved
        let resp = handle_request(
            &mut state,
            Request::ListContradictions {
                status: Some("unresolved".into()),
                limit: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Contradictions { count, .. },
            } => {
                assert_eq!(count, 0, "should have 0 unresolved after resolution");
            }
            other => panic!("expected Contradictions, got {other:?}"),
        }
    }

    #[test]
    fn test_resolve_contradiction_not_found() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let resp = handle_request(
            &mut state,
            Request::ResolveContradiction {
                contradiction_id: "nonexistent".into(),
                resolution: "a".into(),
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(message.contains("not found"));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn test_context_refresh_includes_message_summaries() {
        let mut state = DaemonState::new(":memory:").unwrap();
        // Register sender and recipient sessions
        crate::sessions::register_session(
            &state.conn,
            "sender1",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        crate::sessions::register_session(
            &state.conn,
            "recv1",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Send a message from sender1 to recv1
        let parts = r#"[{"kind":"text","text":"Hello from sender, this is a test message"}]"#;
        crate::sessions::send_message(
            &state.conn,
            "sender1",
            "recv1",
            "notification",
            "greet",
            parts,
            None,
            None,
            None,
        )
        .unwrap();

        // ContextRefresh for recv1 should include the message summary
        let resp = handle_request(
            &mut state,
            Request::ContextRefresh {
                session_id: "recv1".into(),
                since: None,
            },
        );
        match resp {
            Response::Ok {
                data:
                    ResponseData::ContextDelta {
                        messages_pending,
                        message_summaries,
                        ..
                    },
            } => {
                assert_eq!(messages_pending, 1);
                assert_eq!(message_summaries.len(), 1);
                assert!(
                    message_summaries[0].contains("[from:sender1]"),
                    "should contain sender: {}",
                    message_summaries[0]
                );
                assert!(
                    message_summaries[0].contains("(greet)"),
                    "should contain topic: {}",
                    message_summaries[0]
                );
                assert!(
                    message_summaries[0].contains("Hello from sender"),
                    "should contain text: {}",
                    message_summaries[0]
                );
            }
            other => panic!("expected ContextDelta, got {other:?}"),
        }
    }

    #[test]
    fn test_send_message_rate_limit() {
        let mut state = DaemonState::new(":memory:").unwrap();
        crate::sessions::register_session(
            &state.conn,
            "flood_sender",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        crate::sessions::register_session(
            &state.conn,
            "flood_recv",
            "claude-code",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Send 50 messages (should all succeed)
        for i in 0..50 {
            let parts = vec![forge_core::protocol::request::MessagePart {
                kind: "text".into(),
                text: Some(format!("msg {i}")),
                path: None,
                data: None,
                memory_id: None,
            }];
            let resp = handle_request(
                &mut state,
                Request::SessionSend {
                    to: "flood_recv".into(),
                    kind: "notification".into(),
                    topic: "test".into(),
                    parts,
                    project: None,
                    timeout_secs: None,
                    meeting_id: None,
                    from_session: None,
                },
            );
            match &resp {
                Response::Ok {
                    data: ResponseData::MessageSent { .. },
                } => {}
                other => panic!("message {i} should succeed, got {other:?}"),
            }
        }

        // 51st message should be rate-limited
        let parts = vec![forge_core::protocol::request::MessagePart {
            kind: "text".into(),
            text: Some("should fail".into()),
            path: None,
            data: None,
            memory_id: None,
        }];
        let resp = handle_request(
            &mut state,
            Request::SessionSend {
                to: "flood_recv".into(),
                kind: "notification".into(),
                topic: "test".into(),
                parts,
                project: None,
                timeout_secs: None,
                meeting_id: None,
                from_session: None,
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(
                    message.contains("rate limit"),
                    "should contain rate limit message: {message}"
                );
            }
            other => panic!("expected rate limit Error, got {other:?}"),
        }
    }

    #[test]
    fn test_send_message_queue_depth_limit() {
        let mut state = DaemonState::new(":memory:").unwrap();
        crate::sessions::register_session(
            &state.conn,
            "qdl_sender",
            "test",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        crate::sessions::register_session(&state.conn, "qdl_recv", "test", None, None, None, None, None)
            .unwrap();

        // Insert 100 pending messages directly to avoid rate limit
        for _i in 0..100 {
            let id = ulid::Ulid::new().to_string();
            state.conn.execute(
                "INSERT INTO session_message (id, from_session, to_session, kind, topic, parts, status, project, created_at)
                 VALUES (?1, 'other_sender', 'qdl_recv', 'notification', 'test', '[]', 'pending', NULL, datetime('now'))",
                rusqlite::params![id],
            ).unwrap();
        }

        // 101st message via handler should be rejected
        let parts = vec![forge_core::protocol::request::MessagePart {
            kind: "text".into(),
            text: Some("should fail".into()),
            path: None,
            data: None,
            memory_id: None,
        }];
        let resp = handle_request(
            &mut state,
            Request::SessionSend {
                to: "qdl_recv".into(),
                kind: "notification".into(),
                topic: "test".into(),
                parts,
                project: None,
                timeout_secs: None,
                meeting_id: None,
                from_session: None,
            },
        );
        match resp {
            Response::Error { message } => {
                assert!(
                    message.contains("queue full"),
                    "should contain queue full message: {message}"
                );
            }
            other => panic!("expected queue full Error, got {other:?}"),
        }
    }

    #[test]
    fn test_compile_session_kpis() {
        let state = DaemonState::new(":memory:").unwrap();

        // Register a session
        crate::sessions::register_session(
            &state.conn,
            "kpi-test",
            "claude-code",
            Some("forge"),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // Simulate context injection
        let _ = crate::db::effectiveness::record_injection_with_size(
            &state.conn,
            "kpi-test",
            "UserPromptSubmit",
            "delta",
            "test injection",
            150,
        );
        let _ = crate::db::effectiveness::record_injection_with_size(
            &state.conn,
            "kpi-test",
            "PostEdit",
            "proactive",
            "blast radius",
            200,
        );

        // Send a message TO this session
        crate::sessions::send_message(
            &state.conn,
            "other-session",
            "kpi-test",
            "notification",
            "test",
            "[]",
            Some("forge"),
            None,
            None,
        )
        .unwrap();

        // Compile KPIs
        let kpis = crate::sessions::compile_session_kpis(&state.conn, "kpi-test");
        assert!(kpis.is_some(), "KPIs should be Some");
        let kpis = kpis.unwrap();
        assert_eq!(kpis.context_injections, 2, "should have 2 injections");
        assert_eq!(
            kpis.context_chars_injected, 350,
            "should have 350 chars injected"
        );
        assert_eq!(
            kpis.a2a_messages_received, 1,
            "should have 1 message received"
        );
        assert_eq!(kpis.a2a_messages_sent, 0, "should have 0 messages sent");
        assert_eq!(kpis.hooks_fired.len(), 2, "should have 2 hook types");
        assert_eq!(*kpis.hooks_fired.get("UserPromptSubmit").unwrap_or(&0), 1);
        assert_eq!(*kpis.hooks_fired.get("PostEdit").unwrap_or(&0), 1);
    }

    #[test]
    fn test_raw_documents_list_filters_by_source_through_handler() {
        // Exercise the handler arm end-to-end: seed 3 raw documents directly
        // via the db layer (bypassing the embedder-dependent RawIngest path),
        // then dispatch Request::RawDocumentsList through handle_request and
        // verify the response shape.
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        for (id, source, text) in [
            ("doc_a", "forge-persist", "alpha"),
            ("doc_b", "forge-persist", "beta"),
            ("doc_c", "claude-code", "gamma"),
        ] {
            crate::db::raw::insert_document(
                &state.conn,
                &crate::db::raw::RawDocument {
                    id: id.to_string(),
                    project: None,
                    session_id: None,
                    source: source.to_string(),
                    text: text.to_string(),
                    timestamp: "2026-04-15T00:00:00Z".to_string(),
                    metadata_json: "{}".to_string(),
                },
            )
            .unwrap();
        }

        let response = handle_request(
            &mut state,
            Request::RawDocumentsList {
                source: "forge-persist".to_string(),
                limit: Some(100),
            },
        );

        match response {
            Response::Ok {
                data: ResponseData::RawDocumentsList { documents },
            } => {
                assert_eq!(documents.len(), 2);
                let ids: Vec<&str> = documents.iter().map(|d| d.id.as_str()).collect();
                assert!(ids.contains(&"doc_a"), "expected doc_a in {ids:?}");
                assert!(ids.contains(&"doc_b"), "expected doc_b in {ids:?}");
                for doc in &documents {
                    assert_eq!(doc.source, "forge-persist");
                }
                let doc_a = documents.iter().find(|d| d.id == "doc_a").unwrap();
                assert_eq!(doc_a.text, "alpha");
                assert_eq!(doc_a.timestamp, "2026-04-15T00:00:00Z");
            }
            other => panic!("expected RawDocumentsList response, got {other:?}"),
        }
    }

    #[test]
    fn test_version_returns_build_metadata() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let resp = handle_request(&mut state, Request::Version);
        match resp {
            Response::Ok {
                data:
                    ResponseData::Version {
                        version,
                        build_profile,
                        target_triple,
                        rustc_version,
                        ..
                    },
            } => {
                assert!(!version.is_empty(), "version must not be empty");
                assert!(
                    build_profile == "release" || build_profile == "debug",
                    "build_profile must be 'release' or 'debug', got: {build_profile}"
                );
                assert!(!target_triple.is_empty(), "target_triple must not be empty");
                assert!(!rustc_version.is_empty(), "rustc_version must not be empty");
            }
            other => panic!("expected Version response, got: {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_creates_new_memory_with_opposite_valence() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Arrange: store a preference with positive valence
        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "tabs over spaces",
            "prefer tabs",
        );
        pref.id = "01PREF".to_string();
        pref.valence = "positive".to_string();
        pref.intensity = 0.7;
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        // Act: flip it
        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: Some("team switched to spaces".into()),
            },
        );

        // Assert: response carries the flipped data
        let new_id = match resp {
            forge_core::protocol::Response::Ok { data } => match data {
                forge_core::protocol::ResponseData::PreferenceFlipped {
                    old_id,
                    new_id,
                    new_valence,
                    new_intensity,
                    flipped_at,
                } => {
                    assert_eq!(old_id, "01PREF");
                    assert_ne!(new_id, "01PREF");
                    assert_eq!(new_valence, "negative");
                    assert!((new_intensity - 0.8).abs() < 1e-9);
                    assert_eq!(flipped_at.len(), 19); // "YYYY-MM-DD HH:MM:SS"
                    new_id
                }
                other => panic!("expected PreferenceFlipped, got {other:?}"),
            },
            forge_core::protocol::Response::Error { message } => {
                panic!("flip failed: {message}")
            }
        };

        // Assert: old memory marked superseded with valence_flipped_at set
        let (status, superseded_by, flipped_at): (String, Option<String>, Option<String>) = state
            .conn
            .query_row(
                "SELECT status, superseded_by, valence_flipped_at FROM memory WHERE id = ?1",
                rusqlite::params!["01PREF"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "superseded");
        assert_eq!(superseded_by, Some(new_id.clone()));
        assert!(flipped_at.is_some());

        // Assert: new memory has opposite valence and annotated content
        let new = crate::db::ops::fetch_memory_by_id(&state.conn, &new_id)
            .unwrap()
            .unwrap();
        assert_eq!(new.valence, "negative");
        assert!((new.intensity - 0.8).abs() < 1e-9);
        assert!(new
            .content
            .starts_with("[flipped from positive to negative at "));
        assert!(new.content.contains("prefer tabs"));
        assert_eq!(new.status, forge_core::types::memory::MemoryStatus::Active);
        assert_eq!(new.alternatives, Vec::<String>::new());
        assert_eq!(new.participants, Vec::<String>::new());

        // Assert: supersedes edge from new to old
        let edge_count: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'supersedes'",
                rusqlite::params![&new_id, "01PREF"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(edge_count, 1);
    }

    #[test]
    fn test_flip_preference_rejects_missing_memory() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "does-not-exist".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("memory_id not found"),
                    "expected 'memory_id not found', got: {message}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_non_preference_type() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut decision = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Decision,
            "foo",
            "bar",
        );
        decision.id = "01DEC".to_string();
        crate::db::ops::remember(&state.conn, &decision).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01DEC".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("memory_type must be preference"),
                    "got: {message}"
                );
                // T6's lowercase format: should contain 'decision'
                assert!(message.contains("decision"), "got: {message}");
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_already_superseded() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Insert a row already in superseded status via raw SQL.
        state
            .conn
            .execute(
                "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity)
                 VALUES (?1, 'preference', 'x', 'y', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.5)",
                rusqlite::params!["01SUP"],
            )
            .unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01SUP".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(message.contains("already superseded"), "got: {message}");
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_invalid_valence() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "x",
            "y",
        );
        pref.id = "01PREF".to_string();
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "happy".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("new_valence must be positive | negative | neutral"),
                    "got: {message}"
                );
                assert!(message.contains("happy"), "got: {message}");
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_out_of_range_intensity() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "x",
            "y",
        );
        pref.id = "01PREF".to_string();
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "negative".into(),
                new_intensity: 1.5,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("new_intensity must be finite in [0.0, 1.0]"),
                    "got: {message}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_noop_same_valence() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "tabs",
            "prefer tabs",
        );
        pref.id = "01PREF".to_string();
        pref.valence = "positive".to_string();
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "positive".into(), // same as old
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("no-op flip"),
                    "expected 'no-op flip' message, got: {message}"
                );
                assert!(message.contains("positive"), "got: {message}");
            }
            other => panic!("expected error, got {other:?}"),
        }

        // Verify old memory was not mutated by the rejected call.
        let (status, superseded_by, valence_flipped_at): (String, Option<String>, Option<String>) =
            state
                .conn
                .query_row(
                    "SELECT status, superseded_by, valence_flipped_at FROM memory WHERE id = ?1",
                    rusqlite::params!["01PREF"],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
        assert_eq!(status, "active", "no-op flip must not change status");
        assert_eq!(superseded_by, None, "no-op flip must not set superseded_by");
        assert_eq!(
            valence_flipped_at, None,
            "no-op flip must not set valence_flipped_at"
        );
    }

    #[test]
    fn test_flip_preference_emits_preference_flipped_event() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Arrange: store a preference
        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "tabs over spaces",
            "prefer tabs",
        );
        pref.id = "01PREF".to_string();
        pref.valence = "positive".to_string();
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        // Subscribe to events BEFORE issuing the request.
        let mut rx = state.events.subscribe();

        // Act: flip
        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: Some("team switched".into()),
            },
        );
        // Sanity: ensure the flip succeeded; otherwise the event won't have fired.
        assert!(
            matches!(resp, forge_core::protocol::Response::Ok { .. }),
            "flip must succeed"
        );

        // Assert: preference_flipped event emitted with expected payload.
        let evt = rx.try_recv().expect("no event received");
        assert_eq!(evt.event, "preference_flipped");
        assert_eq!(evt.data["old_id"], "01PREF");
        assert_eq!(evt.data["new_valence"], "negative");
        assert_eq!(
            evt.data["new_intensity"]
                .as_f64()
                .expect("new_intensity must be number"),
            0.8,
            "new_intensity in event must match request value"
        );
        assert_eq!(evt.data["reason"], "team switched");
        let flipped_at_str = evt.data["flipped_at"]
            .as_str()
            .expect("flipped_at must be string");
        assert_eq!(
            flipped_at_str.len(),
            19,
            "flipped_at must be YYYY-MM-DD HH:MM:SS (19 chars), got: {flipped_at_str}"
        );
        // new_id should be present and non-empty
        let new_id_val = evt.data["new_id"]
            .as_str()
            .expect("new_id should be string");
        assert!(!new_id_val.is_empty());
        assert_ne!(new_id_val, "01PREF");
    }

    // ── Phase 2A-4b T9: ReaffirmPreference handler tests ────────────────────

    #[test]
    fn reaffirm_preference_happy_path_updates_reaffirmed_at() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-happy".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        // Backdate reaffirmed_at to a known value so we can verify it changed.
        state
            .conn
            .execute(
                "UPDATE memory SET reaffirmed_at = '2026-01-01 00:00:00' WHERE id = ?1",
                rusqlite::params![&pref_id],
            )
            .unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: pref_id.clone(),
            },
        );

        match resp {
            forge_core::protocol::Response::Ok {
                data:
                    forge_core::protocol::ResponseData::PreferenceReaffirmed {
                        memory_id,
                        reaffirmed_at,
                    },
            } => {
                assert_eq!(memory_id, pref_id);
                assert_ne!(
                    reaffirmed_at, "2026-01-01 00:00:00",
                    "reaffirmed_at should be updated"
                );
            }
            other => panic!("expected PreferenceReaffirmed, got: {other:?}"),
        }

        // Verify DB was updated.
        let actual: String = state
            .conn
            .query_row(
                "SELECT reaffirmed_at FROM memory WHERE id = ?1",
                rusqlite::params![&pref_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(actual, "2026-01-01 00:00:00");
    }

    #[test]
    fn reaffirm_preference_rejects_non_preference() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let lesson = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Lesson,
            "topic-reaffirm-wrong-type".to_string(),
            "content".to_string(),
        );
        let lesson_id = lesson.id.clone();
        crate::db::ops::remember_raw(&state.conn, &lesson).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: lesson_id,
            },
        );

        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("must be preference"),
                    "error should mention preference requirement: {message}"
                );
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_rejects_missing_memory() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: "nonexistent-01J".to_string(),
            },
        );

        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("not found") || message.contains("does not exist"),
                    "error should mention not-found: {message}"
                );
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_rejects_superseded_memory() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-superseded".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        state
            .conn
            .execute(
                "UPDATE memory SET status = 'superseded' WHERE id = ?1",
                rusqlite::params![&pref_id],
            )
            .unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: pref_id },
        );

        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("superseded") || message.contains("not active"),
                    "error should mention superseded/inactive: {message}"
                );
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_rejects_flipped_memory() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-flipped".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        state
            .conn
            .execute(
                "UPDATE memory SET valence_flipped_at = '2026-04-18 12:00:00', status = 'superseded' WHERE id = ?1",
                rusqlite::params![&pref_id],
            )
            .unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: pref_id },
        );

        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(!message.is_empty(), "error message should not be empty");
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_rejects_cross_org_access() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Seed a preference belonging to orgA by setting organization_id directly.
        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-cross-org".to_string(),
            "content".to_string(),
        );
        pref.organization_id = Some("orgA".to_string());
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        // Register a session in orgB — caller_org will resolve to orgB.
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, project, started_at, status, organization_id) \
                 VALUES ('sess-orgB', 'test-agent', 'proj', '2026-04-19 00:00:00', 'active', 'orgB')",
                [],
            )
            .unwrap();

        // Link the memory to the orgB session (so get_session_org_id returns orgB).
        state
            .conn
            .execute(
                "UPDATE memory SET session_id = 'sess-orgB' WHERE id = ?1",
                rusqlite::params![&pref_id],
            )
            .unwrap();

        // Restore organization_id = orgA (session update must not have overwritten it).
        state
            .conn
            .execute(
                "UPDATE memory SET organization_id = 'orgA' WHERE id = ?1",
                rusqlite::params![&pref_id],
            )
            .unwrap();

        // Now call ReaffirmPreference. The handler derives caller_org via the memory's
        // session_id ('sess-orgB' → orgB), but the memory belongs to orgA.
        // The UPDATE WHERE clause COALESCE(organization_id,'default') = 'orgB' won't match
        // organization_id = 'orgA', so it returns no rows.
        // The diagnostic SELECT is also org-scoped to orgB → also no rows.
        // Result must be "not found" — no cross-org existence disclosure.
        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: pref_id.clone(),
            },
        );

        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("not found"),
                    "cross-org access should surface as 'not found', got: {message}"
                );
            }
            other => panic!("expected Error, got: {other:?}"),
        }

        // Verify the pref is UNCHANGED — no reaffirmed_at written.
        let reaffirmed_at: Option<String> = state
            .conn
            .query_row(
                "SELECT reaffirmed_at FROM memory WHERE id = ?1",
                rusqlite::params![pref_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            reaffirmed_at.is_none(),
            "cross-org call must not modify pref (reaffirmed_at should be NULL)"
        );
    }

    // ── Phase 2A-4b T10: extended validation (9 new tests) ──────────────────

    #[test]
    fn reaffirm_preference_rejects_faded_memory() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-faded".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        state
            .conn
            .execute(
                "UPDATE memory SET status = 'faded' WHERE id = ?1",
                rusqlite::params![&pref_id],
            )
            .unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: pref_id },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("not active") || message.contains("faded"),
                    "expected not-active/faded message, got: {message}"
                );
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_rejects_conflict_status() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-conflict".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        state
            .conn
            .execute(
                "UPDATE memory SET status = 'conflict' WHERE id = ?1",
                rusqlite::params![&pref_id],
            )
            .unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: pref_id },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("not active") || message.contains("conflict"),
                    "expected not-active/conflict message, got: {message}"
                );
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_rejects_empty_memory_id() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: String::new(),
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("not found")
                        || message.contains("empty")
                        || message.contains("invalid"),
                    "expected not-found/empty/invalid message, got: {message}"
                );
            }
            other => panic!("expected Error, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_succeeds_with_null_organization_id() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // organization_id is None by default in Memory::new — resolves to 'default' bucket.
        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-null-org".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: pref_id.clone(),
            },
        );
        match resp {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PreferenceReaffirmed { memory_id, .. },
            } => {
                assert_eq!(memory_id, pref_id);
            }
            other => panic!("expected PreferenceReaffirmed, got: {other:?}"),
        }
    }

    #[test]
    fn reaffirm_preference_same_memory_twice_advances_timestamp() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-twice".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        // First reaffirm.
        let ts1 = match handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: pref_id.clone(),
            },
        ) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PreferenceReaffirmed { reaffirmed_at, .. },
            } => reaffirmed_at,
            other => panic!("expected PreferenceReaffirmed on first call, got: {other:?}"),
        };

        // Sleep 1.1 s so now_iso() yields a later second-resolution timestamp.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // Second reaffirm.
        let ts2 = match handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: pref_id },
        ) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PreferenceReaffirmed { reaffirmed_at, .. },
            } => reaffirmed_at,
            other => panic!("expected PreferenceReaffirmed on second call, got: {other:?}"),
        };

        assert!(
            ts2 > ts1,
            "second reaffirm timestamp ({ts2}) should advance past first ({ts1})"
        );
    }

    #[test]
    fn reaffirm_then_flip_preference_flips_succeed() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-then-flip".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        // Reaffirm first.
        let resp1 = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: pref_id.clone(),
            },
        );
        assert!(
            matches!(resp1, forge_core::protocol::Response::Ok { .. }),
            "reaffirm should succeed, got: {resp1:?}"
        );

        // Then flip — should also succeed on the now-reaffirmed pref.
        let resp2 = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: pref_id,
                new_valence: "negative".to_string(),
                new_intensity: 0.8,
                reason: Some("changed mind".to_string()),
            },
        );
        assert!(
            matches!(
                resp2,
                forge_core::protocol::Response::Ok {
                    data: forge_core::protocol::ResponseData::PreferenceFlipped { .. }
                }
            ),
            "flip after reaffirm should succeed, got: {resp2:?}"
        );
    }

    #[test]
    fn flip_then_reaffirm_on_new_memory_succeeds() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-flip-then-reaffirm".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        // Flip — produces a new active pref.
        let new_id = match handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: pref_id.clone(),
                new_valence: "negative".to_string(),
                new_intensity: 0.8,
                reason: None,
            },
        ) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PreferenceFlipped { new_id, .. },
            } => new_id,
            other => panic!("expected PreferenceFlipped, got: {other:?}"),
        };

        // Old pref is superseded/flipped — reaffirm must fail.
        let resp2 = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: pref_id },
        );
        assert!(
            matches!(resp2, forge_core::protocol::Response::Error { .. }),
            "reaffirm of old (flipped) pref should fail, got: {resp2:?}"
        );

        // New pref is active and non-flipped — reaffirm must succeed.
        let resp3 = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: new_id },
        );
        assert!(
            matches!(resp3, forge_core::protocol::Response::Ok { .. }),
            "reaffirm of new (active) pref should succeed, got: {resp3:?}"
        );
    }

    #[test]
    fn reaffirm_preference_works_on_decayed_memory() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // confidence very low (below hard-fade threshold 0.1) but status still 'active'.
        // Per T7, preferences are hard-fade exempt — status remains 'active'.
        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-decayed".to_string(),
            "content".to_string(),
        );
        pref.confidence = 0.005;
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference { memory_id: pref_id },
        );
        assert!(
            matches!(
                resp,
                forge_core::protocol::Response::Ok {
                    data: forge_core::protocol::ResponseData::PreferenceReaffirmed { .. }
                }
            ),
            "reaffirm on decayed-but-active pref should succeed, got: {resp:?}"
        );
    }

    #[test]
    fn reaffirm_preference_uppercase_id_treated_literally() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-case".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        // ULIDs are already uppercase — uppercase lookup should match exactly.
        // If pref_id is already uppercase, we expect Ok; if somehow lowercase,
        // we expect Error. Either way the response must be deterministic (no panic).
        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: pref_id.to_uppercase(),
            },
        );
        assert!(
            matches!(
                resp,
                forge_core::protocol::Response::Ok { .. }
                    | forge_core::protocol::Response::Error { .. }
            ),
            "response must be Ok or Error, not something else: {resp:?}"
        );
    }

    // ── Phase 2A-4b T11: ReaffirmPreference event emission ──────────────────

    #[test]
    fn reaffirm_preference_emits_preference_reaffirmed_event() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::memory::Memory::new(
            forge_core::types::MemoryType::Preference,
            "topic-reaffirm-event".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        crate::db::ops::remember_raw(&state.conn, &pref).unwrap();

        // Subscribe BEFORE issuing the request.
        let mut rx = state.events.subscribe();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: pref_id.clone(),
            },
        );
        assert!(
            matches!(resp, forge_core::protocol::Response::Ok { .. }),
            "reaffirm should succeed, got: {resp:?}"
        );

        let event = rx.try_recv().expect("event should have been emitted");
        assert_eq!(event.event, "preference_reaffirmed");
        assert_eq!(
            event.data["memory_id"],
            serde_json::json!(pref_id),
            "memory_id in payload must match"
        );
        assert!(
            event.data["reaffirmed_at"].is_string(),
            "reaffirmed_at should be a string timestamp, got: {:?}",
            event.data["reaffirmed_at"]
        );
    }

    #[test]
    fn reaffirm_preference_emits_no_event_on_error() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Subscribe BEFORE issuing the request.
        let mut rx = state.events.subscribe();

        // Nonexistent memory → Error
        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ReaffirmPreference {
                memory_id: "does-not-exist".to_string(),
            },
        );
        assert!(
            matches!(resp, forge_core::protocol::Response::Error { .. }),
            "expected Error for nonexistent memory, got: {resp:?}"
        );

        let attempt = rx.try_recv();
        assert!(
            attempt.is_err(),
            "no event should be emitted on error path; got: {attempt:?}"
        );
    }

    // ── Phase 2A-4c2 T6: ProbePhase handler tests ────────────────────────────

    #[cfg(feature = "bench")]
    #[test]
    fn probe_phase_returns_correct_index_for_infer_skills() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "infer_skills_from_behavior".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data:
                    forge_core::protocol::ResponseData::PhaseProbe {
                        executed_at_phase_index,
                        ..
                    },
            } => {
                assert_eq!(executed_at_phase_index, 23);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[cfg(feature = "bench")]
    #[test]
    fn probe_phase_executed_after_contains_extract_protocols() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "infer_skills_from_behavior".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::PhaseProbe { executed_after, .. },
            } => {
                assert!(
                    executed_after.contains(&"extract_protocols".to_string()),
                    "executed_after must contain Phase 17 (extract_protocols); got {executed_after:?}"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    #[cfg(feature = "bench")]
    #[test]
    fn probe_phase_unknown_phase_errors() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "not_a_real_phase".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.starts_with("unknown_phase: "),
                    "expected unknown_phase: prefix, got {message}"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    #[cfg(feature = "bench")]
    #[test]
    fn probe_phase_phase_17_executed_at_index_17() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ProbePhase {
            phase_name: "extract_protocols".to_string(),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data:
                    forge_core::protocol::ResponseData::PhaseProbe {
                        executed_at_phase_index,
                        executed_after,
                    },
            } => {
                assert_eq!(executed_at_phase_index, 17);
                assert_eq!(
                    executed_after,
                    Vec::<String>::new(),
                    "Phase 17 is the first in PHASE_ORDER — nothing before it"
                );
            }
            other => panic!("got {other:?}"),
        }
    }

    // ── Phase 2A-4b T12: ComputeRecencyFactor handler tests ────────────────────

    /// Bit-exact parity: handler's returned `factor` == ops::recency_factor
    /// called with the same now_secs. Achieved by re-deriving factor from
    /// the `days_since_anchor` the handler returned, which was computed from
    /// the same now_secs snapshot inside the handler.
    #[cfg(feature = "bench")]
    #[test]
    fn compute_recency_factor_bit_exact_matches_ops_recency_factor_for_preference() {
        use crate::db::ops::{parse_timestamp_to_epoch, recency_factor, remember_raw};
        use forge_core::types::memory::MemoryType;

        let mut state = DaemonState::new(":memory:").unwrap();

        // Seed a preference backdated 30 days so the factor is meaningfully < 1.0.
        let now_secs = ops::current_epoch_secs();
        let created_30d_ago = forge_core::time::epoch_to_iso((now_secs - 30.0 * 86400.0) as u64);

        let mut pref = forge_core::types::Memory::new(
            MemoryType::Preference,
            "topic-recency-parity-pref".to_string(),
            "content".to_string(),
        );
        pref.confidence = 0.9;
        let pref_id = pref.id.clone();
        remember_raw(&state.conn, &pref).unwrap();
        // Backdate created_at so the age is ~30 days.
        state
            .conn
            .execute(
                "UPDATE memory SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![created_30d_ago, pref_id],
            )
            .unwrap();

        // Call via handler → get factor + anchor + days_since_anchor.
        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ComputeRecencyFactor {
                memory_id: pref_id.clone(),
            },
        );
        let (value_via_handler, days_via_handler, anchor_via_handler) = match resp {
            Response::Ok {
                data:
                    ResponseData::RecencyFactor {
                        factor,
                        days_since_anchor,
                        anchor,
                        ..
                    },
            } => (factor, days_since_anchor, anchor),
            other => panic!("expected RecencyFactor, got: {other:?}"),
        };

        // Verify factor > 0 and < 1 (backdated 30 days, not fresh).
        assert!(
            value_via_handler > 0.0 && value_via_handler < 1.0,
            "backdated pref factor should be in (0,1), got: {value_via_handler}"
        );

        // Bit-exact parity: reconstruct factor from the handler's own days_since_anchor.
        // This avoids a second clock call and proves the formula is applied consistently.
        let half_life = crate::config::load_config()
            .recall
            .validated()
            .preference_half_life_days;
        let ground_truth_factor = 2_f64.powf(-days_via_handler / half_life.max(1.0));
        assert_eq!(
            value_via_handler.to_bits(),
            ground_truth_factor.to_bits(),
            "handler factor {value_via_handler} must bit-equal 2^(-days/half_life) {ground_truth_factor}"
        );

        // Also verify anchor parses correctly (not corrupt).
        let anchor_secs = parse_timestamp_to_epoch(&anchor_via_handler);
        assert!(
            anchor_secs.is_some(),
            "handler anchor should be parseable; got: {anchor_via_handler}"
        );

        // Confirm ops::recency_factor with anchor-derived days yields same bits.
        let anchor_secs = anchor_secs.unwrap();
        let fetched = ops::fetch_memory_by_id(&state.conn, &pref_id)
            .unwrap()
            .unwrap();
        // Use now such that days == days_via_handler exactly.
        let reconstructed_now = anchor_secs + days_via_handler * 86400.0;
        let via_ops = recency_factor(&fetched, half_life, reconstructed_now);
        assert_eq!(
            value_via_handler.to_bits(),
            via_ops.to_bits(),
            "handler value {value_via_handler} must bit-equal ops::recency_factor {via_ops}"
        );
    }

    /// Fresh preference (created right now) should return factor ~1.0.
    #[cfg(feature = "bench")]
    #[test]
    fn compute_recency_factor_returns_1_0_for_fresh_preference() {
        use forge_core::types::memory::MemoryType;

        let mut state = DaemonState::new(":memory:").unwrap();

        let pref = forge_core::types::Memory::new(
            MemoryType::Preference,
            "topic-recency-parity-fresh".to_string(),
            "content".to_string(),
        );
        let pref_id = pref.id.clone();
        ops::remember_raw(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ComputeRecencyFactor { memory_id: pref_id },
        );
        let value = match resp {
            Response::Ok {
                data: ResponseData::RecencyFactor { factor, .. },
            } => factor,
            other => panic!("expected RecencyFactor, got: {other:?}"),
        };

        // Fresh memory → age ~0 → factor ~1.0. Allow <1 s clock drift.
        assert!(
            value > 0.99 && value <= 1.0,
            "fresh pref factor should be ~1.0, got: {value}"
        );
    }

    /// Missing memory_id → Error response.
    #[cfg(feature = "bench")]
    #[test]
    fn compute_recency_factor_rejects_missing_memory() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::ComputeRecencyFactor {
                memory_id: "does-not-exist".to_string(),
            },
        );
        assert!(
            matches!(resp, Response::Error { .. }),
            "missing id should return Error, got: {resp:?}"
        );
    }

    // -----------------------------------------------------------------------
    // T13 — <preferences> section in CompileContext
    // -----------------------------------------------------------------------

    #[test]
    fn compile_context_renders_preferences_section() {
        let mut state = DaemonState::new(":memory:").unwrap();

        let mut p = forge_core::types::memory::Memory::new(
            MemoryType::Preference,
            "prefer-vim".to_string(),
            "content".to_string(),
        );
        p.valence = "positive".to_string();
        p.intensity = 0.8;
        crate::db::ops::remember_raw(&state.conn, &p).unwrap();

        let resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: None,
                dry_run: None,
            },
        );

        match resp {
            Response::Ok {
                data: ResponseData::CompiledContext { context, .. },
            } => {
                assert!(
                    context.contains("<preferences>"),
                    "context should contain <preferences> section: {context}"
                );
                assert!(
                    context.contains("prefer-vim"),
                    "preferences section should contain the preference title: {context}"
                );
                assert!(
                    context.contains("valence=\"positive\""),
                    "preferences entry should have valence attribute: {context}"
                );
            }
            other => panic!("expected CompiledContext, got: {other:?}"),
        }
    }

    #[test]
    fn compile_context_omits_preferences_section_when_no_prefs() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // No preferences seeded — only a decision so context is non-trivial
        let mut d = forge_core::types::memory::Memory::new(
            MemoryType::Decision,
            "use-rust".to_string(),
            "we ship in rust".to_string(),
        );
        d.confidence = 0.9;
        crate::db::ops::remember_raw(&state.conn, &d).unwrap();

        let resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
                cwd: None,
                dry_run: None,
            },
        );

        match resp {
            Response::Ok {
                data: ResponseData::CompiledContext { context, .. },
            } => {
                assert!(
                    !context.contains("<preferences>"),
                    "context must NOT contain <preferences> when no prefs exist: {context}"
                );
            }
            other => panic!("expected CompiledContext, got: {other:?}"),
        }
    }

    // ── Phase 2A-4c1 T5: RecordToolUse handler tests ────────────────────────

    #[test]
    fn record_tool_use_happy_path_returns_id_and_created_at() {
        let mut state = DaemonState::new(":memory:").unwrap();

        // Seed a session with organization_id = 'acme' (NOT 'default').
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('SESS1', 'claude-code', '2026-04-19 10:00:00', 'active', 'acme')",
                [],
            )
            .unwrap();

        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "SESS1".to_string(),
            agent: "claude-code".to_string(),
            tool_name: "Read".to_string(),
            tool_args: serde_json::json!({"file_path": "/tmp/a"}),
            tool_result_summary: "ok".to_string(),
            success: true,
            user_correction_flag: false,
        };
        let resp = handle_request(&mut state, req);
        match resp {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallRecorded { id, created_at },
            } => {
                assert_eq!(id.len(), 26, "ULID is 26 chars");
                assert!(!created_at.is_empty(), "created_at present");

                // Verify row persisted with the TARGET SESSION's org ('acme'), not the caller default.
                let org: String = state
                    .conn
                    .query_row(
                        "SELECT organization_id FROM session_tool_call WHERE id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    org, "acme",
                    "organization_id is sourced from target session, not default"
                );
            }
            other => panic!("expected ToolCallRecorded, got {other:?}"),
        }
    }

    #[test]
    fn record_tool_use_persists_all_fields_roundtrip_via_direct_select() {
        let mut state = DaemonState::new(":memory:").unwrap();

        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'claude-code', '2026-04-19 10:00:00', 'active', 'acme')",
                [],
            )
            .unwrap();

        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "claude-code".to_string(),
            tool_name: "Bash".to_string(),
            tool_args: serde_json::json!({"cmd": "ls"}),
            tool_result_summary: "ok".to_string(),
            success: false,
            user_correction_flag: true,
        };
        let _ = handle_request(&mut state, req);

        let (agent, tool, args, summary, success, correction, org): (
            String,
            String,
            String,
            String,
            i64,
            i64,
            String,
        ) = state
            .conn
            .query_row(
                "SELECT agent, tool_name, tool_args, tool_result_summary, success,
                        user_correction_flag, organization_id
                 FROM session_tool_call LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(agent, "claude-code");
        assert_eq!(tool, "Bash");
        assert_eq!(args, r#"{"cmd":"ls"}"#);
        assert_eq!(summary, "ok");
        assert_eq!(success, 0);
        assert_eq!(correction, 1);
        assert_eq!(org, "acme");
    }

    // ── Phase 2A-4c1 T6: RecordToolUse validation tests ─────────────────────

    #[test]
    fn record_tool_use_rejects_empty_tool_name() {
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "a".to_string(),
            tool_name: "".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Error { message } => {
                assert_eq!(message, "empty_field: tool_name")
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn record_tool_use_rejects_whitespace_only_tool_name() {
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "a".to_string(),
            tool_name: "   \t  ".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "empty_field: tool_name"
        ));
    }

    #[test]
    fn record_tool_use_rejects_empty_agent() {
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "empty_field: agent"
        ));
    }

    #[test]
    fn record_tool_use_rejects_control_character_in_session_id() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "abc\0xyz".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "invalid_field: session_id: control_character"
        ));
    }

    #[test]
    fn record_tool_use_rejects_tool_args_over_64kb() {
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
        let big: String = "A".repeat(70_000);
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({"x": big}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "payload_too_large: tool_args: 65536"
        ));
    }

    #[test]
    fn record_tool_use_rejects_tool_result_summary_over_64kb() {
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: "B".repeat(70_000),
            success: true,
            user_correction_flag: false,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "payload_too_large: tool_result_summary: 65536"
        ));
    }

    #[test]
    fn record_tool_use_accepts_unicode_in_tool_name_and_agent() {
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'claude-code', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "αβγ-😀".to_string(),
            tool_name: "Чтение".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        let resp = crate::server::handler::handle_request(&mut state, req);
        assert!(
            matches!(resp, forge_core::protocol::Response::Ok { .. }),
            "unicode strings without control chars must be accepted, got {resp:?}"
        );
    }

    #[test]
    fn record_tool_use_rejects_session_deleted_between_validate_and_execute() {
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
        state
            .conn
            .execute("DELETE FROM session WHERE id = 'S'", [])
            .unwrap();

        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        let resp = crate::server::handler::handle_request(&mut state, req);
        assert!(
            matches!(
                resp,
                forge_core::protocol::Response::Error { ref message }
                    if message.starts_with("unknown_session: ")
            ),
            "expected unknown_session error, got {resp:?}"
        );
        // Atomic INSERT-SELECT proves no orphan.
        let count: i64 = state
            .conn
            .query_row("SELECT COUNT(*) FROM session_tool_call", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, 0,
            "no row should be inserted when session is missing"
        );
    }

    #[test]
    fn record_tool_use_rejects_unknown_session() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "NONEXISTENT".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Error { message } => {
                assert!(message.starts_with("unknown_session: "), "got {message}")
            }
            other => panic!("got {other:?}"),
        }
    }

    // ── Phase 2A-4c1 T7: event emission tests ────────────────────────────────

    #[test]
    fn record_tool_use_emits_tool_use_recorded_event_only_after_insert_succeeds() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'claude-code', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();

        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "claude-code".to_string(),
            tool_name: "Read".to_string(),
            tool_args: serde_json::json!({"file_path": "/tmp/a"}),
            tool_result_summary: "ok".to_string(),
            success: true,
            user_correction_flag: false,
        };
        let _ = crate::server::handler::handle_request(&mut state, req);

        let event = rx.try_recv().expect("event must be emitted");
        assert_eq!(event.event, "tool_use_recorded");
        let data = &event.data;
        assert!(data.get("id").is_some());
        assert_eq!(data.get("session_id").and_then(|v| v.as_str()), Some("S"));
        assert_eq!(
            data.get("agent").and_then(|v| v.as_str()),
            Some("claude-code")
        );
        assert_eq!(data.get("tool_name").and_then(|v| v.as_str()), Some("Read"));
        assert_eq!(data.get("success").and_then(|v| v.as_bool()), Some(true));
        assert!(data.get("created_at").and_then(|v| v.as_str()).is_some());
        assert!(
            data.get("tool_args").is_none(),
            "tool_args MUST NOT be in event"
        );
        assert!(
            data.get("tool_result_summary").is_none(),
            "summary MUST NOT be in event"
        );
        assert!(
            data.get("user_correction_flag").is_none(),
            "correction_flag MUST NOT be in event"
        );
    }

    #[test]
    fn record_tool_use_does_not_emit_event_on_validation_error() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "".to_string(), // invalid — triggers empty_field: agent
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        let _ = crate::server::handler::handle_request(&mut state, req);
        assert!(
            rx.try_recv().is_err(),
            "no event should be emitted on validation error"
        );
    }

    #[test]
    fn record_tool_use_does_not_emit_event_on_unknown_session() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let mut rx = state.events.subscribe();
        let req = forge_core::protocol::Request::RecordToolUse {
            session_id: "NONEXISTENT".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        };
        let _ = crate::server::handler::handle_request(&mut state, req);
        assert!(
            rx.try_recv().is_err(),
            "no event should be emitted when session is unknown"
        );
    }

    // ── Phase 2A-4c1 T8: ListToolCalls happy-path tests ──────────────────────

    fn seed_session_s(state: &DaemonState) {
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn list_tool_calls_happy_path_returns_newest_first() {
        let mut state = DaemonState::new(":memory:").unwrap();
        seed_session_s(&state);
        for (i, id) in ["01A", "01B", "01C"].iter().enumerate() {
            state
                .conn
                .execute(
                    &format!(
                        "INSERT INTO session_tool_call VALUES ('{id}', 'S', 'a', 'T', '{{}}', '', \
                         1, 0, 'default', '2026-04-19 12:00:0{i}')"
                    ),
                    [],
                )
                .unwrap();
        }
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: None,
            limit: None,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls },
            } => {
                let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
                assert_eq!(ids, vec!["01C", "01B", "01A"]);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn list_tool_calls_defaults_limit_to_50_when_none() {
        let mut state = DaemonState::new(":memory:").unwrap();
        seed_session_s(&state);
        for i in 0..60 {
            state
                .conn
                .execute(
                    &format!(
                        "INSERT INTO session_tool_call VALUES ('ID{i:03}', 'S', 'a', 'T', '{{}}', \
                         '', 1, 0, 'default', '2026-04-19 12:00:00')"
                    ),
                    [],
                )
                .unwrap();
        }
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: None,
            limit: None,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls },
            } => assert_eq!(calls.len(), 50),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn list_tool_calls_treats_limit_zero_as_default_50() {
        let mut state = DaemonState::new(":memory:").unwrap();
        seed_session_s(&state);
        for i in 0..60 {
            state
                .conn
                .execute(
                    &format!(
                        "INSERT INTO session_tool_call VALUES ('ID{i:03}', 'S', 'a', 'T', '{{}}', \
                         '', 1, 0, 'default', '2026-04-19 12:00:00')"
                    ),
                    [],
                )
                .unwrap();
        }
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: None,
            limit: Some(0),
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls },
            } => assert_eq!(calls.len(), 50, "limit=0 treated as default 50"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn list_tool_calls_agent_filter_narrows_result() {
        let mut state = DaemonState::new(":memory:").unwrap();
        seed_session_s(&state);
        state
            .conn
            .execute(
                "INSERT INTO session_tool_call VALUES ('A', 'S', 'alice', 'T', '{}', '', 1, 0, \
                 'default', '2026-04-19 12:00:00')",
                [],
            )
            .unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session_tool_call VALUES ('B', 'S', 'bob', 'T', '{}', '', 1, 0, \
                 'default', '2026-04-19 12:00:00')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: Some("alice".to_string()),
            limit: None,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls },
            } => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].agent, "alice");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn list_tool_calls_handler_tiebreaks_identical_created_at_by_id_desc() {
        let mut state = DaemonState::new(":memory:").unwrap();
        seed_session_s(&state);
        for id in ["01A", "01B", "01C"] {
            state
                .conn
                .execute(
                    &format!(
                        "INSERT INTO session_tool_call VALUES ('{id}', 'S', 'a', 'T', '{{}}', '', \
                         1, 0, 'default', '2026-04-19 12:00:00')"
                    ),
                    [],
                )
                .unwrap();
        }
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: None,
            limit: None,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls },
            } => {
                let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
                assert_eq!(ids, vec!["01C", "01B", "01A"], "tiebreak by id DESC");
            }
            other => panic!("got {other:?}"),
        }
    }

    // ── Phase 2A-4c1 T9: ListToolCalls validation + target-session-org ───────

    #[test]
    fn list_tool_calls_rejects_limit_over_500() {
        let mut state = DaemonState::new(":memory:").unwrap();
        seed_session_s(&state);
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: None,
            limit: Some(1000),
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "limit_too_large: requested 1000, max 500"
        ));
    }

    #[test]
    fn list_tool_calls_rejects_unknown_session() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "NONEXISTENT".to_string(),
            agent: None,
            limit: None,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message.starts_with("unknown_session: ")
        ));
    }

    #[test]
    fn list_tool_calls_rejects_control_character_in_session_id() {
        let mut state = DaemonState::new(":memory:").unwrap();
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "abc\0xyz".to_string(),
            agent: None,
            limit: None,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "invalid_field: session_id: control_character"
        ));
    }

    #[test]
    fn list_tool_calls_rejects_control_character_in_agent_filter() {
        let mut state = DaemonState::new(":memory:").unwrap();
        seed_session_s(&state);
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: Some("bad\0agent".to_string()),
            limit: None,
        };
        assert!(matches!(
            crate::server::handler::handle_request(&mut state, req),
            forge_core::protocol::Response::Error { ref message }
                if message == "invalid_field: agent: control_character"
        ));
    }

    #[test]
    fn list_tool_calls_returns_only_target_session_org_rows() {
        // The row with organization_id = 'other_org' is injected directly via
        // raw SQL. The normal RecordToolUse write path atomically copies org
        // from the session row (spec §5.2 atomic INSERT-SELECT), making this
        // state unreachable via the API. This test pins the
        // `WHERE organization_id = ?` filter in ops::list_tool_calls SQL —
        // without it, a bug that removed the org filter would not be caught
        // by the session-scope tests below.
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'acme')",
                [],
            )
            .unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session_tool_call VALUES ('A', 'S', 'a', 'T', '{}', '', 1, 0, 'acme', \
                 '2026-04-19 12:00:00')",
                [],
            )
            .unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session_tool_call VALUES ('B', 'S', 'a', 'T', '{}', '', 1, 0, \
                 'other_org', '2026-04-19 12:00:00')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: None,
            limit: None,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls },
            } => {
                let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
                assert_eq!(ids, vec!["A"], "only target-session-org rows surface");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn list_tool_calls_session_id_scope_excludes_sibling_sessions_within_same_org() {
        // Both sessions are in 'acme', so the `WHERE organization_id = ?`
        // filter matches both rows — the `AND session_id = ?` predicate is
        // what excludes SB's row. This test pins the session_id scoping,
        // NOT org isolation (that's covered by
        // `list_tool_calls_returns_only_target_session_org_rows` above).
        let mut state = DaemonState::new(":memory:").unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('SA', 'a', '2026-04-19 10:00:00', 'active', 'acme')",
                [],
            )
            .unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session (id, agent, started_at, status, organization_id)
                 VALUES ('SB', 'a', '2026-04-19 10:00:00', 'active', 'acme')",
                [],
            )
            .unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session_tool_call VALUES ('A', 'SA', 'a', 'T', '{}', '', 1, 0, \
                 'acme', '2026-04-19 12:00:00')",
                [],
            )
            .unwrap();
        state
            .conn
            .execute(
                "INSERT INTO session_tool_call VALUES ('B', 'SB', 'a', 'T', '{}', '', 1, 0, \
                 'acme', '2026-04-19 12:00:00')",
                [],
            )
            .unwrap();
        let req = forge_core::protocol::Request::ListToolCalls {
            session_id: "SA".to_string(),
            agent: None,
            limit: None,
        };
        match crate::server::handler::handle_request(&mut state, req) {
            forge_core::protocol::Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls },
            } => {
                let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
                assert_eq!(ids, vec!["A"], "listing session SA must not leak SB's rows");
            }
            other => panic!("got {other:?}"),
        }
    }
}
