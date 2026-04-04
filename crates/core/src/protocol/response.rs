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
        embedding_count: usize,
        file_count: usize,
        symbol_count: usize,
        edge_count: usize,
        workers: Vec<String>,
        uptime_secs: u64,
        // Manas layer counts
        platform_count: usize,
        tool_count: usize,
        skill_count: usize,
        domain_dna_count: usize,
        perception_count: usize,
        declared_count: usize,
        identity_count: usize,
        disposition_count: usize,
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
    IngestDeclared {
        ingested: bool,
        path: String,
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
        calling_files: Vec<String>,
        relevant_lessons: Vec<String>,
        dangerous_patterns: Vec<String>,
        applicable_skills: Vec<String>,
    },
    PostEditChecked {
        file: String,
        callers_count: usize,
        calling_files: Vec<String>,
        relevant_lessons: Vec<String>,
        dangerous_patterns: Vec<String>,
        applicable_skills: Vec<String>,
        decisions_to_review: Vec<String>,
        cached_diagnostics: Vec<String>,
    },
    PreBashChecked {
        safe: bool,
        warnings: Vec<String>,
        relevant_skills: Vec<String>,
    },
    PostBashChecked {
        suggestions: Vec<String>,
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

    VerifyResult {
        files_checked: usize,
        errors: usize,
        warnings: usize,
        diagnostics: Vec<DiagnosticEntry>,
    },
    DiagnosticList {
        diagnostics: Vec<DiagnosticEntry>,
        count: usize,
    },

    // ── Manas Layer Responses ──

    PlatformStored { key: String },
    PlatformList { entries: Vec<crate::types::manas::PlatformEntry> },
    ToolStored { id: String },
    ToolList { tools: Vec<crate::types::manas::Tool>, count: usize },
    PerceptionStored { id: String },
    PerceptionList { perceptions: Vec<crate::types::manas::Perception>, count: usize },
    PerceptionsConsumed { count: usize },
    IdentityStored { id: String },
    IdentityList { facets: Vec<crate::types::manas::IdentityFacet>, count: usize },
    IdentityDeactivated { id: String, found: bool },
    DispositionList { traits: Vec<crate::types::manas::Disposition>, count: usize },
    ManasHealthData {
        platform_count: usize,
        tool_count: usize,
        skill_count: usize,
        domain_dna_count: usize,
        perception_unconsumed: usize,
        declared_count: usize,
        identity_facets: usize,
        disposition_traits: usize,
        #[serde(default)]
        experience_count: usize,
        #[serde(default)]
        embedding_count: usize,
        #[serde(default)]
        trait_names: Vec<String>,
    },

    CompiledContext {
        context: String,
        /// Cacheable static prefix (platform, identity, disposition, tools).
        /// Stable within a session — suitable for KV-cache reuse.
        #[serde(default)]
        static_prefix: String,
        /// Per-turn dynamic suffix (decisions, lessons, skills, perceptions, working set).
        /// Changes on each compile.
        #[serde(default)]
        dynamic_suffix: String,
        layers_used: usize,
        chars: usize,
    },

    // ── Sync Responses ──

    SyncExported {
        lines: Vec<String>,
        count: usize,
        node_id: String,
    },
    SyncImported {
        imported: usize,
        conflicts: usize,
        skipped: usize,
    },
    SyncConflictList {
        conflicts: Vec<ConflictPair>,
    },
    SyncResolved {
        id: String,
        resolved: bool,
    },

    HlcBackfilled {
        count: usize,
    },

    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictPair {
    pub title: String,
    pub memory_type: String,
    pub local: ConflictVersion,
    pub remote: ConflictVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictVersion {
    pub id: String,
    pub content: String,
    pub node_id: String,
    pub hlc_timestamp: String,
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
pub struct DiagnosticEntry {
    pub file_path: String,
    pub severity: String,
    pub message: String,
    pub source: String,
    pub line: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { data: ResponseData },
    Error { message: String },
}
