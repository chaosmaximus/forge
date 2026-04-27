//! Integration test: proactive-context handlers record a RecordInjection
//! WriteCommand via the writer channel (#45 — SP1 Fix 2, post-review fixup).
//!
//! After the review fixup, the three proactive Request variants
//! (PreBashCheck / PostBashCheck / PostEditCheck) carry an optional
//! `session_id: Option<String>` field. These tests exercise the new
//! production path end-to-end:
//!
//!   * sessions are registered under the real agent name
//!     (`"claude-code"`), not the old `"cli"` artifact,
//!   * Requests pass an explicit `session_id`, matching how Claude Code
//!     hooks will invoke the daemon,
//!   * a fallback case asserts that `get_latest_active_session_id` still
//!     fires when a Request omits session_id (so old hook clients remain
//!     functional during rollout),
//!   * PostBashCheck is exercised with a seeded `context_effectiveness`
//!     table that pushes learned relevance above the 0.3 threshold, so
//!     we actually observe a non-empty RecordInjection (the previous
//!     harness only asserted no-panic due to the 0.1 bootstrap relevance).

use forge_core::protocol::{Request, Response};
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_daemon::server::writer::WriteCommand;
use forge_daemon::sessions;

/// Build a DaemonState with an attached mpsc channel so we can observe
/// writer commands. Registers a single active `"claude-code"` session
/// (the real production agent name). Seeds memories so `build_proactive_context`
/// surfaces at least one knowledge type for PreBashCheck (KT_UAT_LESSON via
/// memory_type='lesson', relevance 0.7) and PostEditCheck (KT_TEST_REMINDER
/// via tag 'test', relevance 0.8).
fn fresh_state_with_session() -> (
    DaemonState,
    tokio::sync::mpsc::Receiver<WriteCommand>,
    String,
) {
    let mut state = DaemonState::new(":memory:").expect("DaemonState::new(:memory:)");
    let (tx, rx) = tokio::sync::mpsc::channel::<WriteCommand>(64);
    state.writer_tx = Some(tx);

    // Use the production agent name. Prior to the SP1 review fixup this was
    // "cli", which masked a BLOCKER: real Claude Code sessions register as
    // "claude-code" and the handler's hardcoded get_active_session_id lookup
    // for "cli" returned QueryReturnedNoRows, so NO RecordInjections fired
    // in production.
    let session_id = "sp1-proactive-test".to_string();
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

    state
        .conn
        .execute(
            "INSERT INTO memory (
                id, memory_type, title, content,
                confidence, status, project, tags,
                created_at, accessed_at
            ) VALUES (
                'lesson-sp1', 'lesson', 'never run rm -rf on mounted volumes', 'UAT lesson body',
                0.9, 'active', 'forge', '[\"anti-pattern\"]',
                datetime('now'), datetime('now')
            )",
            [],
        )
        .expect("seed lesson memory");

    state
        .conn
        .execute(
            "INSERT INTO memory (
                id, memory_type, title, content,
                confidence, status, project, tags,
                created_at, accessed_at
            ) VALUES (
                'reminder-sp1', 'reminder', 'run cargo test after editing handler.rs', 'test reminder body',
                0.9, 'active', 'forge', '[\"test\"]',
                datetime('now'), datetime('now')
            )",
            [],
        )
        .expect("seed test-reminder memory");

    (state, rx, session_id)
}

fn collect_record_injections(
    rx: &mut tokio::sync::mpsc::Receiver<WriteCommand>,
) -> Vec<(String, String, String, usize)> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(WriteCommand::RecordInjection {
                session_id,
                hook_event,
                context_type,
                chars_injected,
                ..
            }) => out.push((session_id, hook_event, context_type, chars_injected)),
            Ok(_) => {} // ignore TouchMemories, etc.
            Err(_) => break,
        }
    }
    out
}

#[test]
fn pre_bash_check_records_proactive_injection() {
    let (mut state, mut rx, session_id) = fresh_state_with_session();
    let req = Request::PreBashCheck {
        command: "rm -rf /tmp/foo".to_string(),
        session_id: Some(session_id.clone()),
    };
    let resp = handle_request(&mut state, req);
    assert!(
        matches!(resp, Response::Ok { .. }),
        "PreBashCheck should return Ok"
    );

    let injections = collect_record_injections(&mut rx);
    assert!(
        injections
            .iter()
            .any(|(sid, hook, ctype, chars)| sid == &session_id
                && hook == "PreBashCheck"
                && ctype == "proactive"
                && *chars > 0),
        "expected PreBashCheck to emit RecordInjection with context_type=proactive; \
         got {injections:?}"
    );
}

