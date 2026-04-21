use forge_core::types::{ConversationChunk, TranscriptLine};

/// Parse a Claude Code JSONL transcript file into conversation chunks.
/// Groups user + assistant turns. Skips tool-only turns (no text content).
/// Skips unparseable lines. Returns chunks in chronological order.
pub fn parse_transcript(content: &str) -> Vec<ConversationChunk> {
    let mut chunks = Vec::new();
    let mut counter = 0usize;
    let mut skipped = 0u64;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Ok(tl) = serde_json::from_str::<TranscriptLine>(line) else {
            skipped += 1;
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
        let tool_names = tl.tool_names();

        chunks.push(ConversationChunk {
            id,
            session_id: tl.session_id.unwrap_or_default(),
            role: line_type,
            content: text,
            has_tool_use,
            tool_names,
            timestamp: tl.timestamp.unwrap_or_default(),
            extracted: false,
        });
    }

    if skipped > 0 {
        eprintln!("[chunk] skipped {skipped} unparseable lines");
    }

    chunks
}

/// Parse only NEW lines from a transcript, starting after `last_offset` bytes.
/// Returns (new_chunks, new_offset).
pub fn parse_transcript_incremental(
    content: &str,
    last_offset: usize,
) -> (Vec<ConversationChunk>, usize) {
    if last_offset > content.len() {
        // File was truncated or rotated — reset offset to beginning
        eprintln!(
            "[chunk] file truncated (offset {} > len {}), resetting",
            last_offset,
            content.len()
        );
        return parse_transcript_incremental(content, 0);
    }
    if last_offset == content.len() {
        return (Vec::new(), last_offset);
    }

    let new_content = &content[last_offset..];

    // Find the last complete line (ending with \n).
    // Only advance offset to the end of the last complete line so that
    // a partial line being written is not skipped.
    let safe_end = match new_content.rfind('\n') {
        Some(pos) => pos + 1,                     // include the newline
        None => return (Vec::new(), last_offset), // no complete line yet
    };

    let complete_content = &new_content[..safe_end];
    let chunks = parse_transcript(complete_content);
    (chunks, last_offset + safe_end)
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
        assert_eq!(chunks[1].tool_names, vec!["Read".to_string()]);
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
        let full = format!("{line1}\n{line2}\n");

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

    #[test]
    fn test_incremental_partial_line() {
        let complete_line = r#"{"type":"user","message":{"role":"user","content":"hello"},"uuid":"u1","sessionId":"s1"}"#;
        let partial = r#"{"type":"assistant","message":{"role":"assis"#; // incomplete

        let content = format!("{complete_line}\n{partial}");

        let (chunks, offset) = parse_transcript_incremental(&content, 0);
        assert_eq!(chunks.len(), 1, "should parse the complete line");
        // Offset should NOT include the partial line
        assert!(
            offset < content.len(),
            "offset should not advance past partial line"
        );
        assert_eq!(
            offset,
            complete_line.len() + 1,
            "offset should be at end of first line + newline"
        );

        // When more content arrives (partial line completed):
        let completed = format!(
            "{}\n{}\n",
            complete_line,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"world"}]},"uuid":"a1","sessionId":"s1"}"#
        );
        let (chunks2, offset2) = parse_transcript_incremental(&completed, offset);
        assert_eq!(
            chunks2.len(),
            1,
            "should parse the now-complete second line"
        );
        assert_eq!(offset2, completed.len());
    }

    #[test]
    fn test_incremental_file_truncation() {
        // Simulate a file that gets truncated (e.g., log rotation)
        let line1 = r#"{"type":"user","message":{"role":"user","content":"hello"},"uuid":"u1","sessionId":"s1"}"#;
        let original = format!("{line1}\n");

        // First read: parse the full file
        let (chunks, offset) = parse_transcript_incremental(&original, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(offset, original.len());

        // File gets truncated to something shorter
        let truncated = r#"{"type":"user","message":{"role":"user","content":"new"},"uuid":"u2","sessionId":"s2"}"#;
        let truncated_content = format!("{truncated}\n");

        // Old offset > new content length → should reset and parse from beginning
        assert!(offset > truncated_content.len());
        let (chunks2, offset2) = parse_transcript_incremental(&truncated_content, offset);
        assert_eq!(chunks2.len(), 1, "should parse the new file from beginning");
        assert_eq!(chunks2[0].content, "new");
        assert_eq!(offset2, truncated_content.len());
    }
}
