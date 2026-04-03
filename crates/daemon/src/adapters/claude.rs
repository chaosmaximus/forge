use super::AgentAdapter;
use crate::chunk;
use forge_core::types::ConversationChunk;
use std::path::PathBuf;

pub struct ClaudeAdapter {
    watch_dir: PathBuf,
}

impl ClaudeAdapter {
    pub fn new(home: &str) -> Self {
        ClaudeAdapter {
            watch_dir: PathBuf::from(home).join(".claude").join("projects"),
        }
    }
}

impl AgentAdapter for ClaudeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    fn watch_dirs(&self) -> Vec<PathBuf> {
        // Always return the dir — the watcher polls for dirs that appear later
        {
            vec![self.watch_dir.clone()]
        }
    }

    fn matches(&self, path: &std::path::Path) -> bool {
        path.starts_with(&self.watch_dir) && path.extension().is_some_and(|e| e == "jsonl")
    }

    fn file_extension(&self) -> &str {
        "jsonl"
    }

    fn parse(&self, content: &str) -> Vec<ConversationChunk> {
        chunk::parse_transcript(content)
    }

    fn parse_incremental(
        &self,
        content: &str,
        last_offset: usize,
    ) -> (Vec<ConversationChunk>, usize) {
        chunk::parse_transcript_incremental(content, last_offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_adapter_name() {
        let adapter = ClaudeAdapter::new("/tmp/fake_home");
        assert_eq!(adapter.name(), "claude-code");
        assert_eq!(adapter.file_extension(), "jsonl");
    }

    #[test]
    fn test_claude_adapter_matches() {
        let adapter = ClaudeAdapter::new("/home/testuser");

        // Should match .jsonl under ~/.claude/projects/
        let valid = std::path::Path::new(
            "/home/testuser/.claude/projects/-some-project/conversation.jsonl",
        );
        assert!(adapter.matches(valid));

        // Should reject non-jsonl files under watch dir
        let wrong_ext =
            std::path::Path::new("/home/testuser/.claude/projects/-some-project/file.txt");
        assert!(!adapter.matches(wrong_ext));

        // Should reject .jsonl outside watch dir
        let wrong_dir = std::path::Path::new("/home/testuser/other/conversation.jsonl");
        assert!(!adapter.matches(wrong_dir));
    }

    #[test]
    fn test_claude_adapter_parse() {
        let adapter = ClaudeAdapter::new("/tmp/fake_home");
        let line = r#"{"type":"user","message":{"role":"user","content":"Build a JWT auth system"},"uuid":"u1","timestamp":"2026-04-02T12:00:00Z","sessionId":"s1"}"#;
        let content = format!("{}\n", line);

        let chunks = adapter.parse(&content);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].role, "user");
        assert!(chunks[0].content.contains("JWT auth"));
        assert_eq!(chunks[0].id, "u1");
        assert_eq!(chunks[0].session_id, "s1");
        assert!(!chunks[0].has_tool_use);
    }
}
