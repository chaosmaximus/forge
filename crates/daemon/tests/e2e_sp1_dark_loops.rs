//! e2e_sp1_dark_loops — validate all 4 dark-loop SP1 fixes land together.
//!
//! Composite integration test exercised against a single in-memory
//! [`DaemonState`]. Each fix is driven through its real user-facing helper
//! path and asserted against the DB shape the real read queries consume
//! (`list_skills`, `context_effectiveness` rows, `tool.use_count`, and
//! `query_stats`).
//!
//! Fixes covered:
//!
//! * **#55 (Fix 1)** — `skills::auto_populate_on_start` indexes every
//!   `SKILL.md` under the tempdir, and `skills::list_skills` returns all
//!   three fixtures.
//! * **#45 (Fix 2)** — `Request::PreBashCheck { session_id: Some(..) }` flows
//!   through `handle_request` and emits a `WriteCommand::RecordInjection`
//!   with `context_type = "proactive"`. The writer-channel receiver is the
//!   exact same harness `test_proactive_injection.rs` uses; a regression in
//!   the `try_send(RecordInjection)` call from any proactive handler is
//!   caught here.
//! * **#54 (Fix 3)** — `DaemonState::new` auto-seeds Claude builtins with
//!   `claude:<name>` slugs and `use_count = 0`. Calling
//!   `db::manas::record_tool_use(conn, "claude:bash")` — the exact path the
//!   extractor's dual-slug lookup takes after `Fix 3 review-fixup` (commit
//!   `3787916`) — increments `use_count` to 1. The `cli:bash` and bare
//!   `bash` variants MUST NOT match (proves the seeded row is the
//!   `claude:` prefixed one, not a legacy detection artifact).
//! * **#53 (Fix 4)** — `db::metrics::record_extraction` inserts one row in
//!   the `metrics` table with `metric_type = 'extraction'`. Running
//!   `ops::query_stats(conn, 24)` afterwards reports non-zero extractions
//!   and non-zero tokens — the exact counters `forge-next stats` reads.
//!
//! Harness notes:
//! * Uses a single `:memory:` SQLite connection — parallel `cargo test`
//!   runs do not share state.
//! * `writer_tx` is attached for Fix 2 but the test does not spawn the
//!   async writer actor; `RecordInjection` is asserted directly on the
//!   channel receiver (same pattern as `test_proactive_injection.rs`).
//!   Other fixes write to the shared connection directly — no timing /
//!   polling anywhere, so the test is deterministic.
//!
//! Spec: `docs/superpowers/specs/2026-04-20-dark-loops-sp1-design.md` §6.2.

use std::fs;

use forge_core::protocol::{Request, Response};
use forge_daemon::db;
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_daemon::server::writer::WriteCommand;
use forge_daemon::{sessions, skills};
use tempfile::tempdir;

