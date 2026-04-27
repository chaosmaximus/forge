use std::sync::Arc;

use forge_core::protocol::{Request, Response, ResponseData};
use tokio::sync::{mpsc, oneshot};

use super::supervisor::BackgroundTaskSupervisor;

/// Context for audit logging of write operations.
///
/// Attached to HTTP write requests when auth is enabled, or with defaults
/// for Unix socket writes. The WriterActor inserts an audit_log record
/// after processing each Audited command.
#[derive(Debug, Clone)]
pub struct AuditContext {
    pub user_id: String,
    pub email: String,
    pub role: String,
    pub source: String,    // "http" or "socket"
    pub source_ip: String, // empty for socket
}

/// A command sent to the writer actor for serialized write access.
pub enum WriteCommand {
    /// Execute a request through the write connection (no audit).
    Raw {
        request: Request,
        reply: oneshot::Sender<Response>,
    },
    /// Execute a request and log an audit record afterward.
    Audited {
        request: Request,
        reply: oneshot::Sender<Response>,
        audit: AuditContext,
    },
    /// Fire-and-forget: update access_count/accessed_at/activation_level for recalled memories.
    /// Sent from the read-only path (Recall, CompileContext) so that memory usage tracking
    /// doesn't fail silently on read-only connections.
    TouchMemories { ids: Vec<String>, boost_amount: f64 },
    /// Fire-and-forget: record a context injection in context_effectiveness.
    /// Sent from the read-only CompileContext handler since it can't write directly.
    RecordInjection {
        session_id: String,
        hook_event: String,
        context_type: String,
        content_summary: String,
        chars_injected: usize,
    },
    /// Fire-and-forget: record one extraction metric (success or error).
    /// Sent from the background extractor so that `forge-next stats` reads a
    /// live counter instead of zero. SP1 #53.
    RecordExtraction {
        session_id: String,
        memories_created: usize,
        tokens_in: u64,
        tokens_out: u64,
        cost_cents: u64,
        error: Option<String>,
    },
}

/// Returns true if the request is read-only (no DB mutations).
///
/// Read-only requests are served directly on per-connection read-only SQLite
/// connections, bypassing the writer actor entirely. This eliminates mutex
/// contention between API reads and background workers.
///
/// NOTE: Some "read-only" requests (GuardrailsCheck, CompileContext, etc.)
/// emit broadcast events, but those don't mutate the database — they're
/// fire-and-forget notifications.
pub fn is_read_only(req: &Request) -> bool {
    let base = matches!(
        req,
        Request::Health
            | Request::HealthByProject
            | Request::Status
            | Request::Doctor
            | Request::ManasHealth { .. }
            | Request::Recall { .. }
            | Request::CompileContext { .. }
            | Request::CompileContextTrace { .. }
            | Request::ContextRefresh { .. }
            | Request::CompletionCheck { .. }
            | Request::TaskCompletionCheck { .. }
            | Request::ContextStats { .. }
            | Request::Sessions { .. }
            | Request::ListPlatform
            | Request::ListTools
            | Request::ListPerceptions { .. }
            | Request::ListIdentity { .. }
            | Request::ListDisposition { .. }
            | Request::GetConfig
            | Request::GetStats { .. }
            | Request::GetGraphData { .. }
            | Request::BatchRecall { .. }
            | Request::LspStatus
            | Request::Verify { .. }
            | Request::GetDiagnostics { .. }
            | Request::SyncConflicts
            | Request::SyncExport { .. }
            | Request::GuardrailsCheck { .. }
            | Request::PreBashCheck { .. }
            | Request::PostBashCheck { .. }
            | Request::PostEditCheck { .. }
            | Request::BlastRadius { .. }
            | Request::Export { .. }
            | Request::SessionMessages { .. }
            | Request::SessionMessageRead { .. }
            | Request::ListEntities { .. }
            | Request::ListPermissions
            | Request::GetEffectiveConfig { .. }
            | Request::ListScopedConfig { .. }
            | Request::CrossEngineQuery { .. }
            | Request::FileMemoryMap { .. }
            | Request::CodeSearch { .. }
            | Request::ProjectList { .. }
            | Request::ProjectShow { .. }
            | Request::ListContradictions { .. }
            | Request::ListAgentTemplates { .. }
            | Request::GetAgentTemplate { .. }
            | Request::ListAgents { .. }
            | Request::ListTeamMembers { .. }
            | Request::TeamStatus { .. }
            | Request::ListTeamTemplates
            | Request::ListOrganizations
            | Request::TeamTree { .. }
            | Request::MeetingStatus { .. }
            | Request::MeetingResponses { .. }
            | Request::ListMeetings { .. }
            | Request::MeetingTranscript { .. }
            | Request::MeetingResult { .. }
            | Request::ListNotifications { .. }
            | Request::HealingStatus
            | Request::HealingLog { .. }
            | Request::SkillsList { .. }
            | Request::SkillsInfo { .. }
            | Request::GetHudConfig { .. }
            | Request::ExportHudConfig { .. }
            | Request::ListFlipped { .. } // Phase 2A-4a: read-only listing of flipped preferences
                                          // NOTE: ReaffirmPreference is a WRITE — updates reaffirmed_at column
                                          // NOTE: SetHudConfig is a write — modifies config_scope table
                                          // NOTE: HealingRun is a write — triggers healing cycle
                                          // NOTE: AckNotification, DismissNotification, ActOnNotification are writes
                                          // NOTE: ProjectDetect and ProjectInit are NOT read-only — they create project records
                                          // NOTE: CreateMeeting, MeetingSynthesize, MeetingDecide, MeetingVote are writes
                                          // NOTE: ForceIndex is NOT read-only — it triggers indexing
                                          // NOTE: SpawnAgent, UpdateAgentStatus, RetireAgent, CreateTeam, SetTeamOrchestrator are writes
                                          // NOTE: SkillsInstall, SkillsUninstall, SkillsRefresh are writes
                                          // NOTE: FlipPreference is a write — modifies memory state
    );
    // Phase 2A-4b: ComputeRecencyFactor is bench-only and is READ-ONLY
    // (pure formula computation, no DB writes). Gated separately because
    // cfg attributes cannot appear inside matches!() pattern position.
    // Uses feature = "bench" (not any(test,...)) since forge-core's variant
    // is only available when the bench feature propagates from forge-daemon.
    #[cfg(feature = "bench")]
    let base = base || matches!(req, Request::ComputeRecencyFactor { .. });
    base
}

