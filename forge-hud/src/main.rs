mod config;
mod render;
mod state;
mod stdin;

fn main() {
    let stdin_data = stdin::read_stdin();
    let state_dir = std::env::var("CLAUDE_PLUGIN_DATA").unwrap_or_else(|_| ".forge".to_string());
    let hud_state = state::read_state(&state_dir);
    let output = render::render(&stdin_data, &hud_state);
    print!("{output}");
}
