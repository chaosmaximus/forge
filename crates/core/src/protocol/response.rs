use serde::{Deserialize, Serialize};
use crate::types::memory::Memory;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryResult {
    #[serde(flatten)]
    pub memory: Memory,
    pub score: f64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseData {
    Stored { id: String },
    Memories { results: Vec<MemoryResult>, count: usize },
    Forgotten { id: String },
    Health {
        decisions: usize,
        lessons: usize,
        patterns: usize,
        preferences: usize,
        edges: usize,
    },
    Status {
        uptime_secs: u64,
        workers: Vec<String>,
        memory_count: usize,
    },
    Doctor {
        daemon_up: bool,
        db_size_bytes: u64,
        memory_count: usize,
        file_count: usize,
        symbol_count: usize,
        edge_count: usize,
        workers: Vec<String>,
        uptime_secs: u64,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { data: ResponseData },
    Error { message: String },
}
