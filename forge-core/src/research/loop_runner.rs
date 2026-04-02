use crate::research::{ResearchResult, Finding};
use std::process::Command;

pub fn execute(topic: &str, max_iterations: usize, workdir: &str) -> ResearchResult {
    let mut findings = Vec::new();
    let checkpoint = git_rev_parse(workdir);
    eprintln!("Checkpoint: {}", checkpoint.as_deref().unwrap_or("none"));

    for i in 1..=max_iterations {
        eprintln!("--- Iteration {}/{} ---", i, max_iterations);
        findings.push(Finding {
            iteration: i,
            action: format!("Explore iteration {} for: {}", i, topic),
            result: "Awaiting agent action".to_string(),
            kept: false,
        });
    }

    ResearchResult {
        topic: topic.to_string(),
        iterations: findings.len(),
        findings,
        conclusion: format!("Research on '{}' — {} iterations prepared", topic, max_iterations),
    }
}

fn git_rev_parse(workdir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workdir)
        .output()
        .ok()?;
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hash.is_empty() { None } else { Some(hash) }
}
