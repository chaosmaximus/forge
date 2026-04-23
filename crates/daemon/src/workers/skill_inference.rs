//! Phase 23 Behavioral Skill Inference (2A-4c2).
//!
//! Pure helpers for turning a sequence of tool calls into a canonical
//! fingerprint, inferring a domain tag from the tool-name set, and
//! formatting a display name.
//!
//! All helpers here are side-effect-free; the DB-touching orchestrator
//! lives in `consolidator.rs::infer_skills_from_behavior`.

use sha2::{Digest, Sha256};

/// One clean tool call observed in a session.
///
/// Holds only the subset the fingerprint actually consumes: the tool
/// name and the sorted top-level keys of `tool_args`. Not related to
/// `ToolCallRow` (which carries the full record for other callers).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub tool_name: String,
    /// Top-level keys of the call's `tool_args` object, pre-sorted ASC.
    pub arg_keys: Vec<String>,
}

/// Canonical fingerprint of a session's tool-call sequence (Phase 23 input).
///
/// Shape: `sha256(json_canonical([sorted unique tool_names, sorted tool_arg_shapes]))`.
/// Tool-arg shapes are per-call sorted key sets, with the outer list also sorted
/// lexicographically. Values do NOT affect the hash — only structural key
/// presence.
pub fn canonical_fingerprint(calls: &[ToolCall]) -> String {
    let mut tool_names: Vec<String> = calls.iter().map(|c| c.tool_name.clone()).collect();
    tool_names.sort();
    tool_names.dedup();

    let mut arg_shapes: Vec<Vec<String>> = calls.iter().map(|c| c.arg_keys.clone()).collect();
    arg_shapes.sort();

    let canonical = serde_json::json!([tool_names, arg_shapes]);
    let canonical_bytes = canonical.to_string();

    let mut hasher = Sha256::new();
    hasher.update(canonical_bytes.as_bytes());
    let digest = hasher.finalize();
    hex_encode(&digest)
}

/// Rule-based domain tag for a set of tool names.
///
/// Precedence: file-ops > shell > web > workflow > integration > general.
/// MCP-prefixed tools check last so a hypothetical `mcp__write__…` tool doesn't
/// win over the explicit file-ops case.
pub fn infer_domain(tool_names: &[String]) -> &'static str {
    let names: Vec<&str> = tool_names.iter().map(|s| s.as_str()).collect();
    let file_ops = [
        "Read",
        "Write",
        "Edit",
        "Glob",
        "Grep",
        "MultiEdit",
        "NotebookEdit",
    ];
    if names.iter().any(|n| file_ops.contains(n)) {
        return "file-ops";
    }
    if names.contains(&"Bash") {
        return "shell";
    }
    if names.contains(&"WebFetch") || names.contains(&"WebSearch") {
        return "web";
    }
    if names.contains(&"TodoWrite") || names.contains(&"Task") {
        return "workflow";
    }
    if names.iter().any(|n| n.starts_with("mcp__")) {
        return "integration";
    }
    "general"
}

/// Display name per spec Q5: `"Inferred: {sorted-tools} [{hash8}]"`.
pub fn format_skill_name(tool_names: &[String], fingerprint: &str) -> String {
    let mut sorted = tool_names.to_vec();
    sorted.sort();
    sorted.dedup();
    let tools = sorted.join("+");
    let short_hash = fingerprint.chars().take(8).collect::<String>();
    format!("Inferred: {tools} [{short_hash}]")
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(name: &str, keys: &[&str]) -> ToolCall {
        let mut k: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
        k.sort();
        ToolCall {
            tool_name: name.to_string(),
            arg_keys: k,
        }
    }

    #[test]
    fn canonical_fingerprint_is_deterministic() {
        let a = [
            tc("Read", &["file_path"]),
            tc("Edit", &["file_path", "old_string", "new_string"]),
            tc("Bash", &["cmd"]),
        ];
        let b = [
            tc("Bash", &["cmd"]),
            tc("Read", &["file_path"]),
            tc("Edit", &["new_string", "file_path", "old_string"]),
        ];
        assert_eq!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn canonical_fingerprint_ignores_arg_values_only_keys() {
        let a = [tc("Read", &["file_path"])];
        let b = [tc("Read", &["file_path"])];
        assert_eq!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn canonical_fingerprint_distinguishes_different_arg_keys() {
        let a = [tc("Bash", &["cmd"])];
        let b = [tc("Bash", &["cmd", "run_id"])];
        assert_ne!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn canonical_fingerprint_distinguishes_different_tool_sets() {
        let a = [tc("Read", &["file_path"]), tc("Edit", &["file_path"])];
        let b = [
            tc("Read", &["file_path"]),
            tc("Edit", &["file_path"]),
            tc("Bash", &["cmd"]),
        ];
        assert_ne!(canonical_fingerprint(&a), canonical_fingerprint(&b));
    }

    #[test]
    fn infer_domain_file_ops_match() {
        assert_eq!(
            infer_domain(&["Read".to_string(), "Edit".to_string()]),
            "file-ops"
        );
        assert_eq!(infer_domain(&["Glob".to_string()]), "file-ops");
    }

    #[test]
    fn infer_domain_shell_when_only_bash() {
        assert_eq!(infer_domain(&["Bash".to_string()]), "shell");
    }

    #[test]
    fn infer_domain_mcp_prefix() {
        assert_eq!(
            infer_domain(&["mcp__context7__query-docs".to_string()]),
            "integration"
        );
    }

    #[test]
    fn infer_domain_general_fallback() {
        assert_eq!(infer_domain(&["SomeUnknownTool".to_string()]), "general");
    }

    #[test]
    fn format_skill_name_contains_hash_prefix() {
        let n = format_skill_name(
            &["Edit".to_string(), "Read".to_string(), "Bash".to_string()],
            "abcdef1234567890",
        );
        assert_eq!(n, "Inferred: Bash+Edit+Read [abcdef12]");
    }
}
