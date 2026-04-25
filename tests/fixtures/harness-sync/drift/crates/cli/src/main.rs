// Same Commands enum as clean fixture — drift comes from skills/agents only.
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge-next")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Health,
    #[command(name = "health-by-project")]
    HealthByProject,
    Recall {
        query: String,
    },
    Remember {
        content: String,
    },
    #[command(name = "record-tool-use")]
    RecordToolUse {
        name: String,
    },
    #[command(name = "list-tool-calls")]
    ListToolCalls,
}