/// Derive a short request type name from a Request variant for audit logging.
///
/// Uses serde serialization to extract the "method" tag, which gives us the
/// snake_case variant name (e.g., "remember", "set_config", "shutdown").
fn request_type_name(request: &Request) -> String {
    // Serialize to JSON and extract the "method" field
    if let Ok(val) = serde_json::to_value(request) {
        if let Some(method) = val.get("method").and_then(|m| m.as_str()) {
            return method.to_string();
        }
    }
    // Fallback: debug format
    format!("{request:?}").chars().take(50).collect()
}

/// Derive a short summary from a Request for audit logging (truncated to 200 chars).
fn request_summary(request: &Request) -> String {
    let full = match serde_json::to_string(request) {
        Ok(s) => s,
        Err(_) => format!("{request:?}"),
    };
    if full.len() <= 200 {
        full
    } else {
        let mut s: String = full.chars().take(197).collect();
        s.push_str("...");
        s
    }
}

/// Determine the response status string for audit logging.
fn response_status(response: &Response) -> &'static str {
    match response {
        Response::Error { .. } => "error",
        _ => "ok",
    }
}

/// Actor that serializes all write operations through a single connection.
///
/// Receives WriteCommand messages via an mpsc channel and processes them
/// sequentially using the existing `handle_request` function. This ensures
/// only one write operation happens at a time without blocking read paths.
///
/// The WriterActor OWNS its DaemonState (no Arc<Mutex>). This means it is
/// never blocked by workers holding their own Arc<Mutex<DaemonState>>.
/// Both the writer and workers open separate SQLite connections to the same
/// db_path; SQLite WAL mode serializes writes internally.
pub struct WriterActor {
    pub state: super::handler::DaemonState,
    /// Per-daemon supervisor for fire-and-forget blocking tasks
    /// (currently force-index). Shared with `main.rs`'s shutdown path
    /// so SIGTERM can drain in-flight passes before socket teardown.
    /// Constructed once in `main.rs` and cloned into the actor; see
    /// `crates/daemon/src/server/supervisor.rs` for the contract.
    /// P3-4 W1.29 (W23 review HIGH-1 strategic).
    pub bg: Arc<BackgroundTaskSupervisor>,
}

