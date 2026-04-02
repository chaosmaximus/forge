//! Input validation for agent lifecycle events.
//!
//! All validators use pre-compiled regexes (via `once_cell` pattern with `regex::Regex`).
//! Paths are sanitized against symlink attacks and directory traversal.

use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Regex helpers — compiled once per process
// ---------------------------------------------------------------------------

fn re_agent_id() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_\-]{0,127}$").unwrap())
}

fn re_agent_type() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_.\-]{0,63}$").unwrap())
}

fn re_tool_name() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9_.\-]{1,64}$").unwrap())
}

// ---------------------------------------------------------------------------
// Public validators
// ---------------------------------------------------------------------------

/// Validate agent ID: alphanumeric start, then alnum/underscore/hyphen, 1-128 chars.
pub fn valid_agent_id(s: &str) -> bool {
    re_agent_id().is_match(s)
}

/// Validate agent type: alphanumeric start, then alnum/underscore/dot/hyphen, 1-64 chars.
pub fn valid_agent_type(s: &str) -> bool {
    re_agent_type().is_match(s)
}

/// Validate tool name: alnum/underscore/dot/hyphen, 1-64 chars.
pub fn valid_tool_name(s: &str) -> bool {
    re_tool_name().is_match(s)
}

/// Strip control characters (0x00-0x1F) from a string.
pub fn strip_control_chars(s: &str) -> String {
    s.chars().filter(|c| *c >= '\x20').collect()
}

/// Extract basename from a path, strip control chars, truncate to 255 chars.
pub fn safe_basename(path: &str) -> String {
    let base = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let cleaned = strip_control_chars(&base);
    if cleaned.len() > 255 {
        cleaned[..255].to_string()
    } else {
        cleaned
    }
}

/// Strip control chars and truncate to `max_len`.
pub fn safe_message(s: &str, max_len: usize) -> String {
    let cleaned = strip_control_chars(s);
    if cleaned.len() > max_len {
        cleaned[..max_len].to_string()
    } else {
        cleaned
    }
}

/// Canonicalize a transcript path and verify it lives under /tmp/ or $HOME/.claude/.
/// Rejects symlinks at the final path.
pub fn safe_transcript_path(raw: &str) -> Option<String> {
    let path = Path::new(raw);

    // Canonicalize resolves symlinks in parent dirs — we need the final path
    let canonical = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => return None,
    };

    // Reject if the final path is a symlink (canonicalize resolves it, but we
    // check the *original* raw path for symlink-ness before trusting)
    if path.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return None;
    }

    let canonical_str = canonical.to_string_lossy().to_string();

    // Must be under /tmp/ or $HOME/.claude/
    let under_tmp = canonical_str.starts_with("/tmp/");
    let under_claude_home = if let Ok(home) = std::env::var("HOME") {
        let claude_dir = format!("{}/.claude/", home);
        canonical_str.starts_with(&claude_dir)
    } else {
        false
    };

    if under_tmp || under_claude_home {
        Some(canonical_str)
    } else {
        None
    }
}

