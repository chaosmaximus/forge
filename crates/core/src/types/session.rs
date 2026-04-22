use serde::{Deserialize, Serialize};

/// A parsed conversation turn from a Claude Code JSONL transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationChunk {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub has_tool_use: bool,
    /// Tool names referenced by `tool_use` blocks in this turn. Order-preserving,
    /// duplicates included (two Bash calls → two entries). Empty when
    /// `has_tool_use` is false. Closes SESSION-GAPS #54 Layer 1: adapters strip
    /// `<tool_use>` markers at parse time, so the extractor's per-tool counter
    /// reads from this field instead of regex-scanning `content`.
    #[serde(default)]
    pub tool_names: Vec<String>,
    pub timestamp: String,
    pub extracted: bool,
}

/// Raw JSONL line from a Claude Code transcript.
#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptLine {
    #[serde(rename = "type")]
    pub line_type: Option<String>,
    pub message: Option<TranscriptMessage>,
    pub uuid: Option<String>,
    pub timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptMessage {
    pub role: Option<String>,
    pub content: Option<serde_json::Value>,
}

impl TranscriptLine {
    /// Extract text content. Handles string content AND array-of-blocks content.
    /// For arrays: concatenate all blocks with type="text", join with newline.
    /// Returns None if no text content (e.g., tool-only turns).
    pub fn text_content(&self) -> Option<String> {
        let content = self.message.as_ref()?.content.as_ref()?;

        match content {
            // Simple string content (e.g., user messages)
            serde_json::Value::String(s) => {
                if s.is_empty() {
                    None
                } else {
                    Some(s.clone())
                }
            }
            // Array of content blocks (e.g., assistant messages)
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

    /// Check if this line contains tool_use blocks.
    pub fn has_tool_use(&self) -> bool {
        let Some(msg) = &self.message else {
            return false;
        };
        let Some(content) = &msg.content else {
            return false;
        };
        let serde_json::Value::Array(blocks) = content else {
            return false;
        };

        blocks.iter().any(|block| {
            block
                .get("type")
                .and_then(|v| v.as_str())
                .is_some_and(|t| t == "tool_use")
        })
    }

    /// Extract names of all tool_use blocks in this line. Order-preserving,
    /// duplicates included (two Bash calls → two entries). Empty vec when
    /// there are no tool_use blocks or content is not an array.
    pub fn tool_names(&self) -> Vec<String> {
        let Some(msg) = &self.message else {
            return Vec::new();
        };
        let Some(content) = &msg.content else {
            return Vec::new();
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_message() {
        let json = r#"{"type":"user","message":{"role":"user","content":"hello world"},"uuid":"abc","timestamp":"2026-04-02T12:00:00Z","sessionId":"sess1"}"#;
        let line: TranscriptLine = serde_json::from_str(json).expect("parse user message");

        assert_eq!(line.line_type, Some("user".to_string()));
        assert_eq!(line.uuid, Some("abc".to_string()));
        assert_eq!(line.timestamp, Some("2026-04-02T12:00:00Z".to_string()));
        assert_eq!(line.session_id, Some("sess1".to_string()));

        let text = line.text_content();
        assert_eq!(text, Some("hello world".to_string()));
        assert!(!line.has_tool_use());
    }

    #[test]
    fn test_parse_assistant_with_blocks() {
        let json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I'll help"},{"type":"tool_use","id":"t1","name":"Read","input":{}}]},"uuid":"def"}"#;
        let line: TranscriptLine = serde_json::from_str(json).expect("parse assistant with blocks");

        assert_eq!(line.line_type, Some("assistant".to_string()));
        assert_eq!(line.uuid, Some("def".to_string()));

        let text = line.text_content();
        assert_eq!(text, Some("I'll help".to_string()));
        assert!(line.has_tool_use());
    }

    #[test]
    fn test_parse_empty_content() {
        let json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{}}]}}"#;
        let line: TranscriptLine = serde_json::from_str(json).expect("parse tool-only message");

        let text = line.text_content();
        assert!(text.is_none());
        assert!(line.has_tool_use());
    }

    #[test]
    fn test_parse_empty_text_blocks() {
        let json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":""},{"type":"text","text":"  "}]}}"#;
        let line: TranscriptLine = serde_json::from_str(json).unwrap();
        assert!(
            line.text_content().is_none(),
            "empty/whitespace-only text should return None"
        );
    }

    #[test]
    fn test_chunk_serde_roundtrip() {
        let chunk = ConversationChunk {
            id: "chunk-001".to_string(),
            session_id: "sess1".to_string(),
            role: "user".to_string(),
            content: "hello world".to_string(),
            has_tool_use: false,
            tool_names: Vec::new(),
            timestamp: "2026-04-02T12:00:00Z".to_string(),
            extracted: false,
        };

        let json = serde_json::to_string(&chunk).expect("serialize chunk");
        let restored: ConversationChunk = serde_json::from_str(&json).expect("deserialize chunk");

        assert_eq!(chunk.id, restored.id);
        assert_eq!(chunk.session_id, restored.session_id);
        assert_eq!(chunk.role, restored.role);
        assert_eq!(chunk.content, restored.content);
        assert_eq!(chunk.has_tool_use, restored.has_tool_use);
        assert_eq!(chunk.tool_names, restored.tool_names);
        assert_eq!(chunk.timestamp, restored.timestamp);
        assert_eq!(chunk.extracted, restored.extracted);
    }

    #[test]
    fn test_tool_names_single() {
        let json = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I'll help"},{"type":"tool_use","id":"t1","name":"Read","input":{}}]}}"#;
        let line: TranscriptLine = serde_json::from_str(json).unwrap();
        assert_eq!(line.tool_names(), vec!["Read".to_string()]);
    }

    #[test]
    fn test_tool_names_multiple_preserves_order_and_dups() {
        let json = r#"{"type":"assistant","message":{"role":"assistant","content":[
            {"type":"tool_use","id":"t1","name":"Bash","input":{}},
            {"type":"text","text":"intermezzo"},
            {"type":"tool_use","id":"t2","name":"Read","input":{}},
            {"type":"tool_use","id":"t3","name":"Bash","input":{}}
        ]}}"#;
        let line: TranscriptLine = serde_json::from_str(json).unwrap();
        assert_eq!(
            line.tool_names(),
            vec!["Bash".to_string(), "Read".to_string(), "Bash".to_string()]
        );
    }

    #[test]
    fn test_tool_names_none_when_no_tool_use() {
        let json = r#"{"type":"user","message":{"role":"user","content":"hello"}}"#;
        let line: TranscriptLine = serde_json::from_str(json).unwrap();
        assert!(line.tool_names().is_empty());
    }

    #[test]
    fn test_tool_names_backwards_compat_deserialize() {
        // Legacy JSON without tool_names field (pre-#54 Layer 1 fix) should
        // default to empty vec via #[serde(default)].
        let legacy = r#"{"id":"c1","session_id":"s1","role":"user","content":"hi","has_tool_use":false,"timestamp":"","extracted":false}"#;
        let chunk: ConversationChunk = serde_json::from_str(legacy).unwrap();
        assert!(chunk.tool_names.is_empty());
    }
}
