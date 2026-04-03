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
    /// Daemon health diagnostics
    Doctor,
    /// Import v1 cache.json into daemon
    Migrate {
        /// Path to v1 state directory containing cache.json
        state_dir: String,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Show daemon status (uptime, memory count)
    Status,
    /// Stop the daemon
    Stop,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Recall {
            query,
            r#type,
            limit,
        } => {
            commands::memory::recall(query, r#type, limit).await;
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
        Commands::Doctor => {
            commands::system::doctor().await;
        }
        Commands::Migrate { state_dir } => {
            commands::system::migrate(state_dir).await;
        }
    }
}
