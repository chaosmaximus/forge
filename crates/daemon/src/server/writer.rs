use forge_core::protocol::{Request, Response};
use tokio::sync::{mpsc, oneshot};

/// A command sent to the writer actor for serialized write access.
pub enum WriteCommand {
    /// Execute a request through the write connection.
    Raw {
        request: Request,
        reply: oneshot::Sender<Response>,
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
    matches!(
        req,
        Request::Health
            | Request::HealthByProject
            | Request::Status
            | Request::Doctor
            | Request::ManasHealth { .. }
            | Request::Recall { .. }
            | Request::CompileContext { .. }
            | Request::CompileContextTrace { .. }
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
            | Request::ListEntities { .. }
            | Request::ListPermissions
            | Request::GetEffectiveConfig { .. }
            | Request::ListScopedConfig { .. }
            | Request::CrossEngineQuery { .. }
            | Request::FileMemoryMap { .. }
            | Request::CodeSearch { .. }
            | Request::ListRealities { .. }
            | Request::ListAgentTemplates { .. }
            | Request::GetAgentTemplate { .. }
            | Request::ListAgents { .. }
            | Request::ListTeamMembers { .. }
            | Request::TeamStatus { .. }
            // NOTE: DetectReality is NOT read-only — it may create a reality record
            // NOTE: ForceIndex is NOT read-only — it triggers indexing
            // NOTE: SpawnAgent, UpdateAgentStatus, RetireAgent, CreateTeam, SetTeamOrchestrator are writes
    )
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
}

impl WriterActor {
    pub async fn run(mut self, mut rx: mpsc::Receiver<WriteCommand>) {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                WriteCommand::Raw { request, reply } => {
                    let response = super::handler::handle_request(&mut self.state, request);
                    let _ = reply.send(response);
                }
            }
        }
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
        }));

        assert!(is_read_only(&Request::CompileContext {
            agent: None,
            project: None,
            static_only: None,
            excluded_layers: None,
        }));

        assert!(is_read_only(&Request::Sessions {
            active_only: None,
        }));

        assert!(is_read_only(&Request::ManasHealth {
            project: None,
        }));

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

        assert!(is_read_only(&Request::BatchRecall {
            queries: vec![],
        }));

        assert!(is_read_only(&Request::GuardrailsCheck {
            file: "f".into(),
            action: "edit".into(),
        }));

        assert!(is_read_only(&Request::PreBashCheck {
            command: "ls".into(),
        }));

        assert!(is_read_only(&Request::PostBashCheck {
            command: "ls".into(),
            exit_code: 0,
        }));

        assert!(is_read_only(&Request::PostEditCheck {
            file: "f.rs".into(),
        }));

        assert!(is_read_only(&Request::BlastRadius {
            file: "f.rs".into(),
        }));

        assert!(is_read_only(&Request::ListPerceptions {
            project: None,
            limit: None,
        }));

        assert!(is_read_only(&Request::ListIdentity {
            agent: "test".into(),
        }));

        assert!(is_read_only(&Request::ListDisposition {
            agent: "test".into(),
        }));

        assert!(is_read_only(&Request::CompileContextTrace {
            agent: None,
            project: None,
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
        assert!(!is_read_only(&Request::SyncImport {
            lines: vec![],
        }));
        assert!(!is_read_only(&Request::SyncResolve {
            keep_id: "x".into(),
        }));
        assert!(!is_read_only(&Request::StorePlatform {
            key: "k".into(),
            value: "v".into(),
        }));
        assert!(!is_read_only(&Request::CleanupSessions {
            prefix: None,
        }));
        assert!(!is_read_only(&Request::Bootstrap {
            project: None,
        }));
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
        }));

        // ListRealities: read-only
        assert!(is_read_only(&Request::ListRealities {
            organization_id: Some("default".into()),
        }));

        // ForceIndex: write (triggers indexing)
        assert!(!is_read_only(&Request::ForceIndex));

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
    }

    #[tokio::test]
    async fn test_writer_actor_processes_health() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let actor = WriterActor { state };
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
            other => panic!("expected Ok, got {:?}", other),
        }

        drop(tx); // close channel -> actor exits
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn test_writer_actor_handles_write_request() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let actor = WriterActor { state };
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
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok for Remember, got {:?}", other),
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
        let actor = WriterActor { state: writer_state };
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
            },
            reply: reply_tx,
        })
        .await
        .unwrap();

        let resp = reply_rx.await.unwrap();
        let elapsed = start.elapsed();

        match resp {
            Response::Ok { .. } => {}
            other => panic!("expected Ok, got {:?}", other),
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
        let writer_state = crate::server::handler::DaemonState::new_writer(
            db_path, events, hlc, started_at,
        ).unwrap();
        let actor = WriterActor { state: writer_state };
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
                },
            );
            match resp {
                Response::Ok { .. } => {}
                other => panic!("worker write failed: {:?}", other),
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
            },
            reply: reply_tx,
        })
        .await
        .unwrap();
        let resp = reply_rx.await.unwrap();
        match resp {
            Response::Ok { .. } => {}
            other => panic!("writer write failed: {:?}", other),
        }

        // Both writes should have succeeded — verify via the worker connection.
        // The worker connection can see both memories because SQLite WAL makes
        // committed writes visible to all connections.
        {
            let locked = worker_state.lock().await;
            let count: i64 = locked.conn
                .query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0))
                .unwrap();
            assert!(
                count >= 2,
                "expected at least 2 memories (worker + writer), got {}",
                count
            );
        }

        drop(tx);
        handle.await.unwrap();
    }
}