#[test]
fn post_edit_check_records_proactive_injection() {
    let (mut state, mut rx, session_id) = fresh_state_with_session();
    let req = Request::PostEditCheck {
        file: "src/main.rs".to_string(),
        session_id: Some(session_id.clone()),
    };
    let resp = handle_request(&mut state, req);
    assert!(
        matches!(resp, Response::Ok { .. }),
        "PostEditCheck should return Ok"
    );

    let injections = collect_record_injections(&mut rx);
    assert!(
        injections
            .iter()
            .any(|(sid, hook, ctype, chars)| sid == &session_id
                && hook == "PostEditCheck"
                && ctype == "proactive"
                && *chars > 0),
        "expected PostEditCheck to emit RecordInjection with context_type=proactive; \
         got {injections:?}"
    );
}

#[test]
fn post_bash_check_records_proactive_injection_with_seeded_relevance() {
    // HIGH coverage gap from the adversarial review: PostBashCheck was
    // never asserted to actually fire an injection. Bootstrap relevance
    // for every (PostBash, *) pair is 0.1 (< RELEVANCE_THRESHOLD 0.3),
    // so without priming, `build_proactive_context` returns empty and
    // the helper short-circuits.
    //
    // `learned_effectiveness_rate` (in `proactive.rs`) requires ≥5 samples
    // before trusting the learned rate, and matches on `context_type`
    // equal to the knowledge-type constant (not "proactive"). Seed 5
    // acknowledged rows for (hook_event=PostBash, context_type=uat_lesson)
    // so the learned rate becomes 5/5 = 1.0, trivially above 0.3.
    let (mut state, mut rx, session_id) = fresh_state_with_session();

    for i in 0..5 {
        state
            .conn
            .execute(
                "INSERT INTO context_effectiveness (
                    id, session_id, hook_event, context_type,
                    content_summary, acknowledged, chars_injected
                ) VALUES (?1, ?2, 'PostBash', 'uat_lesson', 'seed', 1, 32)",
                rusqlite::params![format!("seed-ce-{i}"), &session_id],
            )
            .expect("seed context_effectiveness row");
    }

    let req = Request::PostBashCheck {
        command: "cargo build".to_string(),
        exit_code: 1,
        session_id: Some(session_id.clone()),
    };
    let resp = handle_request(&mut state, req);
    assert!(
        matches!(resp, Response::Ok { .. }),
        "PostBashCheck should return Ok"
    );

    let injections = collect_record_injections(&mut rx);
    assert!(
        injections.iter().any(|(sid, hook, ctype, chars)| {
            sid == &session_id && hook == "PostBashCheck" && ctype == "proactive" && *chars > 0
        }),
        "PostBashCheck with seeded learned relevance should record an injection; \
         got {injections:?}"
    );
}

#[test]
fn pre_bash_check_fallback_uses_latest_active_session_when_request_omits_id() {
    // Old hook clients (pre-SP1-fixup) do not send `session_id`; it
    // deserializes to None. The handler then falls back to
    // `get_latest_active_session_id` so the row is still recorded.
    let (mut state, mut rx, session_id) = fresh_state_with_session();

    let req = Request::PreBashCheck {
        command: "rm -rf /tmp/foo".to_string(),
        session_id: None, // exercise fallback path explicitly
    };
    let resp = handle_request(&mut state, req);
    assert!(
        matches!(resp, Response::Ok { .. }),
        "PreBashCheck should return Ok"
    );

    let injections = collect_record_injections(&mut rx);
    assert!(
        injections
            .iter()
            .any(|(sid, hook, ctype, chars)| sid == &session_id
                && hook == "PreBashCheck"
                && ctype == "proactive"
                && *chars > 0),
        "PreBashCheck with session_id=None should still record an injection \
         via get_latest_active_session_id fallback; got {injections:?}"
    );
}
