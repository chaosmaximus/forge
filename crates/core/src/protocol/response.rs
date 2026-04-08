use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::code::{CodeFile, CodeSymbol};
use crate::types::memory::Memory;

/// A single trace entry from context compilation, showing why a memory was included or excluded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceEntry {
    pub id: String,
    pub title: String,
    pub memory_type: String,
    pub confidence: f64,
    pub activation_level: f64,
    pub reason: String,
}

/// An edge connecting a memory to another memory or entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryEdge {
    pub target_id: String,
    pub edge_type: String,
}

/// A memory node for Cortex 3D visualization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    pub memory_type: String,
    pub layer: String,
    pub confidence: f64,
    pub activation_level: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// An edge for Cortex 3D visualization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphEdge {
    pub from_id: String,
    pub to_id: String,
    pub edge_type: String,
    pub strength: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryResult {
    #[serde(flatten)]
    pub memory: Memory,
    pub score: f64,
    pub source: String,
    /// Edges connecting this memory to other memories/entities.
    /// Populated by hybrid_recall; empty for other recall sources.
    #[serde(default)]
    pub edges: Vec<MemoryEdge>,
}

/// A single structured health check result for `forge doctor`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthCheck {
    pub name: String,
    /// "ok", "warn", or "error"
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseData {
    Stored { id: String },
    Memories { results: Vec<MemoryResult>, count: usize },
    Forgotten { id: String },
    Superseded { old_id: String, new_id: String },
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
        /// Structured health checks with ok/warn/error status.
        #[serde(default)]
        checks: Vec<HealthCheck>,
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
        /// Cluster this file belongs to (from community detection), if any.
        #[serde(default)]
        cluster_name: Option<String>,
        /// Other files in the same cluster.
        #[serde(default)]
        cluster_files: Vec<String>,
        /// Warnings (e.g., "Language not indexed — blast-radius unavailable for .py files")
        #[serde(default)]
        warnings: Vec<String>,
        /// Files that call symbols in this file (from edge table).
        #[serde(default)]
        calling_files: Vec<String>,
    },
    /// Heartbeat acknowledgment for session liveness tracking.
    Heartbeat {
        session_id: String,
        status: String,
    },

    // ── Proactive Context (Prajna) ──

    /// Delta context since last refresh — only new/changed items.
    ContextDelta {
        notifications: Vec<String>,
        warnings: Vec<String>,
        anti_patterns: Vec<String>,
        messages_pending: usize,
    },
    /// Result of completion signal check.
    CompletionCheckResult {
        has_completion_signal: bool,
        relevant_lessons: Vec<String>,
        severity: String,
    },
    /// Task completion verification result.
    TaskCompletionCheckResult {
        warnings: Vec<String>,
        checklists: Vec<String>,
    },
    /// Context injection observability.
    ContextStatsResult {
        total_injections: usize,
        total_chars: usize,
        estimated_tokens: usize,
        acknowledged: usize,
        effectiveness_rate: f64,
        per_hook: Vec<(String, usize, usize)>, // (hook_event, count, chars)
    },

    // ── Memory Self-Healing ──

    HealingStatusResult {
        total_healed: usize,
        auto_superseded: usize,
        auto_faded: usize,
        last_cycle_at: Option<String>,
        stale_candidates: usize,
    },
    HealingRunResult {
        topic_superseded: usize,
        session_faded: usize,
        quality_adjusted: usize,
    },
    HealingLogResult {
        entries: Vec<serde_json::Value>,
        count: usize,
    },

    SessionRegistered { id: String },
    SessionEnded { id: String, found: bool },
    Sessions { sessions: Vec<SessionInfo>, count: usize },
    SessionsCleaned { ended: usize },
    /// Acknowledgment that current_task was updated on a session.
    CurrentTaskSet { session_id: String, task: String },
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
        /// True if the project has zero active memories (brand new project).
        #[serde(default)]
        is_new_project: bool,
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

    ContextTrace {
        considered: Vec<TraceEntry>,
        included: Vec<TraceEntry>,
        excluded: Vec<TraceEntry>,
        budget_total: usize,
        budget_used: usize,
        layer_chars: HashMap<String, usize>,
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

    BackfillProjectResult {
        updated: usize,
        skipped: usize,
    },

    CleanupMemoryResult {
        garbage_deleted: usize,
        projects_normalized: usize,
        perceptions_purged: usize,
        declared_cleaned: usize,
    },

    EvaluationStored {
        lessons_created: usize,
        diagnostics_created: usize,
    },
    BootstrapComplete {
        files_processed: usize,
        files_skipped: usize,
        memories_extracted: usize,
        errors: usize,
    },
    ConsolidationComplete {
        exact_dedup: usize,
        semantic_dedup: usize,
        linked: usize,
        faded: usize,
        promoted: usize,
        reconsolidated: usize,
        embedding_merged: usize,
        strengthened: usize,
        contradictions: usize,
        entities_detected: usize,
        #[serde(default)]
        synthesized: usize,
        #[serde(default)]
        gaps_detected: usize,
        #[serde(default)]
        reweaved: usize,
        #[serde(default)]
        scored: usize,
    },
    ExtractionTriggered {
        files_queued: usize,
    },
    ConfigData {
        backend: String,
        ollama_model: String,
        ollama_endpoint: String,
        claude_cli_model: String,
        claude_api_model: String,
        claude_api_key_set: bool,
        openai_model: String,
        openai_endpoint: String,
        openai_key_set: bool,
        gemini_model: String,
        gemini_key_set: bool,
        embedding_model: String,
    },
    ConfigUpdated {
        key: String,
        value: String,
    },

    ExtractionResult {
        provider: String,
        model: String,
        memories_extracted: usize,
        tokens_in_estimate: usize,
        tokens_out_estimate: usize,
        latency_ms: u64,
    },

    Stats {
        period_hours: u64,
        extractions: usize,
        extraction_errors: usize,
        tokens_in: usize,
        tokens_out: usize,
        total_cost_usd: f64,
        avg_latency_ms: usize,
        memories_created: usize,
    },

    /// Graph data for Cortex 3D visualization (brain map)
    GraphData {
        nodes: Vec<GraphNode>,
        edges: Vec<GraphEdge>,
        total_nodes: usize,
        total_edges: usize,
    },

    /// Batch recall results — one result set per query
    BatchRecallResults {
        results: Vec<Vec<MemoryResult>>,
    },

    /// Entity list for Knowledge Intelligence
    EntityList {
        entities: Vec<crate::types::manas::Entity>,
        count: usize,
    },

    // ── A2A Inter-Session Protocol (FISP) ──

    /// A message was sent to another session
    MessageSent { id: String, status: String },
    /// A response was sent to a received message
    MessageResponded { id: String, status: String },
    /// List of messages for a session
    SessionMessageList { messages: Vec<SessionMessage>, count: usize },
    /// Messages were acknowledged
    MessagesAcked { count: usize },

    // ── A2A Permission Responses ──

    /// A permission was granted
    PermissionGranted { id: String },
    /// A permission was revoked
    PermissionRevoked { id: String, found: bool },
    /// List of all A2A permissions
    PermissionList { permissions: Vec<A2aPermission>, count: usize },

    // ── Scoped Configuration Responses ──

    /// Effective (resolved) configuration for a scope chain
    EffectiveConfig {
        config: std::collections::HashMap<String, crate::types::entity::ResolvedConfigValue>,
    },
    /// A scoped config entry was set
    ScopedConfigSet {
        scope_type: String,
        scope_id: String,
        key: String,
    },
    /// A scoped config entry was deleted
    ScopedConfigDeleted {
        deleted: bool,
    },
    /// List of scoped config entries
    ScopedConfigList {
        entries: Vec<crate::types::entity::ConfigScopeEntry>,
    },

    /// A reality was detected (or already existed) for a project path.
    RealityDetected {
        reality_id: String,
        name: String,
        reality_type: String,
        domain: String,
        detected_from: String,
        confidence: f64,
        is_new: bool,
        metadata: serde_json::Value,
    },

    /// Cross-engine query result: symbols + callers + cluster + related memories for a file.
    CrossEngineResult {
        file: String,
        symbols: Vec<serde_json::Value>,
        callers: usize,
        calling_files: Vec<String>,
        cluster: Option<String>,
        cluster_files: Vec<String>,
        related_memories: Vec<serde_json::Value>,
    },

    /// File-memory map result: for each file, its memory info.
    FileMemoryMapResult {
        mappings: std::collections::HashMap<String, FileMemoryInfo>,
    },

    /// Code search result: matching symbols.
    CodeSearchResult {
        hits: Vec<serde_json::Value>,
    },

    /// List of known realities (projects).
    RealitiesList {
        realities: Vec<crate::types::entity::Reality>,
    },

    /// Code index counts after a force-index trigger.
    IndexComplete {
        files_indexed: usize,
        symbols_indexed: usize,
    },

    // ── Agent Teams ──

    /// Agent template was created
    AgentTemplateCreated { id: String, name: String },
    /// Single agent template
    AgentTemplateData { template: crate::types::team::AgentTemplate },
    /// List of agent templates
    AgentTemplateList { templates: Vec<crate::types::team::AgentTemplate>, count: usize },
    /// Agent template was deleted
    AgentTemplateDeleted { id: String, found: bool },
    /// Agent template was updated
    AgentTemplateUpdated { id: String, updated: bool },

    /// Agent was spawned from a template
    AgentSpawned { session_id: String, template_name: String, team: Option<String> },
    /// List of active agents
    AgentList { agents: Vec<serde_json::Value>, count: usize },
    /// Agent status was updated
    AgentStatusUpdated { session_id: String, status: String },
    /// Agent was retired
    AgentRetired { session_id: String },

    /// Team was created
    TeamCreated { id: String, name: String },
    /// List of team members
    TeamMemberList { members: Vec<serde_json::Value>, count: usize },
    /// Team orchestrator was set
    TeamOrchestratorSet { team_name: String, session_id: String },
    /// Full team status data
    TeamStatusData { team: serde_json::Value },
    /// A full team was started (run_team)
    RunTeamResult { team_name: String, agents_spawned: usize, session_ids: Vec<String> },
    /// A team was stopped (stop_team)
    TeamStopped { team_name: String, agents_retired: usize },
    /// List of pre-built team templates
    TeamTemplateList { templates: Vec<serde_json::Value>, count: usize },

    // ── Organization Hierarchy ──

    /// Organization was created
    OrganizationCreated { id: String },
    /// List of all organizations
    OrganizationList { organizations: Vec<serde_json::Value> },
    /// Messages sent to team members
    TeamSent { messages_sent: usize },
    /// Team hierarchy tree
    TeamTreeData { tree: Vec<serde_json::Value> },
    /// Organization created from template
    OrgFromTemplateCreated { org_id: String, teams_created: usize },

    // ── Meeting Protocol ──

    /// A meeting was created
    MeetingCreated { meeting_id: String, participant_count: usize },
    /// Meeting status + participant statuses
    MeetingStatusData { meeting: serde_json::Value, participants: Vec<serde_json::Value> },
    /// List of participant responses for a meeting
    MeetingResponseList { responses: Vec<serde_json::Value>, count: usize },
    /// Synthesis was stored
    MeetingSynthesized { meeting_id: String },
    /// Decision was recorded and stored as memory
    MeetingDecided { meeting_id: String, decision_memory_id: String },
    /// List of meetings
    MeetingList { meetings: Vec<serde_json::Value>, count: usize },
    /// Full meeting transcript
    MeetingTranscriptData { transcript: serde_json::Value },
    /// Meeting response recorded
    MeetingResponseRecorded { meeting_id: String, all_responded: bool },

    /// A vote was recorded in a meeting
    MeetingVoteRecorded { meeting_id: String, choice: String },
    /// Vote results for a meeting (counts per option, quorum status, outcome)
    MeetingResultData {
        meeting_id: String,
        outcome: Option<String>,
        votes: HashMap<String, usize>,
        quorum_met: bool,
        total_votes: usize,
        required_votes: usize,
    },

    // ── Notification Engine ──

    /// List of notifications
    NotificationList { notifications: Vec<serde_json::Value>, count: usize },
    /// A notification was acknowledged
    NotificationAcked { id: String },
    /// A notification was dismissed
    NotificationDismissed { id: String },
    /// A notification action was taken
    NotificationActed { id: String, result: Option<String> },

    // ── Workspace ──

    /// Workspace was initialized (directories created)
    WorkspaceInitialized { path: String, teams_created: usize },
    /// Current workspace status
    WorkspaceStatusData {
        mode: String,
        org: String,
        root: String,
        teams: Vec<String>,
    },

    LicenseStatusResult {
        tier: String,
        has_key: bool,
    },
    LicenseSet {
        tier: String,
    },

    // ── Skills Registry ──

    /// List of skills from the registry
    SkillsList {
        skills: Vec<serde_json::Value>,
        count: usize,
    },
    /// Skill was installed for a project
    SkillInstalled {
        name: String,
        project: String,
    },
    /// Skill was uninstalled from a project
    SkillUninstalled {
        name: String,
        project: String,
    },
    /// Full skill details
    SkillInfo {
        skill: Option<serde_json::Value>,
    },
    /// Skills directory was re-indexed
    SkillsRefreshed {
        count: usize,
    },

    /// Smart Model Router: routing statistics
    RoutingStats {
        total_routed: usize,
        tiers: Vec<RoutingTierStats>,
        total_tokens_saved: i64,
    },

    // ── Per-Agent Budget Enforcement ──

    /// Cost was recorded against an agent session
    CostRecorded {
        session_id: String,
        total_spent: f64,
        budget_limit: Option<f64>,
        exceeded: bool,
    },
    /// Budget status for agent session(s)
    BudgetStatusResult {
        entries: Vec<serde_json::Value>,
    },

    /// Database vacuum result: purged faded memories, removed orphan code entries, freed bytes.
    Vacuumed {
        faded_purged: usize,
        orphan_files_removed: usize,
        orphan_symbols_removed: usize,
        freed_bytes: u64,
    },

    /// Backfill affects edges result
    BackfillAffectsResult {
        memories_scanned: usize,
        edges_created: usize,
    },

    /// Symbol search results
    SymbolResults {
        symbols: Vec<SymbolInfo>,
    },

    Shutdown,
}