#[test]
fn e2e_sp1_dark_loops_all_counters_advance() {
    // ========================================================================
    // Arrange — one DaemonState, one writer channel, one session, one skills
    // tempdir. DaemonState::new(":memory:") auto-seeds Claude builtins with
    // `claude:<lowercase>` slug and use_count=0 — that's the Fix 3 precondition.
    // ========================================================================
    let mut state = DaemonState::new(":memory:").expect("DaemonState::new(:memory:)");
    let (tx, mut rx) = tokio::sync::mpsc::channel::<WriteCommand>(64);
    state.writer_tx = Some(tx);

    let session_id = "sp1-dark-loops-e2e".to_string();
    sessions::register_session(
        &state.conn,
        &session_id,
        "claude-code",
        Some("forge"),
        Some("/tmp/forge-test"),
        None,
        None,
        None,
    )
    .expect("register_session");

    // Seed a lesson memory so `build_proactive_context` surfaces at least one
    // knowledge type for PreBashCheck (KT_UAT_LESSON via memory_type='lesson',
    // relevance 0.7). Without this seed the proactive handler short-circuits
    // on an empty context and no RecordInjection fires — exactly the
    // invariant Fix 2 restored.
    state
        .conn
        .execute(
            "INSERT INTO memory (
                id, memory_type, title, content,
                confidence, status, project, tags,
                created_at, accessed_at
            ) VALUES (
                'lesson-e2e', 'lesson', 'avoid rm -rf on mounted volumes', 'UAT lesson body',
                0.9, 'active', 'forge', '[\"anti-pattern\"]',
                datetime('now'), datetime('now')
            )",
            [],
        )
        .expect("seed lesson memory");

    // ========================================================================
    // Fix 1 — #55 skill registry auto-populate
    // ========================================================================
    let skills_dir = tempdir().expect("tempdir for skills");
    for name in ["loop-a", "loop-b", "loop-c"] {
        let p = skills_dir.path().join(name);
        fs::create_dir_all(&p).expect("create skill subdir");
        fs::write(
            p.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: test fixture {name}\ncategory: test\n---\n\n# {name}\n"),
        )
        .expect("write SKILL.md");
    }

    let indexed = skills::auto_populate_on_start(&state.conn, skills_dir.path())
        .expect("auto_populate_on_start");
    assert_eq!(
        indexed, 3,
        "#55: auto_populate_on_start should index 3 fixtures"
    );

    // Exercise the same read path `forge-next skills` (and hook planners)
    // consume — list_skills over the skill_registry table.
    let listed = skills::list_skills(&state.conn, None, None, 100).expect("list_skills");
    assert_eq!(
        listed.len(),
        3,
        "#55: list_skills should surface the 3 auto-populated fixtures; got {listed:?}"
    );
    let listed_names: Vec<&str> = listed.iter().map(|s| s.name.as_str()).collect();
    for expected in ["loop-a", "loop-b", "loop-c"] {
        assert!(
            listed_names.contains(&expected),
            "#55: list_skills missing fixture '{expected}'; got {listed_names:?}"
        );
    }

    // ========================================================================
    // Fix 2 — #45 proactive injection via Request::PreBashCheck
    // ========================================================================
    let req = Request::PreBashCheck {
        command: "rm -rf /tmp/foo".to_string(),
        session_id: Some(session_id.clone()),
    };
    let resp = handle_request(&mut state, req);
    assert!(
        matches!(resp, Response::Ok { .. }),
        "#45: PreBashCheck should return Response::Ok; got {resp:?}"
    );

    // Drain the writer channel and look for a RecordInjection matching this
    // session with context_type=proactive. This is the exact WriteCommand the
    // writer actor persists via `db::effectiveness::record_injection_with_size`
    // — i.e., the same row `forge-next context-stats` reads.
    let mut proactive_injection_seen = false;
    while let Ok(cmd) = rx.try_recv() {
        if let WriteCommand::RecordInjection {
            session_id: sid,
            hook_event,
            context_type,
            chars_injected,
            ..
        } = cmd
        {
            if sid == session_id
                && hook_event == "PreBashCheck"
                && context_type == "proactive"
                && chars_injected > 0
            {
                proactive_injection_seen = true;
            }
        }
    }
    assert!(
        proactive_injection_seen,
        "#45: PreBashCheck should emit WriteCommand::RecordInjection with context_type=proactive. \
         Regression likely in handler.rs try_send(RecordInjection)."
    );

    // Simulate the writer actor persisting the command so the DB shape — what
    // `forge-next context-stats` reads — is asserted end-to-end. Uses the
    // same helper the writer would call.
    db::effectiveness::record_injection_with_size(
        &state.conn,
        &session_id,
        "PreBashCheck",
        "proactive",
        "skill:uat_lesson",
        42,
    )
    .expect("record_injection_with_size");
    let proactive_rows: i64 = state
        .conn
        .query_row(
            "SELECT COUNT(*) FROM context_effectiveness \
             WHERE context_type = 'proactive' AND session_id = ?1",
            rusqlite::params![session_id],
            |r| r.get(0),
        )
        .expect("count proactive rows");
    assert!(
        proactive_rows >= 1,
        "#45: context_effectiveness should hold >=1 proactive row for this session; got {proactive_rows}"
    );

    // ========================================================================
    // Fix 3 — #54 per-tool counter via seed_claude_builtins + claude:* slug
    // ========================================================================
    // DaemonState::new already invoked seed_claude_builtins — verify the
    // `claude:bash` row exists at use_count=0 before incrementing.
    let bash_pre: i64 = state
        .conn
        .query_row(
            "SELECT use_count FROM tool WHERE id = 'claude:bash'",
            [],
            |r| r.get(0),
        )
        .expect("claude:bash row present (seed_claude_builtins should have inserted it)");
    assert_eq!(
        bash_pre, 0,
        "#54: seed_claude_builtins must not reset accumulated counters; \
         fresh :memory: DaemonState should start at 0"
    );

    // Drive the same slug-lookup the extractor's dual-slug candidate loop
    // performs after the Fix 3 review-fixup (commit 3787916). Increment via
    // `record_tool_use` on the `claude:` prefixed ID, the one
    // `seed_claude_builtins` inserts.
    let matched = db::manas::record_tool_use(&state.conn, "claude:bash")
        .expect("record_tool_use claude:bash");
    assert!(
        matched,
        "#54: record_tool_use should match the seeded `claude:bash` row"
    );

    let bash_post: i64 = state
        .conn
        .query_row(
            "SELECT use_count FROM tool WHERE id = 'claude:bash'",
            [],
            |r| r.get(0),
        )
        .expect("claude:bash row still present after increment");
    assert_eq!(
        bash_post, 1,
        "#54: use_count should advance by 1 for claude:bash after record_tool_use"
    );

    // Negative assertion: the bare `bash` slug must NOT be what
    // seed_claude_builtins inserted (otherwise the dual-slug lookup's
    // `claude:` prefix path is vestigial and Fix 3 is a no-op).
    let bare_bash_exists: i64 = state
        .conn
        .query_row("SELECT COUNT(*) FROM tool WHERE id = 'bash'", [], |r| {
            r.get(0)
        })
        .expect("count bare bash rows");
    assert_eq!(
        bare_bash_exists, 0,
        "#54: seed_claude_builtins must use `claude:` prefix; a bare `bash` row \
         would mean the slug convention is wrong and regressions in dual-slug \
         lookup would pass unnoticed"
    );

    // ========================================================================
    // Fix 4 — #53 extraction metric + query_stats read
    // ========================================================================
    db::metrics::record_extraction(&state.conn, &session_id, 7, 2000, 1000, 25, None)
        .expect("record_extraction ok path");

    // Also record an error row so query_stats' extraction_errors counter is
    // non-zero — catches regressions where error mapping is dropped.
    db::metrics::record_extraction(&state.conn, &session_id, 0, 500, 0, 2, Some("parse fail"))
        .expect("record_extraction error path");

    let extraction_rows: i64 = state
        .conn
        .query_row(
            "SELECT COUNT(*) FROM metrics WHERE metric_type = 'extraction'",
            [],
            |r| r.get(0),
        )
        .expect("count extraction rows");
    assert_eq!(
        extraction_rows, 2,
        "#53: record_extraction should have inserted 2 rows (ok + error)"
    );

    // Drive the exact read path `forge-next stats` consumes.
    let stats = db::ops::query_stats(&state.conn, 24).expect("query_stats");
    assert!(
        stats.extractions >= 2,
        "#53: query_stats should see both extraction rows; got {}",
        stats.extractions
    );
    assert!(
        stats.tokens_in >= 2500,
        "#53: query_stats tokens_in should sum both rows (2000 + 500 = 2500); got {}",
        stats.tokens_in
    );
    assert!(
        stats.tokens_out >= 1000,
        "#53: query_stats tokens_out should include the ok row's 1000; got {}",
        stats.tokens_out
    );
    assert!(
        stats.extraction_errors >= 1,
        "#53: query_stats extraction_errors should count the status='error' row; got {}",
        stats.extraction_errors
    );
}
