//! Layer 3: through Request::CompileContext. T13 adds active preferences to
//! ctx_touched_ids, so CompileContext now emits them in the TouchMemories channel
//! command. The T6 touch() SQL predicate then filters preferences back out so
//! their accessed_at timestamp never changes.
//!
//! This test was deferred during T6 because compile_dynamic_suffix did not
//! include preferences in ctx_touched_ids at that time. It is now meaningful:
//! (a) pref_id DOES appear in the drained TouchMemories, and
//! (b) pref accessed_at is unchanged after ops::touch() applies the SQL filter.

use forge_core::protocol::*;
use forge_core::types::memory::{Memory, MemoryType};
use forge_daemon::db::ops;
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_daemon::server::writer::WriteCommand;
use rusqlite::params;

fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

/// Drain all pending TouchMemories commands from the channel without blocking.
fn drain_touch_ids(rx: &mut tokio::sync::mpsc::Receiver<WriteCommand>) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(WriteCommand::TouchMemories { ids: batch, .. }) => ids.extend(batch),
            Ok(_) => {} // skip other commands (RecordInjection, etc.)
            Err(_) => break,
        }
    }
    ids
}

#[test]
fn touch_exemption_compile_context_preference_not_touched() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut state = fresh_state();

    // Attach a real mpsc channel so send_touch fires
    let (tx, mut rx) = rt.block_on(async { tokio::sync::mpsc::channel::<WriteCommand>(128) });
    state.writer_tx = Some(tx);

    // Seed a preference
    let pref = Memory::new(
        MemoryType::Preference,
        "topic-cc-touch-pref".to_string(),
        "yes".to_string(),
    );
    let pref_id = pref.id.clone();
    ops::remember_raw(&state.conn, &pref).unwrap();

    // Seed a decision (positive control — must be touched)
    let dec = Memory::new(
        MemoryType::Decision,
        "topic-cc-touch-dec".to_string(),
        "ship".to_string(),
    );
    let dec_id = dec.id.clone();
    ops::remember_raw(&state.conn, &dec).unwrap();

    // Backdate both so the 60s gate inside touch() allows updates
    state
        .conn
        .execute(
            "UPDATE memory SET accessed_at = datetime('now', '-2 hours') WHERE id IN (?1, ?2)",
            params![pref_id, dec_id],
        )
        .unwrap();

    // Capture before values
    let pref_before: String = state
        .conn
        .query_row(
            "SELECT accessed_at FROM memory WHERE id = ?1",
            params![pref_id],
            |r| r.get(0),
        )
        .unwrap();
    let dec_before: String = state
        .conn
        .query_row(
            "SELECT accessed_at FROM memory WHERE id = ?1",
            params![dec_id],
            |r| r.get(0),
        )
        .unwrap();

    // Trigger CompileContext — should include both in ctx_touched_ids
    let resp = handle_request(
        &mut state,
        Request::CompileContext {
            agent: None,
            project: None,
            static_only: None,
            excluded_layers: None,
            session_id: None,
            focus: None,
        },
    );
    assert!(
        matches!(resp, Response::Ok { .. }),
        "CompileContext should succeed: {resp:?}"
    );

    // Drain all TouchMemories commands
    let touch_ids: Vec<String> = drain_touch_ids(&mut rx);

    // Assert: handler emitted pref_id in the touch set (handler doesn't filter; SQL does)
    assert!(
        touch_ids.iter().any(|id| id == &pref_id),
        "pref_id must appear in ctx_touched_ids after T13 wires preferences; got: {touch_ids:?}"
    );

    // Assert: decision ID also in touch set
    assert!(
        touch_ids.iter().any(|id| id == &dec_id),
        "dec_id must appear in ctx_touched_ids; got: {touch_ids:?}"
    );

    // Simulate what WriterActor does: call ops::touch on the same conn
    let id_refs: Vec<&str> = touch_ids.iter().map(|s| s.as_str()).collect();
    ops::touch(&state.conn, &id_refs);

    // Read back after values
    let pref_after: String = state
        .conn
        .query_row(
            "SELECT accessed_at FROM memory WHERE id = ?1",
            params![pref_id],
            |r| r.get(0),
        )
        .unwrap();
    let dec_after: String = state
        .conn
        .query_row(
            "SELECT accessed_at FROM memory WHERE id = ?1",
            params![dec_id],
            |r| r.get(0),
        )
        .unwrap();

    // Key assertion: preference accessed_at must NOT change (T6 SQL predicate)
    assert_eq!(
        pref_before, pref_after,
        "preference accessed_at must NOT change after touch() (T6 SQL filter)"
    );

    // Positive control: decision accessed_at MUST change
    assert_ne!(
        dec_before, dec_after,
        "decision accessed_at MUST change after touch() (positive control)"
    );
}
