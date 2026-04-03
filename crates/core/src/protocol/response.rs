use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    HealthByProject {
        projects: HashMap<String, HealthProjectData>,
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
    IngestClaude {
        imported: usize,
        skipped: usize,
    },
    Backfill {
        chunks_processed: usize,
        memories_stored: usize,
    },
    GuardrailsCheck {
        safe: bool,
        warnings: Vec<String>,
        decisions_affected: Vec<String>,
        callers_count: usize,
    },
    BlastRadius {
        decisions: Vec<BlastRadiusDecision>,
        callers: usize,
        importers: Vec<String>,
        files_affected: Vec<String>,
    },
    SessionRegistered { id: String },
    SessionEnded { id: String, found: bool },
    Sessions { sessions: Vec<SessionInfo>, count: usize },
    LspStatus { servers: Vec<LspServerInfo> },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthProjectData {
    pub decisions: usize,
    pub lessons: usize,
    pub patterns: usize,
    pub preferences: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportEdge {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
    pub properties: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlastRadiusDecision {
    pub id: String,
    pub title: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInfo {
    pub id: String,
    pub agent: String,
    pub project: Option<String>,
    pub cwd: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LspServerInfo {
    pub language: String,
    pub command: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { data: ResponseData },
    Error { message: String },
}
