//! Integration test: proactive-context handlers record a RecordInjection
//! WriteCommand via the writer channel (#45 — SP1 Fix 2).
//!
//! Two of the three Request variants reliably produce non-empty
//! `proactive_context` from a fresh in-memory DB (when seeded with the
//! right memory types/tags): `PreBashCheck` and `PostEditCheck`. Those
//! are asserted to emit `WriteCommand::RecordInjection` with
//! `context_type = "proactive"`.
//!
//! `PostBashCheck`'s bootstrap relevance matrix (see `proactive.rs`)
//! scores every knowledge type at the 0.1 default (< RELEVANCE_THRESHOLD
//! 0.3), so its `proactive_context` is empty on fresh state and the
//! helper correctly no-ops (empty-context short-circuit). That site is
//! still exercised end-to-end to confirm the handler doesn't panic and
//! returns `Ok`, even though we can't assert a positive row without
//! learned-effectiveness priming.

use forge_core::protocol::{Request, Response};
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_daemon::server::writer::WriteCommand;
use forge_daemon::sessions;

/// Build a DaemonState with an attached mpsc channel so we can observe
/// writer commands. Registers a single active "cli" session so the
/// helper's `get_active_session_id(..., "cli")` lookup succeeds.
fn fresh_state_with_session() -> (
    DaemonState,
    tokio::sync::mpsc::Receiver<WriteCommand>,
    String,
) {
    let mut state = DaemonState::new(":memory:").expect("DaemonState::new(:memory:)");
    let (tx, rx) = tokio::sync::mpsc::channel::<WriteCommand>(64);
    state.writer_tx = Some(tx);

    // Register an active "cli" session (matches agent name used by the
    // handler's get_active_session_id lookup precedent at handler.rs:259).
    let session_id = "sp1-proactive-test".to_string();
    sessions::register_session(
        &state.conn,
        &session_id,
        "cli",
        Some("forge"),
        Some("/tmp/forge-test"),
        None,
        None,
    )
    .expect("register_session");

    // Seed memories so `build_proactive_context` surfaces at least one
    // knowledge type for PreBashCheck (KT_UAT_LESSON via memory_type='lesson',
    // relevance 0.7) and PostEditCheck (KT_TEST_REMINDER via tag 'test',
    // relevance 0.8).
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
fn post_bash_check_hits_record_helper_without_panic() {
    // PostBashCheck's bootstrap relevance is 0.1 for every knowledge type
    // (< RELEVANCE_THRESHOLD 0.3), so proactive_context is empty on a
    // fresh DB and the helper short-circuits on chars == 0 — no
    // RecordInjection is emitted.
    //
    // This test verifies the 3rd site wires up cleanly: the handler
    // returns Ok, and if any RecordInjection did fire, it carries the
    // correct (session_id, hook_event, context_type) shape.
    let (mut state, mut rx, session_id) = fresh_state_with_session();
    let req = Request::PostBashCheck {
        command: "cargo build".to_string(),
        exit_code: 1,
    };
    let resp = handle_request(&mut state, req);
    assert!(
        matches!(resp, Response::Ok { .. }),
        "PostBashCheck should return Ok"
    );

    // Every RecordInjection from this call, if any, must be proactive
    // and tagged with the PostBashCheck hook_event + our session_id.
    for (sid, hook, ctype, _chars) in collect_record_injections(&mut rx) {
        assert_eq!(sid, session_id, "session_id mismatch: {sid}");
        assert_eq!(hook, "PostBashCheck", "hook_event mismatch: {hook}");
        assert_eq!(ctype, "proactive", "context_type mismatch: {ctype}");
    }
}
