//! Adversarial input tests for the Forge daemon.
//!
//! These tests verify that the daemon handles malformed, malicious, and edge-case
//! input safely — no crashes, no SQL injection, no data corruption.

use forge_core::protocol::*;
use forge_core::types::MemoryType;
use forge_daemon::db::vec;
use forge_daemon::server::handler::{handle_request, DaemonState};

/// Helper: create an in-memory DaemonState with sqlite-vec initialized.
fn make_state() -> DaemonState {
    vec::init_sqlite_vec();
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:) should succeed")
}

/// Helper: issue a Remember request and return the stored ID.
fn remember(
    state: &mut DaemonState,
    memory_type: MemoryType,
    title: &str,
    content: &str,
    tags: Option<Vec<String>>,
) -> String {
    let req = Request::Remember {
        memory_type,
        title: title.to_string(),
        content: content.to_string(),
        confidence: Some(0.9),
        tags,
        project: None,
        metadata: None,
    };
    let resp = handle_request(state, req);
    match resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => {
            assert!(!id.is_empty(), "stored id must be non-empty");
            id
        }
        other => panic!("expected Stored response, got: {other:?}"),
    }
}

/// Helper: issue a Recall request and return the results vec.
fn recall(state: &mut DaemonState, query: &str) -> Vec<MemoryResult> {
    let req = Request::Recall {
        query: query.to_string(),
        memory_type: None,
        project: None,
        limit: None,
        layer: None,
        since: None,
        include_flipped: None,
        query_embedding: None,
    };
    let resp = handle_request(state, req);
    match resp {
        Response::Ok {
            data: ResponseData::Memories { results, count },
        } => {
            assert_eq!(
                results.len(),
                count,
                "results.len() should equal count field"
            );
            results
        }
        other => panic!("expected Memories response, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 1: SQL injection attempts in Recall
// ---------------------------------------------------------------------------
#[test]
fn test_sql_injection_in_recall() {
    let mut state = make_state();

    // Store a normal memory to have something in the DB
    remember(
        &mut state,
        MemoryType::Decision,
        "Use JWT for auth",
        "JSON Web Tokens for stateless authentication",
        None,
    );

    // Attempt SQL injection via Recall queries — none should crash,
    // all should return 0 results (FTS5 sanitizer strips these)
    let injection_attempts = [
        "'; DROP TABLE memory; --",
        "\" OR 1=1 --",
        "MATCH * OR 1=1",
        "'; DELETE FROM memory WHERE 1=1; --",
        "1; SELECT * FROM sqlite_master; --",
        "UNION SELECT id, title FROM memory--",
        "') OR ('1'='1",
        "Robert'); DROP TABLE memory;--",
    ];

    for injection in &injection_attempts {
        let results = recall(&mut state, injection);
        // The sanitizer strips non-alphanumeric characters.
        // Some injections may have alphabetical remnants that match (e.g., "DROP" or "TABLE"),
        // but crucially the daemon must NOT crash and the DB must remain intact.
        // We simply verify no panic occurred (implicit) and recall still works.
        let _ = results; // no crash = pass
    }

    // Verify the database is still intact after all injection attempts
    let results = recall(&mut state, "JWT authentication");
    assert!(
        !results.is_empty(),
        "original memory should still be recallable after injection attempts"
    );
    assert!(
        results.iter().any(|r| r.memory.title.contains("JWT")),
        "JWT memory should still exist"
    );

    // Verify health still works (tables intact)
    let resp = handle_request(&mut state, Request::Health);
    match resp {
        Response::Ok {
            data: ResponseData::Health { decisions, .. },
        } => {
            assert_eq!(
                decisions, 1,
                "should still have exactly 1 decision after injection attempts"
            );
        }
        other => panic!("expected Health response, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 2: SQL injection in GuardrailsCheck and BlastRadius
// ---------------------------------------------------------------------------
#[test]
fn test_sql_injection_in_guardrails() {
    let mut state = make_state();

    // GuardrailsCheck with SQL injection in file parameter
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "'; DROP TABLE edge; --".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, .. },
        } => {
            assert!(safe, "injection file should be safe (no decisions linked)");
        }
        other => panic!("expected GuardrailsCheck response, got: {other:?}"),
    }

    // GuardrailsCheck with SQL injection in action parameter
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/main.rs".into(),
            action: "'; DROP TABLE memory; --".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, .. },
        } => {
            assert!(safe, "injection action should be safe");
        }
        other => panic!("expected GuardrailsCheck response, got: {other:?}"),
    }

    // BlastRadius with path traversal attempt
    let resp = handle_request(
        &mut state,
        Request::BlastRadius {
            file: "../../etc/passwd".into(),
        },
    );
    match resp {
        Response::Ok {
            data:
                ResponseData::BlastRadius {
                    decisions,
                    files_affected,
                    ..
                },
        } => {
            assert!(
                decisions.is_empty(),
                "path traversal should find no decisions"
            );
            assert!(
                files_affected.is_empty(),
                "path traversal should find no affected files"
            );
        }
        other => panic!("expected BlastRadius response, got: {other:?}"),
    }

    // BlastRadius with SQL injection
    let resp = handle_request(
        &mut state,
        Request::BlastRadius {
            file: "' UNION SELECT * FROM memory--".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::BlastRadius { decisions, .. },
        } => {
            assert!(
                decisions.is_empty(),
                "SQL injection in blast radius should find no decisions"
            );
        }
        other => panic!("expected BlastRadius response, got: {other:?}"),
    }

    // Verify tables still intact
    let resp = handle_request(&mut state, Request::Health);
    match resp {
        Response::Ok {
            data: ResponseData::Health { .. },
        } => { /* tables intact */ }
        other => panic!("expected Health response after injections, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 3: Unicode edge cases
// ---------------------------------------------------------------------------
#[test]
fn test_unicode_edge_cases() {
    let mut state = make_state();

    // Emoji in title
    let id_emoji = remember(
        &mut state,
        MemoryType::Decision,
        "Use JWT \u{1F510} for auth",
        "JSON Web Tokens with lock emoji in title",
        None,
    );

    // CJK characters in content
    let id_cjk = remember(
        &mut state,
        MemoryType::Lesson,
        "Authentication system design",
        "\u{8BA4}\u{8BC1}\u{7CFB}\u{7EDF}\u{4F7F}\u{7528}JWT",
        None,
    );

    // Mixed RTL/LTR text (Arabic + English)
    let id_rtl = remember(
        &mut state,
        MemoryType::Pattern,
        "\u{0645}\u{0639}\u{0645}\u{0627}\u{0631}\u{064A}\u{0629} REST API",
        "Arabic text mixed with English for REST architecture",
        None,
    );

    // Combining characters and diacritics
    let id_combining = remember(
        &mut state,
        MemoryType::Preference,
        "caf\u{00E9} na\u{00EF}ve r\u{00E9}sum\u{00E9}",
        "French accented characters in memory",
        None,
    );

    // All IDs should be distinct
    let ids = [&id_emoji, &id_cjk, &id_rtl, &id_combining];
    for (i, a) in ids.iter().enumerate() {
        for (j, b) in ids.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "all IDs must be unique");
            }
        }
    }

    // Recall with emoji-containing query — FTS5 sanitizer will strip the emoji
    // but the alphabetical words should match
    let results = recall(&mut state, "JWT auth");
    assert!(!results.is_empty(), "should find results for 'JWT auth'");

    // Verify titles are preserved in export
    let resp = handle_request(
        &mut state,
        Request::Export {
            format: None,
            since: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Export { memories, .. },
        } => {
            let titles: Vec<&str> = memories.iter().map(|m| m.memory.title.as_str()).collect();
            assert!(
                titles.iter().any(|t| t.contains('\u{1F510}')),
                "emoji should be preserved in export, got titles: {titles:?}"
            );
            assert!(
                memories
                    .iter()
                    .any(|m| m.memory.content.contains('\u{8BA4}')),
                "CJK content should be preserved in export"
            );
            assert!(
                titles.iter().any(|t| t.contains('\u{0645}')),
                "Arabic text should be preserved in export"
            );
            assert!(
                titles.iter().any(|t| t.contains("caf\u{00E9}")),
                "accented characters should be preserved in export"
            );
        }
        other => panic!("expected Export response, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 4: Extremely long strings
// ---------------------------------------------------------------------------
#[test]
fn test_extremely_long_strings() {
    let mut state = make_state();

    // 10,000 character title
    let long_title = "A".repeat(10_000);
    let id_long_title = remember(
        &mut state,
        MemoryType::Decision,
        &long_title,
        "Short content",
        None,
    );
    assert!(
        !id_long_title.is_empty(),
        "should store memory with 10k char title"
    );

    // 100,000 character content
    let long_content = "B".repeat(100_000);
    let id_long_content = remember(
        &mut state,
        MemoryType::Lesson,
        "Long content memory",
        &long_content,
        None,
    );
    assert!(
        !id_long_content.is_empty(),
        "should store memory with 100k char content"
    );

    // Recall should still work
    let results = recall(&mut state, "Long content memory");
    assert!(!results.is_empty(), "should recall the long content memory");
    assert!(
        results
            .iter()
            .any(|r| r.memory.title == "Long content memory"),
        "should find the long content memory by title"
    );

    // Verify the long content is actually stored
    let resp = handle_request(
        &mut state,
        Request::Export {
            format: None,
            since: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Export { memories, .. },
        } => {
            let long_title_mem = memories.iter().find(|m| m.memory.id == id_long_title);
            assert!(
                long_title_mem.is_some(),
                "long title memory should be in export"
            );
            assert_eq!(
                long_title_mem.unwrap().memory.title.len(),
                10_000,
                "10k title should be fully preserved"
            );

            let long_content_mem = memories.iter().find(|m| m.memory.id == id_long_content);
            assert!(
                long_content_mem.is_some(),
                "long content memory should be in export"
            );
            assert_eq!(
                long_content_mem.unwrap().memory.content.len(),
                100_000,
                "100k content should be fully preserved"
            );
        }
        other => panic!("expected Export response, got: {other:?}"),
    }

    // GuardrailsCheck with 5,000 character path — should not crash
    let long_path = "x/".repeat(2500);
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: long_path,
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, .. },
        } => {
            assert!(safe, "long path should be safe (no decisions linked)");
        }
        other => panic!("expected GuardrailsCheck response, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 5: Empty and null-like inputs
// ---------------------------------------------------------------------------
#[test]
fn test_empty_and_null_inputs() {
    let mut state = make_state();

    // Remember with empty title — should store (no validation rejects it)
    let resp = handle_request(
        &mut state,
        Request::Remember {
            memory_type: MemoryType::Decision,
            title: "".into(),
            content: "Some content".into(),
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
        },
    );
    match &resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => {
            assert!(!id.is_empty(), "should store even with empty title");
        }
        other => panic!("expected Stored for empty title, got: {other:?}"),
    }

    // Remember with empty content — should store
    let resp = handle_request(
        &mut state,
        Request::Remember {
            memory_type: MemoryType::Lesson,
            title: "Empty content test".into(),
            content: "".into(),
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
        },
    );
    match &resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => {
            assert!(!id.is_empty(), "should store even with empty content");
        }
        other => panic!("expected Stored for empty content, got: {other:?}"),
    }

    // Recall with empty query — should not crash, returns empty (sanitizer strips all)
    let results = recall(&mut state, "");
    // Empty query after sanitization → no FTS5 match → 0 results
    assert_eq!(results.len(), 0, "empty query should return 0 results");

    // Forget with empty id — should return error (no such memory)
    let resp = handle_request(&mut state, Request::Forget { id: "".into() });
    match resp {
        Response::Error { message } => {
            assert!(
                message.contains("not found") || message.contains("already deleted"),
                "empty id forget should error, got: {message}"
            );
        }
        other => panic!("expected Error for empty id forget, got: {other:?}"),
    }

    // GuardrailsCheck with empty file — should return safe
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, .. },
        } => {
            assert!(safe, "empty file should be safe");
        }
        other => panic!("expected GuardrailsCheck for empty file, got: {other:?}"),
    }

    // BlastRadius with empty file — should return empty result
    let resp = handle_request(&mut state, Request::BlastRadius { file: "".into() });
    match resp {
        Response::Ok {
            data:
                ResponseData::BlastRadius {
                    decisions,
                    files_affected,
                    ..
                },
        } => {
            assert!(
                decisions.is_empty(),
                "empty file blast radius should have no decisions"
            );
            assert!(
                files_affected.is_empty(),
                "empty file blast radius should have no affected files"
            );
        }
        other => panic!("expected BlastRadius for empty file, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 6: Special characters in fields
// ---------------------------------------------------------------------------
#[test]
fn test_special_characters_in_fields() {
    let mut state = make_state();

    // Title with newlines, tabs, and backslashes
    let id_special = remember(
        &mut state,
        MemoryType::Decision,
        "Line1\nLine2\tTabbed\\Backslash",
        "Content with\nnewlines\tand\ttabs",
        None,
    );
    assert!(
        !id_special.is_empty(),
        "should store memory with special chars in title"
    );

    // Title with null bytes — Rust strings can't contain \0 in str, but can in content
    // Actually Rust &str cannot have interior null bytes, so we test what we can:
    // control characters that are valid UTF-8
    let id_control = remember(
        &mut state,
        MemoryType::Lesson,
        "Control\x01\x02\x03chars",
        "Content with\x07bell\x08backspace",
        None,
    );
    assert!(
        !id_control.is_empty(),
        "should store memory with control chars"
    );

    // Tags with special characters
    let id_tags = remember(
        &mut state,
        MemoryType::Pattern,
        "Tagged memory",
        "Has tags with special chars",
        Some(vec![
            "tag-with-dash".into(),
            "tag.with.dots".into(),
            "tag/with/slashes".into(),
            "tag@with@at".into(),
            "tag with spaces".into(),
        ]),
    );
    assert!(
        !id_tags.is_empty(),
        "should store memory with special-char tags"
    );

    // Recall with FTS5 operators — should not crash
    let fts5_operator_queries = [
        "AND",
        "OR",
        "NOT",
        "NEAR",
        "*",
        "^",
        "JWT AND OR NOT",
        "NEAR(JWT, authentication, 5)",
        "JWT*",
        "^JWT",
        "{JWT}",
        "JWT + authentication",
        "\"JWT\" AND \"auth\"",
    ];

    for query in &fts5_operator_queries {
        let results = recall(&mut state, query);
        // No crash is the key assertion (implicit from reaching this point)
        let _ = results;
    }

    // Verify data integrity — special char memories should be in export
    let resp = handle_request(
        &mut state,
        Request::Export {
            format: None,
            since: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Export { memories, .. },
        } => {
            assert!(
                memories
                    .iter()
                    .any(|m| m.memory.title.contains("Line1\nLine2")),
                "newline in title should be preserved"
            );
            assert!(
                memories.iter().any(|m| m.memory.title.contains("Control")),
                "control-char memory should be in export"
            );
            // Verify tags are preserved
            let tagged = memories.iter().find(|m| m.memory.title == "Tagged memory");
            assert!(tagged.is_some(), "tagged memory should be in export");
            let tags = &tagged.unwrap().memory.tags;
            assert!(
                tags.contains(&"tag-with-dash".to_string()),
                "dash tag preserved"
            );
            assert!(
                tags.contains(&"tag.with.dots".to_string()),
                "dots tag preserved"
            );
            assert!(
                tags.contains(&"tag/with/slashes".to_string()),
                "slashes tag preserved"
            );
        }
        other => panic!("expected Export response, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 7: Duplicate handling (dedup by title + type)
// ---------------------------------------------------------------------------
#[test]
fn test_duplicate_handling() {
    let mut state = make_state();

    // Remember the same title+type 10 times
    for i in 0..10 {
        let _id = remember(
            &mut state,
            MemoryType::Decision,
            "Use JWT for auth",
            &format!("Attempt #{i}"),
            None,
        );
    }

    // Health should show exactly 1 decision (dedup by title + memory_type)
    let resp = handle_request(&mut state, Request::Health);
    match resp {
        Response::Ok {
            data: ResponseData::Health { decisions, .. },
        } => {
            assert_eq!(
                decisions, 1,
                "10 inserts of same title+type should dedup to 1"
            );
        }
        other => panic!("expected Health response, got: {other:?}"),
    }

    // Content should be the last one (dedup updates content)
    let results = recall(&mut state, "JWT auth");
    assert_eq!(results.len(), 1, "should have exactly 1 result after dedup");
    assert!(
        results[0].memory.content.contains("Attempt #9"),
        "content should be from last insert, got: {}",
        results[0].memory.content
    );

    // Same title but DIFFERENT type → should store both
    let _id_lesson = remember(
        &mut state,
        MemoryType::Lesson,
        "Use JWT for auth",
        "This is a lesson about JWT",
        None,
    );

    let resp = handle_request(&mut state, Request::Health);
    match resp {
        Response::Ok {
            data: ResponseData::Health {
                decisions, lessons, ..
            },
        } => {
            assert_eq!(decisions, 1, "should still have 1 decision");
            assert_eq!(
                lessons, 1,
                "should have 1 lesson with same title but different type"
            );
        }
        other => panic!("expected Health response, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 8: Import with malformed data
// ---------------------------------------------------------------------------
#[test]
fn test_import_malformed_data() {
    let mut state = make_state();

    // 8a: Invalid JSON → error response
    let resp = handle_request(
        &mut state,
        Request::Import {
            data: "this is not valid JSON {{{".into(),
        },
    );
    match resp {
        Response::Error { message } => {
            assert!(
                message.contains("parse error"),
                "invalid JSON should produce parse error, got: {message}"
            );
        }
        other => panic!("expected Error for invalid JSON import, got: {other:?}"),
    }

    // 8b: JSON missing required fields → skipped count > 0
    let partial_data = serde_json::json!({
        "memories": [
            {
                "title": "Missing fields",
                "content": "No id, no type"
            },
            {
                "garbage_field": true,
                "another": 42
            }
        ],
        "files": [],
        "symbols": []
    });
    let resp = handle_request(
        &mut state,
        Request::Import {
            data: partial_data.to_string(),
        },
    );
    match resp {
        Response::Ok {
            data:
                ResponseData::Import {
                    memories_imported,
                    skipped,
                    ..
                },
        } => {
            assert_eq!(
                memories_imported, 0,
                "malformed memories should not be imported"
            );
            assert!(
                skipped > 0,
                "malformed memories should be skipped, skipped={skipped}"
            );
        }
        other => panic!("expected Import response for partial data, got: {other:?}"),
    }

    // 8c: > 10,000 records → rejected (record limit)
    // Build a payload with 10,001 memory entries
    let oversized_memories: Vec<serde_json::Value> = (0..10_001)
        .map(|i| {
            serde_json::json!({
                "id": format!("oversized-{}", i),
                "memory_type": "decision",
                "title": format!("Memory {}", i),
                "content": "content",
                "confidence": 0.9,
                "status": "active",
                "project": null,
                "tags": [],
                "embedding": null,
                "created_at": "2026-04-02 10:00:00",
                "accessed_at": "2026-04-02 10:00:00"
            })
        })
        .collect();
    let oversized_payload = serde_json::json!({
        "memories": oversized_memories,
        "files": [],
        "symbols": []
    });
    let resp = handle_request(
        &mut state,
        Request::Import {
            data: oversized_payload.to_string(),
        },
    );
    match resp {
        Response::Error { message } => {
            assert!(
                message.contains("10000") || message.contains("record limit"),
                "oversized import should mention record limit, got: {message}"
            );
        }
        other => panic!("expected Error for oversized import, got: {other:?}"),
    }

    // 8d: Empty data with valid structure → 0 imported
    let empty_payload = serde_json::json!({
        "memories": [],
        "files": [],
        "symbols": []
    });
    let resp = handle_request(
        &mut state,
        Request::Import {
            data: empty_payload.to_string(),
        },
    );
    match resp {
        Response::Ok {
            data:
                ResponseData::Import {
                    memories_imported,
                    files_imported,
                    symbols_imported,
                    skipped,
                },
        } => {
            assert_eq!(
                memories_imported, 0,
                "empty import should import 0 memories"
            );
            assert_eq!(files_imported, 0, "empty import should import 0 files");
            assert_eq!(symbols_imported, 0, "empty import should import 0 symbols");
            assert_eq!(skipped, 0, "empty import should skip 0");
        }
        other => panic!("expected Import response for empty data, got: {other:?}"),
    }
}