impl WriterActor {
    pub async fn run(mut self, mut rx: mpsc::Receiver<WriteCommand>) {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                WriteCommand::TouchMemories { ids, boost_amount } => {
                    // Fire-and-forget: touch accessed_at + boost activation on the write connection.
                    // This is the fix for the read-only path: Recall/CompileContext collect IDs
                    // and send them here instead of attempting writes on their read-only conn.
                    let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
                    crate::db::ops::touch(&self.state.conn, &id_refs);
                    for id in &ids {
                        let _ =
                            crate::db::ops::boost_activation(&self.state.conn, id, boost_amount);
                    }
                }
                WriteCommand::RecordInjection {
                    session_id,
                    hook_event,
                    context_type,
                    content_summary,
                    chars_injected,
                } => {
                    // Fire-and-forget: record context injection on the write connection.
                    // CompileContext is read-only but needs to track effectiveness metrics.
                    let _ = crate::db::effectiveness::record_injection_with_size(
                        &self.state.conn,
                        &session_id,
                        &hook_event,
                        &context_type,
                        &content_summary,
                        chars_injected,
                    );
                }
                WriteCommand::RecordExtraction {
                    session_id,
                    memories_created,
                    tokens_in,
                    tokens_out,
                    cost_cents,
                    error,
                } => {
                    // Fire-and-forget: record one extraction metric row. The
                    // background extractor runs on a shared Arc<Mutex<DaemonState>>
                    // but its writes must go through the writer actor so that
                    // `forge-next stats` reads a live `extraction` counter.
                    let _ = crate::db::metrics::record_extraction(
                        &self.state.conn,
                        &session_id,
                        memories_created,
                        tokens_in,
                        tokens_out,
                        cost_cents,
                        error.as_deref(),
                    );
                }
                WriteCommand::Raw { request, reply } => {
                    let response = match request {
                        Request::ForceIndex { path } => self.process_force_index_async(path),
                        other => super::handler::handle_request(&mut self.state, other),
                    };
                    let _ = reply.send(response);
                }
                WriteCommand::Audited {
                    request,
                    reply,
                    audit,
                } => {
                    let req_type = request_type_name(&request);
                    let summary = request_summary(&request);
                    let response = match request {
                        Request::ForceIndex { path } => self.process_force_index_async(path),
                        other => super::handler::handle_request(&mut self.state, other),
                    };
                    let status = response_status(&response);

                    // Insert audit log record (best-effort — don't fail the request)
                    let audit_id = ulid::Ulid::new().to_string();
                    if let Err(e) = self.state.conn.execute(
                        "INSERT INTO audit_log (id, actor_type, actor_id, action, resource_type, resource_id, timestamp,
                         user_id, email, role, request_type, request_summary, source, source_ip, response_status)
                         VALUES (?1, 'api', ?2, ?3, 'request', '', datetime('now'),
                         ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        rusqlite::params![
                            audit_id,
                            audit.user_id,
                            req_type,
                            audit.user_id,
                            audit.email,
                            audit.role,
                            req_type,
                            summary,
                            audit.source,
                            audit.source_ip,
                            status,
                        ],
                    ) {
                        tracing::warn!("failed to insert audit log: {e}");
                    }

                    let _ = reply.send(response);
                }
            }
        }
    }

    /// F23 (W22): handle `Request::ForceIndex` without blocking the writer
    /// loop. Validates the optional path synchronously so the caller still
    /// gets immediate "not a directory" / canonicalize errors, then dispatches
    /// the heavy indexing work (file walking, regex extraction, import edges,
    /// clustering) onto a `tokio::task::spawn_blocking` worker that opens its
    /// own write-capable SQLite connection. Returns
    /// `ResponseData::IndexComplete { 0, 0 }` immediately so the writer-actor
    /// can process the next request — the CLI-side surface treats `0,0` as
    /// "indexer started in background" (see `commands::system::force_index`).
    ///
    /// **P3-4 W1.29 (W23 review HIGH-1 strategic)**: the dispatched task is
    /// now (a) gated by `BackgroundTaskSupervisor::try_claim_indexer` so a
    /// second concurrent `force-index` returns a structured error instead of
    /// spawning a duplicate writer, and (b) registered in the supervisor's
    /// JoinSet so `main.rs`'s shutdown path can drain it (with a deadline)
    /// before socket teardown. Pre-fix the supervisor was fire-and-forget,
    /// which (i) didn't reject overlap and (ii) let SIGTERM strand the
    /// indexer mid-pass. SQLite WAL preserved DB integrity at COMMIT
    /// granularity even before this fix; what was at risk was
    /// per-pass invariant coherence (file rows + import edges + cluster
    /// rebuild ordered as a unit) on the next start.
    fn process_force_index_async(&self, path: Option<String>) -> Response {
        // Resolve `path` to its canonical form once, on the writer-actor thread,
        // and hand the canonical string into the spawn closure. Avoids a
        // canonicalize-twice TOCTOU window where the directory could be
        // renamed/deleted/symlink-swapped between sync validation and the
        // spawn_blocking re-resolution. (W23 review MED-1.)
        let canonical_path: Option<String> = match path {
            Some(ref dir) => match std::fs::canonicalize(dir) {
                Ok(p) => {
                    if !p.is_dir() {
                        return Response::Error {
                            message: format!("'{dir}' is not a directory"),
                        };
                    }
                    Some(p.to_string_lossy().to_string())
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("cannot resolve path '{dir}': {e}"),
                    };
                }
            },
            None => None,
        };

        // P3-4 W1.30 review MED-2: reject force-index requests that
        // arrive AFTER `main.rs` has signaled shutdown (e.g. from a
        // worker still sending writes via the mpsc during the drain
        // window). Pre-fix such a late request would slip past the
        // supervisor and get stranded by process exit, masking the
        // drain's "completed cleanly" log line.
        if self.bg.is_shutting_down() {
            tracing::warn!(
                target: "forge_daemon::indexer",
                path = canonical_path.as_deref().unwrap_or("<all-projects>"),
                "force-index rejected: daemon is shutting down"
            );
            return Response::Error {
                message: "force-index rejected — daemon is shutting down".to_string(),
            };
        }

        // P3-4 W1.29: atomic claim. If the previous pass hasn't completed,
        // refuse rather than spawning a duplicate writer that would race on
        // the same db_path's WAL. The CLI surface translates this Error
        // back to the user as "force-index already running, retry shortly".
        if !self.bg.try_claim_indexer() {
            tracing::warn!(
                target: "forge_daemon::indexer",
                path = canonical_path.as_deref().unwrap_or("<all-projects>"),
                "force-index rejected: a previous pass is still in flight"
            );
            return Response::Error {
                message: "force-index already in progress — wait for the previous pass to complete or retry shortly".to_string(),
            };
        }

        let db_path = self.state.db_path.clone();
        tracing::info!(
            target: "forge_daemon::indexer",
            path = canonical_path.as_deref().unwrap_or("<all-projects>"),
            "force-index dispatched to background task"
        );
        // P3-4 W1.13 (W23 review HIGH-1): supervise the spawn_blocking
        // JoinHandle. Pre-fix it was dropped, swallowing any panic in
        // the indexer worker. The supervisor logs cancellations
        // (SIGTERM mid-run) and panics (rusqlite/io failure) so the
        // operator has a breadcrumb instead of a silent partial index.
        //
        // P3-4 W1.29 (W23 HIGH-1 strategic close): the supervisor
        // task is now registered in `bg.in_flight` (a tracked JoinSet)
        // so `main.rs`'s shutdown path can drain it before exit.
        // The closure also calls `release_indexer()` in ALL completion
        // paths (Ok / Err / panic) so a transient failure doesn't
        // leave the slot stuck — without that, every subsequent
        // `force-index` would be rejected until daemon restart.
        let logged_path = canonical_path
            .as_deref()
            .unwrap_or("<all-projects>")
            .to_string();
        let bg = Arc::clone(&self.bg);
        // Outer tokio::spawn: this fn is sync (called from
        // `WriterActor::run`'s match arm), but `bg.spawn_supervised`
        // is async (it acquires the Mutex around the JoinSet).
        // Detach the lock-acquire+register into a tiny async task so
        // the writer-actor returns immediately to its next message.
        tokio::spawn(async move {
            let bg_for_release = Arc::clone(&bg);
            let logged_path_inner = logged_path.clone();
            bg.spawn_supervised(async move {
                let join = tokio::task::spawn_blocking(move || {
                    run_force_index_in_task(&db_path, canonical_path);
                });
                match join.await {
                    Ok(_) => {
                        // The blocking body itself logs success
                        // (run_force_index_in_task → tracing::info on
                        // completion); nothing extra to log here.
                    }
                    Err(e) if e.is_panic() => {
                        tracing::error!(
                            target: "forge_daemon::indexer",
                            path = %logged_path_inner,
                            "force-index background task PANICKED — partial state may be on disk"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "forge_daemon::indexer",
                            path = %logged_path_inner,
                            error = %e,
                            "force-index background task cancelled (likely SIGTERM mid-run)"
                        );
                    }
                }
                // Release the slot in ALL paths so the next
                // force-index can proceed.
                bg_for_release.release_indexer();
            })
            .await;
        });

        Response::Ok {
            data: ResponseData::IndexComplete {
                files_indexed: 0,
                symbols_indexed: 0,
                // P3-4 W1.30 (W23 review MED-3): typed signal that
                // these counts are background-dispatch placeholders,
                // not a real (0, 0) result from a legitimately-empty
                // project. The CLI keys off this instead of the
                // brittle `files == 0 && symbols == 0` heuristic.
                dispatched: true,
            },
        }
    }
}

