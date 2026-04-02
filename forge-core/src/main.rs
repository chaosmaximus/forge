mod hook;
mod index;
mod scan;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge-core", version, about = "Rust hot paths for Forge")]
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
    },
    /// Handle Claude Code hook events
    Hook {
        #[command(subcommand)]
        hook_type: HookType,
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
        Commands::Scan { path } => scan::run(&path),
        Commands::Hook { hook_type } => match hook_type {
            HookType::SessionStart { state_dir } => hook::session_start::run(&state_dir),
            HookType::PostEdit { file } => hook::post_edit::run(&file),
            HookType::SessionEnd { state_dir } => hook::session_end::run(&state_dir),
        },
    }
}