/// Symbol information returned by FindSymbol and GetSymbolsOverview
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub parent: Option<String>,
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
    /// A2A: capabilities this session advertises
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// A2A: what the session is currently working on
    #[serde(default)]
    pub current_task: String,
}

/// A message exchanged between sessions via the FISP protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionMessage {
    pub id: String,
    pub from_session: String,
    pub to_session: String,
    pub kind: String,
    pub topic: String,
    pub parts: Vec<crate::protocol::request::MessagePart>,
    pub status: String,
    pub in_reply_to: Option<String>,
    pub project: Option<String>,
    pub created_at: String,
    pub delivered_at: Option<String>,
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

/// An A2A permission rule controlling inter-session messaging in "controlled" mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct A2aPermission {
    pub id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub from_project: Option<String>,
    pub to_project: Option<String>,
    pub allowed: bool,
    pub created_by: String,
    pub created_at: String,
}

/// Per-tier routing statistics for the Smart Model Router.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingTierStats {
    pub tier: String,
    pub count: usize,
    pub successes: usize,
    pub tokens_saved: i64,
}

/// Information about memories related to a file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileMemoryInfo {
    pub memory_count: usize,
    pub decision_count: usize,
    pub entity_names: Vec<String>,
    pub last_perception: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)] // ResponseData has many variants by design
pub enum Response {
    Ok { data: ResponseData },
    Error { message: String },
}
