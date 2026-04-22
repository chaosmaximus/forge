use super::AgentAdapter;
use forge_core::types::ConversationChunk;
use serde::Deserialize;
use std::path::PathBuf;

pub struct CodexAdapter {
    sessions_dir: PathBuf,
}

impl CodexAdapter {
    pub fn new(home: &str) -> Self {
        CodexAdapter {
            sessions_dir: PathBuf::from(home).join(".codex").join("sessions"),
        }
    }
}

/// A single JSONL line from a Codex CLI transcript.
#[derive(Debug, Deserialize)]
struct CodexLine {
    #[serde(rename = "type")]
    line_type: String,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

/// The payload inside a `response_item` line.
#[derive(Debug, Deserialize)]
struct CodexPayload {
    role: String,
    #[serde(default)]
    content: serde_json::Value,
}

/// Extract text, tool_use flag, and tool names from a Codex payload content field.
/// Content can be a plain string or an array of content blocks.
fn extract_content(content: &serde_json::Value) -> (String, bool, Vec<String>) {
    match content {
        serde_json::Value::String(s) => (s.clone(), false, Vec::new()),
        serde_json::Value::Array(blocks) => {
            let mut texts = Vec::new();
            let mut has_tool_use = false;
            let mut tool_names = Vec::new();

            for block in blocks {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match block_type {
                    "output_text" | "input_text" | "text" => {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            texts.push(text.to_string());
                        }
                    }
                    "tool_use" | "function_call" => {
                        has_tool_use = true;
                        if let Some(name) = block
                            .get("name")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                        {
                            tool_names.push(name.to_string());
                        }
                    }
                    _ => {}
                }
            }

            (texts.join("\n"), has_tool_use, tool_names)
        }
        _ => (String::new(), false, Vec::new()),
    }
}

/// Parse Codex CLI JSONL transcript into ConversationChunks.
fn parse_codex_transcript(content: &str) -> Vec<ConversationChunk> {
    let mut chunks = Vec::new();
    let mut counter = 0usize;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(codex_line) = serde_json::from_str::<CodexLine>(line) else {
            continue;
        };

        // Only process response_item lines
        if codex_line.line_type != "response_item" {
            continue;
        }

        let Some(payload_value) = codex_line.payload else {
            continue;
        };

        let Ok(payload) = serde_json::from_value::<CodexPayload>(payload_value) else {
            continue;
        };

        // Map "developer" → "user", keep "user" and "assistant"
        let role = match payload.role.as_str() {
            "user" | "developer" => "user".to_string(),
            "assistant" => "assistant".to_string(),
            _ => continue,
        };

        let (text, has_tool_use, tool_names) = extract_content(&payload.content);

        // Skip empty content
        if text.trim().is_empty() {
            continue;
        }

        counter += 1;
        chunks.push(ConversationChunk {
            id: format!("codex-{counter}"),
            session_id: String::new(),
            role,
            content: text,
            has_tool_use,
            tool_names,
            timestamp: codex_line.timestamp.unwrap_or_default(),
            extracted: false,
        });
    }

    chunks
}

impl AgentAdapter for CodexAdapter {
    fn name(&self) -> &str {
        "codex"
    }

    fn watch_dirs(&self) -> Vec<PathBuf> {
        vec![self.sessions_dir.clone()]
    }

    fn matches(&self, path: &std::path::Path) -> bool {
        path.starts_with(&self.sessions_dir) && path.extension().is_some_and(|e| e == "jsonl")
    }

    fn file_extension(&self) -> &str {
        "jsonl"
    }

    fn parse(&self, content: &str) -> Vec<ConversationChunk> {
        parse_codex_transcript(content)
    }

