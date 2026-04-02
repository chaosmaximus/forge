use serde_json::{json, Value};
use std::path::Path;

pub fn run(state_dir: &str) {
    let hud_path = Path::new(state_dir).join("hud").join("hud-state.json");

    // Update HUD state to show session ended
    if let Ok(content) = std::fs::read_to_string(&hud_path) {
        if let Ok(mut state) = serde_json::from_str::<Value>(&content) {
            if let Some(session) = state.get_mut("session") {
                session["phase"] = json!("ended");
            }
            // Atomic write via tmp + rename
            let tmp_path = hud_path.with_extension("tmp");
            if let Ok(json_str) = serde_json::to_string(&state) {
                if std::fs::write(&tmp_path, &json_str).is_ok() {
                    let _ = std::fs::rename(&tmp_path, &hud_path);
                }
            }
        }
    }

    let output = json!({
        "hookSpecificOutput": {
            "additionalContext": "Session ended."
        }
    });
    println!("{}", output);
}