/// Background body for an async `force-index` dispatch. Opens a fresh
/// write-capable connection to `db_path` so the writer-actor's connection
/// is free to serve other writes; SQLite WAL mode handles inter-connection
/// write serialisation per-transaction (≪ 30 s, vs. the previous
/// whole-handler hold). The work mirrors the synchronous `Request::ForceIndex`
/// handler in `handler.rs` so the on-disk effect is identical.
fn run_force_index_in_task(db_path: &str, path: Option<String>) {
    crate::db::vec::init_sqlite_vec();
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(
                target: "forge_daemon::indexer",
                error = %e,
                db_path,
                "force-index: failed to open background connection"
            );
            return;
        }
    };
    // P3-4 W1.30 (W23 review MED-4): canonical PRAGMA helper.
    let _ = crate::db::apply_runtime_pragmas(&conn);

    if let Some(canonical) = path {
        // `canonical` was resolved on the writer-actor thread before spawn —
        // do NOT re-canonicalize here (W23 review MED-1).
        let (files_indexed, symbols_indexed) =
            crate::workers::indexer::index_directory_sync(&conn, &canonical);
        tracing::info!(
            target: "forge_daemon::indexer",
            files_indexed,
            symbols_indexed,
            project = %canonical,
            "force-index complete (background)"
        );
    } else {
        let files = crate::db::ops::list_code_files(&conn);
        let import_edges = crate::workers::indexer::extract_and_store_imports(&conn, &files);
        let projects: std::collections::HashSet<String> =
            files.iter().map(|f| f.project.clone()).collect();
        for project_dir in &projects {
            crate::workers::indexer::run_clustering(&conn, project_dir);
        }
        let symbols_indexed: usize = conn
            .query_row("SELECT COUNT(*) FROM code_symbol", [], |r| r.get(0))
            .unwrap_or(0);
        tracing::info!(
            target: "forge_daemon::indexer",
            files_indexed = files.len(),
            import_edges,
            symbols_indexed,
            "force-index (no-path) complete (background)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[test]
    fn test_read_only_classification() {
        // Verify known read-only requests
        assert!(is_read_only(&Request::Health));
        assert!(is_read_only(&Request::HealthByProject));
        assert!(is_read_only(&Request::Status));
        assert!(is_read_only(&Request::Doctor));
        assert!(is_read_only(&Request::LspStatus));
        assert!(is_read_only(&Request::GetConfig));
        assert!(is_read_only(&Request::SyncConflicts));
        assert!(is_read_only(&Request::ListPlatform));
        assert!(is_read_only(&Request::ListTools));

        assert!(is_read_only(&Request::Recall {
            query: "test".into(),
            memory_type: None,
            project: None,
            limit: None,
            layer: None,
            since: None,
            include_flipped: None,
            include_globals: None,
            query_embedding: None,
        }));

        assert!(is_read_only(&Request::CompileContext {
            agent: None,
            project: None,
            static_only: None,
            excluded_layers: None,
            session_id: None,
            focus: None,
            cwd: None,
            dry_run: None,
        }));

        assert!(is_read_only(&Request::Sessions { active_only: None }));

        assert!(is_read_only(&Request::ManasHealth { project: None }));

        assert!(is_read_only(&Request::Export {
            format: None,
            since: None,
        }));

        assert!(is_read_only(&Request::SyncExport {
            project: None,
            since: None,
        }));

        assert!(is_read_only(&Request::Verify { file: None }));

        assert!(is_read_only(&Request::GetDiagnostics {
            file: "test.rs".into(),
        }));

        assert!(is_read_only(&Request::GetStats { hours: None }));

        assert!(is_read_only(&Request::GetGraphData {
            layer: None,
            limit: None,
        }));

        assert!(is_read_only(&Request::BatchRecall { queries: vec![] }));

        assert!(is_read_only(&Request::GuardrailsCheck {
            file: "f".into(),
            action: "edit".into(),
        }));

        assert!(is_read_only(&Request::PreBashCheck {
            command: "ls".into(),
            session_id: None,
        }));

        assert!(is_read_only(&Request::PostBashCheck {
            command: "ls".into(),
            exit_code: 0,
            session_id: None,
        }));

        assert!(is_read_only(&Request::PostEditCheck {
            file: "f.rs".into(),
            session_id: None,
        }));

        assert!(is_read_only(&Request::BlastRadius {
            file: "f.rs".into(),
            project: None,
        }));

        assert!(is_read_only(&Request::ListPerceptions {
            project: None,
            limit: None,
            offset: None,
        }));

        assert!(is_read_only(&Request::ListIdentity {
            agent: "test".into(),
            project: None,
            include_global_identity: None,
        }));

        assert!(is_read_only(&Request::ListDisposition {
            agent: "test".into(),
        }));

        assert!(is_read_only(&Request::CompileContextTrace {
            agent: None,
            project: None,
            session_id: None,
        }));

        assert!(is_read_only(&Request::ListPermissions));

        // Verify known write requests
        assert!(!is_read_only(&Request::Remember {
            memory_type: forge_core::types::MemoryType::Decision,
            title: "t".into(),
            content: "c".into(),
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
            valence: None,
            intensity: None,
        }));
        assert!(!is_read_only(&Request::Forget { id: "x".into() }));
        assert!(!is_read_only(&Request::ForceConsolidate));
        assert!(!is_read_only(&Request::ForceExtract));
        assert!(!is_read_only(&Request::Import { data: "{}".into() }));
        assert!(!is_read_only(&Request::IngestClaude));
        assert!(!is_read_only(&Request::Shutdown));
        assert!(!is_read_only(&Request::RegisterSession {
            id: "s".into(),
            agent: "a".into(),
            project: None,
            cwd: None,
            capabilities: None,
            current_task: None,
        }));
        assert!(!is_read_only(&Request::EndSession { id: "s".into() }));
        assert!(!is_read_only(&Request::HlcBackfill));
        assert!(!is_read_only(&Request::SetConfig {
            key: "k".into(),
            value: "v".into(),
        }));
        assert!(!is_read_only(&Request::SyncImport { lines: vec![] }));
        assert!(!is_read_only(&Request::SyncResolve {
            keep_id: "x".into(),
        }));
        assert!(!is_read_only(&Request::StorePlatform {
            key: "k".into(),
            value: "v".into(),
        }));
        assert!(!is_read_only(&Request::CleanupSessions {
            prefix: None,
            older_than_secs: None,
            prune_ended: false,
        }));
        assert!(!is_read_only(&Request::Bootstrap { project: None }));
        assert!(!is_read_only(&Request::GrantPermission {
            from_agent: "claude-code".into(),
            to_agent: "cline".into(),
            from_project: None,
            to_project: None,
        }));
        assert!(!is_read_only(&Request::RevokePermission {
            id: "perm-1".into(),
        }));

        // Scoped config: read-only
        assert!(is_read_only(&Request::GetEffectiveConfig {
            session_id: None,
            agent: None,
            reality_id: None,
            user_id: None,
            team_id: None,
            organization_id: Some("default".into()),
        }));
        assert!(is_read_only(&Request::ListScopedConfig {
            scope_type: "organization".into(),
            scope_id: "default".into(),
        }));

        // Cross-engine queries: read-only
        assert!(is_read_only(&Request::CrossEngineQuery {
            file: "src/main.rs".into(),
            reality_id: None,
        }));
        assert!(is_read_only(&Request::FileMemoryMap {
            files: vec!["src/main.rs".into()],
            reality_id: None,
        }));
        assert!(is_read_only(&Request::CodeSearch {
            query: "test".into(),
            kind: None,
            limit: None,
            project: None,
        }));

        // ProjectList: read-only
        assert!(is_read_only(&Request::ProjectList {
            organization_id: Some("default".into()),
        }));

        // ForceIndex: write (triggers indexing)
        assert!(!is_read_only(&Request::ForceIndex { path: None }));

        // Scoped config: write
        assert!(!is_read_only(&Request::SetScopedConfig {
            scope_type: "organization".into(),
            scope_id: "default".into(),
            key: "max_tokens".into(),
            value: "4096".into(),
            locked: false,
            ceiling: None,
        }));
        assert!(!is_read_only(&Request::DeleteScopedConfig {
            scope_type: "organization".into(),
            scope_id: "default".into(),
            key: "max_tokens".into(),
        }));

        // Organization Hierarchy: read-only
        assert!(is_read_only(&Request::ListOrganizations));
        assert!(is_read_only(&Request::TeamTree {
            organization_id: Some("default".into()),
        }));

        // Organization Hierarchy: writes
        assert!(!is_read_only(&Request::CreateOrganization {
            name: "acme".into(),
            description: None,
        }));
        assert!(!is_read_only(&Request::TeamSend {
            team_name: "leadership".into(),
            kind: "notification".into(),
            topic: "deploy".into(),
            parts: vec![],
            from_session: None,
            recursive: false,
        }));
        assert!(!is_read_only(&Request::CreateOrgFromTemplate {
            template_name: "startup".into(),
            org_name: "acme".into(),
        }));

        // Memory Self-Healing: read-only
        assert!(is_read_only(&Request::HealingStatus));
        assert!(is_read_only(&Request::HealingLog {
            limit: None,
            action: None,
        }));

        // Memory Self-Healing: write
        assert!(!is_read_only(&Request::HealingRun));
    }

    #[tokio::test]
    async fn test_writer_actor_processes_health() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let actor = WriterActor {
            state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Raw {
            request: Request::Health,
            reply: reply_tx,
        })
        .await
        .unwrap();
        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok, got {other:?}"),
        }

        drop(tx); // close channel -> actor exits
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_writer_actor_handles_write_request() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let actor = WriterActor {
            state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        // Send a Remember (write) request
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Raw {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "test decision".into(),
                content: "test content".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok for Remember, got {other:?}"),
        }

        drop(tx);
        handle.await.unwrap();
    }

    /// Test that the writer is NOT blocked when workers hold a mutex on a
    /// separate DaemonState. This is the core fix for the 30s timeout bug.
    ///
    /// Before the fix: WriterActor shared Arc<Mutex<DaemonState>> with workers.
    /// When a worker held the lock for seconds, the writer couldn't process
    /// socket requests, causing timeouts.
    ///
    /// After the fix: WriterActor owns its own DaemonState. Workers have their
    /// own Arc<Mutex<DaemonState>>. No shared mutex = no blocking.
    #[tokio::test]
    async fn test_write_doesnt_timeout_when_worker_holds_mutex() {
        // Simulate the production setup:
        // - writer_state: owned by WriterActor (no mutex)
        // - worker_state: Arc<Mutex> held by background workers

        // Worker state (simulating what workers use)
        let worker_state = Arc::new(Mutex::new(
            crate::server::handler::DaemonState::new(":memory:").unwrap(),
        ));

        // Writer state (owned, independent connection)
        let writer_state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let actor = WriterActor {
            state: writer_state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        // Simulate a worker holding the mutex for 2 seconds
        let worker_clone = Arc::clone(&worker_state);
        let worker_handle = tokio::spawn(async move {
            let _locked = worker_clone.lock().await;
            // Hold the lock for 2 seconds (simulating extraction work)
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });

        // Send a write to the WriterActor while the worker holds its mutex.
        // This MUST complete quickly (< 1 second), not wait 2+ seconds.
        let start = std::time::Instant::now();
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Raw {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "urgent decision".into(),
                content: "must not be blocked by workers".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        let elapsed = start.elapsed();

        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok, got {other:?}"),
        }

        // The write should complete in well under 1 second.
        // Before the fix, it would block for 2+ seconds waiting on the worker mutex.
        assert!(
            elapsed.as_millis() < 1000,
            "Write took {}ms — should be <1000ms (not blocked by worker mutex)",
            elapsed.as_millis()
        );

        // Clean up
        drop(tx);
        handle.await.unwrap();
        worker_handle.await.unwrap();
    }

    /// Test that concurrent writes from both the writer actor and workers
    /// succeed independently (both use separate connections to the same DB).
    #[tokio::test]
    async fn test_concurrent_writes_from_writer_and_worker() {
        use tempfile::TempDir;

        // Use a real file-based DB so both connections share the same data.
        // TempDir gives us a directory; we put the DB file inside it.
        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("test.db");
        let db_path = db_path.to_str().unwrap();

        // Worker state (first connection, creates schema)
        let worker_state = Arc::new(Mutex::new(
            crate::server::handler::DaemonState::new(db_path).unwrap(),
        ));

        // Writer state (second connection to same file, uses new_writer to
        // share resources; schema already created by worker_state)
        let events;
        let hlc;
        let started_at;
        {
            let locked = worker_state.lock().await;
            events = locked.events.clone();
            hlc = Arc::clone(&locked.hlc);
            started_at = locked.started_at;
        }
        let writer_state =
            crate::server::handler::DaemonState::new_writer(db_path, events, hlc, started_at)
                .unwrap();
        let actor = WriterActor {
            state: writer_state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        // Worker writes a memory directly via its own state
        {
            let mut locked = worker_state.lock().await;
            let resp = crate::server::handler::handle_request(
                &mut locked,
                Request::Remember {
                    memory_type: forge_core::types::MemoryType::Lesson,
                    title: "worker memory".into(),
                    content: "written by worker".into(),
                    confidence: None,
                    tags: None,
                    project: None,
                    metadata: None,
                    valence: None,
                    intensity: None,
                },
            );
            match resp {
                Response::Ok { .. } => {}
                other => panic!("worker write failed: {other:?}"),
            }
        }

        // Writer actor writes a memory via the channel
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Raw {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "writer memory".into(),
                content: "written by writer actor".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("writer write failed: {other:?}"),
        }

        // Both writes should have succeeded — verify via the worker connection.
        // The worker connection can see both memories because SQLite WAL makes
        // committed writes visible to all connections.
        {
            let locked = worker_state.lock().await;
            let count: i64 = locked
                .conn
                .query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0))
                .unwrap();
            assert!(
                count >= 2,
                "expected at least 2 memories (worker + writer), got {count}"
            );
        }

        drop(tx);
        handle.await.unwrap();
    }

    #[test]
    fn test_request_type_name() {
        assert_eq!(request_type_name(&Request::Health), "health");
        assert_eq!(request_type_name(&Request::Shutdown), "shutdown");
        assert_eq!(
            request_type_name(&Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "t".into(),
                content: "c".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            }),
            "remember"
        );
        assert_eq!(
            request_type_name(&Request::SetConfig {
                key: "k".into(),
                value: "v".into(),
            }),
            "set_config"
        );
    }

    #[test]
    fn test_request_summary_truncation() {
        let short = request_summary(&Request::Health);
        assert!(short.len() <= 200);

        // A request with long content should be truncated
        let long_content = "x".repeat(500);
        let summary = request_summary(&Request::Remember {
            memory_type: forge_core::types::MemoryType::Decision,
            title: "t".into(),
            content: long_content,
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
            valence: None,
            intensity: None,
        });
        assert!(summary.len() <= 200);
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn test_response_status() {
        assert_eq!(
            response_status(&Response::Ok {
                data: forge_core::protocol::ResponseData::Health {
                    decisions: 0,
                    lessons: 0,
                    patterns: 0,
                    preferences: 0,
                    edges: 0,
                }
            }),
            "ok"
        );
        assert_eq!(
            response_status(&Response::Error {
                message: "bad".into()
            }),
            "error"
        );
    }

    #[tokio::test]
    async fn test_audited_command_creates_audit_record() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let actor = WriterActor {
            state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        let audit = AuditContext {
            user_id: "user-42".to_string(),
            email: "user@test.com".to_string(),
            role: "member".to_string(),
            source: "http".to_string(),
            source_ip: "10.0.0.1".to_string(),
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Audited {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "audited decision".into(),
                content: "audited content".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
            reply: reply_tx,
            audit,
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok, got {other:?}"),
        }

        // We can't directly query the in-memory DB from here because the actor
        // owns it. But we verified the command was processed without errors.
        // The audit insert is best-effort (logged on failure).

        drop(tx);
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_audited_command_records_in_db() {
        use tempfile::TempDir;

        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("audit_test.db");
        let db_path_str = db_path.to_str().unwrap();

        // Create initial state (sets up schema)
        let state = crate::server::handler::DaemonState::new(db_path_str).unwrap();
        let events = state.events.clone();
        let hlc = Arc::clone(&state.hlc);
        let started_at = state.started_at;
        drop(state);

        // Create the writer actor with its own connection
        let writer_state = crate::server::handler::DaemonState::new_writer(
            db_path_str,
            events.clone(),
            hlc.clone(),
            started_at,
        )
        .unwrap();
        let actor = WriterActor {
            state: writer_state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        // Send an audited write
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Audited {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "audit test".into(),
                content: "audit content".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
            reply: reply_tx,
            audit: AuditContext {
                user_id: "uid-99".to_string(),
                email: "audit@test.com".to_string(),
                role: "admin".to_string(),
                source: "http".to_string(),
                source_ip: "192.168.1.1".to_string(),
            },
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok, got {other:?}"),
        }

        drop(tx);
        handle.await.unwrap();

        // Now open a reader connection to verify the audit record
        let reader = crate::server::handler::DaemonState::new_reader(
            db_path_str,
            events,
            hlc,
            started_at,
            None,
            None,
        )
        .unwrap();

        let (user_id, email, role, req_type, source, source_ip, status): (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
        ) = reader
            .conn
            .query_row(
                "SELECT user_id, email, role, request_type, source, source_ip, response_status FROM audit_log WHERE user_id = 'uid-99'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(user_id, "uid-99");
        assert_eq!(email, "audit@test.com");
        assert_eq!(role, "admin");
        assert_eq!(req_type, "remember");
        assert_eq!(source, "http");
        assert_eq!(source_ip, "192.168.1.1");
        assert_eq!(status, "ok");
    }

    // ── Wave 5: Additional audit verification tests ──

    /// AC6: Verify audit record fields are correctly populated after a Forget write.
    #[tokio::test]
    async fn test_audit_record_for_forget_operation() {
        use tempfile::TempDir;

        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("audit_forget_test.db");
        let db_path_str = db_path.to_str().unwrap();

        let state = crate::server::handler::DaemonState::new(db_path_str).unwrap();
        let events = state.events.clone();
        let hlc = Arc::clone(&state.hlc);
        let started_at = state.started_at;
        drop(state);

        let writer_state = crate::server::handler::DaemonState::new_writer(
            db_path_str,
            events.clone(),
            hlc.clone(),
            started_at,
        )
        .unwrap();
        let actor = WriterActor {
            state: writer_state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Audited {
            request: Request::Forget {
                id: "mem-xyz".into(),
            },
            reply: reply_tx,
            audit: AuditContext {
                user_id: "uid-forget".to_string(),
                email: "forgetful@test.com".to_string(),
                role: "member".to_string(),
                source: "http".to_string(),
                source_ip: "10.0.0.42".to_string(),
            },
        })
        .await
        .unwrap();

        let _resp = reply_rx.await.unwrap();

        drop(tx);
        handle.await.unwrap();

        // Verify audit record
        let reader = crate::server::handler::DaemonState::new_reader(
            db_path_str,
            events,
            hlc,
            started_at,
            None,
            None,
        )
        .unwrap();

        let (user_id, req_type, role, source_ip): (String, String, String, String) = reader
            .conn
            .query_row(
                "SELECT user_id, request_type, role, source_ip FROM audit_log WHERE user_id = 'uid-forget'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();

        assert_eq!(user_id, "uid-forget");
        assert_eq!(req_type, "forget");
        assert_eq!(role, "member");
        assert_eq!(source_ip, "10.0.0.42");
    }

    /// AC6: Verify that Raw (socket) writes do NOT create audit records.
    #[tokio::test]
    async fn test_raw_write_does_not_create_audit_record() {
        use tempfile::TempDir;

        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("no_audit_test.db");
        let db_path_str = db_path.to_str().unwrap();

        let state = crate::server::handler::DaemonState::new(db_path_str).unwrap();
        let events = state.events.clone();
        let hlc = Arc::clone(&state.hlc);
        let started_at = state.started_at;
        drop(state);

        let writer_state = crate::server::handler::DaemonState::new_writer(
            db_path_str,
            events.clone(),
            hlc.clone(),
            started_at,
        )
        .unwrap();
        let actor = WriterActor {
            state: writer_state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        // Send a Raw write (socket path — no audit)
        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Raw {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "raw write".into(),
                content: "no audit expected".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
            reply: reply_tx,
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok, got {other:?}"),
        }

        drop(tx);
        handle.await.unwrap();

        // Verify NO audit records exist
        let reader = crate::server::handler::DaemonState::new_reader(
            db_path_str,
            events,
            hlc,
            started_at,
            None,
            None,
        )
        .unwrap();

        let count: i64 = reader
            .conn
            .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
            .unwrap();

        assert_eq!(count, 0, "Raw writes should not create audit records");
    }

    /// AC6: Multiple audited writes produce multiple audit records.
    #[tokio::test]
    async fn test_multiple_audited_writes_produce_multiple_records() {
        use tempfile::TempDir;

        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("multi_audit.db");
        let db_path_str = db_path.to_str().unwrap();

        let state = crate::server::handler::DaemonState::new(db_path_str).unwrap();
        let events = state.events.clone();
        let hlc = Arc::clone(&state.hlc);
        let started_at = state.started_at;
        drop(state);

        let writer_state = crate::server::handler::DaemonState::new_writer(
            db_path_str,
            events.clone(),
            hlc.clone(),
            started_at,
        )
        .unwrap();
        let actor = WriterActor {
            state: writer_state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        // Send 3 audited writes from different users
        for i in 0..3 {
            let (reply_tx, reply_rx) = oneshot::channel();
            tx.send(WriteCommand::Audited {
                request: Request::Remember {
                    memory_type: forge_core::types::MemoryType::Lesson,
                    title: format!("lesson {i}"),
                    content: format!("content {i}"),
                    confidence: None,
                    tags: None,
                    project: None,
                    metadata: None,
                    valence: None,
                    intensity: None,
                },
                reply: reply_tx,
                audit: AuditContext {
                    user_id: format!("user-{i}"),
                    email: format!("user{i}@test.com"),
                    role: "member".to_string(),
                    source: "http".to_string(),
                    source_ip: format!("10.0.0.{i}"),
                },
            })
            .await
            .unwrap();
            let _ = reply_rx.await.unwrap();
        }

        drop(tx);
        handle.await.unwrap();

        // Verify 3 audit records exist
        let reader = crate::server::handler::DaemonState::new_reader(
            db_path_str,
            events,
            hlc,
            started_at,
            None,
            None,
        )
        .unwrap();

        let count: i64 = reader
            .conn
            .query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))
            .unwrap();

        assert_eq!(count, 3, "Expected 3 audit records for 3 audited writes");
    }

    /// AC6: Audit records contain request_summary (truncated).
    #[tokio::test]
    async fn test_audit_record_contains_request_summary() {
        use tempfile::TempDir;

        let tmp_dir = TempDir::new().unwrap();
        let db_path = tmp_dir.path().join("audit_summary.db");
        let db_path_str = db_path.to_str().unwrap();

        let state = crate::server::handler::DaemonState::new(db_path_str).unwrap();
        let events = state.events.clone();
        let hlc = Arc::clone(&state.hlc);
        let started_at = state.started_at;
        drop(state);

        let writer_state = crate::server::handler::DaemonState::new_writer(
            db_path_str,
            events.clone(),
            hlc.clone(),
            started_at,
        )
        .unwrap();
        let actor = WriterActor {
            state: writer_state,
            bg: std::sync::Arc::new(BackgroundTaskSupervisor::new()),
        };
        let (tx, rx) = mpsc::channel(10);
        let handle = tokio::spawn(async move { actor.run(rx).await });

        let (reply_tx, reply_rx) = oneshot::channel();
        tx.send(WriteCommand::Audited {
            request: Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "important decision".into(),
                content: "detailed content here".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
                valence: None,
                intensity: None,
            },
            reply: reply_tx,
            audit: AuditContext {
                user_id: "uid-summary".to_string(),
                email: "summary@test.com".to_string(),
                role: "admin".to_string(),
                source: "http".to_string(),
                source_ip: "127.0.0.1".to_string(),
            },
        })
        .await
        .unwrap();

        let _ = reply_rx.await.unwrap();
        drop(tx);
        handle.await.unwrap();

        let reader = crate::server::handler::DaemonState::new_reader(
            db_path_str,
            events,
            hlc,
            started_at,
            None,
            None,
        )
        .unwrap();

        let (summary, status): (String, String) = reader
            .conn
            .query_row(
                "SELECT request_summary, response_status FROM audit_log WHERE user_id = 'uid-summary'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();

        // Summary should contain part of the request
        assert!(!summary.is_empty(), "request_summary should not be empty");
        assert!(
            summary.len() <= 200,
            "request_summary should be truncated to <= 200 chars"
        );
        assert_eq!(status, "ok");
    }
}
