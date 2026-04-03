use crate::db::{ops, schema};
use crate::graph::GraphStore;
use crate::recall::hybrid_recall;
use crate::vector::VectorIndex;
use forge_v2_core::protocol::*;
use forge_v2_core::types::Memory;
use rusqlite::Connection;
use std::time::Instant;

pub struct DaemonState {
    pub conn: Connection,
    pub vector_idx: VectorIndex,
    pub graph: GraphStore,
    pub started_at: Instant,
}

impl DaemonState {
    pub fn new(db_path: &str) -> rusqlite::Result<Self> {
        let conn = if db_path == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(db_path)?
        };
        schema::create_schema(&conn)?;
        Ok(DaemonState {
            conn,
            vector_idx: VectorIndex::new(768),
            graph: GraphStore::new(),
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

        Request::Recall { query, memory_type, limit } => {
            let lim = limit.unwrap_or(10);
            let results =
                hybrid_recall(&state.conn, &state.vector_idx, &state.graph, &query, None, memory_type.as_ref(), lim);
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

        Request::Shutdown => Response::Ok {
            data: ResponseData::Shutdown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_v2_core::types::MemoryType;

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
}
