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
#[serde(untagged)]
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
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { data: ResponseData },
    Error { message: String },
}
