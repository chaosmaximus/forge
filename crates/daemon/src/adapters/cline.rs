use super::AgentAdapter;
use forge_core::types::ConversationChunk;
use serde::Deserialize;
use std::path::PathBuf;

/// Adapter for Cline (VS Code extension: saoudrizwan.claude-dev).
///
/// Cline stores transcripts as JSON arrays of Anthropic API messages at:
///   ~/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/<task-id>/api_conversation_history.json
pub struct ClineAdapter {
    tasks_dir: PathBuf,
}

/// One message in a Cline conversation history array.
#[derive(Debug, Deserialize)]
struct ClineMessage {
    role: Option<String>,
    content: Option<serde_json::Value>,
}

impl ClineAdapter {
    pub fn new(home: &str) -> Self {
        ClineAdapter {
            tasks_dir: PathBuf::from(home)
                .join(".config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks"),
        }
    }

    /// Extract text from a content value.
    /// - If it's a plain string, return it directly.
    /// - If it's an array of content blocks, collect all "text" blocks.
    fn extract_text(content: &serde_json::Value) -> Option<String> {
        match content {
            serde_json::Value::String(s) => {
                if s.is_empty() {
                    None
                } else {
                    Some(s.clone())
                }
            }
            serde_json::Value::Array(blocks) => {
                let texts: Vec<&str> = blocks
                    .iter()
                    .filter_map(|block| {
                        let block_type = block.get("type")?.as_str()?;
                        if block_type == "text" {
                            block.get("text")?.as_str()
                        } else {
                            None
                        }
                    })
                    .collect();

                if texts.is_empty() {
                    None
                } else {
                    let joined = texts.join("\n");
                    if joined.trim().is_empty() {
                        None
                    } else {
                        Some(joined)
                    }
                }
            }
            _ => None,
        }
    }

    /// Check if a content value contains any tool_use blocks.
    fn has_tool_use(content: &serde_json::Value) -> bool {
        if let serde_json::Value::Array(blocks) = content {
            blocks.iter().any(|block| {
                block
                    .get("type")
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| t == "tool_use")
            })
        } else {
            false
        }
    }

    /// Extract names of all tool_use blocks in a content value. Order-preserving,
    /// duplicates included. Returns empty vec when content is not an array or
    /// contains no tool_use blocks.
    fn tool_names(content: &serde_json::Value) -> Vec<String> {
        let serde_json::Value::Array(blocks) = content else {
            return Vec::new();
        };
        blocks
            .iter()
            .filter(|block| block.get("type").and_then(|v| v.as_str()) == Some("tool_use"))
            .filter_map(|block| {
                block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            })
            .collect()
    }
}

impl AgentAdapter for ClineAdapter {
    fn name(&self) -> &str {
        "cline"
    }

    fn watch_dirs(&self) -> Vec<PathBuf> {
        vec![self.tasks_dir.clone()]
    }

    fn matches(&self, path: &std::path::Path) -> bool {
        path.starts_with(&self.tasks_dir)
            && path
                .file_name()
                .is_some_and(|f| f == "api_conversation_history.json")
    }

    fn file_extension(&self) -> &str {
        "json"
    }

    fn parse(&self, content: &str) -> Vec<ConversationChunk> {
        let messages: Vec<ClineMessage> = match serde_json::from_str(content) {
            Ok(m) => m,
            Err(_) => return vec![],
        };

        let mut chunks = Vec::new();

        for (index, msg) in messages.iter().enumerate() {
            // Only keep user and assistant roles
            let role = match msg.role.as_deref() {
                Some("user") | Some("assistant") => msg.role.as_deref().unwrap(),
                _ => continue,
            };

            let Some(content_val) = &msg.content else {
                continue;
            };

            let text = match Self::extract_text(content_val) {
                Some(t) => t,
                None => continue,
            };

            let tool_use = Self::has_tool_use(content_val);
            let tool_names = Self::tool_names(content_val);

            chunks.push(ConversationChunk {
                id: format!("cline-{index}"),
                session_id: String::new(),
                role: role.to_string(),
                content: text,
                has_tool_use: tool_use,
                tool_names,
                timestamp: String::new(),
                extracted: false,
            });
        }

        chunks
    }

