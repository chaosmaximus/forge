use serde_json::{json, Value};
use std::path::Path;

pub fn run(state_dir: &str) {
    let hud_path = Path::new(state_dir).join("hud").join("hud-state.json");

    let mut context_parts: Vec<String> = vec!["[Forge v0.2.0]".to_string()];

    // Read HUD state for security/skill warnings
    if let Ok(content) = std::fs::read_to_string(&hud_path) {
        if let Ok(state) = serde_json::from_str::<Value>(&content) {
            let stale = state["security"]["stale"].as_u64().unwrap_or(0);
            let fix_candidates = state["skills"]["fix_candidates"].as_u64().unwrap_or(0);

            if stale > 0 {
                context_parts.push(format!("WARNING: {} secrets need rotation.", stale));
            }
            if fix_candidates > 0 {
                context_parts.push(format!("{} skill(s) need attention.", fix_candidates));
            }
        }
    }

    context_parts.push(
        "Tools: forge_remember, forge_recall, forge_link, forge_decisions, forge_patterns, forge_timeline, forge_forget, forge_usage, forge_scan, forge_index, forge_cypher.".to_string()
    );

    let output = json!({
        "hookSpecificOutput": {
            "additionalContext": context_parts.join(" ")
        }
    });

    println!("{}", output);
}
