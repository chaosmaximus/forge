mod render;
mod state;
mod stdin;

/// Validate CLAUDE_PLUGIN_DATA: canonicalize and verify it's under $HOME or /tmp.
fn safe_state_dir() -> Option<String> {
    let dir = std::env::var("CLAUDE_PLUGIN_DATA").ok()?;
    let canonical = std::fs::canonicalize(&dir).ok()?;

    // Must be under user's home directory
    if let Some(home) = std::env::var("HOME").ok() {
        if let Ok(home_canonical) = std::fs::canonicalize(&home) {
            if canonical.starts_with(&home_canonical) {
                return Some(canonical.to_string_lossy().to_string());
            }
        }
    }

    // Fallback: allow /tmp paths (for testing)
    if canonical.starts_with("/tmp") {
        return Some(canonical.to_string_lossy().to_string());
    }

    None // Reject everything else
}

fn main() {
    let stdin_data = stdin::read_stdin();
    let state_dir = safe_state_dir().unwrap_or_else(|| ".forge".to_string());
    let hud_state = state::read_state(&state_dir);
    let output = render::render(&stdin_data, &hud_state);
    print!("{output}");
}