    fn parse_incremental(
        &self,
        content: &str,
        last_offset: usize,
    ) -> (Vec<ConversationChunk>, usize) {
        if last_offset > content.len() {
            eprintln!(
                "[codex] file truncated (offset {} > len {}), resetting",
                last_offset,
                content.len()
            );
            return self.parse_incremental(content, 0);
        }
        if last_offset == content.len() {
            return (Vec::new(), last_offset);
        }

        let new_content = &content[last_offset..];

        // Find the last complete line (ending with \n).
        let safe_end = match new_content.rfind('\n') {
            Some(pos) => pos + 1,
            None => return (Vec::new(), last_offset),
        };

        let complete_content = &new_content[..safe_end];
        let chunks = parse_codex_transcript(complete_content);
        (chunks, last_offset + safe_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codex_adapter_name() {
        let adapter = CodexAdapter::new("/tmp/fake_home");
        assert_eq!(adapter.name(), "codex");
        assert_eq!(adapter.file_extension(), "jsonl");
    }

    #[test]
    fn test_codex_adapter_matches() {
        let adapter = CodexAdapter::new("/home/testuser");

        // Should match .jsonl under ~/.codex/sessions/
        let valid = std::path::Path::new(
            "/home/testuser/.codex/sessions/2026/04/03/rollout-1234-abcd.jsonl",
        );
        assert!(adapter.matches(valid));

        // Should reject non-jsonl files under sessions dir
        let wrong_ext =
            std::path::Path::new("/home/testuser/.codex/sessions/2026/04/03/rollout-1234-abcd.txt");
        assert!(!adapter.matches(wrong_ext));

        // Should reject .jsonl outside sessions dir (e.g. Claude paths)
        let claude_path = std::path::Path::new(
            "/home/testuser/.claude/projects/-some-project/conversation.jsonl",
        );
        assert!(!adapter.matches(claude_path));

        // Should reject .jsonl in a completely different dir
        let other_dir = std::path::Path::new("/home/testuser/other/transcript.jsonl");
        assert!(!adapter.matches(other_dir));
    }

    #[test]
    fn test_parse_codex_transcript() {
        let transcript = r#"{"type":"session_meta","timestamp":"2026-04-03T10:00:00Z","payload":{"session_id":"sess-1","model":"o4-mini"}}
{"type":"response_item","timestamp":"2026-04-03T10:00:01Z","payload":{"role":"user","content":"Build a REST API"}}
{"type":"event_msg","timestamp":"2026-04-03T10:00:02Z","payload":{"event":"thinking"}}
{"type":"response_item","timestamp":"2026-04-03T10:00:03Z","payload":{"role":"assistant","content":[{"type":"output_text","text":"I'll create the REST API."},{"type":"tool_use","id":"t1","name":"write_file"}]}}
{"type":"response_item","timestamp":"2026-04-03T10:00:04Z","payload":{"role":"assistant","content":[{"type":"text","text":"The API is ready."}]}}
"#;

        let chunks = parse_codex_transcript(transcript);
        assert_eq!(chunks.len(), 3, "expected 3 chunks, got {}", chunks.len());

        // First: user message (plain string content)
        assert_eq!(chunks[0].id, "codex-1");
        assert_eq!(chunks[0].role, "user");
        assert_eq!(chunks[0].content, "Build a REST API");
        assert!(!chunks[0].has_tool_use);
        assert_eq!(chunks[0].timestamp, "2026-04-03T10:00:01Z");
        assert!(!chunks[0].extracted);

        // Second: assistant with tool_use
        assert_eq!(chunks[1].id, "codex-2");
        assert_eq!(chunks[1].role, "assistant");
        assert!(chunks[1].content.contains("I'll create the REST API"));
        assert!(chunks[1].has_tool_use);
        assert_eq!(chunks[1].tool_names, vec!["write_file".to_string()]);
        assert_eq!(chunks[1].timestamp, "2026-04-03T10:00:03Z");

        // Third: assistant text-only
        assert_eq!(chunks[2].id, "codex-3");
        assert_eq!(chunks[2].role, "assistant");
        assert!(chunks[2].content.contains("The API is ready"));
        assert!(!chunks[2].has_tool_use);
    }

    #[test]
    fn test_parse_codex_developer_role_maps_to_user() {
        let transcript = r#"{"type":"response_item","timestamp":"2026-04-03T10:00:00Z","payload":{"role":"developer","content":"You are a coding assistant. Always use Rust."}}
"#;

        let chunks = parse_codex_transcript(transcript);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].role, "user");
        assert!(chunks[0].content.contains("coding assistant"));
    }

    #[test]
    fn test_parse_codex_skips_non_response_items() {
        let transcript = r#"{"type":"session_meta","timestamp":"2026-04-03T10:00:00Z","payload":{"session_id":"sess-1"}}
{"type":"event_msg","timestamp":"2026-04-03T10:00:01Z","payload":{"event":"start"}}
{"type":"event_msg","timestamp":"2026-04-03T10:00:02Z","payload":{"event":"thinking"}}
"#;

        let chunks = parse_codex_transcript(transcript);
        assert!(
            chunks.is_empty(),
            "session_meta and event_msg should be skipped"
        );
    }

    #[test]
    fn test_parse_codex_empty_and_malformed() {
        // Empty string
        let chunks = parse_codex_transcript("");
        assert!(chunks.is_empty());

        // Malformed JSON
        let chunks = parse_codex_transcript("not json at all\n{\"broken\n");
        assert!(chunks.is_empty());

        // Valid JSON but wrong structure
        let chunks = parse_codex_transcript("{\"type\":\"response_item\"}\n");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_codex_incremental_parse() {
        let line1 = r#"{"type":"response_item","timestamp":"2026-04-03T10:00:00Z","payload":{"role":"user","content":"Hello"}}"#;
        let line2 = r#"{"type":"response_item","timestamp":"2026-04-03T10:00:01Z","payload":{"role":"assistant","content":[{"type":"output_text","text":"Hi there!"}]}}"#;
        let full = format!("{line1}\n{line2}\n");

        let adapter = CodexAdapter::new("/tmp/fake_home");

        // First parse: get both lines
        let (chunks, offset) = adapter.parse_incremental(&full, 0);
        assert_eq!(chunks.len(), 2, "expected 2 chunks on first parse");
        assert_eq!(chunks[0].role, "user");
        assert_eq!(chunks[1].role, "assistant");
        assert_eq!(offset, full.len());

        // Second call with same offset: nothing new
        let (chunks2, offset2) = adapter.parse_incremental(&full, offset);
        assert!(chunks2.is_empty(), "expected empty on second call");
        assert_eq!(offset2, offset);
    }
}
