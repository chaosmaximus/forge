//! Layer 3: through Request::CompileContext. Verifies preference accessed_at
//! stays unchanged after CompileContext even when recalled memories include
//! preferences.
//!
//! Strategy mirrors touch_exemption_recall.rs: attach real mpsc channel,
//! trigger CompileContext, drain TouchMemories commands, simulate WriterActor
//! by calling ops::touch directly, assert pref unchanged.

use forge_core::protocol::*;
use forge_core::types::memory::{Memory, MemoryType};
use forge_daemon::db::ops;
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_daemon::server::writer::WriteCommand;
use rusqlite::params;

fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

fn drain_touch_ids(rx: &mut tokio::sync::mpsc::Receiver<WriteCommand>) -> Vec<String> {
    let mut ids = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(WriteCommand::TouchMemories { ids: batch, .. }) => ids.extend(batch),
            Ok(_) => {}
            Err(_) => break,
        }
    }
    ids
}

#[test]
fn touch_exemption_compile_context_preference_unchanged() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut state = fresh_state();

    let (tx, mut rx) = rt.block_on(async { tokio::sync::mpsc::channel::<WriteCommand>(64) });
    state.writer_tx = Some(tx);

    let pref = Memory::new(MemoryType::Preference, "prefer-vim-cc-test", "yes");
    let pref_id = pref.id.clone();
    ops::remember_raw(&state.conn, &pref).unwrap();

    let dec = Memory::new(MemoryType::Decision, "rust-yes-cc-test", "shipping in rust");
    let dec_id = dec.id.clone();
    ops::remember_raw(&state.conn, &dec).unwrap();

    // Backdate both so the 60s gate lets the decision through
    state
        .conn
        .execute(
            "UPDATE memory SET accessed_at = '2026-01-01 00:00:00' WHERE id IN (?1, ?2)",
            params![pref_id, dec_id],
        )
        .unwrap();

    let resp = handle_request(
        &mut state,
        Request::CompileContext {
            agent: Some("claude-code".into()),
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

    // Drain touch IDs and simulate WriterActor
    let touch_ids: Vec<String> = drain_touch_ids(&mut rx);
    let id_refs: Vec<&str> = touch_ids.iter().map(|s| s.as_str()).collect();
    ops::touch(&state.conn, &id_refs);

    let pref_after: String = state
        .conn
        .query_row(
            "SELECT accessed_at FROM memory WHERE id = ?1",
            params![pref_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        pref_after, "2026-01-01 00:00:00",
        "preference accessed_at must NOT change after CompileContext (touch exemption)"
    );
}
