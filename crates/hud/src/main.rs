mod render;
mod state;
mod stdin;

/// Get state directory. Checks: CLI arg, FORGE_DATA env, CLAUDE_PLUGIN_DATA env, auto-detect.
fn state_dir() -> String {
    // 1. CLI argument (--state-dir)
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--state-dir" && i + 1 < args.len() {
            return args[i + 1].clone();
        }
    }

    // 2. FORGE_DATA env (explicit)
    if let Ok(d) = std::env::var("FORGE_DATA") {
        if !d.is_empty() {
            return d;
        }
    }

    // 3. CLAUDE_PLUGIN_DATA env (set by Claude Code for MCP servers)
    if let Ok(d) = std::env::var("CLAUDE_PLUGIN_DATA") {
        if !d.is_empty() && !d.contains("codex") {
            return d;
        }
    }

    // 4. Auto-detect: look for forge data in standard Claude plugin paths
    if let Ok(home) = std::env::var("HOME") {
        let candidates = [
            format!("{home}/.claude/plugins/data/forge-forge-marketplace"),
            format!("{home}/.claude/plugins/data/forge"),
            format!("{home}/.claude/plugin-data/forge"),
        ];
        for c in &candidates {
            let hud_path = format!("{c}/hud/hud-state.json");
            if std::path::Path::new(&hud_path).exists() {
                return c.to_string();
            }
        }
        // Return first candidate that exists as a directory
        for c in &candidates {
            if std::path::Path::new(c).is_dir() {
                return c.to_string();
            }
        }
    }

    ".forge".to_string()
}

fn main() {
    let stdin_data = stdin::read_stdin();
    let sd = state_dir();
    let hud_state = state::read_state(&sd);
    let output = render::render(&stdin_data, &hud_state);
    print!("{output}");
}
