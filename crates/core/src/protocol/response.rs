use serde::{Deserialize, Serialize};
use crate::types::code::{CodeFile, CodeSymbol};
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
    Export {
        memories: Vec<MemoryResult>,
        files: Vec<CodeFile>,
        symbols: Vec<CodeSymbol>,
        edges: Vec<ExportEdge>,
    },
    Import {
        memories_imported: usize,
        files_imported: usize,
        symbols_imported: usize,
        skipped: usize,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportEdge {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { data: ResponseData },
    Error { message: String },
}
