use forge_v2_core::types::{ConversationChunk, TranscriptLine};

/// Parse a Claude Code JSONL transcript file into conversation chunks.
/// Groups user + assistant turns. Skips tool-only turns (no text content).
/// Skips unparseable lines. Returns chunks in chronological order.
pub fn parse_transcript(content: &str) -> Vec<ConversationChunk> {
    let mut chunks = Vec::new();
    let mut counter = 0usize;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(tl) = serde_json::from_str::<TranscriptLine>(line) else {
            continue;
        };

        // Only process "user" or "assistant" lines
        let line_type = match &tl.line_type {
            Some(lt) if lt == "user" || lt == "assistant" => lt.clone(),
            _ => continue,
        };

        // Extract text content; skip if none or empty after trim
        let text = match tl.text_content() {
            Some(t) if !t.trim().is_empty() => t,
            _ => continue,
        };

        counter += 1;
        let id = tl
            .uuid
            .clone()
            .unwrap_or_else(|| format!("chunk-{counter}"));
        let has_tool_use = tl.has_tool_use();

        chunks.push(ConversationChunk {
            id,
            session_id: tl.session_id.unwrap_or_default(),
            role: line_type,
            content: text,
            has_tool_use,
            timestamp: tl.timestamp.unwrap_or_default(),
            extracted: false,
        });
    }

    chunks
}

/// Parse only NEW lines from a transcript, starting after `last_offset` bytes.
/// Returns (new_chunks, new_offset).
pub fn parse_transcript_incremental(
    content: &str,
    last_offset: usize,
) -> (Vec<ConversationChunk>, usize) {
    let slice = if last_offset >= content.len() {
        ""
    } else {
        &content[last_offset..]
    };

    let new_chunks = parse_transcript(slice);
    (new_chunks, content.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSCRIPT: &str = r#"{"type":"user","message":{"role":"user","content":"Build a JWT auth system"},"uuid":"u1","timestamp":"2026-04-02T12:00:00Z","sessionId":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"I'll implement JWT authentication."},{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/src/auth.rs"}}]},"uuid":"a1","timestamp":"2026-04-02T12:00:05Z","sessionId":"s1"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"file contents"}]},"uuid":"u2","sessionId":"s1"}
{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Based on the code, I recommend RS256."}]},"uuid":"a2","timestamp":"2026-04-02T12:01:00Z","sessionId":"s1"}"#;

    #[test]
    fn test_parse_transcript() {
        let chunks = parse_transcript(TRANSCRIPT);

        // tool_result-only user line (u2) should be skipped — 3 chunks expected
        assert_eq!(chunks.len(), 3, "expected 3 chunks, got {}", chunks.len());

        // First chunk: user message
        assert_eq!(chunks[0].role, "user");
        assert!(chunks[0].content.contains("JWT auth"));
        assert!(!chunks[0].has_tool_use);
        assert_eq!(chunks[0].id, "u1");
        assert_eq!(chunks[0].session_id, "s1");
        assert_eq!(chunks[0].timestamp, "2026-04-02T12:00:00Z");

        // Second chunk: assistant with text + tool_use
        assert_eq!(chunks[1].role, "assistant");
        assert!(chunks[1].content.contains("JWT authentication"));
        assert!(chunks[1].has_tool_use);
        assert_eq!(chunks[1].id, "a1");

        // Third chunk: assistant text-only
        assert_eq!(chunks[2].role, "assistant");
        assert!(chunks[2].content.contains("RS256"));
        assert!(!chunks[2].has_tool_use);
        assert_eq!(chunks[2].id, "a2");

        // All chunks should have extracted = false
        for chunk in &chunks {
            assert!(!chunk.extracted);
        }
    }

    #[test]
    fn test_parse_empty() {
        let chunks = parse_transcript("");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_parse_malformed_lines() {
        let chunks = parse_transcript("not json\n{\"type\":\"garbage\"}\n");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_incremental_parse() {
        // Build a 2-line transcript
        let line1 = r#"{"type":"user","message":{"role":"user","content":"Hello"},"uuid":"u1","timestamp":"2026-04-02T12:00:00Z","sessionId":"s1"}"#;
        let line2 = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Hi there!"}]},"uuid":"a1","timestamp":"2026-04-02T12:00:05Z","sessionId":"s1"}"#;
        let full = format!("{}\n{}\n", line1, line2);

        // Offset = length of first line + newline (simulating already processed)
        let first_line_offset = line1.len() + 1; // +1 for the newline

        let (chunks, new_offset) = parse_transcript_incremental(&full, first_line_offset);

        // Should only get the second line's chunk
        assert_eq!(chunks.len(), 1, "expected 1 chunk, got {}", chunks.len());
        assert_eq!(chunks[0].role, "assistant");
        assert!(chunks[0].content.contains("Hi there!"));
        assert_eq!(chunks[0].id, "a1");

        // New offset should be total length
        assert_eq!(new_offset, full.len());
    }
}
