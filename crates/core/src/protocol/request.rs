use serde::{Deserialize, Serialize};
use crate::types::memory::MemoryType;

/// A single finding from an evaluator review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationFinding {
    /// What was found (e.g., "Missing error handling in auth.rs:42")
    pub description: String,
    /// Severity: "critical", "high", "medium", "low", "info"
    pub severity: String,
    /// File paths affected
    pub files: Vec<String>,
    /// Category: "bug", "security", "performance", "style", "good_pattern"
    pub category: String,
}

/// A single recall query for BatchRecall.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecallQuery {
    pub text: String,
    pub memory_type: Option<MemoryType>,
    pub limit: Option<usize>,
}

/// A part of a session message (A2A-inspired).
/// Supports text, file references, structured data, and memory references.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePart {
    pub kind: String,              // "text", "file", "data", "memory_ref"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Remember {
        memory_type: MemoryType,
        title: String,
        content: String,
        confidence: Option<f64>,
        tags: Option<Vec<String>>,
        project: Option<String>,
    },
    Recall {
        query: String,
        memory_type: Option<MemoryType>,
        project: Option<String>,
        limit: Option<usize>,
        /// Layer filter: "experience", "declared", "domain_dna", "perception", "identity"
        /// None = search all layers (current behavior)
        #[serde(default)]
        layer: Option<String>,
    },
    Forget {
        id: String,
    },
    Health,
    /// Health counts grouped by project
    HealthByProject,
    Status,
    Doctor,
    /// Export all data as JSON (for visualization, backup, or sync)
    Export {
        format: Option<String>,  // "json" (default) | "ndjson"
        since: Option<String>,   // timestamp filter (optional)
    },
    /// Import data from JSON (stdin or file)
    Import {
        data: String,  // JSON string of exported data
    },
    /// Ingest Claude Code's MEMORY.md files from ~/.claude/projects/*/memory/
    IngestClaude,
    /// Ingest a file as declared knowledge (Layer 5)
    IngestDeclared {
        path: String,
        source: String,
        project: Option<String>,
    },
    /// Backfill: re-process a transcript file from scratch (ignoring offsets)
    Backfill {
        path: String,
    },
    /// Subscribe to real-time event stream (keeps connection open, streams NDJSON)
    Subscribe {
        events: Option<Vec<String>>, // filter by event type; None = all events
    },
    /// Pre-execution guardrail check: are there decisions linked to this file?
    GuardrailsCheck {
        file: String,
        action: String,
    },
    /// Pre-bash check: warn about destructive commands, surface relevant skills/lessons
    PreBashCheck {
        command: String,
    },
    /// Post-bash check: on failure, surface relevant lessons and skills
    PostBashCheck {
        command: String,
        exit_code: i32,
    },
    /// Post-edit check: surface callers, lessons, and patterns after a file edit
    PostEditCheck {
        file: String,
    },
    /// Blast radius analysis: what is the impact of changing this file?
    BlastRadius {
        file: String,
    },
    /// Register an active agent session
    RegisterSession {
        id: String,
        agent: String,
        project: Option<String>,
        cwd: Option<String>,
        /// A2A: capabilities this session advertises (e.g., "code_review", "testing")
        #[serde(default)]
        capabilities: Option<Vec<String>>,
        /// A2A: description of what this session is currently working on
        #[serde(default)]
        current_task: Option<String>,
    },
    /// Mark a session as ended
    EndSession { id: String },
    /// List sessions
    Sessions { active_only: Option<bool> },
    /// Cleanup sessions: end all active sessions matching optional prefix filter.
    /// If prefix is None, ends ALL active sessions (nuclear option).
    CleanupSessions { prefix: Option<String> },
    /// Query which language servers are available for the current project
    LspStatus,

    /// Run proactive checks on a file or show all active diagnostics
    Verify { file: Option<String> },
    /// Show cached diagnostics for a file
    GetDiagnostics { file: String },

    // ── Manas Layer Operations ──

    /// Store a platform key-value pair (Layer 0)
    StorePlatform { key: String, value: String },
    /// List all platform entries (Layer 0)
    ListPlatform,
    /// Store a tool (Layer 1)
    StoreTool { tool: crate::types::manas::Tool },
    /// List all tools (Layer 1)
    ListTools,
    /// Store a perception (Layer 4)
    StorePerception { perception: crate::types::manas::Perception },
    /// List unconsumed perceptions (Layer 4)
    ListPerceptions { project: Option<String>, limit: Option<usize> },
    /// Consume (mark as read) perceptions by ID (Layer 4)
    ConsumePerceptions { ids: Vec<String> },
    /// Store an identity facet (Layer 6 — Ahankara)
    StoreIdentity { facet: crate::types::manas::IdentityFacet },
    /// List identity facets for an agent (Layer 6)
    ListIdentity { agent: String },
    /// Deactivate an identity facet (Layer 6)
    DeactivateIdentity { id: String },
    /// List disposition traits for an agent (Layer 7)
    ListDisposition { agent: String },
    /// Extended health across all 8 Manas layers
    ManasHealth {
        /// Optional project filter for is_new_project check.
        #[serde(default)]
        project: Option<String>,
    },

    /// Compile optimized context from all Manas layers (for session-start)
    CompileContext {
        agent: Option<String>,
        project: Option<String>,
        /// If true, only return the static prefix (platform, identity, disposition, tools).
        /// Used by session-start hook to cache the stable part for KV-cache optimization.
        #[serde(default)]
        static_only: Option<bool>,
        /// Layer names to exclude from the dynamic suffix.
        /// Valid names: "decisions", "lessons", "skills", "perceptions", "working_set", "active_sessions".
        /// Excluded layers emit empty self-closing tags to maintain XML structure stability for KV-cache.
        #[serde(default)]
        excluded_layers: Option<Vec<String>>,
    },

    /// Compile context with full trace of considered/included/excluded memories + reasons.
    /// Used for debugging and visualization of the context assembly process.
    CompileContextTrace {
        agent: Option<String>,
        project: Option<String>,
    },

    // ── Sync Operations ──

    /// Export memories as NDJSON lines with HLC metadata for sync
    SyncExport {
        project: Option<String>,
        since: Option<String>,
    },
    /// Import NDJSON memory lines from a remote node
    SyncImport {
        lines: Vec<String>,
    },
    /// List unresolved sync conflicts
    SyncConflicts,
    /// Resolve a sync conflict by keeping the given memory ID
    SyncResolve {
        keep_id: String,
    },

    /// Backfill HLC timestamps on existing memories that have empty hlc_timestamp
    HlcBackfill,

    /// Store evaluation findings as lessons for the agent self-evaluation feedback loop.
    /// Each finding becomes a lesson memory; high-severity findings also create diagnostics.
    StoreEvaluation {
        findings: Vec<EvaluationFinding>,
        project: Option<String>,
        session_id: Option<String>,
    },
    /// Bootstrap: scan and process all existing transcript files
    Bootstrap {
        project: Option<String>,
    },
    /// Force-run ALL consolidation phases synchronously (exact dedup, semantic dedup, etc.)
    ForceConsolidate,
    /// Trigger extraction on all pending transcripts (skip debounce)
    ForceExtract,
    /// Extract memories using a specific provider (for testing/comparison in app).
    /// Does NOT store memories — returns a preview of what WOULD be extracted.
    ExtractWithProvider {
        provider: String,         // "ollama", "claude", "claude_api", "openai", "gemini"
        model: Option<String>,    // override model, or use default for provider
        text: String,             // conversation text to extract from
    },

    /// Get current daemon configuration
    GetConfig,
    /// Update a config value by dotted key (e.g., "extraction.backend")
    SetConfig {
        key: String,
        value: String,
    },

    /// Query aggregated metrics/stats for a time period
    GetStats {
        hours: Option<u64>,
    },

    /// Get graph data for Cortex 3D visualization — nodes (memories) + edges
    GetGraphData {
        layer: Option<String>,  // filter by layer name, or None for all
        limit: Option<usize>,   // max nodes per layer (default 50)
    },

    /// Batch recall — multiple queries in single request (eliminates N+1 for sidebar)
    BatchRecall {
        queries: Vec<RecallQuery>,
    },

    // ── A2A Inter-Session Protocol (FISP) ──

    /// Send a message to another session (notification or request)
    SessionSend {
        to: String,                    // session ID or "*" for broadcast
        kind: String,                  // "notification" or "request"
        topic: String,
        parts: Vec<MessagePart>,
        project: Option<String>,
        timeout_secs: Option<u64>,
    },
    /// Respond to a received request
    SessionRespond {
        message_id: String,
        status: String,                // "accepted", "rejected", "completed", "failed"
        parts: Vec<MessagePart>,
    },
    /// Get pending messages for a session
    SessionMessages {
        session_id: String,
        status: Option<String>,
        limit: Option<usize>,
    },
    /// Mark messages as read/consumed
    SessionAck {
        message_ids: Vec<String>,
    },

    /// List entities (Knowledge Intelligence)
    ListEntities {
        project: Option<String>,
        limit: Option<usize>,
    },

    // ── A2A Permission Management ──

    /// Grant A2A permission for inter-session messaging
    GrantPermission {
        from_agent: String,
        to_agent: String,
        from_project: Option<String>,
        to_project: Option<String>,
    },
    /// Revoke an A2A permission by ID
    RevokePermission { id: String },
    /// List all A2A permissions
    ListPermissions,

    // ── Scoped Configuration ──

    /// Get effective (resolved) config for a scope chain
    GetEffectiveConfig {
        session_id: Option<String>,
        agent: Option<String>,
        reality_id: Option<String>,
        user_id: Option<String>,
        team_id: Option<String>,
        organization_id: Option<String>,
    },
    /// Set a scoped configuration value
    SetScopedConfig {
        scope_type: String,
        scope_id: String,
        key: String,
        value: String,
        locked: bool,
        ceiling: Option<f64>,
    },
    /// Delete a scoped configuration value
    DeleteScopedConfig {
        scope_type: String,
        scope_id: String,
        key: String,
    },
    /// List all configuration entries for a scope
    ListScopedConfig {
        scope_type: String,
        scope_id: String,
    },

    /// Detect what kind of reality a project path represents.
    /// Auto-creates a reality record if one doesn't exist for the path.
    DetectReality {
        path: String,
    },

    /// Cross-engine query: given a file, return its symbols, callers, cluster, and related memories.
    CrossEngineQuery {
        file: String,
        reality_id: Option<String>,
    },

    /// File-memory map: for each file, return how many memories mention it, decisions, entities.
    FileMemoryMap {
        files: Vec<String>,
        reality_id: Option<String>,
    },

    /// Code search: find symbols by name pattern with optional kind filter.
    CodeSearch {
        query: String,
        kind: Option<String>,  // "function", "class", "file"
        limit: Option<usize>,
    },

    Shutdown,
}
