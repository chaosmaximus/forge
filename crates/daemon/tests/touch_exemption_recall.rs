//! Layer 2: through Request::Recall. Verifies preference accessed_at stays
//! unchanged end-to-end after a Recall that returns it.
//!
//! Strategy: inject pref + decision, backdate both, attach a real mpsc channel
//! to writer_tx, trigger Recall, drain all TouchMemories commands, simulate
//! what WriterActor does (call ops::touch directly on same conn), then assert
//! pref accessed_at unchanged and decision accessed_at changed.
//!
//! NOTE: A CompileContext touch-exemption test is deferred until T13 lands
//! the <preferences> XML section. Currently compile_dynamic_suffix does
//! not include preferences in ctx_touched_ids (only decisions+lessons),
//! so any test asserting "preference accessed_at unchanged" passes
//! vacuously. T13 will add prefs to the touched set, at which point a
//! touch_exemption_compile_context.rs test becomes meaningful.

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
/// Collects every id that would have been passed to ops::touch().
fn drain_touch_ids(rx: &mut tokio::sync::mpsc::Receiver<WriteCommand>) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(WriteCommand::TouchMemories { ids: batch, .. }) => ids.extend(batch),
            Ok(_) => {} // skip other commands
            Err(_) => break,
        }
    }
    ids
}

#[test]
fn touch_exemption_recall_preference_unchanged() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut state = fresh_state();

    // Set up the writer_tx channel so send_touch actually fires
    let (tx, mut rx) = rt.block_on(async { tokio::sync::mpsc::channel::<WriteCommand>(64) });
    state.writer_tx = Some(tx);

    let pref = Memory::new(MemoryType::Preference, "prefer-vim-recall-test", "yes");
    let pref_id = pref.id.clone();
    ops::remember_raw(&state.conn, &pref).unwrap();

    let dec = Memory::new(
        MemoryType::Decision,
        "rust-yes-recall-test",
        "shipping in rust",
    );
    let dec_id = dec.id.clone();
    ops::remember_raw(&state.conn, &dec).unwrap();

    // Backdate both with relative datetime so the 60s gate lets the decision through.
    // Use relative datetime instead of a magic wall-clock constant.
    state
        .conn
        .execute(
            "UPDATE memory SET accessed_at = datetime('now', '-2 hours') WHERE id IN (?1, ?2)",
            params![pref_id, dec_id],
        )
        .unwrap();

    // Capture before values via SELECT so we can compare without a literal date
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

    // Trigger Recall — should match both memories
    let resp = handle_request(
        &mut state,
        Request::Recall {
            query: "vim rust shipping".into(),
            memory_type: None,
            project: None,
            limit: Some(10),
            layer: None,
            since: None,
            include_flipped: None,
        },
    );
    assert!(
        matches!(resp, Response::Ok { .. }),
        "Recall should succeed: {resp:?}"
    );

    // Drain the touch IDs and simulate what WriterActor does
    let touch_ids: Vec<String> = drain_touch_ids(&mut rx);

    // Assert: handler actually sent TouchMemories command (not silent no-op)
    assert!(
        !touch_ids.is_empty(),
        "handler must have sent TouchMemories with non-empty ids"
    );

    // Assert: preference ID WAS in the touch set (handler doesn't filter; SQL does).
    // This catches a bug where the exemption gets moved from ops.rs to handler.rs incorrectly.
    assert!(
        touch_ids.iter().any(|id| id == &pref_id),
        "preference ID should be in touch_ids (handler doesn't filter; SQL does); got: {:?}",
        touch_ids
    );

    // Assert: decision ID was also in the touch set
    assert!(
        touch_ids.iter().any(|id| id == &dec_id),
        "decision ID should be in touch_ids; got: {:?}",
        touch_ids
    );

    // Apply ops::touch() on same conn — this is what WriterActor would do
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

    // Preference must NOT have been touched
    assert_eq!(
        pref_before, pref_after,
        "preference accessed_at must NOT change after Recall (touch exemption)"
    );

    // Positive control: decision accessed_at MUST have been updated
    assert_ne!(
        dec_before, dec_after,
        "decision accessed_at MUST change after Recall (positive control)"
    );
}
