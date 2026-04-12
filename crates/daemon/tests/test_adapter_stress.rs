//! Stress tests for all 3 transcript adapters.
//! Exercises malformed, edge-case, and adversarial input.

use forge_daemon::adapters::{claude, cline, codex, AgentAdapter};

// ---------------------------------------------------------------------------
// Claude adapter stress tests
// ---------------------------------------------------------------------------

#[test]
fn test_claude_empty_jsonl() {
    let adapter = claude::ClaudeAdapter::new("/tmp/fake");
    let chunks = adapter.parse("");
    assert!(chunks.is_empty(), "empty string should produce 0 chunks");
}

#[test]
fn test_claude_malformed_lines() {
    let adapter = claude::ClaudeAdapter::new("/tmp/fake");
    // Line 1: valid user message
    // Line 2: garbage
    // Line 3: valid JSON but missing required fields
    // Line 4: valid assistant message
    let content = concat!(
        r#"{"type":"user","message":{"role":"user","content":"hello"},"uuid":"u1","sessionId":"s1"}"#, "\n",
        "this is not json at all\n",
        r#"{"some":"random","json":"object"}"#, "\n",
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"world"}]},"uuid":"a1","sessionId":"s1"}"#, "\n",
    );
    let chunks = adapter.parse(content);
    assert_eq!(chunks.len(), 2, "should parse only the 2 valid lines, got {}", chunks.len());
    assert_eq!(chunks[0].role, "user");
    assert_eq!(chunks[0].content, "hello");
    assert_eq!(chunks[1].role, "assistant");
    assert!(chunks[1].content.contains("world"));
}

#[test]
fn test_claude_huge_single_line() {
    let adapter = claude::ClaudeAdapter::new("/tmp/fake");
    // Build a ~1MB content string embedded in a valid Claude JSONL line
    let big_text = "A".repeat(1_000_000);
    let line = format!(
        r#"{{"type":"user","message":{{"role":"user","content":"{big_text}"}},"uuid":"big1","sessionId":"s1"}}"#
    );
    let content = format!("{line}\n");
    let chunks = adapter.parse(&content);
    // Should parse successfully — 1 chunk with the huge content
    assert_eq!(chunks.len(), 1, "should parse the 1MB line");
    assert_eq!(chunks[0].content.len(), 1_000_000);
}

#[test]
fn test_claude_incremental_partial_line() {
    let adapter = claude::ClaudeAdapter::new("/tmp/fake");
    // A complete line followed by a partial line (no trailing newline)
    let complete = r#"{"type":"user","message":{"role":"user","content":"first"},"uuid":"u1","sessionId":"s1"}"#;
    let partial = r#"{"type":"assistant","message":{"role":"assis"#;
    let content = format!("{complete}\n{partial}");

    let (chunks, offset) = adapter.parse_incremental(&content, 0);
    assert_eq!(chunks.len(), 1, "should only parse the complete line");
    assert_eq!(chunks[0].content, "first");
    // Offset must NOT advance past the partial line
    assert_eq!(
        offset,
        complete.len() + 1,
        "offset should stop at end of first complete line"
    );
    assert!(
        offset < content.len(),
        "offset must not include the partial line"
    );
}

#[test]
fn test_claude_binary_content() {
    let adapter = claude::ClaudeAdapter::new("/tmp/fake");
    // Feed raw binary garbage — every byte value 0x00-0xFF
    let binary: Vec<u8> = (0..=255).collect();
    // This may not be valid UTF-8, so we use from_utf8_lossy
    let content = String::from_utf8_lossy(&binary);
    let chunks = adapter.parse(&content);
    assert!(chunks.is_empty(), "binary garbage should produce 0 chunks");
}

// ---------------------------------------------------------------------------
// Cline adapter stress tests
// ---------------------------------------------------------------------------

#[test]
fn test_cline_truncated_json() {
    let adapter = cline::ClineAdapter::new("/tmp/fake");
    // Missing closing bracket — malformed JSON array
    let content = r#"[{"role":"user","content":"hello"}"#;
    let chunks = adapter.parse(content);
    assert!(chunks.is_empty(), "truncated JSON array should produce 0 chunks");
}

