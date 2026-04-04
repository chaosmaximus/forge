use serde::{Deserialize, Serialize};
use crate::types::memory::MemoryType;

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
    },
    /// Mark a session as ended
    EndSession { id: String },
    /// List sessions
    Sessions { active_only: Option<bool> },
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
    ManasHealth,

    /// Compile optimized context from all Manas layers (for session-start)
    CompileContext {
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

    Shutdown,
}
