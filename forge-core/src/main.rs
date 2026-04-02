mod agent;
mod doctor;
mod hook;
pub mod hud_state;
mod index;
mod memory;
mod research;
mod review;
mod scan;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge", version, about = "Forge — Agentic OS for Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index a codebase: parse with tree-sitter, output NDJSON
    Index {
        #[arg(default_value = ".")]
        path: String,
    },
    /// Scan directory for exposed secrets
    Scan {
        #[arg(default_value = ".")]
        path: String,
        #[arg(long)]
        watch: bool,
        #[arg(long, default_value = "30")]
        interval: u64,
    },
    /// Handle Claude Code hook events
    Hook {
        #[command(subcommand)]
        hook_type: HookType,
    },
    /// Autonomous research loop
    Research {
        /// Topic or question
        topic: String,
        /// Max iterations
        #[arg(long, default_value = "5")]
        max_iterations: usize,
        /// Working directory
        #[arg(long, default_value = ".")]
        workdir: String,
    },
    /// Council review of code changes
    Review {
        /// Path to review
        #[arg(default_value = ".")]
        path: String,
        /// Base commit
        #[arg(long, default_value = "HEAD~1")]
        base: String,
        /// Output format
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Handle agent lifecycle events (reads hook payload from stdin)
    Agent {
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// Store a memory (decision, pattern, lesson, preference)
    Remember {
        /// Memory type: decision, pattern, lesson, preference
        #[arg(long, short = 't')]
        r#type: String,
        /// Title or name
        #[arg(long)]
        title: String,
        /// Content / rationale / description
        #[arg(long)]
        content: String,
        /// Confidence (0.0 - 1.0)
        #[arg(long, default_value = "0.9")]
        confidence: f64,
        /// Sync immediately to graph DB (slower, ~200ms)
        #[arg(long)]
        sync: bool,
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// Search memory by keyword
    Recall {
        /// Search query
        query: Option<String>,
        /// Filter by type: decision, pattern, lesson, preference
        #[arg(long, short = 't')]
        r#type: Option<String>,
        /// List all (no search)
        #[arg(long)]
        list: bool,
        /// Query graph DB directly (slower, ~200ms)
        #[arg(long)]
        graph: bool,
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// Forget (soft-delete) a memory node
    Forget {
        /// Node ID
        node_id: String,
        /// Node label: Decision, Pattern, Lesson, Preference
        #[arg(long)]
        label: String,
        /// Reason for deletion
        #[arg(long, default_value = "")]
        reason: String,
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// Graph health check
    Health {
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// Execute read-only Cypher query on code graph
    Query {
        /// Cypher query string
        cypher: String,
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// Sync pending memory entries to graph DB
    Sync {
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// System health checks — verify entire Forge installation
    Doctor {
        /// Output format: json (default) or text
        #[arg(long, default_value = "json")]
        format: String,
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
}

#[derive(Subcommand)]
enum HookType {
    /// SessionStart: read HUD state, output context
    SessionStart {
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
    /// PostToolUse: scan edited file for secrets
    PostEdit {
        /// File path to scan
        file: String,
    },
    /// SessionEnd: update HUD state
    SessionEnd {
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Index { path } => index::run(&path),
        Commands::Scan { path, watch, interval } => {
            if watch { scan::watch(&path, interval); } else { scan::run(&path); }
        }
        Commands::Hook { hook_type } => match hook_type {
            HookType::SessionStart { state_dir } => hook::session_start::run(&state_dir),
            HookType::PostEdit { file } => hook::post_edit::run(&file),
            HookType::SessionEnd { state_dir } => hook::session_end::run(&state_dir),
        },
        Commands::Research { topic, max_iterations, workdir } => {
            research::run(&topic, max_iterations, &workdir);
        }
        Commands::Review { path, base, format } => {
            review::run(&path, &base, &format);
        }
        Commands::Agent { state_dir } => agent::run(&state_dir),
        Commands::Remember { r#type, title, content, confidence, sync, state_dir } => {
            memory::remember::run(&state_dir, &r#type, &title, &content, confidence, sync);
        }
        Commands::Recall { query, r#type, list, graph, state_dir } => {
            if graph {
                // Direct graph query via Python
                let q = query.as_deref().unwrap_or("");
                let mut args = vec!["recall", q];
                if let Some(t) = &r#type { args.extend(["--type", t]); }
                match memory::python::call_graph(&state_dir, &args) {
                    Ok(json) => println!("{}", json),
                    Err(e) => eprintln!("{{\"error\":\"{}\"}}", e),
                }
            } else if list {
                memory::recall::list(&state_dir, r#type.as_deref());
            } else if let Some(q) = &query {
                memory::recall::run(&state_dir, q, r#type.as_deref());
            } else {
                memory::recall::list(&state_dir, r#type.as_deref());
            }
        }
        Commands::Forget { node_id, label, reason, state_dir } => {
            let args = ["forget", &node_id, "--label", &label, "--reason", &reason];
            match memory::python::call_graph(&state_dir, &args) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("{{\"error\":\"{}\"}}", e),
            }
        }
        Commands::Health { state_dir } => {
            match memory::python::call_graph(&state_dir, &["health"]) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("{{\"error\":\"{}\"}}", e),
            }
        }
        Commands::Query { cypher, state_dir } => {
            match memory::python::call_graph(&state_dir, &["query", &cypher]) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("{{\"error\":\"{}\"}}", e),
            }
        }
        Commands::Sync { state_dir } => {
            let pending = format!("{}/memory/pending.jsonl", state_dir);
            match memory::python::call_graph(&state_dir, &["sync", "--pending-path", &pending]) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("{{\"error\":\"{}\"}}", e),
            }
        }
        Commands::Doctor { format, state_dir } => {
            doctor::run(&state_dir, &format);
        }
    }
}
