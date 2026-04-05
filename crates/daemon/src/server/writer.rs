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
    )
}

/// Actor that serializes all write operations through a single connection.
///
/// Receives WriteCommand messages via an mpsc channel and processes them
/// sequentially using the existing `handle_request` function. This ensures
/// only one write operation happens at a time without blocking read paths.
pub struct WriterActor {
    pub state: std::sync::Arc<tokio::sync::Mutex<super::handler::DaemonState>>,
}

impl WriterActor {
    pub async fn run(self, mut rx: mpsc::Receiver<WriteCommand>) {
        while let Some(cmd) = rx.recv().await {
            match cmd {
                WriteCommand::Raw { request, reply } => {
                    let mut locked = self.state.lock().await;
                    let response = super::handler::handle_request(&mut locked, request);
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
    }

    #[tokio::test]
    async fn test_writer_actor_processes_health() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let state = Arc::new(Mutex::new(state));
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
        let state = Arc::new(Mutex::new(state));
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
}
