mod agent;
mod doctor;
mod hook;
pub mod hud_state;
mod index;
mod memory;
mod research;
mod review;
mod scan;
#[cfg(test)]
mod security_tests;
mod test_runner;
mod verify;
mod watch;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "forge", version, about = "Forge — Agentic OS for Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index a codebase: parse with tree-sitter, output NDJSON + populate caches
    Index {
        #[arg(default_value = ".")]
        path: String,
        /// Plugin data directory (for import + signature caches)
        #[arg(long, env = "CLAUDE_PLUGIN_DATA")]
        state_dir: Option<String>,
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
    /// Autonomous research loop with git-backed checkpoints
    Research {
        /// Topic or question (ignored when --discard is used)
        #[arg(default_value = "")]
        topic: String,
        /// Max iterations
        #[arg(long, default_value = "5")]
        max_iterations: usize,
        /// Working directory
        #[arg(long, default_value = ".")]
        workdir: String,
        /// Discard the last research iteration (revert last commit)
        #[arg(long)]
        discard: bool,
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
        /// Council mode: produce structured review request for multi-model dispatch
        #[arg(long)]
        council: bool,
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
    /// Lint a file or project (auto-detects language)
    Lint {
        /// File or directory to lint
        #[arg(default_value = ".")]
        path: String,
        /// Output format: json (default) or text
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Format a file (auto-detects language)
    Fmt {
        /// File to format
        path: String,
        /// Check only, don't write
        #[arg(long)]
        check: bool,
        /// Output format: json (default) or text
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Type check a file or project (auto-detects language)
    Check {
        /// File or directory to check
        #[arg(default_value = ".")]
        path: String,
        /// Output format: json (default) or text
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Unified verification: syntax + format + lint + security + cross-file + types
    Verify {
        /// File or directory to verify
        #[arg(default_value = ".")]
        path: String,
        /// Auto-fix formatting issues
        #[arg(long)]
        fix: bool,
        /// Output format: json (default) or text
        #[arg(long, default_value = "json")]
        format: String,
        /// Plugin data directory
        #[arg(long, env = "CLAUDE_PLUGIN_DATA", default_value = ".forge")]
        state_dir: String,
        /// Also run type checker (slower)
        #[arg(long)]
        types: bool,
    },
    /// Run tests, check page health, capture screenshots
    Test {
        #[command(subcommand)]
        test_type: TestType,
    },
    /// Watch project for changes and run continuous verification
    Watch {
        /// Directory to watch
        #[arg(default_value = ".")]
        path: String,
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

#[derive(Subcommand)]
enum TestType {
    /// Run project tests (auto-detects framework: pytest/vitest/jest/cargo/go)
    Run {
        /// File or directory to test
        #[arg(default_value = ".")]
        path: String,
        /// Output format: json (default) or text
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Check page health (HTTP status, response time, error detection)
    Check {
        /// URL to check
        url: String,
        /// Save screenshot (requires Playwright)
        #[arg(long)]
        screenshot: Option<String>,
        /// Output format: json (default) or text
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// Capture screenshot via Playwright CLI
    Screenshot {
        /// URL to capture
        url: String,
        /// Output file path
        #[arg(default_value = "screenshot.png")]
        output: String,
        /// Capture full page (not just viewport)
        #[arg(long)]
        full_page: bool,
    },
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Index { path, state_dir } => {
            index::run_with_state(&path, state_dir.as_deref());
        }
        Commands::Scan { path, watch, interval } => {
            if watch { scan::watch(&path, interval); } else { scan::run(&path); }
        }
        Commands::Hook { hook_type } => match hook_type {
            HookType::SessionStart { state_dir } => hook::session_start::run(&state_dir),
            HookType::PostEdit { file } => hook::post_edit::run(&file),
            HookType::SessionEnd { state_dir } => hook::session_end::run(&state_dir),
        },
        Commands::Research { topic, max_iterations, workdir, discard } => {
            if discard {
                research::discard(&workdir);
            } else if topic.is_empty() {
                eprintln!("Error: topic is required (unless --discard is used)");
                println!("{{\"error\":\"Topic is required\"}}");
            } else {
                research::run(&topic, max_iterations, &workdir);
            }
        }
        Commands::Review { path, base, format, council } => {
            review::run(&path, &base, &format, council);
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
            // 1. Remove from local cache
            let cache_path = std::path::Path::new(&state_dir).join("memory").join("cache.json");
            if let Ok(content) = std::fs::read_to_string(&cache_path) {
                if let Ok(mut cache) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(entries) = cache.get_mut("entries").and_then(|e| e.as_array_mut()) {
                        entries.retain(|e| e.get("id").and_then(|v| v.as_str()) != Some(&node_id));
                        if let Ok(json_str) = serde_json::to_string(&cache) {
                            let _ = std::fs::write(&cache_path, json_str);
                        }
                    }
                }
            }
            // 2. Update HUD counts
            hud_state::update(&state_dir, |state| {
                match label.as_str() {
                    "Decision" => state.memory.decisions = state.memory.decisions.saturating_sub(1),
                    "Pattern" => state.memory.patterns = state.memory.patterns.saturating_sub(1),
                    "Lesson" => state.memory.lessons = state.memory.lessons.saturating_sub(1),
                    _ => {}
                }
            });
            // 3. Soft-delete in graph (best-effort)
            let args = ["forget", &node_id, "--label", &label, "--reason", &reason];
            match memory::python::call_graph(&state_dir, &args) {
                Ok(json) => println!("{}", json),
                Err(_) => println!("{}", serde_json::json!({"status": "forgotten_local", "id": node_id, "note": "Removed from cache. Graph update will happen on next sync."})),
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
                Err(e) => {
                    // Sanitize error — strip control chars that break JSON
                    let clean: String = e.chars().filter(|c| !c.is_control() || *c == '\n').collect();
                    println!("{}", serde_json::json!({"error": clean}));
                }
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
        Commands::Lint { path, format } => {
            verify::lint::run(&path, &format);
        }
        Commands::Fmt { path, check, format } => {
            verify::fmt::run(&path, check, &format);
        }
        Commands::Check { path, format } => {
            verify::check::run(&path, &format);
        }
        Commands::Verify {
            path,
            fix,
            format,
            state_dir,
            types,
        } => {
            verify::unified::run(&path, fix, &format, &state_dir, types);
        }
        Commands::Test { test_type } => match test_type {
            TestType::Run { path, format } => {
                test_runner::run::run(&path, &format);
            }
            TestType::Check { url, screenshot, format } => {
                test_runner::check::run(&url, screenshot.as_deref(), &format);
            }
            TestType::Screenshot { url, output, full_page } => {
                test_runner::screenshot::run(&url, &output, full_page);
            }
        },
        Commands::Watch { path, state_dir } => watch::run(&path, &state_dir),
    }
}
