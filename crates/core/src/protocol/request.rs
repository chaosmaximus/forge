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
    Shutdown,
}