    fn parse_incremental(
        &self,
        content: &str,
        _last_offset: usize,
    ) -> (Vec<ConversationChunk>, usize) {
        // JSON arrays aren't incrementally parseable — re-parse fully each time
        let chunks = self.parse(content);
        (chunks, content.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cline_adapter_name() {
        let adapter = ClineAdapter::new("/tmp/fake_home");
        assert_eq!(adapter.name(), "cline");
        assert_eq!(adapter.file_extension(), "json");
    }

    #[test]
    fn test_cline_adapter_matches() {
        let adapter = ClineAdapter::new("/home/testuser");

        // Should match api_conversation_history.json under tasks dir
        let valid = std::path::Path::new(
            "/home/testuser/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/task-123/api_conversation_history.json",
        );
        assert!(adapter.matches(valid));

        // Should reject ui_messages.json under tasks dir
        let wrong_file = std::path::Path::new(
            "/home/testuser/.config/Code/User/globalStorage/saoudrizwan.claude-dev/tasks/task-123/ui_messages.json",
        );
        assert!(!adapter.matches(wrong_file));

        // Should reject api_conversation_history.json outside tasks dir
        let wrong_dir = std::path::Path::new("/home/testuser/other/api_conversation_history.json");
        assert!(!adapter.matches(wrong_dir));
    }

    #[test]
    fn test_parse_cline_string_content() {
        let adapter = ClineAdapter::new("/tmp/fake_home");
        let json = r#"[
            {"role":"user","content":"Hello, build me an API"},
            {"role":"assistant","content":"Sure, I will build an API for you."}
        ]"#;

        let chunks = adapter.parse(json);
        assert_eq!(chunks.len(), 2);

        assert_eq!(chunks[0].id, "cline-0");
        assert_eq!(chunks[0].role, "user");
        assert_eq!(chunks[0].content, "Hello, build me an API");
        assert!(!chunks[0].has_tool_use);

        assert_eq!(chunks[1].id, "cline-1");
        assert_eq!(chunks[1].role, "assistant");
        assert_eq!(chunks[1].content, "Sure, I will build an API for you.");
        assert!(!chunks[1].has_tool_use);
    }

    #[test]
    fn test_parse_cline_array_content() {
        let adapter = ClineAdapter::new("/tmp/fake_home");
        let json = r#"[
            {"role":"assistant","content":[
                {"type":"text","text":"Let me read that file."},
                {"type":"tool_use","id":"t1","name":"Read","input":{"path":"src/main.rs"}}
            ]},
            {"role":"user","content":[
                {"type":"text","text":"Here is the file content."}
            ]}
        ]"#;

        let chunks = adapter.parse(json);
        assert_eq!(chunks.len(), 2);

        assert_eq!(chunks[0].id, "cline-0");
        assert_eq!(chunks[0].role, "assistant");
        assert_eq!(chunks[0].content, "Let me read that file.");
        assert!(chunks[0].has_tool_use);
        assert_eq!(chunks[0].tool_names, vec!["Read".to_string()]);

        assert_eq!(chunks[1].id, "cline-1");
        assert_eq!(chunks[1].role, "user");
        assert_eq!(chunks[1].content, "Here is the file content.");
        assert!(!chunks[1].has_tool_use);
        assert!(chunks[1].tool_names.is_empty());
    }

    #[test]
    fn test_parse_cline_empty_and_malformed() {
        let adapter = ClineAdapter::new("/tmp/fake_home");

        // Empty string
        let chunks = adapter.parse("");
        assert!(chunks.is_empty());

        // Bad JSON
        let chunks = adapter.parse("not json at all");
        assert!(chunks.is_empty());

        // Empty array
        let chunks = adapter.parse("[]");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_parse_cline_skips_system_role() {
        let adapter = ClineAdapter::new("/tmp/fake_home");
        let json = r#"[
            {"role":"system","content":"You are a helpful assistant."},
            {"role":"user","content":"Hello"},
            {"role":"assistant","content":"Hi there!"},
            {"role":"tool","content":"some tool result"}
        ]"#;

        let chunks = adapter.parse(json);
        assert_eq!(chunks.len(), 2);

        assert_eq!(chunks[0].role, "user");
        assert_eq!(chunks[0].content, "Hello");
        assert_eq!(chunks[0].id, "cline-1"); // index 1 because system was at 0

        assert_eq!(chunks[1].role, "assistant");
        assert_eq!(chunks[1].content, "Hi there!");
        assert_eq!(chunks[1].id, "cline-2"); // index 2
    }
}
