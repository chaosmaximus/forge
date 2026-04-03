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
        } => {
            commands::memory::recall(query, r#type, project, limit).await;
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
    }
}
