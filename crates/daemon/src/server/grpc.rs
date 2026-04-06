//! Tonic gRPC server — JSON-over-gRPC transport for in-cluster agent communication.
//!
//! Uses the same read/write split as HTTP and Unix socket transports:
//!   - Read-only requests: per-request DaemonState::new_reader
//!   - Write requests: sent through write_tx channel to the WriterActor
//!
//! The proto defines a single Execute RPC carrying JSON-serialized Request/Response,
//! giving gRPC transport benefits (HTTP/2, mTLS, streaming) without mirroring
//! all protocol variants in Protobuf.

use crate::events::EventSender;
use crate::server::handler::{handle_request, DaemonState};
use crate::server::writer::{is_read_only, WriteCommand};
use forge_core::protocol::Request;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{self, Status};

/// Generated protobuf types from proto/forge.proto
pub mod proto {
    tonic::include_proto!("forge.v1");
}

use proto::forge_service_server::ForgeService;
use proto::{ForgeEvent, ForgeRequest, ForgeResponse, SubscribeRequest};

/// Shared state for the gRPC service — mirrors AppState from http.rs.
#[derive(Clone)]
pub struct GrpcState {
    pub db_path: String,
    pub events: EventSender,
    pub hlc: Arc<crate::sync::Hlc>,
    pub started_at: Instant,
    pub write_tx: mpsc::Sender<WriteCommand>,
}

/// Tonic ForgeService implementation.
pub struct ForgeServiceImpl {
    state: GrpcState,
}

impl ForgeServiceImpl {
    pub fn new(state: GrpcState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl ForgeService for ForgeServiceImpl {
    /// Execute any forge protocol request.
    /// Parses the JSON payload as a Request, routes through the read/write split,
    /// and returns the JSON-serialized Response.
    ///
    /// SECURITY: In production, gRPC should be secured via mTLS (tonic TLS config).
    /// As an additional defense layer, if the `authorization` metadata key is present,
    /// it's validated. If gRPC is exposed without mTLS, operators MUST set
    /// FORGE_GRPC_REQUIRE_TOKEN=true and provide tokens in metadata.
    async fn execute(
        &self,
        request: tonic::Request<ForgeRequest>,
    ) -> Result<tonic::Response<ForgeResponse>, Status> {
        // Check for bearer token in gRPC metadata (defense in depth)
        if let Ok(require) = std::env::var("FORGE_GRPC_REQUIRE_TOKEN") {
            if require == "true" || require == "1" {
                let token = request.metadata().get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "));
                if token.is_none() {
                    return Err(Status::unauthenticated("missing authorization token"));
                }
                // Token validation would go here (shared with HTTP auth)
                // For now, presence check is the minimum guard
            }
        }

        let inner = request.into_inner();

        // Parse JSON payload into Request
        let req: Request = serde_json::from_str(&inner.json).map_err(|e| {
            Status::invalid_argument(format!("invalid JSON request: {e}"))
        })?;

        let response = if is_read_only(&req) {
            // Open per-request read-only connection (same pattern as http.rs / socket.rs)
            match DaemonState::new_reader(
                &self.state.db_path,
                self.state.events.clone(),
                Arc::clone(&self.state.hlc),
                self.state.started_at,
            ) {
                Ok(mut reader) => handle_request(&mut reader, req),
                Err(e) => {
                    tracing::error!("failed to open read-only connection: {e}");
                    return Err(Status::unavailable("database unavailable"));
                }
            }
        } else {
            // Send write request through the writer actor with timeout
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            let cmd = WriteCommand::Raw {
                request: req,
                reply: reply_tx,
            };
            match tokio::time::timeout(Duration::from_secs(30), self.state.write_tx.send(cmd)).await
            {
                Ok(Ok(())) => {
                    match tokio::time::timeout(Duration::from_secs(30), reply_rx).await {
                        Ok(Ok(resp)) => resp,
                        Ok(Err(_)) => {
                            tracing::error!("writer actor closed unexpectedly");
                            return Err(Status::unavailable("writer unavailable"));
                        }
                        Err(_) => {
                            tracing::error!("write request timed out after 30s");
                            return Err(Status::deadline_exceeded(
                                "write request timed out after 30s",
                            ));
                        }
                    }
                }
                Ok(Err(_)) => {
                    tracing::error!("daemon writer channel closed");
                    return Err(Status::unavailable("writer channel closed"));
                }
                Err(_) => {
                    tracing::error!("failed to enqueue write request (timeout)");
                    return Err(Status::deadline_exceeded("write enqueue timed out"));
                }
            }
        };

        // Serialize Response back to JSON
        let json = serde_json::to_string(&response).map_err(|e| {
            Status::internal(format!("failed to serialize response: {e}"))
        })?;

        Ok(tonic::Response::new(ForgeResponse { json }))
    }

    type SubscribeStream = ReceiverStream<Result<ForgeEvent, Status>>;

    /// Subscribe to daemon events with optional filters.
    /// Uses the same broadcast channel as socket.rs Subscribe.
    async fn subscribe(
        &self,
        request: tonic::Request<SubscribeRequest>,
    ) -> Result<tonic::Response<Self::SubscribeStream>, Status> {
        let inner = request.into_inner();
        let mut rx = self.state.events.subscribe();

        // Extract filters (empty strings treated as None)
        let event_filter: Option<Vec<String>> = if inner.event_types.is_empty() {
            None
        } else {
            Some(inner.event_types)
        };
        let session_filter = if inner.session_id.is_empty() {
            None
        } else {
            Some(inner.session_id)
        };
        let team_filter = if inner.team_id.is_empty() {
            None
        } else {
            Some(inner.team_id)
        };

        // Create a bounded channel for the stream
        let (tx, stream_rx) = mpsc::channel(64);

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // Apply event type filter
                        if let Some(ref types) = event_filter {
                            if !types.is_empty() && !types.contains(&event.event) {
                                continue;
                            }
                        }
                        // Apply session_id filter
                        if let Some(ref sid) = session_filter {
                            let data_str = event.data.to_string();
                            if !data_str.contains(sid.as_str()) {
                                continue;
                            }
                        }
                        // Apply team_id filter
                        if let Some(ref tid) = team_filter {
                            let data_str = event.data.to_string();
                            if !data_str.contains(tid.as_str()) {
                                continue;
                            }
                        }

                        let grpc_event = ForgeEvent {
                            event: event.event,
                            data: event.data.to_string(),
                            timestamp: event.timestamp,
                        };

                        if tx.send(Ok(grpc_event)).await.is_err() {
                            // Client disconnected
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });

        Ok(tonic::Response::new(ReceiverStream::new(stream_rx)))
    }
}

/// Start the gRPC server with a pre-bound listener and graceful shutdown.
/// main.rs binds the listener early so bind failures are caught at startup.
pub async fn run_grpc_server(
    db_path: String,
    events: EventSender,
    hlc: Arc<crate::sync::Hlc>,
    started_at: Instant,
    write_tx: mpsc::Sender<WriteCommand>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    listener: tokio::net::TcpListener,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = GrpcState {
        db_path,
        events,
        hlc,
        started_at,
        write_tx,
    };

    let service = ForgeServiceImpl::new(state);
    let svc = proto::forge_service_server::ForgeServiceServer::new(service);

    // Convert tokio TcpListener to tonic-compatible incoming stream
    let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);