#[test]
fn test_cline_deeply_nested() {
    let adapter = cline::ClineAdapter::new("/tmp/fake");
    // Build content with 50 levels of nested arrays
    let mut inner = String::from(r#""deep""#);
    for _ in 0..50 {
        inner = format!("[{inner}]");
    }
    let content = format!(
        r#"[{{"role":"user","content":{inner}}}]"#
    );
    // Should either parse (extracting nothing useful from nested arrays) or return empty.
    // Must NOT stack overflow or panic.
    let chunks = adapter.parse(&content);
    // The deeply nested content is not a string or array of content blocks with "text" type,
    // so it should be skipped → 0 chunks
    assert!(
        chunks.is_empty(),
        "deeply nested non-text content should be skipped, got {} chunks",
        chunks.len()
    );
}

#[test]
fn test_cline_null_content_field() {
    let adapter = cline::ClineAdapter::new("/tmp/fake");
    let content = r#"[{"role":"user","content":null}]"#;
    let chunks = adapter.parse(content);
    assert!(chunks.is_empty(), "null content should be skipped");
}

#[test]
fn test_cline_mixed_content_types() {
    let adapter = cline::ClineAdapter::new("/tmp/fake");
    let content = r#"[
        {"role":"user","content":"valid string"},
        {"role":"assistant","content":[{"type":"text","text":"valid block"}]},
        {"role":"user","content":null},
        {"role":"assistant","content":42},
        {"role":"user","content":true},
        {"role":"assistant","content":{"type":"text","text":"object not array"}},
        {"role":"user","content":"another valid"}
    ]"#;
    let chunks = adapter.parse(content);
    // Only string content and array-of-blocks content with "text" type should parse:
    // 1. "valid string" (string)
    // 2. "valid block" (array with text block)
    // 3. null → skipped
    // 4. 42 (number) → skipped
    // 5. true (bool) → skipped
    // 6. object (not string/array) → skipped
    // 7. "another valid" (string)
    assert_eq!(chunks.len(), 3, "expected 3 valid chunks, got {}", chunks.len());
    assert_eq!(chunks[0].content, "valid string");
    assert_eq!(chunks[1].content, "valid block");
    assert_eq!(chunks[2].content, "another valid");
}

#[test]
fn test_cline_empty_array() {
    let adapter = cline::ClineAdapter::new("/tmp/fake");
    let chunks = adapter.parse("[]");
    assert!(chunks.is_empty(), "empty array should produce 0 chunks");
}

#[test]
fn test_cline_huge_conversation() {
    let adapter = cline::ClineAdapter::new("/tmp/fake");
    // Build a JSON array with 1000 alternating user/assistant messages
    let mut messages = Vec::new();
    for i in 0..1000 {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        messages.push(format!(
            r#"{{"role":"{role}","content":"Message number {i}"}}"#
        ));
    }
    let content = format!("[{}]", messages.join(","));
    let chunks = adapter.parse(&content);
    assert_eq!(chunks.len(), 1000, "all 1000 messages should be parsed, got {}", chunks.len());
    // Verify first and last
    assert_eq!(chunks[0].role, "user");
    assert_eq!(chunks[0].content, "Message number 0");
    assert_eq!(chunks[999].role, "assistant");
    assert_eq!(chunks[999].content, "Message number 999");
}

// ---------------------------------------------------------------------------
// Codex adapter stress tests
// ---------------------------------------------------------------------------

#[test]
fn test_codex_empty_jsonl() {
    let adapter = codex::CodexAdapter::new("/tmp/fake");
    let chunks = adapter.parse("");
    assert!(chunks.is_empty(), "empty string should produce 0 chunks");
}

#[test]
fn test_codex_malformed_lines() {
    let adapter = codex::CodexAdapter::new("/tmp/fake");
    let content = concat!(
        r#"{"type":"response_item","timestamp":"t1","payload":{"role":"user","content":"valid line 1"}}"#, "\n",
        "complete garbage not json\n",
        r#"{"broken json"#, "\n",
        r#"{"type":"response_item","timestamp":"t2","payload":{"role":"assistant","content":"valid line 2"}}"#, "\n",
    );
    let chunks = adapter.parse(content);
    assert_eq!(chunks.len(), 2, "should parse only the 2 valid lines, got {}", chunks.len());
    assert_eq!(chunks[0].role, "user");
    assert_eq!(chunks[0].content, "valid line 1");
    assert_eq!(chunks[1].role, "assistant");
    assert_eq!(chunks[1].content, "valid line 2");
}

#[test]
fn test_codex_missing_payload() {
    let adapter = codex::CodexAdapter::new("/tmp/fake");
    // response_item with no payload field
    let content = r#"{"type":"response_item","timestamp":"t1"}"#.to_string() + "\n";
    let chunks = adapter.parse(&content);
    assert!(chunks.is_empty(), "missing payload should be skipped");
}

