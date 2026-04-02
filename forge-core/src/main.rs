mod index;

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
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Index { path } => index::run(&path),
    }
}
