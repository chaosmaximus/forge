//! Replay a real Claude Code transcript through the parser + per-tool
//! counter path to prove the #54 Layer 1 wiring works on production data.
//!
//! This test guards against regressions in:
//!  - `chunk::parse_transcript` populating `tool_names`
//!  - `TranscriptLine::tool_names()` extraction from the real Anthropic
//!    message schema
//!  - `extractor::record_tool_names` three-slug candidate lookup
//!    (`bare`, `cli:<name>`, `claude:<name>`)
//!
//! The fixture transcript lives outside the repo at
//! `~/.claude/projects/.../<uuid>.jsonl` and is a real session snapshot.
//! The test reads it via `$FORGE_TEST_TRANSCRIPT` or falls back to a
//! bundled inline fixture (see below) so it works in CI.
//!
//! Harness notes:
//! * Uses `DaemonState::new(":memory:")` so sqlite-vec + `create_schema` +
//!   `seed_claude_builtins` run through the exact production path — the
//!   same setup `e2e_sp1_dark_loops.rs` uses.
//! * Only exercises the sync subset (parse + counter), so no tokio runtime
//!   is needed.

use std::env;
use std::fs;

use forge_daemon::chunk::parse_transcript;
use forge_daemon::server::handler::DaemonState;
use forge_daemon::workers::extractor::record_tool_names;

fn fixture_transcript() -> String {
    // Prefer $FORGE_TEST_TRANSCRIPT (real data, local-only). Fallback: inline
    // minimal JSONL with two tool_use blocks (Bash + Read) in Anthropic
    // assistant message format for CI reproducibility.
    if let Ok(path) = env::var("FORGE_TEST_TRANSCRIPT") {
        return fs::read_to_string(&path).expect("read FORGE_TEST_TRANSCRIPT transcript");
    }
    let line1 = serde_json::json!({
        "type": "assistant",
        "uuid": "msg-a",
        "timestamp": "2026-04-21T00:00:00Z",
        "sessionId": "real-transcript-test",
        "message": {
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Let me check that." },
                { "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } },
                { "type": "tool_use", "id": "t2", "name": "Read", "input": { "file_path": "/tmp/a" } }
            ]
        }
    });
    let line2 = serde_json::json!({
        "type": "user",
        "uuid": "msg-b",
        "timestamp": "2026-04-21T00:00:01Z",
        "sessionId": "real-transcript-test",
        "message": { "role": "user", "content": "OK" }
    });
    format!("{line1}\n{line2}\n")
}

#[test]
fn real_transcript_feeds_per_tool_counter_end_to_end() {
    // DaemonState::new(":memory:") runs init_sqlite_vec + create_schema +
    // seed_claude_builtins through the exact production path, so the
    // `tool` table is populated with `claude:<lowercase>` rows before the
    // test asserts on counters.
    let state = DaemonState::new(":memory:").expect("DaemonState::new(:memory:)");
    let transcript = fixture_transcript();

    let chunks = parse_transcript(&transcript);
    assert!(!chunks.is_empty(), "parse_transcript produced 0 chunks");

    // Accumulate tool_names across all chunks (mirrors the real extractor
    // loop at workers/extractor.rs:303-312 — iterate chunks, forward
    // tool_names to record_tool_names per chunk).
    let mut total_increments = 0usize;
    let mut chunks_with_tools = 0usize;
    for chunk in &chunks {
        if !chunk.tool_names.is_empty() {
            chunks_with_tools += 1;
            let n = record_tool_names(&state.conn, &chunk.tool_names).expect("record_tool_names");
            total_increments += n;
        }
    }

    assert!(
        total_increments > 0,
        "expected ≥1 increment from transcript's tool_use blocks; got 0 \
         (chunks={}, chunks_with_tools={})",
        chunks.len(),
        chunks_with_tools,
    );

    // Verify at least one Claude builtin's counter moved. This asserts the
    // three-slug lookup resolved to `claude:<name>` — the only seeded
    // source in a fresh in-memory DB (detect_and_store_tools depends on
    // PATH and may return 0 rows in CI).
    let any_builtin_nonzero: i64 = state
        .conn
        .query_row(
            "SELECT COUNT(*) FROM tool WHERE id LIKE 'claude:%' AND use_count > 0",
            [],
            |r| r.get(0),
        )
        .expect("query claude:* tool counter");
    assert!(
        any_builtin_nonzero >= 1,
        "expected at least one claude:* builtin to have use_count > 0; got {any_builtin_nonzero}"
    );

    eprintln!(
        "real_transcript test: {} chunks, {} with tool_use, {} total per-tool increments, \
         {} claude:* counters > 0",
        chunks.len(),
        chunks_with_tools,
        total_increments,
        any_builtin_nonzero,
    );
}
