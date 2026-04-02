use crate::scan::rules::RULES;
use serde_json::json;
use std::path::Path;

pub fn run(file_path: &str) {
    let path = Path::new(file_path);

    // Symlink check
    if path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return;
    }

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut alerts: Vec<String> = Vec::new();

    for line in content.lines() {
        for rule in RULES.iter() {
            if rule.regex.is_match(line) {
                alerts.push(format!("{} detected.", rule.description));
                break; // One alert per line is enough
            }
        }
    }

    if !alerts.is_empty() {
        let alert_str = alerts.join(" ");
        let output = json!({
            "hookSpecificOutput": {
                "additionalContext": format!(
                    "SECRET ALERT in {}: {} Consider moving to .env or .gitignore.",
                    file_path, alert_str
                )
            }
        });
        println!("{}", output);
    }
}