#[test]
fn test_codex_missing_role() {
    let adapter = codex::CodexAdapter::new("/tmp/fake");
    // payload present but missing "role" field
    let content =
        r#"{"type":"response_item","timestamp":"t1","payload":{"content":"no role here"}}"#
            .to_string()
            + "\n";
    let chunks = adapter.parse(&content);
    assert!(chunks.is_empty(), "payload without role should be skipped");
}

#[test]
fn test_codex_incremental_boundary() {
    let adapter = codex::CodexAdapter::new("/tmp/fake");
    let line1 = r#"{"type":"response_item","timestamp":"t1","payload":{"role":"user","content":"one"}}"#;
    let line2 = r#"{"type":"response_item","timestamp":"t2","payload":{"role":"assistant","content":"two"}}"#;
    let line3 = r#"{"type":"response_item","timestamp":"t3","payload":{"role":"user","content":"three"}}"#;
    let partial = r#"{"type":"response_item","timestamp":"t4","payload":{"role":"assi"#;

    // 3 complete lines + partial 4th (no trailing newline)
    let content = format!("{line1}\n{line2}\n{line3}\n{partial}");

    let (chunks, offset) = adapter.parse_incremental(&content, 0);
    assert_eq!(chunks.len(), 3, "should parse only the 3 complete lines, got {}", chunks.len());
    assert_eq!(chunks[0].content, "one");
    assert_eq!(chunks[1].content, "two");
    assert_eq!(chunks[2].content, "three");

    // Offset should be at end of line 3 (after the last \n before the partial)
    let expected_offset = line1.len() + 1 + line2.len() + 1 + line3.len() + 1;
    assert_eq!(offset, expected_offset, "offset should be at end of line 3");
    assert!(offset < content.len(), "offset must not include partial line");
}

#[test]
fn test_codex_large_content_blocks() {
    let adapter = codex::CodexAdapter::new("/tmp/fake");
    // Build a response_item with 100 content blocks
    let mut blocks = Vec::new();
    for i in 0..100 {
        blocks.push(format!(r#"{{"type":"output_text","text":"Block {i}"}}"#));
    }
    let blocks_json = blocks.join(",");
    let line = format!(
        r#"{{"type":"response_item","timestamp":"t1","payload":{{"role":"assistant","content":[{blocks_json}]}}}}"#
    );
    let content = format!("{line}\n");
    let chunks = adapter.parse(&content);
    assert_eq!(chunks.len(), 1, "should produce 1 chunk");
    // All 100 blocks should be extracted (joined with \n)
    for i in 0..100 {
        assert!(
            chunks[0].content.contains(&format!("Block {i}")),
            "missing Block {i} in content"
        );
    }
    // Count the number of blocks by splitting on newline
    let text_parts: Vec<&str> = chunks[0].content.split('\n').collect();
    assert_eq!(text_parts.len(), 100, "should have 100 text parts joined by newlines");
}

// ---------------------------------------------------------------------------
// Cross-adapter: no-panic on arbitrary input
// ---------------------------------------------------------------------------

#[test]
fn test_adapter_parse_does_not_panic_on_any_input() {
    let claude = claude::ClaudeAdapter::new("/tmp/fake");
    let cline_adapter = cline::ClineAdapter::new("/tmp/fake");
    let codex_adapter = codex::CodexAdapter::new("/tmp/fake");

    // Build a string with 256 copies of each byte value 0-255 (256 * 256 = 65536 bytes)
    // Use lossy conversion since raw bytes may not be valid UTF-8
    let all_bytes: Vec<u8> = (0..=255u8).flat_map(|b| std::iter::repeat_n(b, 256)).collect();
    let garbage = String::from_utf8_lossy(&all_bytes).into_owned();

    // Must not panic — just return empty (or whatever, as long as no crash)
    let _c1 = claude.parse(&garbage);
    let _c2 = cline_adapter.parse(&garbage);
    let _c3 = codex_adapter.parse(&garbage);

    // Also test parse_incremental
    let (_c4, _o1) = claude.parse_incremental(&garbage, 0);
    let (_c5, _o2) = cline_adapter.parse_incremental(&garbage, 0);
    let (_c6, _o3) = codex_adapter.parse_incremental(&garbage, 0);

    // If we got here without panicking, the test passes.
    // Verify they returned something (even if empty)
    assert!(_c1.len() <= all_bytes.len(), "should not produce more chunks than input bytes");
    assert!(_c2.len() <= all_bytes.len(), "should not produce more chunks than input bytes");
    assert!(_c3.len() <= all_bytes.len(), "should not produce more chunks than input bytes");
}
