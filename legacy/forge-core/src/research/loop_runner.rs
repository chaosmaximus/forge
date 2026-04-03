use crate::research::{ResearchResult, Finding};
use std::process::Command;

/// Execute the research loop infrastructure.
///
/// The Rust code does NOT do the actual research — that is the agent's job.
/// This provides: git branch creation, per-iteration checkpoints, structured
/// iteration state output, and keep/discard bookkeeping via git revert.
pub fn execute(topic: &str, max_iterations: usize, workdir: &str) -> ResearchResult {
    let branch = format!("research/{}", sanitize_branch_name(topic));
    let origin_ref = git_rev_parse(workdir);
    eprintln!("Origin ref: {}", origin_ref.as_deref().unwrap_or("none"));

    // Create research branch (or check out if it already exists)
    match git_create_branch(workdir, &branch) {
        Ok(()) => eprintln!("Created research branch: {}", branch),
        Err(e) => {
            eprintln!("Warning: could not create branch '{}': {}", branch, e);
            // Continue anyway — we might already be on a research branch
        }
    }

    let mut findings = Vec::new();

    for i in 0..max_iterations {
        let msg = format!("research-iter-{}", i);
        eprintln!("--- Iteration {}/{} ---", i + 1, max_iterations);

        // Create a git checkpoint for this iteration
        let checkpoint = git_checkpoint(workdir, &msg);
        let commit_hash = match &checkpoint {
            Ok(hash) => hash.clone(),
            Err(e) => {
                eprintln!("Checkpoint (iter {}): {}", i, e);
                "no-commit".to_string()
            }
        };

        let finding = Finding {
            iteration: i,
            action: format!("Iteration {} — explore", i),
            result: String::new(), // Agent fills this in
            kept: true,            // Default to kept; agent calls --discard to flip
            checkpoint: commit_hash,
        };
        findings.push(finding);

        // Output iteration state as JSON line for the agent to read
        if let Ok(json) = serde_json::to_string(&IterationState {
            iteration: i,
            max_iterations,
            topic,
            branch: &branch,
            status: "awaiting_agent",
        }) {
            eprintln!("ITERATION_STATE: {}", json);
        }
    }

    ResearchResult {
        topic: topic.to_string(),
        branch: branch.clone(),
        origin_ref: origin_ref.unwrap_or_default(),
        iterations: findings.len(),
        findings,
        conclusion: format!("Research on '{}' — {} iterations prepared on branch '{}'", topic, max_iterations, branch),
    }
}

/// Discard the last research iteration by reverting the most recent commit.
pub fn discard_last(workdir: &str) -> Result<String, String> {
    git_discard_last(workdir)
}

// ---------------------------------------------------------------------------
// Git helpers
// ---------------------------------------------------------------------------

/// Create and check out a new git branch for this research session.
fn git_create_branch(workdir: &str, branch: &str) -> Result<(), String> {
    let output = Command::new("git")
        .args(["checkout", "-b", branch])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("Failed to run git checkout: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        // If branch already exists, try to check it out
        if stderr.contains("already exists") {
            let retry = Command::new("git")
                .args(["checkout", branch])
                .current_dir(workdir)
                .output()
                .map_err(|e| format!("Failed to checkout existing branch: {}", e))?;
            if retry.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&retry.stderr).to_string())
            }
        } else {
            Err(stderr)
        }
    }
}

/// Commit the current working tree state as a research checkpoint.
/// Returns the resulting commit hash.
fn git_checkpoint(workdir: &str, message: &str) -> Result<String, String> {
    // Stage all changes
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("git add failed: {}", e))?;

    if !add.status.success() {
        return Err(String::from_utf8_lossy(&add.stderr).to_string());
    }

    // Check if there's anything to commit
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("git status failed: {}", e))?;

    let status_text = String::from_utf8_lossy(&status.stdout).to_string();
    if status_text.trim().is_empty() {
        // Nothing to commit — create an empty checkpoint commit
        let commit = Command::new("git")
            .args(["commit", "--allow-empty", "-m", message])
            .current_dir(workdir)
            .output()
            .map_err(|e| format!("git commit failed: {}", e))?;

        if !commit.status.success() {
            return Err(String::from_utf8_lossy(&commit.stderr).to_string());
        }
    } else {
        let commit = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(workdir)
            .output()
            .map_err(|e| format!("git commit failed: {}", e))?;

        if !commit.status.success() {
            return Err(String::from_utf8_lossy(&commit.stderr).to_string());
        }
    }

    // Return the new HEAD hash
    git_rev_parse(workdir).ok_or_else(|| "Could not read HEAD after commit".to_string())
}

/// Revert the last commit (discard an iteration's findings).
fn git_discard_last(workdir: &str) -> Result<String, String> {
    let output = Command::new("git")
        .args(["revert", "HEAD", "--no-edit"])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("git revert failed: {}", e))?;

    if output.status.success() {
        let hash = git_rev_parse(workdir).unwrap_or_default();
        Ok(format!("Reverted HEAD. New HEAD: {}", hash))
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// Get the current HEAD commit hash.
fn git_rev_parse(workdir: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workdir)
        .output()
        .ok()?;
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hash.is_empty() { None } else { Some(hash) }
}

/// Make a topic string safe for use as a git branch name.
/// Replaces non-alphanumeric characters (except hyphens) with hyphens,
/// collapses consecutive hyphens, and trims leading/trailing hyphens.
fn sanitize_branch_name(topic: &str) -> String {
    let sanitized: String = topic
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    // Collapse consecutive hyphens
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    // Trim leading/trailing hyphens, lowercase
    result.trim_matches('-').to_lowercase()
}

/// Per-iteration state emitted for the agent to read.
#[derive(serde::Serialize)]
struct IterationState<'a> {
    iteration: usize,
    max_iterations: usize,
    topic: &'a str,
    branch: &'a str,
    status: &'a str,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_branch_name_basic() {
        assert_eq!(sanitize_branch_name("test topic"), "test-topic");
    }

    #[test]
    fn test_sanitize_branch_name_special_chars() {
        assert_eq!(sanitize_branch_name("What is Rust's ownership model?"), "what-is-rust-s-ownership-model");
    }

    #[test]
    fn test_sanitize_branch_name_consecutive_spaces() {
        assert_eq!(sanitize_branch_name("foo   bar"), "foo-bar");
    }

    #[test]
    fn test_sanitize_branch_name_leading_trailing() {
        assert_eq!(sanitize_branch_name("  hello  "), "hello");
    }

    #[test]
    fn test_sanitize_branch_name_already_clean() {
        assert_eq!(sanitize_branch_name("my-topic"), "my-topic");
    }
}
