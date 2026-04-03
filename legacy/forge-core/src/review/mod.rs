pub mod report;
use std::process::Command;

pub fn run(path: &str, base: &str, format: &str, council: bool) {
    eprintln!("=== Council Review ===");
    // P2 fix: reject base values that look like git options (prevent option injection)
    if base.starts_with('-') && !base.starts_with("--") {
        eprintln!("Error: base '{}' looks like a git option, not a commit ref", base);
        println!("{{\"error\":\"Invalid base ref\"}}");
        return;
    }
    let diff = get_diff(path, base);
    if diff.is_empty() {
        println!("{{\"status\":\"no_changes\",\"message\":\"No changes to review\"}}");
        return;
    }
    let files = get_changed_files(path, base);
    let context = report::ReviewContext {
        path: path.to_string(),
        base: base.to_string(),
        diff_lines: diff.lines().count(),
        files_changed: files.len(),
        files,
        diff_preview: diff.chars().take(5000).collect(),
    };

    if council {
        // Council mode: structured output with pre-built prompts for multi-model dispatch
        println!("{}", report::to_council_json(&context));
    } else {
        println!("{}", match format {
            "markdown" => report::to_markdown(&context),
            _ => report::to_json(&context),
        });
    }
}

fn get_diff(path: &str, base: &str) -> String {
    Command::new("git")
        .args(["diff", base, "--", path])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn get_changed_files(path: &str, base: &str) -> Vec<String> {
    Command::new("git")
        .args(["diff", "--name-only", base, "--", path])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout)
            .lines().filter(|l| !l.is_empty()).map(|l| l.to_string()).collect())
        .unwrap_or_default()
}
