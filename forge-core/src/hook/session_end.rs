use serde_json::json;

use crate::hud_state;

pub fn run(state_dir: &str) {
    // Update HUD state to mark session as ended
    hud_state::update(state_dir, |state| {
        state.session.phase = Some("ended".to_string());
    });

    let output = json!({
        "hookSpecificOutput": {
            "additionalContext": "Session ended."
        }
    });
    println!("{}", output);
}