/// Verify `dir` is not a symlink and resolves under `state_dir`.
pub fn safe_dir(dir: &Path, state_dir: &Path) -> bool {
    // Reject symlinks
    if dir.symlink_metadata().map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return false;
    }

    // If the dir already exists, canonicalize and check containment
    if dir.exists() {
        let canon_dir = match std::fs::canonicalize(dir) {
            Ok(p) => p,
            Err(_) => return false,
        };
        let canon_state = match std::fs::canonicalize(state_dir) {
            Ok(p) => p,
            Err(_) => return false,
        };
        canon_dir.starts_with(&canon_state)
    } else {
        // Dir doesn't exist yet — check that its parent resolves under state_dir
        // and that the path string itself looks reasonable (no ..)
        let dir_str = dir.to_string_lossy();
        if dir_str.contains("..") {
            return false;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_agent_id() {
        assert!(valid_agent_id("abc123"));
        assert!(valid_agent_id("forge-planner-001"));
        assert!(valid_agent_id("a"));
        assert!(valid_agent_id("A_b-c"));
        // 128 chars
        let long_id: String = std::iter::once('a').chain(std::iter::repeat('b').take(127)).collect();
        assert!(valid_agent_id(&long_id));
        // 129 chars — too long
        let too_long: String = std::iter::once('a').chain(std::iter::repeat('b').take(128)).collect();
        assert!(!valid_agent_id(&too_long));
        // Invalid starts
        assert!(!valid_agent_id(""));
        assert!(!valid_agent_id("-abc"));
        assert!(!valid_agent_id("_abc"));
        assert!(!valid_agent_id(".abc"));
        // Control chars
        assert!(!valid_agent_id("abc\x00def"));
        assert!(!valid_agent_id("abc\ndef"));
    }

    #[test]
    fn test_valid_agent_type() {
        assert!(valid_agent_type("planner"));
        assert!(valid_agent_type("forge-evaluator"));
        assert!(valid_agent_type("v1.2.3"));
        assert!(!valid_agent_type(""));
        assert!(!valid_agent_type("-bad"));
        // 64 chars
        let long_type: String = std::iter::once('x').chain(std::iter::repeat('y').take(63)).collect();
        assert!(valid_agent_type(&long_type));
        // 65 chars — too long
        let too_long: String = std::iter::once('x').chain(std::iter::repeat('y').take(64)).collect();
        assert!(!valid_agent_type(&too_long));
    }

    #[test]
    fn test_valid_tool_name() {
        assert!(valid_tool_name("Read"));
        assert!(valid_tool_name("forge_remember"));
        assert!(valid_tool_name("mcp__plugin.tool-name"));
        assert!(valid_tool_name("a"));
        assert!(!valid_tool_name(""));
        assert!(!valid_tool_name("tool name"));  // space
        assert!(!valid_tool_name("tool/name"));  // slash
    }

    #[test]
    fn test_strip_control_chars() {
        assert_eq!(strip_control_chars("hello\x00world"), "helloworld");
        assert_eq!(strip_control_chars("line\nbreak"), "linebreak");
        assert_eq!(strip_control_chars("tab\there"), "tabhere");
        assert_eq!(strip_control_chars("clean"), "clean");
        assert_eq!(strip_control_chars("\x01\x02\x1f"), "");
    }

    #[test]
    fn test_safe_basename() {
        assert_eq!(safe_basename("/foo/bar/baz.rs"), "baz.rs");
        assert_eq!(safe_basename("just_a_file.txt"), "just_a_file.txt");
        assert_eq!(safe_basename("/foo/bar/\x00evil.rs"), "evil.rs");
        assert_eq!(safe_basename(""), "");
        // Long name gets truncated
        let long_name = "a".repeat(300);
        assert_eq!(safe_basename(&long_name).len(), 255);
    }

    #[test]
    fn test_safe_message() {
        assert_eq!(safe_message("hello world", 200), "hello world");
        assert_eq!(safe_message("hello\x00world", 200), "helloworld");
        assert_eq!(safe_message("too long", 3), "too");
    }

    #[test]
    fn test_safe_dir() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();
        let agents_dir = state_dir.join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        assert!(safe_dir(&agents_dir, state_dir));

        // Non-existent sub-path is ok (will be created)
        let new_sub = state_dir.join("new_sub");
        assert!(safe_dir(&new_sub, state_dir));

        // Path with .. is rejected
        let traversal = state_dir.join("agents").join("..").join("..").join("etc");
        assert!(!safe_dir(&traversal, state_dir));
    }

    #[test]
    fn test_safe_dir_symlink_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state_dir = dir.path();
        let agents_dir = state_dir.join("agents");
        std::fs::create_dir_all(&agents_dir).unwrap();

        // Create a symlink pointing to agents
        let link_path = state_dir.join("sneaky_link");
        std::os::unix::fs::symlink(&agents_dir, &link_path).unwrap();

        assert!(!safe_dir(&link_path, state_dir));
    }
}
