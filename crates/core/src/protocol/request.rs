use crate::types::memory::MemoryType;
use serde::{Deserialize, Serialize};

fn default_empty_args() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

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

/// Phase 2A-4d.3.1 #2 (bench/test only): a synthetic session for
/// driving one disposition-worker cycle without writing rows to the
/// `session` table. Mirrors the only field consumed by the disposition
/// trait-update math (`duration_secs`).
///
/// Used by `Request::StepDispositionOnce` for forge-identity Dim 2.
#[cfg(any(test, feature = "bench"))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionFixture {
    /// Session duration in seconds. The disposition worker buckets these
    /// against `SHORT_SESSION_THRESHOLD_SECS` (60) and
    /// `LONG_SESSION_THRESHOLD_SECS` (600).
    pub duration_secs: i64,
}

/// A part of a session message (A2A-inspired).
/// Supports text, file references, structured data, and memory references.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessagePart {
    pub kind: String, // "text", "file", "data", "memory_ref"
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
        /// Arbitrary structured metadata (e.g., test results: {"passed": 17, "failed": 3, "failures": ["test1", "test2"]})
        #[serde(default)]
        metadata: Option<serde_json::Value>,
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
        /// Temporal filter: only return memories created after this ISO timestamp.
        /// Example: "2026-04-01 00:00:00". Parsed from relative durations by CLI.
        #[serde(default)]
        since: Option<String>,
        /// Phase 2A-4a: when Some(true), include superseded-and-flipped preferences
        /// in the candidate set. Default (None or Some(false)) matches pre-2A-4a behavior.
        #[serde(default)]
        include_flipped: Option<bool>,
        /// Phase 2A-4d.3 T3: bench/test-only caller-provided query embedding.
        /// In production builds the handler ALWAYS ignores this field and
        /// falls back to the embedder. Under `cfg(any(test, feature = "bench"))`
        /// the handler honors `Some(v)` and passes it through to
        /// `hybrid_recall` verbatim. The field is unconditional at the struct
        /// level to avoid cfg-gated struct-literal pain across call sites.
        #[serde(default)]
        query_embedding: Option<Vec<f32>>,
    },
    Forget {
        id: String,
    },
    /// Mark old memory as superseded by a newer one. Keeps old in history, stops surfacing in context.
    Supersede {
        old_id: String,
        new_id: String,
    },
    /// Phase 2A-4a: flip a user preference's valence, preserving the original as flipped-history.
    /// Creates a new memory with `new_valence` and marks the old as superseded with
    /// `valence_flipped_at` set to the flip timestamp.
    FlipPreference {
        memory_id: String,
        new_valence: String, // "positive" | "negative" | "neutral"
        new_intensity: f64,  // 0.0..=1.0
        #[serde(default)]
        reason: Option<String>,
    },
    /// Phase 2A-4a: list preferences whose valence was flipped (i.e. superseded via FlipPreference).
    ListFlipped {
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Phase 2A-4b: reaffirm an existing preference's recency anchor.
    /// Sets `reaffirmed_at = now_iso()`. Validates memory_type='preference',
    /// status='active', cross-org. TOCTOU-safe via in-SQL preconditions and
    /// RETURNING + discriminating SELECT on 0-row result.
    ReaffirmPreference {
        memory_id: String,
    },
    /// Phase 2A-4c1: record a tool invocation for a session.
    /// Stores tool_name, args, result summary, success flag, and optional
    /// user correction flag. session_id is REQUIRED (target-session org safety, spec §10).
    RecordToolUse {
        session_id: String,
        agent: String,
        tool_name: String,
        #[serde(default = "default_empty_args")]
        tool_args: serde_json::Value,
        #[serde(default)]
        tool_result_summary: String,
        success: bool,
        #[serde(default)]
        user_correction_flag: bool,
    },
    /// Phase 2A-4c1: list tool calls for a session, optionally filtered by agent.
    /// session_id is REQUIRED (target-session org safety, spec §10).
    ListToolCalls {
        session_id: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// Phase 2A-4b (bench/test only): compute the post-RRF recency multiplier
    /// for a memory. Bypasses BM25/vector/RRF/graph for direct formula testing
    /// in 2A-4d Dim 6a.
    #[cfg(any(test, feature = "bench"))]
    ComputeRecencyFactor {
        memory_id: String,
    },
    /// Probe consolidator phase execution order (test/bench-only).
    /// Returns the phase_number (1-based doc numbering) and the list of
    /// phase fn_names that execute before `phase_name`.
    #[cfg(any(test, feature = "bench"))]
    ProbePhase {
        phase_name: String,
    },
    /// Phase 2A-4d.3.1 #2 (bench/test only): drive one disposition-worker
    /// cycle on caller-provided synthetic sessions. Bypasses the
    /// `session` table entirely so Dim 2 can deterministically push
    /// per-trait deltas without polluting the bench DB. Returns the
    /// per-trait before/after/delta state for the requested agent.
    ///
    /// Master v6 §13 D7. Used by forge-identity Dim 2.
    #[cfg(any(test, feature = "bench"))]
    StepDispositionOnce {
        agent: String,
        synthetic_sessions: Vec<SessionFixture>,
    },
    Health,
    /// Health counts grouped by project
    HealthByProject,
    Status,
    Doctor,
    /// Export all data as JSON (for visualization, backup, or sync)
    Export {
        format: Option<String>, // "json" (default) | "ndjson"
        since: Option<String>,  // timestamp filter (optional)
    },
    /// Import data from JSON (stdin or file)
    Import {
        data: String, // JSON string of exported data
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
        /// Only include events referencing this session_id
        #[serde(default)]
        session_id: Option<String>,
        /// Only include events referencing this team_id
        #[serde(default)]
        team_id: Option<String>,
    },
    /// Pre-execution guardrail check: are there decisions linked to this file?
    GuardrailsCheck {
        file: String,
        action: String,
    },
    /// Pre-bash check: warn about destructive commands, surface relevant skills/lessons
    PreBashCheck {
        command: String,
        /// Optional session_id for proactive-injection recording. When omitted,
        /// the handler falls back to `get_latest_active_session_id` (best-effort).
        /// Field added in SP1 review-fixup after the hardcoded agent="cli" lookup
        /// was found to miss Claude Code sessions (which register as "claude-code").
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Post-bash check: on failure, surface relevant lessons and skills
    PostBashCheck {
        command: String,
        exit_code: i32,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Post-edit check: surface callers, lessons, and patterns after a file edit
    PostEditCheck {
        file: String,
        #[serde(default)]
        session_id: Option<String>,
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
    /// Lightweight keep-alive ping for session liveness tracking.
    /// Updates last_heartbeat_at timestamp. Routed through WriterActor.
    SessionHeartbeat {
        session_id: String,
    },

    // ── Proactive Context (Prajna) ──
    /// Lightweight per-turn context delta check.
    /// Returns only NEW notifications, anti-pattern warnings, and pending messages since `since`.
    ContextRefresh {
        session_id: String,
        #[serde(default)]
        since: Option<String>,
    },
    /// Check if agent claimed completion — daemon recalls relevant testing/shipping lessons.
    CompletionCheck {
        session_id: String,
        claimed_done: bool,
    },
    /// Verify task completion criteria when a task is marked done.
    TaskCompletionCheck {
        session_id: String,
        task_subject: String,
        #[serde(default)]
        task_description: Option<String>,
    },
    /// Context injection observability stats.
    ContextStats {
        #[serde(default)]
        session_id: Option<String>,
    },

    /// Mark a session as ended
    EndSession {
        id: String,
    },
    /// List sessions
    Sessions {
        active_only: Option<bool>,
    },
    /// Cleanup sessions: end sessions matching optional prefix and/or age filter.
    /// If prefix is None AND older_than_secs is None, ends ALL active sessions (nuclear option).
    CleanupSessions {
        prefix: Option<String>,
        /// End sessions older than this many seconds. Also prunes ended sessions past this age.
        #[serde(default)]
        older_than_secs: Option<u64>,
        /// If true, also delete (not just end) sessions that are already ended and past the age threshold.
        #[serde(default)]
        prune_ended: bool,
    },
    /// Query which language servers are available for the current project
    LspStatus,

    /// Run proactive checks on a file or show all active diagnostics
    Verify {
        file: Option<String>,
    },
    /// Show cached diagnostics for a file
    GetDiagnostics {
        file: String,
    },

    // ── Manas Layer Operations ──
    /// Store a platform key-value pair (Layer 0)
    StorePlatform {
        key: String,
        value: String,
    },
    /// List all platform entries (Layer 0)
    ListPlatform,
    /// Store a tool (Layer 1)
    StoreTool {
        tool: crate::types::manas::Tool,
    },
    /// List all tools (Layer 1)
    ListTools,
    /// Store a perception (Layer 4)
    StorePerception {
        perception: crate::types::manas::Perception,
    },
    /// List unconsumed perceptions (Layer 4)
    ListPerceptions {
        project: Option<String>,
        limit: Option<usize>,
        #[serde(default)]
        offset: Option<usize>,
    },
    /// Consume (mark as read) perceptions by ID (Layer 4)
    ConsumePerceptions {
        ids: Vec<String>,
    },
    /// Store an identity facet (Layer 6 — Ahankara)
    StoreIdentity {
        facet: crate::types::manas::IdentityFacet,
    },
    /// List identity facets for an agent (Layer 6)
    ListIdentity {
        agent: String,
    },
    /// Deactivate an identity facet (Layer 6)
    DeactivateIdentity {
        id: String,
    },
    /// List disposition traits for an agent (Layer 7)
    ListDisposition {
        agent: String,
    },
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
        /// Valid names: "decisions", "lessons", "skills", "perceptions", "working_set", "active_sessions",
        /// "agents" (2A-4a), "preferences_flipped" (2A-4a),
        /// "preferences" (2A-4b NEW — listed here ahead of time; the <preferences> section in
        /// compile_dynamic_suffix does not exist yet and will be wired in T13 of 2A-4b).
        /// Excluded layers emit empty self-closing tags to maintain XML structure stability for KV-cache.
        #[serde(default)]
        excluded_layers: Option<Vec<String>>,
        /// Session ID for role-context, pending-messages, meeting-context injection
        #[serde(default)]
        session_id: Option<String>,
        /// Focus topic: when set, filters context to memories semantically related to this topic.
        /// Uses FTS5 MATCH to restrict decisions, lessons, skills, etc. to relevant results only.
        #[serde(default)]
        focus: Option<String>,
    },

    /// Compile context with full trace of considered/included/excluded memories + reasons.
    /// Used for debugging and visualization of the context assembly process.
    CompileContextTrace {
        agent: Option<String>,
        project: Option<String>,
        /// Optional session id. When provided, per-scope `context_injection`
        /// overrides (org / team / user / reality / agent / session) are
        /// resolved against the session and applied to the trace surface,
        /// matching the behavior of `Request::CompileContext`. Required to
        /// reflect session-scoped overrides set via
        /// `forge-next config set ... --scope session=<id>`.
        ///
        /// P3-2 W1 (was Tier 3 review M3): closes the trace/compile parity
        /// gap noted in `recall::compile_context_trace`.
        #[serde(default)]
        session_id: Option<String>,
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
        provider: String,      // "ollama", "claude", "claude_api", "openai", "gemini"
        model: Option<String>, // override model, or use default for provider
        text: String,          // conversation text to extract from
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
        layer: Option<String>, // filter by layer name, or None for all
        limit: Option<usize>,  // max nodes per layer (default 50)
    },

    /// Batch recall — multiple queries in single request (eliminates N+1 for sidebar)
    BatchRecall {
        queries: Vec<RecallQuery>,
    },

    // ── A2A Inter-Session Protocol (FISP) ──
    /// Send a message to another session (notification or request)
    SessionSend {
        #[serde(alias = "to_session")]
        to: String, // session ID or "*" for broadcast
        kind: String, // "notification" or "request"
        topic: String,
        parts: Vec<MessagePart>,
        project: Option<String>,
        timeout_secs: Option<u64>,
        /// If set, this message is a response to a meeting question.
        /// The daemon auto-records it as a meeting participant response.
        meeting_id: Option<String>,
        /// Sender session ID — used as from_session in the message.
        /// When absent, defaults to "api".
        #[serde(default)]
        from_session: Option<String>,
    },
    /// Respond to a received request
    SessionRespond {
        message_id: String,
        status: String, // "accepted", "rejected", "completed", "failed"
        parts: Vec<MessagePart>,
    },
    /// Get pending messages for a session
    SessionMessages {
        session_id: String,
        status: Option<String>,
        limit: Option<usize>,
        #[serde(default)]
        offset: Option<usize>,
    },
    /// Mark messages as read/consumed
    SessionAck {
        message_ids: Vec<String>,
        /// If set, only ack messages addressed to this session (ownership check).
        /// If None, ack messages regardless of to_session (CLI/admin usage).
        session_id: Option<String>,
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
    RevokePermission {
        id: String,
    },
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
        kind: Option<String>, // "function", "class", "file"
        limit: Option<usize>,
    },

    /// List all known realities (projects) in an organization.
    ListRealities {
        organization_id: Option<String>,
    },

    /// Force-trigger the code indexer and return current index counts.
    /// When `path` is provided, indexes that specific directory instead of
    /// the daemon's primary workspace. This enables multi-project indexing.
    ForceIndex {
        /// Optional directory to index. If None, re-processes existing indexed files.
        #[serde(default)]
        path: Option<String>,
    },

    // ── Contradictions ──
    /// List detected contradictions between active memories.
    ListContradictions {
        #[serde(default)]
        status: Option<String>, // "unresolved" | "resolved" | None (all)
        #[serde(default)]
        limit: Option<usize>,
    },

    /// Resolve a contradiction by choosing a winner or providing a synthesis.
    ResolveContradiction {
        /// The contradiction edge ID (e.g., "edge-contradiction-{id_a}-{id_b}")
        contradiction_id: String,
        /// Which memory wins ("a" or "b"), or "synthesize" for auto-resolution
        resolution: String,
    },

    // ── Agent Teams ──
    /// Create a reusable agent template (CTO, CMO, etc.)
    CreateAgentTemplate {
        name: String,
        description: String,
        agent_type: String,
        organization_id: Option<String>,
        system_context: Option<String>,
        identity_facets: Option<String>,
        config_overrides: Option<String>,
        knowledge_domains: Option<String>,
        decision_style: Option<String>,
    },
    /// List agent templates, optionally filtered by organization
    ListAgentTemplates {
        organization_id: Option<String>,
        limit: Option<usize>,
    },
    /// Get a single agent template by ID or name
    GetAgentTemplate {
        id: Option<String>,
        name: Option<String>,
    },
    /// Delete an agent template
    DeleteAgentTemplate {
        id: String,
    },
    /// Update fields on an agent template
    UpdateAgentTemplate {
        id: String,
        name: Option<String>,
        description: Option<String>,
        system_context: Option<String>,
        identity_facets: Option<String>,
        config_overrides: Option<String>,
        knowledge_domains: Option<String>,
        decision_style: Option<String>,
    },

    /// Spawn an agent from a template — creates session, sets identity, joins team
    SpawnAgent {
        template_name: String,
        session_id: String,
        project: Option<String>,
        team: Option<String>,
    },

    /// Run a full team: create team + spawn all agents from templates as a unit.
    /// On any spawn failure, rolls back all already-spawned agents.
    RunTeam {
        team_name: String,
        template_names: Vec<String>,
        /// Optional topology: "star", "mesh", "chain" (default: "mesh")
        #[serde(default)]
        topology: Option<String>,
        /// Goal ancestry: traces this team's work to a project mission
        #[serde(default)]
        goal: Option<String>,
        /// Project scope to assign to each spawned agent's session.project.
        /// When `None`, agents inherit the daemon's working-directory project
        /// (or `"(none)"` if unset). W26 (F8).
        #[serde(default)]
        project: Option<String>,
    },
    /// Stop a running team: retire all agents, end all sessions.
    StopTeam {
        team_name: String,
    },
    /// List pre-built team templates (seeded on boot).
    ListTeamTemplates,
    /// List active agents (sessions with template_id set)
    ListAgents {
        team: Option<String>,
        limit: Option<usize>,
    },
    /// Manually update an agent's status
    UpdateAgentStatus {
        session_id: String,
        status: String,
        current_task: Option<String>,
    },
    /// Retire an agent (soft delete — preserves memories)
    RetireAgent {
        session_id: String,
    },

    /// Create a team with type (human/agent/mixed)
    CreateTeam {
        name: String,
        team_type: Option<String>,
        purpose: Option<String>,
        organization_id: Option<String>,
    },
    /// List members of a team (including agent sessions)
    ListTeamMembers {
        team_name: String,
    },
    /// Set the orchestrator session for a team
    SetTeamOrchestrator {
        team_name: String,
        session_id: String,
    },
    /// Get full team status (members, meetings, decisions)
    TeamStatus {
        team_name: String,
        #[serde(default)]
        team_id: Option<String>,
    },

    // ── Organization Hierarchy ──
    /// Create a new organization
    CreateOrganization {
        name: String,
        #[serde(default)]
        description: Option<String>,
    },
    /// List all organizations
    ListOrganizations,
    /// Send a message to all members of a team (optionally recursive to sub-teams)
    TeamSend {
        team_name: String,
        kind: String,
        topic: String,
        parts: Vec<MessagePart>,
        #[serde(default)]
        from_session: Option<String>,
        #[serde(default)]
        recursive: bool,
    },
    /// Get team hierarchy tree
    TeamTree {
        #[serde(default)]
        organization_id: Option<String>,
    },
    /// Create an organization from a predefined template
    CreateOrgFromTemplate {
        template_name: String,
        org_name: String,
    },

    // ── Meeting Protocol ──
    /// Create a meeting — sends FISP messages to all participants
    CreateMeeting {
        team_id: String,
        topic: String,
        context: Option<String>,
        orchestrator_session_id: String,
        participant_session_ids: Vec<String>,
        /// Goal ancestry: traces this meeting to a project mission
        #[serde(default)]
        goal: Option<String>,
    },
    /// Get meeting status + participant response statuses
    MeetingStatus {
        meeting_id: String,
    },
    /// Get all participant responses for a meeting
    MeetingResponses {
        meeting_id: String,
    },
    /// Store orchestrator synthesis
    MeetingSynthesize {
        meeting_id: String,
        synthesis: String,
    },
    /// Record decision, store as memory, close meeting
    MeetingDecide {
        meeting_id: String,
        decision: String,
    },
    /// List meetings for a team
    ListMeetings {
        team_id: Option<String>,
        status: Option<String>,
        limit: Option<usize>,
    },
    /// Full meeting transcript (topic + context + responses + synthesis + decision)
    MeetingTranscript {
        meeting_id: String,
    },
    /// Directly record a meeting participant's response (alternative to FISP side-effect)
    RecordMeetingResponse {
        meeting_id: String,
        session_id: String,
        response: String,
        confidence: Option<f64>,
    },

    /// Cast a vote in a meeting with structured voting options
    MeetingVote {
        meeting_id: String,
        session_id: String,
        choice: String,
    },
    /// Get vote results for a meeting (vote counts, quorum status, outcome)
    MeetingResult {
        meeting_id: String,
    },

    // ── Notification Engine ──
    /// List notifications with optional filters
    ListNotifications {
        status: Option<String>,
        category: Option<String>,
        limit: Option<usize>,
    },
    /// Acknowledge a notification
    AckNotification {
        id: String,
    },
    /// Dismiss a notification
    DismissNotification {
        id: String,
    },
    /// Act on a confirmation notification (approve or reject)
    ActOnNotification {
        id: String,
        approved: bool,
    },

    // ── Memory Self-Healing ──
    /// Get healing status (metrics, last cycle, stale candidates)
    HealingStatus,
    /// Trigger a manual healing cycle
    HealingRun,
    /// Get healing log entries
    HealingLog {
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        action: Option<String>,
    },

    // ── Workspace ──
    /// Initialize workspace directories for an organization
    WorkspaceInit {
        org_name: String,
        #[serde(default)]
        template: Option<String>,
    },
    /// Get current workspace status (mode, paths, org info)
    WorkspaceStatus,

    /// Backfill project field on memories that have project = NULL or empty.
    /// Derives project from the session registry (sessions have a project field)
    /// and from the transcript_log table.
    BackfillProject,

    /// Cleanup garbage memories, normalize project names, and purge duplicate perceptions.
    /// One-time data quality fix that should be run after upgrading to v0.7.1+.
    CleanupMemory,

    /// Update the current_task on a session (session card auto-populate).
    /// This is a lightweight update — no re-registration needed.
    SetCurrentTask {
        session_id: String,
        task: String,
    },

    /// Get current license tier and key status.
    LicenseStatus,

    /// Set or update the license tier and key.
    SetLicense {
        tier: String,
        key: String,
    },

    // ── Skills Registry ──
    /// List skills from the registry with optional category filter and FTS5 search
    SkillsList {
        category: Option<String>,
        search: Option<String>,
        limit: Option<usize>,
    },
    /// Install a skill for a project
    SkillsInstall {
        name: String,
        project: String,
    },
    /// Uninstall a skill from a project
    SkillsUninstall {
        name: String,
        project: String,
    },
    /// Get full details of a skill by name
    SkillsInfo {
        name: String,
    },
    /// Re-index the skills directory (pick up new/changed/deleted skills)
    SkillsRefresh,

    /// Smart Model Router: query routing decisions and token savings
    RoutingStats,

    // ── Per-Agent Budget Enforcement ──
    /// Record a cost against an agent session's budget
    RecordAgentCost {
        session_id: String,
        amount: f64,
        description: String,
    },
    /// Query budget status for agent sessions
    BudgetStatus {
        #[serde(default)]
        session_id: Option<String>,
    },

    /// Run database vacuum: purge faded memories, cleanup orphan code entries, then VACUUM.
    VacuumDb,

    /// Backfill affects edges on existing decision/lesson memories by scanning their content/title
    /// for file path patterns and creating affects edges to matched files.
    BackfillAffects,

    /// Query code symbols by name (Serena find_symbol replacement).
    FindSymbol {
        name: String,
        /// Optional file path filter
        file: Option<String>,
    },

    /// Get all symbols in a file (Serena get_symbols_overview replacement).
    GetSymbolsOverview {
        file: String,
    },

    /// Get merged HUD configuration (cascade: org → team → project → user).
    /// Returns the effective config with provenance for each field.
    GetHudConfig {
        user_id: Option<String>,
        team_id: Option<String>,
        organization_id: Option<String>,
        project: Option<String>,
    },

    /// Set a HUD configuration value at a specific scope.
    /// Keys must start with "hud." — validated by handler.
    SetHudConfig {
        scope_type: String, // "organization" | "team" | "project" | "user"
        scope_id: String,
        key: String, // e.g. "hud.sections", "hud.density", "hud.theme"
        value: String,
        locked: bool,
    },

    /// Export HUD configuration as TOML for committing to .forge/hud.toml
    ExportHudConfig {
        scope_type: String,
        scope_id: String,
    },

    // ── Raw layer (benchmark parity + verbatim retrieval) ──
    //
    // Sit alongside the extraction pipeline; never block on LLMs. See
    // docs/benchmarks/plan.md §4 for the full design.
    /// Ingest a block of text into the raw storage layer: chunks it,
    /// embeds every chunk via the shared MiniLM embedder, and stores both
    /// the verbatim chunks and their vectors in one SQLite transaction.
    RawIngest {
        /// The text to ingest. No preprocessing; stored verbatim.
        text: String,
        /// Project scope — mirrors `project` on extracted memories.
        #[serde(default)]
        project: Option<String>,
        /// Session scope — groups chunks from the same conversation.
        #[serde(default)]
        session_id: Option<String>,
        /// Where the text came from (e.g. `"claude-code"`, `"bench:longmemeval"`).
        source: String,
        /// Optional ISO-8601 timestamp override. Defaults to `now()`.
        #[serde(default)]
        timestamp: Option<String>,
        /// Arbitrary structured metadata serialized as JSON.
        #[serde(default)]
        metadata: Option<serde_json::Value>,
    },
    /// Search the raw storage layer via KNN on MiniLM embeddings.
    RawSearch {
        query: String,
        #[serde(default)]
        project: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        /// Top-K to return. Defaults to 50 (the published benchmark default).
        #[serde(default)]
        k: Option<usize>,
        /// Cosine-distance cutoff; results with distance > this are dropped.
        /// Defaults to 0.6 (MemPalace's empirical LongMemEval threshold).
        #[serde(default)]
        max_distance: Option<f64>,
    },
    /// List raw documents filtered by their `source` tag.
    ///
    /// Primary caller is the Forge-Persist benchmark harness, which uses a
    /// per-run source string to enumerate the documents it ingested pre-kill
    /// and verify they survived a restart. Returns verbatim document text so
    /// callers can compute content hashes client-side.
    RawDocumentsList {
        source: String,
        /// Maximum rows to return. Omitted → daemon default (10 000).
        #[serde(default)]
        limit: Option<usize>,
    },

    /// Runtime version and build metadata. Lightweight (no DB queries).
    Version,

    /// Phase 2A-4d.2: Observability API. Queries `kpi_events` or the per-layer
    /// gauge snapshot through one shape-parameterized RPC.
    Inspect {
        shape: crate::protocol::InspectShape,
        #[serde(default = "crate::protocol::default_inspect_window")]
        window: String,
        #[serde(default)]
        filter: crate::protocol::InspectFilter,
        #[serde(default)]
        group_by: Option<crate::protocol::InspectGroupBy>,
    },

    Shutdown,
}