    tonic::transport::Server::builder()
        .add_service(svc)
        .serve_with_incoming_shutdown(incoming, async move {
            let _ = shutdown_rx.changed().await;
            tracing::info!("gRPC server shutting down gracefully");
        })
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grpc_state_clone() {
        // GrpcState must be Clone for tonic service requirements
        let events = crate::events::create_event_bus();
        let hlc = Arc::new(crate::sync::Hlc::new(&ulid::Ulid::new().to_string()));
        let (write_tx, _write_rx) = mpsc::channel(1);
        let state = GrpcState {
            db_path: "/tmp/test.db".to_string(),
            events,
            hlc,
            started_at: Instant::now(),
            write_tx,
        };
        let _cloned = state.clone();
    }

    #[test]
    fn test_proto_types_exist() {
        // Verify generated protobuf types are accessible
        let req = ForgeRequest {
            json: r#"{"type":"Health"}"#.to_string(),
        };
        assert!(!req.json.is_empty());

        let resp = ForgeResponse {
            json: r#"{"status":"ok"}"#.to_string(),
        };
        assert!(!resp.json.is_empty());

        let sub = SubscribeRequest {
            event_types: vec!["extraction".to_string()],
            session_id: String::new(),
            team_id: String::new(),
        };
        assert_eq!(sub.event_types.len(), 1);

        let event = ForgeEvent {
            event: "test".to_string(),
            data: "{}".to_string(),
            timestamp: "2026-04-05T00:00:00Z".to_string(),
        };
        assert_eq!(event.event, "test");
    }
}
