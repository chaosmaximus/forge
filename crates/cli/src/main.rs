mod client;
mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge-next", about = "Forge — memory for AI coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
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
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Show daemon status (uptime, memory count)
    Status,
    /// Stop the daemon
    Stop,
}

#[derive(Subcommand)]
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
        Commands::CompileContext { agent, project } => {
            commands::manas::compile_context(agent, project).await;
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
    }
}
