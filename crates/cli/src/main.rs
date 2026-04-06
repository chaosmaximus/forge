mod client;
mod commands;
mod transport;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge-next", about = "Forge — memory for AI coding agents")]
struct Cli {
    /// Remote daemon endpoint (e.g., https://forge.company.com).
    /// Overrides FORGE_ENDPOINT env var. Omit for local Unix socket.
    #[arg(long, global = true)]
    endpoint: Option<String>,

    /// JWT auth token for remote connections.
    /// Overrides FORGE_TOKEN env var.
    #[arg(long, global = true)]
    token: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Search memories (hybrid BM25 + vector + graph)
    Recall {
        /// The search query
        query: String,
        /// Filter by memory type (decision, lesson, pattern, preference)
        #[arg(long)]
        r#type: Option<String>,
        /// Filter by project (global memories always included)
        #[arg(long)]
        project: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Filter by Manas layer (experience, declared, domain_dna, skill, perception, identity)
        #[arg(long)]
        layer: Option<String>,
    },
    /// Store a memory
    Remember {
        /// Memory type (decision, lesson, pattern, preference)
        #[arg(long)]
        r#type: String,
        /// Memory title
        #[arg(long)]
        title: String,
        /// Memory content
        #[arg(long)]
        content: String,
        /// Confidence score (0.0 to 1.0)
        #[arg(long)]
        confidence: Option<f64>,
        /// Tags (comma-separated)
        #[arg(long, value_delimiter = ',')]
        tags: Option<Vec<String>>,
        /// Project name
        #[arg(long)]
        project: Option<String>,
    },
    /// Soft-delete a memory
    Forget {
        /// Memory ID to forget
        id: String,
    },
    /// Daemon management
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// System health
    Health,
    /// Memory counts grouped by project
    #[command(name = "health-by-project")]
    HealthByProject,
    /// Daemon health diagnostics
    Doctor,
    /// Import v1 cache.json into daemon
    Migrate {
        /// Path to v1 state directory containing cache.json
        state_dir: String,
    },
    /// Export all data as JSON (for visualization, backup, or sync)
    Export {
        /// Output format: json (default) or ndjson
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Import data from JSON (stdin or file)
    Import {
        /// File to import (reads stdin if not specified)
        #[arg(long)]
        file: Option<String>,
    },
    /// Ingest Claude Code's MEMORY.md files into Forge
    IngestClaude,
    /// Backfill: re-process a transcript file from scratch
    Backfill {
        /// Path to transcript file
        path: String,
    },
    /// Pre-execution guardrail check
    Check {
        /// File path to check
        #[arg(long)]
        file: String,
        /// Action type: edit, delete, or rename
        #[arg(long, default_value = "edit")]
        action: String,
    },
    /// Post-edit check — surface callers, lessons, and warnings after editing a file
    #[command(name = "post-edit-check")]
    PostEditCheck {
        /// File path that was edited
        #[arg(long)]
        file: String,
    },
    /// Pre-bash check — warn about destructive commands, surface relevant skills/lessons
    #[command(name = "pre-bash-check")]
    PreBashCheck {
        /// The bash command to check
        #[arg(long)]
        command: String,
    },
    /// Post-bash check — surface lessons and skills after command failure
    #[command(name = "post-bash-check")]
    PostBashCheck {
        /// The bash command that was run
        #[arg(long)]
        command: String,
        /// Exit code of the command (default: 1)
        #[arg(long, default_value = "1")]
        exit_code: i32,
    },
    /// Blast radius analysis for a file
    #[command(name = "blast-radius")]
    BlastRadius {
        /// File path to analyze
        #[arg(long)]
        file: String,
    },
    /// List active agent sessions
    Sessions {
        /// Show all sessions (including ended)
        #[arg(long)]
        all: bool,
    },
    /// Show available language servers for the current project
    #[command(name = "lsp-status")]
    LspStatus,
    /// Show Manas 8-layer memory health
    #[command(name = "manas-health")]
    ManasHealth,
    /// Manage agent identity (Ahankara)
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },
    /// Show platform information (Layer 1)
    Platform,
    /// List discovered tools (Layer 2)
    Tools,
    /// List unconsumed perceptions (Layer 6)
    Perceptions {
        /// Filter by project
        #[arg(long)]
        project: Option<String>,
        /// Maximum results
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Compile optimized context from all Manas layers (for session-start)
    #[command(name = "compile-context")]
    CompileContext {
        /// Agent name (default: claude-code)
        #[arg(long, default_value = "claude-code")]
        agent: String,
        /// Project name
        #[arg(long)]
        project: Option<String>,
        /// Only return the static prefix (platform, identity, disposition, tools).
        /// Useful for caching the stable part for KV-cache optimization.
        #[arg(long)]
        static_only: bool,
        /// Session ID for role-context, pending-messages, meeting-context injection
        #[arg(long)]
        session: Option<String>,
    },
    /// Register an active agent session
    #[command(name = "register-session")]
    RegisterSession {
        /// Session ID (e.g., UUID)
        #[arg(long)]
        id: String,
        /// Agent name (claude-code, cline, codex, etc.)
        #[arg(long)]
        agent: String,
        /// Project name
        #[arg(long)]
        project: Option<String>,
        /// Working directory
        #[arg(long)]
        cwd: Option<String>,
    },
    /// End an active agent session
    #[command(name = "end-session")]
    EndSession {
        /// Session ID to end
        #[arg(long)]
        id: String,
    },
    /// Cleanup sessions: end all active sessions matching an optional prefix
    #[command(name = "cleanup-sessions")]
    CleanupSessions {
        /// Only end sessions whose ID starts with this prefix (e.g., "hook-test")
        #[arg(long)]
        prefix: Option<String>,
    },

    // ── A2A Inter-Session Messaging ──

    /// Send a message to another session
    #[command(name = "send")]
    Send {
        /// Target session ID (or "*" for broadcast)
        #[arg(long)]
        to: String,
        /// Message kind: "notification" or "request"
        #[arg(long)]
        kind: String,
        /// Topic (e.g., "file_changed", "review_code")
        #[arg(long)]
        topic: String,
        /// Message text
        #[arg(long)]
        text: String,
        /// Project scope (for broadcasts)
        #[arg(long)]
        project: Option<String>,
        /// Timeout in seconds (for requests)
        #[arg(long)]
        timeout: Option<u64>,
    },
    /// Get pending messages for a session
    #[command(name = "messages")]
    Messages {
        /// Session ID to check inbox for
        #[arg(long)]
        session: String,
        /// Filter by status (pending, read, completed)
        #[arg(long)]
        status: Option<String>,
        /// Max messages to return
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Acknowledge (mark as read) messages by ID
    #[command(name = "ack")]
    Ack {
        /// Message IDs to acknowledge
        ids: Vec<String>,
    },

    // ── A2A Permission Management ──

    /// Grant A2A permission (from agent → to agent)
    #[command(name = "grant-permission")]
    GrantPermission {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        from_project: Option<String>,
        #[arg(long)]
        to_project: Option<String>,
    },
    /// Revoke an A2A permission by ID
    #[command(name = "revoke-permission")]
    RevokePermission {
        #[arg(long)]
        id: String,
    },
    /// List all A2A permissions
    #[command(name = "list-permissions")]
    ListPermissions,

    // ── Knowledge Intelligence ──

    /// List detected entities (recurring concepts in project memories)
    #[command(name = "entities")]
    Entities {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show context compilation trace (which memories included/excluded and why)
    #[command(name = "context-trace")]
    ContextTrace {
        #[arg(long, default_value = "claude-code")]
        agent: String,
        #[arg(long)]
        project: Option<String>,
    },

    /// Detect the reality (project type) for a path
    #[command(name = "detect-reality")]
    DetectReality {
        /// Path to detect (defaults to current directory)
        #[arg(long)]
        path: Option<String>,
    },
    /// List all known realities (projects)
    #[command(name = "realities")]
    Realities {
        /// Organization ID (default: "default")
        #[arg(long)]
        organization: Option<String>,
    },
    /// Search code symbols by name pattern
    #[command(name = "code-search")]
    CodeSearch {
        /// Search query (symbol name pattern)
        query: String,
        /// Filter by symbol kind: function, class, file
        #[arg(long)]
        kind: Option<String>,
        /// Maximum number of results
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Force-trigger the code indexer and show current index counts
    #[command(name = "force-index")]
    ForceIndex,

    /// Run proactive checks on a file or show all active diagnostics
    Verify {
        /// File to check (omit to show all active diagnostics)
        #[arg(long)]
        file: Option<String>,
    },
    /// Show cached diagnostics for a file
    Diagnostics {
        /// File path to query diagnostics for
        #[arg(long)]
        file: String,
    },

    // ── Sync Commands ──

    /// Export memories as NDJSON with HLC metadata (for sync)
    #[command(name = "sync-export")]
    SyncExport {
        /// Filter by project
        #[arg(long)]
        project: Option<String>,
        /// Only export memories with HLC timestamp after this value
        #[arg(long)]
        since: Option<String>,
    },
    /// Import NDJSON memory lines from stdin (for sync)
    #[command(name = "sync-import")]
    SyncImport,
    /// Pull memories from a remote host via SSH
    #[command(name = "sync-pull")]
    SyncPull {
        /// Remote host (SSH destination, e.g. user@host)
        host: String,
        /// Filter by project
        #[arg(long)]
        project: Option<String>,
    },
    /// Push memories to a remote host via SSH
    #[command(name = "sync-push")]
    SyncPush {
        /// Remote host (SSH destination, e.g. user@host)
        host: String,
        /// Filter by project
        #[arg(long)]
        project: Option<String>,
    },
    /// List unresolved sync conflicts
    #[command(name = "sync-conflicts")]
    SyncConflicts,
    /// Resolve a sync conflict by keeping the given memory ID
    #[command(name = "sync-resolve")]
    SyncResolve {
        /// Memory ID to keep
        id: String,
    },

    /// Backfill HLC timestamps on existing memories that have empty hlc_timestamp
    #[command(name = "hlc-backfill")]
    HlcBackfill,

    /// Bootstrap: scan and process all existing transcript files
    #[command(name = "bootstrap")]
    Bootstrap {
        /// Only process transcripts for this project
        #[arg(long)]
        project: Option<String>,
    },
    /// Force-run all consolidation phases (dedup, decay, promotion, etc.)
    Consolidate,
    /// Trigger extraction on pending transcripts
    #[command(name = "extract")]
    Extract {
        /// Force extraction immediately, skipping debounce
        #[arg(long)]
        force: bool,
    },
    /// View or update daemon configuration
    #[command(name = "config")]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Show extraction metrics, token usage, and cost tracking
    #[command(name = "stats")]
    Stats {
        /// Time period in hours (default: 24)
        #[arg(long, default_value = "24")]
        hours: u64,
    },
    /// Manage the daemon as a system service (install, start, stop, status)
    #[command(name = "service")]
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },

    // ── Agent Teams ──

    /// Manage agent templates
    #[command(name = "agent-template")]
    AgentTemplate {
        #[command(subcommand)]
        action: AgentTemplateAction,
    },

    /// Spawn an agent from a template
    #[command(name = "agent")]
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },

    /// List active agents
    #[command(name = "agents")]
    Agents {
        /// Filter by team
        #[arg(long)]
        team: Option<String>,
    },

    /// Update an agent's status
    #[command(name = "agent-status")]
    AgentStatus {
        /// Session ID of the agent
        #[arg(long)]
        session: String,
        /// New status (idle, thinking, responding, in_meeting, error)
        #[arg(long)]
        status: String,
        /// Current task description
        #[arg(long)]
        task: Option<String>,
    },

    // ── Teams ──

    /// Manage teams
    #[command(name = "team")]
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },

    // ── Meetings ──

    /// Manage meetings
    #[command(name = "meeting")]
    Meeting {
        #[command(subcommand)]
        action: MeetingAction,
    },

    // ── Notifications ──

    /// List notifications
    #[command(name = "notifications")]
    Notifications {
        /// Filter by status (pending, acknowledged, dismissed)
        #[arg(long)]
        status: Option<String>,
        /// Filter by category (alert, insight, confirmation, progress)
        #[arg(long)]
        category: Option<String>,
        /// Maximum results
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Acknowledge a notification
    #[command(name = "ack-notification")]
    AckNotification {
        /// Notification ID
        id: String,
    },
    /// Dismiss a notification
    #[command(name = "dismiss-notification")]
    DismissNotification {
        /// Notification ID
        id: String,
    },
    /// Act on a confirmation notification
    #[command(name = "act-notification")]
    ActNotification {
        /// Notification ID
        #[arg(long)]
        id: String,
        /// Approve the action
        #[arg(long, conflicts_with = "reject")]
        approve: bool,
        /// Reject the action
        #[arg(long, conflicts_with = "approve")]
        reject: bool,
    },

    // ── Streaming & Heartbeat ──

    /// Subscribe to real-time daemon events (streams NDJSON to stdout)
    Subscribe {
        /// Filter event types (comma-separated)
        #[arg(long, value_delimiter = ',')]
        events: Option<Vec<String>>,
        /// Filter by session ID
        #[arg(long)]
        session: Option<String>,
        /// Filter by team ID
        #[arg(long)]
        team: Option<String>,
    },

    /// Send a heartbeat to keep a session alive
    #[command(name = "session-heartbeat")]
    SessionHeartbeat {
        /// Session ID to heartbeat
        #[arg(long)]
        session: String,
    },

    // ── Proactive Context (Prajna) ──

    /// Per-turn context delta check (used by UserPromptSubmit hook)
    #[command(name = "context-refresh")]
    ContextRefresh {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        since: Option<String>,
    },

    /// Check for premature completion signals (used by Stop hook)
    #[command(name = "completion-check")]
    CompletionCheck {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        claimed_done: bool,
    },

    /// Verify task completion criteria (used by TaskCompleted hook)
    #[command(name = "task-completion-check")]
    TaskCompletionCheck {
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        subject: String,
        #[arg(long)]
        description: Option<String>,
    },

    /// Context injection observability — token cost, effectiveness, per-hook breakdown
    #[command(name = "context-stats")]
    ContextStats {
        /// Session ID (omit for global stats across all sessions)
        #[arg(long)]
        session_id: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigAction {
    /// Display current daemon configuration
    Show,
    /// Update a config value (dotted key, e.g., extraction.backend)
    Set {
        /// Config key (e.g., extraction.backend, extraction.ollama.model)
        key: String,
        /// New value
        value: String,
    },
    /// Set a scoped config value at a specific scope level
    SetScoped {
        /// Scope type: organization, team, user, reality, agent, session
        #[arg(long)]
        scope: String,
        /// Scope entity ID
        #[arg(long, name = "scope-id")]
        scope_id: String,
        /// Config key (e.g., context.budget_chars)
        #[arg(long)]
        key: String,
        /// Config value
        #[arg(long)]
        value: String,
        /// Lock this value (prevent lower scopes from overriding)
        #[arg(long)]
        locked: bool,
        /// Set a ceiling for numeric values
        #[arg(long)]
        ceiling: Option<f64>,
    },
    /// Get the effective (resolved) config for a session context
    GetEffective {
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        reality: Option<String>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        team: Option<String>,
        #[arg(long)]
        organization: Option<String>,
    },
    /// List all scoped config entries for a scope
    ListScoped {
        #[arg(long)]
        scope: String,
        #[arg(long, name = "scope-id")]
        scope_id: String,
    },
    /// Delete a scoped config entry
    DeleteScoped {
        #[arg(long)]
        scope: String,
        #[arg(long, name = "scope-id")]
        scope_id: String,
        #[arg(long)]
        key: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ServiceAction {
    /// Install forge-daemon as a system service (systemd on Linux, launchd on macOS)
    Install,
    /// Start the daemon service
    Start,
    /// Stop the daemon service
    Stop,
    /// Show daemon service status
    Status,
    /// Uninstall the daemon service
    Uninstall,
}

#[derive(Subcommand, Debug)]
enum AgentTemplateAction {
    /// Create a reusable agent template
    Create {
        /// Template name (e.g., CTO, CMO)
        #[arg(long)]
        name: String,
        /// Description of the agent's role
        #[arg(long)]
        description: String,
        /// Agent type (e.g., claude-code, cline)
        #[arg(long, name = "agent-type")]
        agent_type: String,
        /// System context / prompt
        #[arg(long, name = "system-context")]
        system_context: Option<String>,
        /// Identity facets as JSON array
        #[arg(long, name = "identity-facets")]
        identity_facets: Option<String>,
        /// Config overrides as JSON object
        #[arg(long, name = "config-overrides")]
        config_overrides: Option<String>,
        /// Knowledge domains as JSON array
        #[arg(long, name = "knowledge-domains")]
        knowledge_domains: Option<String>,
        /// Decision style (analytical, intuitive, consensus, directive)
        #[arg(long, name = "decision-style")]
        decision_style: Option<String>,
    },
    /// List agent templates
    List {
        /// Filter by organization ID
        #[arg(long)]
        org: Option<String>,
    },
    /// Get a single agent template
    Get {
        /// Template name
        #[arg(long)]
        name: Option<String>,
        /// Template ID
        #[arg(long)]
        id: Option<String>,
    },
    /// Delete an agent template
    Delete {
        /// Template ID to delete
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum AgentAction {
    /// Spawn an agent from a template
    Spawn {
        /// Template name to spawn from
        #[arg(long)]
        template: String,
        /// Session ID for the new agent
        #[arg(long, name = "session-id")]
        session_id: String,
        /// Project scope
        #[arg(long)]
        project: Option<String>,
        /// Team to join
        #[arg(long)]
        team: Option<String>,
    },
    /// Retire an agent (soft delete)
    Retire {
        /// Session ID of the agent to retire
        #[arg(long)]
        session: String,
    },
}

#[derive(Subcommand, Debug)]
enum TeamAction {
    /// Create a team
    Create {
        /// Team name
        #[arg(long)]
        name: String,
        /// Team type: human, agent, or mixed
        #[arg(long = "type")]
        team_type: Option<String>,
        /// Purpose of the team
        #[arg(long)]
        purpose: Option<String>,
    },
    /// List team members
    Members {
        /// Team name
        #[arg(long)]
        name: String,
    },
    /// Set the orchestrator session for a team
    #[command(name = "set-orchestrator")]
    SetOrchestrator {
        /// Team name
        #[arg(long)]
        name: String,
        /// Orchestrator session ID
        #[arg(long)]
        session: String,
    },
    /// Show full team status
    Status {
        /// Team name
        #[arg(long)]
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum MeetingAction {
    /// Create a meeting
    Create {
        /// Team ID
        #[arg(long)]
        team: String,
        /// Meeting topic
        #[arg(long)]
        topic: String,
        /// Additional context
        #[arg(long)]
        context: Option<String>,
        /// Orchestrator session ID
        #[arg(long)]
        orchestrator: String,
        /// Participant session IDs (comma-separated)
        #[arg(long, value_delimiter = ',')]
        participants: Vec<String>,
    },
    /// Get meeting status
    Status {
        /// Meeting ID
        #[arg(long)]
        id: String,
    },
    /// Get participant responses
    Responses {
        /// Meeting ID
        #[arg(long)]
        id: String,
    },
    /// Store orchestrator synthesis
    Synthesize {
        /// Meeting ID
        #[arg(long)]
        id: String,
        /// Synthesis text
        #[arg(long)]
        synthesis: String,
    },
    /// Record decision and close meeting
    Decide {
        /// Meeting ID
        #[arg(long)]
        id: String,
        /// Decision text
        #[arg(long)]
        decision: String,
    },
    /// List meetings
    List {
        /// Filter by team ID
        #[arg(long)]
        team: Option<String>,
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
    },
    /// Show full meeting transcript
    Transcript {
        /// Meeting ID
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand, Debug)]
enum DaemonAction {
    /// Show daemon status (uptime, memory count)
    Status,
    /// Stop the daemon
    Stop,
}

#[derive(Subcommand, Debug)]
enum IdentityAction {
    /// List identity facets
    List {
        /// Agent name (default: claude-code)
        #[arg(long, default_value = "claude-code")]
        agent: String,
    },
    /// Set an identity facet
    Set {
        /// Facet type (role, expertise, values, goals, constraints)
        #[arg(long)]
        facet: String,
        /// Description
        #[arg(long)]
        description: String,
        /// Agent name
        #[arg(long, default_value = "claude-code")]
        agent: String,
        /// Strength (0.0-1.0)
        #[arg(long, default_value = "0.5")]
        strength: f64,
    },
    /// Remove (deactivate) an identity facet
    Remove {
        /// Facet ID to deactivate
        id: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Initialize global transport from CLI flags / env vars
    let t = transport::Transport::detect(cli.endpoint.as_deref(), cli.token.as_deref());
    transport::Transport::init_global(t);

    match cli.command {
        Commands::Recall {
            query,
            r#type,
            project,
            limit,
            layer,
        } => {
            commands::memory::recall(query, r#type, project, limit, layer).await;
        }
        Commands::Remember {
            r#type,
            title,
            content,
            confidence,
            tags,
            project,
        } => {
            commands::memory::remember(r#type, title, content, confidence, tags, project).await;
        }
        Commands::Forget { id } => {
            commands::memory::forget(id).await;
        }
        Commands::Daemon { action } => match action {
            DaemonAction::Status => {
                commands::daemon::status().await;
            }
            DaemonAction::Stop => {
                commands::daemon::stop().await;
            }
        },
        Commands::Health => {
            commands::system::health().await;
        }
        Commands::HealthByProject => {
            commands::system::health_by_project().await;
        }
        Commands::Doctor => {
            commands::system::doctor().await;
        }
        Commands::Migrate { state_dir } => {
            commands::system::migrate(state_dir).await;
        }
        Commands::Export { format } => {
            commands::system::export(&format).await;
        }
        Commands::Import { file } => {
            commands::system::import(file).await;
        }
        Commands::IngestClaude => {
            commands::system::ingest_claude().await;
        }
        Commands::Backfill { path } => {
            commands::system::backfill(path).await;
        }
        Commands::Check { file, action } => {
            commands::system::check(file, action).await;
        }
        Commands::PostEditCheck { file } => {
            commands::system::post_edit_check(file).await;
        }
        Commands::PreBashCheck { command } => {
            commands::system::pre_bash_check(command).await;
        }
        Commands::PostBashCheck { command, exit_code } => {
            commands::system::post_bash_check(command, exit_code).await;
        }
        Commands::BlastRadius { file } => {
            commands::system::blast_radius(file).await;
        }
        Commands::Sessions { all } => {
            commands::system::sessions(!all).await;
        }
        Commands::LspStatus => {
            commands::system::lsp_status().await;
        }
        Commands::RegisterSession { id, agent, project, cwd } => {
            commands::system::register_session(id, agent, project, cwd).await;
        }
        Commands::EndSession { id } => {
            commands::system::end_session(id).await;
        }
        Commands::CleanupSessions { prefix } => {
            commands::system::cleanup_sessions(prefix).await;
        }
        Commands::Send { to, kind, topic, text, project, timeout } => {
            commands::system::send_message(to, kind, topic, text, project, timeout).await;
        }
        Commands::Messages { session, status, limit } => {
            commands::system::list_messages(session, status, limit).await;
        }
        Commands::Ack { ids } => {
            commands::system::ack_messages(ids).await;
        }
        Commands::GrantPermission { from, to, from_project, to_project } => {
            commands::system::grant_permission(from, to, from_project, to_project).await;
        }
        Commands::RevokePermission { id } => {
            commands::system::revoke_permission(id).await;
        }
        Commands::ListPermissions => {
            commands::system::list_permissions().await;
        }
        Commands::Entities { project, limit } => {
            commands::system::list_entities(project, limit).await;
        }
        Commands::ContextTrace { agent, project } => {
            commands::system::context_trace(agent, project).await;
        }
        Commands::CompileContext { agent, project, static_only, session } => {
            commands::manas::compile_context(agent, project, static_only, session).await;
        }
        Commands::ManasHealth => {
            commands::manas::manas_health().await;
        }
        Commands::Identity { action } => match action {
            IdentityAction::List { agent } => {
                commands::manas::identity_list(agent).await;
            }
            IdentityAction::Set {
                facet,
                description,
                agent,
                strength,
            } => {
                commands::manas::identity_set(facet, description, agent, strength).await;
            }
            IdentityAction::Remove { id } => {
                commands::manas::identity_remove(id).await;
            }
        },
        Commands::Platform => {
            commands::manas::platform().await;
        }
        Commands::Tools => {
            commands::manas::tools().await;
        }
        Commands::Perceptions { project, limit } => {
            commands::manas::perceptions(project, limit).await;
        }

        Commands::DetectReality { path } => {
            commands::system::detect_reality(path).await;
        }
        Commands::Realities { organization } => {
            commands::system::list_realities(organization).await;
        }
        Commands::CodeSearch { query, kind, limit } => {
            commands::system::code_search(query, kind, limit).await;
        }

        Commands::ForceIndex => {
            commands::system::force_index().await;
        }

        Commands::Verify { file } => {
            commands::system::verify(file).await;
        }
        Commands::Diagnostics { file } => {
            commands::system::diagnostics(file).await;
        }

        // ── Sync Commands ──
        Commands::SyncExport { project, since } => {
            commands::sync::sync_export(project, since).await;
        }
        Commands::SyncImport => {
            commands::sync::sync_import().await;
        }
        Commands::SyncPull { host, project } => {
            commands::sync::sync_pull(host, project).await;
        }
        Commands::SyncPush { host, project } => {
            commands::sync::sync_push(host, project).await;
        }
        Commands::SyncConflicts => {
            commands::sync::sync_conflicts().await;
        }
        Commands::SyncResolve { id } => {
            commands::sync::sync_resolve(id).await;
        }
        Commands::HlcBackfill => {
            commands::sync::hlc_backfill().await;
        }
        Commands::Bootstrap { project } => {
            commands::system::bootstrap(project).await;
        }
        Commands::Consolidate => {
            commands::system::consolidate().await;
        }
        Commands::Extract { force } => {
            commands::system::extract(force).await;
        }
        Commands::Config { action } => match action {
            ConfigAction::Show => {
                commands::system::config_show().await;
            }
            ConfigAction::Set { key, value } => {
                commands::system::config_set(key, value).await;
            }
            ConfigAction::SetScoped { scope, scope_id, key, value, locked, ceiling } => {
                commands::system::config_set_scoped(scope, scope_id, key, value, locked, ceiling).await;
            }
            ConfigAction::GetEffective { session, agent, reality, user, team, organization } => {
                commands::system::config_get_effective(session, agent, reality, user, team, organization).await;
            }
            ConfigAction::ListScoped { scope, scope_id } => {
                commands::system::config_list_scoped(scope, scope_id).await;
            }
            ConfigAction::DeleteScoped { scope, scope_id, key } => {
                commands::system::config_delete_scoped(scope, scope_id, key).await;
            }
        },
        Commands::Service { action } => {
            commands::system::service(action).await;
        }
        Commands::Stats { hours } => {
            commands::system::stats(hours).await;
        }

        // ── Agent Teams ──
        Commands::AgentTemplate { action } => match action {
            AgentTemplateAction::Create {
                name, description, agent_type, system_context,
                identity_facets, config_overrides, knowledge_domains, decision_style,
            } => {
                commands::teams::create_agent_template(
                    name, description, agent_type, system_context,
                    identity_facets, config_overrides, knowledge_domains, decision_style,
                ).await;
            }
            AgentTemplateAction::List { org } => {
                commands::teams::list_agent_templates(org).await;
            }
            AgentTemplateAction::Get { name, id } => {
                commands::teams::get_agent_template(id, name).await;
            }
            AgentTemplateAction::Delete { id } => {
                commands::teams::delete_agent_template(id).await;
            }
        },
        Commands::Agent { action } => match action {
            AgentAction::Spawn { template, session_id, project, team } => {
                commands::teams::spawn_agent(template, session_id, project, team).await;
            }
            AgentAction::Retire { session } => {
                commands::teams::retire_agent(session).await;
            }
        },
        Commands::Agents { team } => {
            commands::teams::list_agents(team).await;
        }
        Commands::AgentStatus { session, status, task } => {
            commands::teams::update_agent_status(session, status, task).await;
        }
        Commands::Team { action } => match action {
            TeamAction::Create { name, team_type, purpose } => {
                commands::teams::create_team(name, team_type, purpose).await;
            }
            TeamAction::Members { name } => {
                commands::teams::list_team_members(name).await;
            }
            TeamAction::SetOrchestrator { name, session } => {
                commands::teams::set_team_orchestrator(name, session).await;
            }
            TeamAction::Status { name } => {
                commands::teams::team_status(name).await;
            }
        },
        Commands::Meeting { action } => match action {
            MeetingAction::Create { team, topic, context, orchestrator, participants } => {
                commands::teams::create_meeting(team, topic, context, orchestrator, participants).await;
            }
            MeetingAction::Status { id } => {
                commands::teams::meeting_status(id).await;
            }
            MeetingAction::Responses { id } => {
                commands::teams::meeting_responses(id).await;
            }
            MeetingAction::Synthesize { id, synthesis } => {
                commands::teams::meeting_synthesize(id, synthesis).await;
            }
            MeetingAction::Decide { id, decision } => {
                commands::teams::meeting_decide(id, decision).await;
            }
            MeetingAction::List { team, status } => {
                commands::teams::list_meetings(team, status).await;
            }
            MeetingAction::Transcript { id } => {
                commands::teams::meeting_transcript(id).await;
            }
        },

        // ── Notifications ──
        Commands::Notifications { status, category, limit } => {
            commands::teams::list_notifications(status, category, limit).await;
        }
        Commands::AckNotification { id } => {
            commands::teams::ack_notification(id).await;
        }
        Commands::DismissNotification { id } => {
            commands::teams::dismiss_notification(id).await;
        }
        Commands::ActNotification { id, approve, reject } => {
            let approved = if reject { false } else { approve };
            commands::teams::act_on_notification(id, approved).await;
        }

        // ── Streaming & Heartbeat ──
        Commands::Subscribe { events, session, team } => {
            commands::system::subscribe(events, session, team).await;
        }
        Commands::SessionHeartbeat { session } => {
            commands::system::session_heartbeat(session).await;
        }

        // ── Proactive Context (Prajna) ──
        Commands::ContextRefresh { session_id, since } => {
            commands::system::context_refresh(session_id, since).await;
        }
        Commands::CompletionCheck { session_id, claimed_done } => {
            commands::system::completion_check(session_id, claimed_done).await;
        }
        Commands::TaskCompletionCheck { session_id, subject, description } => {
            commands::system::task_completion_check(session_id, subject, description).await;
        }
        Commands::ContextStats { session_id } => {
            commands::system::context_stats(session_id).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_detect_reality_command_parse() {
        let cli = Cli::try_parse_from(["forge-next", "detect-reality", "--path", "/tmp/myproject"]);
        assert!(cli.is_ok(), "detect-reality should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::DetectReality { path } => {
                assert_eq!(path.as_deref(), Some("/tmp/myproject"));
            }
            other => panic!("expected DetectReality, got {:?}", other),
        }
    }

    #[test]
    fn test_detect_reality_command_parse_no_path() {
        let cli = Cli::try_parse_from(["forge-next", "detect-reality"]);
        assert!(cli.is_ok(), "detect-reality without --path should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::DetectReality { path } => {
                assert!(path.is_none());
            }
            other => panic!("expected DetectReality, got {:?}", other),
        }
    }

    #[test]
    fn test_code_search_command_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "code-search", "authenticate", "--kind", "function", "--limit", "5",
        ]);
        assert!(cli.is_ok(), "code-search should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::CodeSearch { query, kind, limit } => {
                assert_eq!(query, "authenticate");
                assert_eq!(kind.as_deref(), Some("function"));
                assert_eq!(limit, 5);
            }
            other => panic!("expected CodeSearch, got {:?}", other),
        }
    }

    #[test]
    fn test_code_search_command_parse_defaults() {
        let cli = Cli::try_parse_from(["forge-next", "code-search", "MyClass"]);
        assert!(cli.is_ok(), "code-search with defaults should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::CodeSearch { query, kind, limit } => {
                assert_eq!(query, "MyClass");
                assert!(kind.is_none());
                assert_eq!(limit, 20);
            }
            other => panic!("expected CodeSearch, got {:?}", other),
        }
    }

    #[test]
    fn test_realities_command_parse() {
        let cli = Cli::try_parse_from(["forge-next", "realities"]);
        assert!(cli.is_ok(), "realities should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Realities { organization } => {
                assert!(organization.is_none());
            }
            other => panic!("expected Realities, got {:?}", other),
        }
    }

    #[test]
    fn test_config_set_scoped_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "config", "set-scoped",
            "--scope", "organization",
            "--scope-id", "default",
            "--key", "context.budget_chars",
            "--value", "50000",
            "--locked",
            "--ceiling", "100000",
        ]);
        assert!(cli.is_ok(), "config set-scoped should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Config { action } => match action {
                ConfigAction::SetScoped { scope, scope_id, key, value, locked, ceiling } => {
                    assert_eq!(scope, "organization");
                    assert_eq!(scope_id, "default");
                    assert_eq!(key, "context.budget_chars");
                    assert_eq!(value, "50000");
                    assert!(locked);
                    assert_eq!(ceiling, Some(100000.0));
                }
                other => panic!("expected SetScoped, got {:?}", other),
            },
            other => panic!("expected Config, got {:?}", other),
        }
    }

    #[test]
    fn test_config_get_effective_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "config", "get-effective",
            "--organization", "default",
            "--agent", "claude-code",
        ]);
        assert!(cli.is_ok(), "config get-effective should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Config { action } => match action {
                ConfigAction::GetEffective { session, agent, reality, user, team, organization } => {
                    assert!(session.is_none());
                    assert_eq!(agent.as_deref(), Some("claude-code"));
                    assert!(reality.is_none());
                    assert!(user.is_none());
                    assert!(team.is_none());
                    assert_eq!(organization.as_deref(), Some("default"));
                }
                other => panic!("expected GetEffective, got {:?}", other),
            },
            other => panic!("expected Config, got {:?}", other),
        }
    }

    #[test]
    fn test_config_list_scoped_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "config", "list-scoped",
            "--scope", "reality",
            "--scope-id", "r1",
        ]);
        assert!(cli.is_ok(), "config list-scoped should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Config { action } => match action {
                ConfigAction::ListScoped { scope, scope_id } => {
                    assert_eq!(scope, "reality");
                    assert_eq!(scope_id, "r1");
                }
                other => panic!("expected ListScoped, got {:?}", other),
            },
            other => panic!("expected Config, got {:?}", other),
        }
    }

    #[test]
    fn test_config_delete_scoped_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "config", "delete-scoped",
            "--scope", "organization",
            "--scope-id", "default",
            "--key", "max_tokens",
        ]);
        assert!(cli.is_ok(), "config delete-scoped should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Config { action } => match action {
                ConfigAction::DeleteScoped { scope, scope_id, key } => {
                    assert_eq!(scope, "organization");
                    assert_eq!(scope_id, "default");
                    assert_eq!(key, "max_tokens");
                }
                other => panic!("expected DeleteScoped, got {:?}", other),
            },
            other => panic!("expected Config, got {:?}", other),
        }
    }

    #[test]
    fn test_force_index_parse() {
        let cli = Cli::try_parse_from(["forge-next", "force-index"]);
        assert!(cli.is_ok(), "force-index should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::ForceIndex => {}
            other => panic!("expected ForceIndex, got {:?}", other),
        }
    }

    // ── Agent Template tests ──

    #[test]
    fn test_agent_template_create_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "agent-template", "create",
            "--name", "CTO",
            "--description", "Chief Technology Officer",
            "--agent-type", "claude-code",
            "--system-context", "You are the CTO",
            "--decision-style", "analytical",
        ]);
        assert!(cli.is_ok(), "agent-template create should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::AgentTemplate { action } => match action {
                AgentTemplateAction::Create { name, description, agent_type, system_context, decision_style, .. } => {
                    assert_eq!(name, "CTO");
                    assert_eq!(description, "Chief Technology Officer");
                    assert_eq!(agent_type, "claude-code");
                    assert_eq!(system_context.as_deref(), Some("You are the CTO"));
                    assert_eq!(decision_style.as_deref(), Some("analytical"));
                }
                other => panic!("expected Create, got {:?}", other),
            },
            other => panic!("expected AgentTemplate, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_template_list_parse() {
        let cli = Cli::try_parse_from(["forge-next", "agent-template", "list"]);
        assert!(cli.is_ok(), "agent-template list should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::AgentTemplate { action } => match action {
                AgentTemplateAction::List { org } => {
                    assert!(org.is_none());
                }
                other => panic!("expected List, got {:?}", other),
            },
            other => panic!("expected AgentTemplate, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_template_get_by_name_parse() {
        let cli = Cli::try_parse_from(["forge-next", "agent-template", "get", "--name", "CTO"]);
        assert!(cli.is_ok(), "agent-template get should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::AgentTemplate { action } => match action {
                AgentTemplateAction::Get { name, id } => {
                    assert_eq!(name.as_deref(), Some("CTO"));
                    assert!(id.is_none());
                }
                other => panic!("expected Get, got {:?}", other),
            },
            other => panic!("expected AgentTemplate, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_template_delete_parse() {
        let cli = Cli::try_parse_from(["forge-next", "agent-template", "delete", "--id", "01KNF123"]);
        assert!(cli.is_ok(), "agent-template delete should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::AgentTemplate { action } => match action {
                AgentTemplateAction::Delete { id } => {
                    assert_eq!(id, "01KNF123");
                }
                other => panic!("expected Delete, got {:?}", other),
            },
            other => panic!("expected AgentTemplate, got {:?}", other),
        }
    }

    // ── Agent tests ──

    #[test]
    fn test_agent_spawn_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "agent", "spawn",
            "--template", "CTO",
            "--session-id", "cto-board",
            "--project", "forge",
            "--team", "board",
        ]);
        assert!(cli.is_ok(), "agent spawn should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Agent { action } => match action {
                AgentAction::Spawn { template, session_id, project, team } => {
                    assert_eq!(template, "CTO");
                    assert_eq!(session_id, "cto-board");
                    assert_eq!(project.as_deref(), Some("forge"));
                    assert_eq!(team.as_deref(), Some("board"));
                }
                other => panic!("expected Spawn, got {:?}", other),
            },
            other => panic!("expected Agent, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_retire_parse() {
        let cli = Cli::try_parse_from(["forge-next", "agent", "retire", "--session", "cto-board"]);
        assert!(cli.is_ok(), "agent retire should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Agent { action } => match action {
                AgentAction::Retire { session } => {
                    assert_eq!(session, "cto-board");
                }
                other => panic!("expected Retire, got {:?}", other),
            },
            other => panic!("expected Agent, got {:?}", other),
        }
    }

    #[test]
    fn test_agents_list_parse() {
        let cli = Cli::try_parse_from(["forge-next", "agents", "--team", "board"]);
        assert!(cli.is_ok(), "agents should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Agents { team } => {
                assert_eq!(team.as_deref(), Some("board"));
            }
            other => panic!("expected Agents, got {:?}", other),
        }
    }

    #[test]
    fn test_agents_list_no_filter_parse() {
        let cli = Cli::try_parse_from(["forge-next", "agents"]);
        assert!(cli.is_ok(), "agents without filter should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Agents { team } => {
                assert!(team.is_none());
            }
            other => panic!("expected Agents, got {:?}", other),
        }
    }

    #[test]
    fn test_agent_status_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "agent-status",
            "--session", "cto-board",
            "--status", "thinking",
            "--task", "reviewing architecture",
        ]);
        assert!(cli.is_ok(), "agent-status should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::AgentStatus { session, status, task } => {
                assert_eq!(session, "cto-board");
                assert_eq!(status, "thinking");
                assert_eq!(task.as_deref(), Some("reviewing architecture"));
            }
            other => panic!("expected AgentStatus, got {:?}", other),
        }
    }

    // ── Team tests ──

    #[test]
    fn test_team_create_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "team", "create",
            "--name", "board",
            "--type", "agent",
            "--purpose", "Strategic decisions",
        ]);
        assert!(cli.is_ok(), "team create should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Team { action } => match action {
                TeamAction::Create { name, team_type, purpose } => {
                    assert_eq!(name, "board");
                    assert_eq!(team_type.as_deref(), Some("agent"));
                    assert_eq!(purpose.as_deref(), Some("Strategic decisions"));
                }
                other => panic!("expected Create, got {:?}", other),
            },
            other => panic!("expected Team, got {:?}", other),
        }
    }

    #[test]
    fn test_team_members_parse() {
        let cli = Cli::try_parse_from(["forge-next", "team", "members", "--name", "board"]);
        assert!(cli.is_ok(), "team members should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Team { action } => match action {
                TeamAction::Members { name } => {
                    assert_eq!(name, "board");
                }
                other => panic!("expected Members, got {:?}", other),
            },
            other => panic!("expected Team, got {:?}", other),
        }
    }

    #[test]
    fn test_team_set_orchestrator_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "team", "set-orchestrator",
            "--name", "board",
            "--session", "cto-board",
        ]);
        assert!(cli.is_ok(), "team set-orchestrator should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Team { action } => match action {
                TeamAction::SetOrchestrator { name, session } => {
                    assert_eq!(name, "board");
                    assert_eq!(session, "cto-board");
                }
                other => panic!("expected SetOrchestrator, got {:?}", other),
            },
            other => panic!("expected Team, got {:?}", other),
        }
    }

    #[test]
    fn test_team_status_parse() {
        let cli = Cli::try_parse_from(["forge-next", "team", "status", "--name", "board"]);
        assert!(cli.is_ok(), "team status should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Team { action } => match action {
                TeamAction::Status { name } => {
                    assert_eq!(name, "board");
                }
                other => panic!("expected Status, got {:?}", other),
            },
            other => panic!("expected Team, got {:?}", other),
        }
    }

    // ── Meeting tests ──

    #[test]
    fn test_meeting_create_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "meeting", "create",
            "--team", "team-01",
            "--topic", "Architecture review",
            "--context", "We need to decide on the DB",
            "--orchestrator", "ceo-session",
            "--participants", "cto-board,cmo-board,cfo-board",
        ]);
        assert!(cli.is_ok(), "meeting create should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::Create { team, topic, context, orchestrator, participants } => {
                    assert_eq!(team, "team-01");
                    assert_eq!(topic, "Architecture review");
                    assert_eq!(context.as_deref(), Some("We need to decide on the DB"));
                    assert_eq!(orchestrator, "ceo-session");
                    assert_eq!(participants, vec!["cto-board", "cmo-board", "cfo-board"]);
                }
                other => panic!("expected Create, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    #[test]
    fn test_meeting_status_parse() {
        let cli = Cli::try_parse_from(["forge-next", "meeting", "status", "--id", "m-01"]);
        assert!(cli.is_ok(), "meeting status should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::Status { id } => {
                    assert_eq!(id, "m-01");
                }
                other => panic!("expected Status, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    #[test]
    fn test_meeting_responses_parse() {
        let cli = Cli::try_parse_from(["forge-next", "meeting", "responses", "--id", "m-01"]);
        assert!(cli.is_ok(), "meeting responses should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::Responses { id } => {
                    assert_eq!(id, "m-01");
                }
                other => panic!("expected Responses, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    #[test]
    fn test_meeting_synthesize_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "meeting", "synthesize",
            "--id", "m-01",
            "--synthesis", "All agreed on PostgreSQL",
        ]);
        assert!(cli.is_ok(), "meeting synthesize should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::Synthesize { id, synthesis } => {
                    assert_eq!(id, "m-01");
                    assert_eq!(synthesis, "All agreed on PostgreSQL");
                }
                other => panic!("expected Synthesize, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    #[test]
    fn test_meeting_decide_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "meeting", "decide",
            "--id", "m-01",
            "--decision", "Use PostgreSQL for prod",
        ]);
        assert!(cli.is_ok(), "meeting decide should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::Decide { id, decision } => {
                    assert_eq!(id, "m-01");
                    assert_eq!(decision, "Use PostgreSQL for prod");
                }
                other => panic!("expected Decide, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    #[test]
    fn test_meeting_list_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "meeting", "list",
            "--team", "team-01",
            "--status", "open",
        ]);
        assert!(cli.is_ok(), "meeting list should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::List { team, status } => {
                    assert_eq!(team.as_deref(), Some("team-01"));
                    assert_eq!(status.as_deref(), Some("open"));
                }
                other => panic!("expected List, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    #[test]
    fn test_meeting_list_no_filter_parse() {
        let cli = Cli::try_parse_from(["forge-next", "meeting", "list"]);
        assert!(cli.is_ok(), "meeting list without filter should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::List { team, status } => {
                    assert!(team.is_none());
                    assert!(status.is_none());
                }
                other => panic!("expected List, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    #[test]
    fn test_meeting_transcript_parse() {
        let cli = Cli::try_parse_from(["forge-next", "meeting", "transcript", "--id", "m-01"]);
        assert!(cli.is_ok(), "meeting transcript should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Meeting { action } => match action {
                MeetingAction::Transcript { id } => {
                    assert_eq!(id, "m-01");
                }
                other => panic!("expected Transcript, got {:?}", other),
            },
            other => panic!("expected Meeting, got {:?}", other),
        }
    }

    // ── HTTP transport flag tests ──

    #[test]
    fn test_endpoint_flag_before_subcommand() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "--endpoint", "https://forge.example.com",
            "--token", "my-jwt",
            "health",
        ]);
        assert!(cli.is_ok(), "endpoint+token before subcommand should parse: {:?}", cli.err());
        let cli = cli.unwrap();
        assert_eq!(cli.endpoint.as_deref(), Some("https://forge.example.com"));
        assert_eq!(cli.token.as_deref(), Some("my-jwt"));
        assert!(matches!(cli.command, Commands::Health));
    }

    #[test]
    fn test_endpoint_flag_after_subcommand() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "health",
            "--endpoint", "https://forge.example.com",
        ]);
        assert!(cli.is_ok(), "endpoint after subcommand should parse (global flag): {:?}", cli.err());
        let cli = cli.unwrap();
        assert_eq!(cli.endpoint.as_deref(), Some("https://forge.example.com"));
        assert!(cli.token.is_none());
    }

    #[test]
    fn test_no_endpoint_defaults_to_none() {
        let cli = Cli::try_parse_from(["forge-next", "health"]);
        assert!(cli.is_ok());
        let cli = cli.unwrap();
        assert!(cli.endpoint.is_none());
        assert!(cli.token.is_none());
    }

    #[test]
    fn test_endpoint_with_recall_command() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "--endpoint", "http://localhost:8080",
            "recall", "test query", "--limit", "5",
        ]);
        assert!(cli.is_ok(), "endpoint with recall should parse: {:?}", cli.err());
        let cli = cli.unwrap();
        assert_eq!(cli.endpoint.as_deref(), Some("http://localhost:8080"));
        match cli.command {
            Commands::Recall { query, limit, .. } => {
                assert_eq!(query, "test query");
                assert_eq!(limit, 5);
            }
            other => panic!("expected Recall, got {:?}", other),
        }
    }

    // ── Subscribe & SessionHeartbeat tests ──

    #[test]
    fn test_subscribe_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "subscribe",
            "--events", "memory_created,session_changed",
            "--session", "s1",
        ]);
        assert!(cli.is_ok(), "subscribe should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Subscribe { events, session, team } => {
                assert_eq!(events, Some(vec!["memory_created".into(), "session_changed".into()]));
                assert_eq!(session, Some("s1".into()));
                assert!(team.is_none());
            }
            other => panic!("expected Subscribe, got {:?}", other),
        }
    }

    #[test]
    fn test_subscribe_no_args_parse() {
        let cli = Cli::try_parse_from(["forge-next", "subscribe"]);
        assert!(cli.is_ok(), "subscribe with no args should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Subscribe { events, session, team } => {
                assert!(events.is_none());
                assert!(session.is_none());
                assert!(team.is_none());
            }
            other => panic!("expected Subscribe, got {:?}", other),
        }
    }

    #[test]
    fn test_subscribe_with_team_parse() {
        let cli = Cli::try_parse_from([
            "forge-next", "subscribe",
            "--team", "team-alpha",
        ]);
        assert!(cli.is_ok(), "subscribe with team should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::Subscribe { events, session, team } => {
                assert!(events.is_none());
                assert!(session.is_none());
                assert_eq!(team, Some("team-alpha".into()));
            }
            other => panic!("expected Subscribe, got {:?}", other),
        }
    }

    #[test]
    fn test_session_heartbeat_parse() {
        let cli = Cli::try_parse_from(["forge-next", "session-heartbeat", "--session", "s1"]);
        assert!(cli.is_ok(), "session-heartbeat should parse: {:?}", cli.err());
        match cli.unwrap().command {
            Commands::SessionHeartbeat { session } => {
                assert_eq!(session, "s1");
            }
            other => panic!("expected SessionHeartbeat, got {:?}", other),
        }
    }

    // ── Proactive Context (Prajna) tests ──

    #[test]
    fn test_context_refresh_parse() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "context-refresh",
            "--session-id",
            "s1",
            "--since",
            "2026-04-06T12:00:00Z",
        ]);
        assert!(
            cli.is_ok(),
            "context-refresh should parse: {:?}",
            cli.err()
        );
    }

    #[test]
    fn test_context_refresh_no_since_parse() {
        let cli =
            Cli::try_parse_from(["forge-next", "context-refresh", "--session-id", "s1"]);
        assert!(
            cli.is_ok(),
            "context-refresh without --since should parse: {:?}",
            cli.err()
        );
    }

    #[test]
    fn test_completion_check_parse() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "completion-check",
            "--session-id",
            "s1",
            "--claimed-done",
        ]);
        assert!(
            cli.is_ok(),
            "completion-check should parse: {:?}",
            cli.err()
        );
    }

    #[test]
    fn test_completion_check_no_flag_parse() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "completion-check",
            "--session-id",
            "s1",
        ]);
        assert!(
            cli.is_ok(),
            "completion-check without --claimed-done should parse: {:?}",
            cli.err()
        );
    }

    #[test]
    fn test_task_completion_check_parse() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "task-completion-check",
            "--session-id",
            "s1",
            "--subject",
            "deploy to prod",
        ]);
        assert!(
            cli.is_ok(),
            "task-completion-check should parse: {:?}",
            cli.err()
        );
    }

    #[test]
    fn test_task_completion_check_with_description_parse() {
        let cli = Cli::try_parse_from([
            "forge-next",
            "task-completion-check",
            "--session-id",
            "s1",
            "--subject",
            "deploy to prod",
            "--description",
            "Deploy the staging environment to production",
        ]);
        assert!(
            cli.is_ok(),
            "task-completion-check with --description should parse: {:?}",
            cli.err()
        );
    }
}
