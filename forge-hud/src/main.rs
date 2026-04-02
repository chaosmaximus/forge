mod render;
mod state;
mod stdin;

/// Get state directory from CLAUDE_PLUGIN_DATA, with safety validation.
fn state_dir() -> String {
    let dir = match std::env::var("CLAUDE_PLUGIN_DATA") {
        Ok(d) if !d.is_empty() => d,
        _ => return ".forge".to_string(),
    };

    // Resolve to canonical path if possible
    let resolved = std::fs::canonicalize(&dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or(dir);

    // Safety: must be under $HOME or /tmp
    if let Ok(home) = std::env::var("HOME") {
        if let Ok(home_canonical) = std::fs::canonicalize(&home) {
            if resolved.starts_with(&home_canonical.to_string_lossy().to_string()) {
                return resolved;
            }
        }
    }
    if resolved.starts_with("/tmp") {
        return resolved;
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
