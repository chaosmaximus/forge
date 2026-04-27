//! T14 integration test: recency-weighted preference decay end-to-end flow.
//! Exercises: Remember → age → decay → Reaffirm → re-age → re-decay → Flip
//! → list_active / compile_context / list_flipped interactions.

use forge_core::protocol::*;
use forge_core::types::memory::MemoryType;
use forge_daemon::db::ops;
use forge_daemon::server::handler::{handle_request, DaemonState};
use rusqlite::params;

fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

/// Age a specific timestamp field on a memory row by `days` into the past.
fn age_field_by_days(conn: &rusqlite::Connection, id: &str, field: &str, days: f64) {
    let sql = format!("UPDATE memory SET {field} = datetime('now', ?1) WHERE id = ?2");
    let offset = format!("-{} days", days as i64);
    conn.execute(&sql, params![offset, id]).unwrap();
}

#[test]
fn recency_decay_flow_end_to_end() {
    let mut state = fresh_state();

    // ── Phase 1: seed a fresh preference via Remember ──────────────────────
    let pref_id = match handle_request(
        &mut state,
        Request::Remember {
            memory_type: MemoryType::Preference,
            title: "prefer rust syntax".to_string(),
            content: "statically typed FTW".to_string(),
            confidence: Some(0.9),
            tags: None,
            project: None,
            metadata: None,
            valence: None,
            intensity: None,
        },
    ) {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => id,
        other => panic!("expected Stored, got: {other:?}"),
    };

    // Confidence should be ~0.9 as seeded
    let initial_conf: f64 = state
        .conn
        .query_row(
            "SELECT confidence FROM memory WHERE id = ?1",
            params![pref_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        initial_conf >= 0.85,
        "initial confidence should be ~0.9, got {initial_conf}"
    );

    // ── Phase 2: age 30 days and decay ────────────────────────────────────
    age_field_by_days(&state.conn, &pref_id, "created_at", 30.0);

    let (_checked, _faded) = ops::decay_memories(&state.conn, 1000, 14.0).unwrap();

    // Confidence should be decayed: 0.9 × 2^(-30/14) ≈ 0.2037
    let conf_after_30d: f64 = state
        .conn
        .query_row(
            "SELECT confidence FROM memory WHERE id = ?1",
            params![pref_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (conf_after_30d - 0.204).abs() < 0.02,
        "confidence after 30d decay should be ~0.204, got {conf_after_30d}"
    );

    // Hard-fade exemption: pref must remain active despite low confidence (T7)
    let status: String = state
        .conn
        .query_row(
            "SELECT status FROM memory WHERE id = ?1",
            params![pref_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "active",
        "pref must remain active despite low confidence"
    );

    // ── Phase 3: Reaffirm ─────────────────────────────────────────────────
    let reaffirm_resp = handle_request(
        &mut state,
        Request::ReaffirmPreference {
            memory_id: pref_id.clone(),
        },
    );
    assert!(
        matches!(
            reaffirm_resp,
            Response::Ok {
                data: ResponseData::PreferenceReaffirmed { .. }
            }
        ),
        "expected PreferenceReaffirmed, got: {reaffirm_resp:?}"
    );

    // ── Phase 4: age created_at 60 days back, reaffirmed_at 30 days back ──
    // Decay anchor must be reaffirmed_at (30 days), NOT created_at (60 days).
    age_field_by_days(&state.conn, &pref_id, "created_at", 60.0);
    age_field_by_days(&state.conn, &pref_id, "reaffirmed_at", 30.0);

    // Reset confidence to 0.9 so we see a clean 30d decay from reaffirmed_at
    state
        .conn
        .execute(
            "UPDATE memory SET confidence = 0.9 WHERE id = ?1",
            params![pref_id],
        )
        .unwrap();

    let (_checked, _faded) = ops::decay_memories(&state.conn, 1000, 14.0).unwrap();

    // Decay anchored on reaffirmed_at (30d ago) → still ~0.204
    // If anchored on created_at (60d ago) it would be ~0.046 — far outside tolerance
    let conf_after_reaffirm_decay: f64 = state
        .conn
        .query_row(
            "SELECT confidence FROM memory WHERE id = ?1",
            params![pref_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        (conf_after_reaffirm_decay - 0.204).abs() < 0.02,
        "confidence after 30d since reaffirm should be ~0.204 (anchored on reaffirmed_at), got {conf_after_reaffirm_decay}"
    );

    // ── Phase 5: <preferences> in CompileContext ───────────────────────────
    // Reset timestamps so the pref is fresh and visible in context
    state
        .conn
        .execute(
            "UPDATE memory SET confidence = 0.9, created_at = datetime('now'), reaffirmed_at = NULL WHERE id = ?1",
            params![pref_id],
        )
        .unwrap();

    let cc_resp = handle_request(
        &mut state,
        Request::CompileContext {
            agent: None,
            project: None,
            static_only: None,
            excluded_layers: None,
            session_id: None,
            focus: None,
            cwd: None,
            dry_run: None,
        },
    );
    let context = match cc_resp {
        Response::Ok {
            data: ResponseData::CompiledContext { context, .. },
        } => context,
        other => panic!("expected CompiledContext, got: {other:?}"),
    };
    assert!(
        context.contains("<preferences>"),
        "CompileContext must contain <preferences> section; context snippet: {}",
        context.chars().take(600).collect::<String>()
    );
    assert!(
        context.contains("prefer rust syntax"),
        "<preferences> must contain the pref title; context snippet: {}",
        context.chars().take(600).collect::<String>()
    );

    // ── Phase 6: FlipPreference ────────────────────────────────────────────
    let flip_resp = handle_request(
        &mut state,
        Request::FlipPreference {
            memory_id: pref_id.clone(),
            new_valence: "negative".to_string(),
            new_intensity: 0.8,
            reason: Some("changed mind".to_string()),
        },
    );
    let new_id = match flip_resp {
        Response::Ok {
            data: ResponseData::PreferenceFlipped { new_id, .. },
        } => new_id,
        other => panic!("expected PreferenceFlipped, got: {other:?}"),
    };

    // ── Phase 7: old excluded from active, new present ────────────────────
    let active = ops::list_active_preferences(&state.conn, "default", 10).unwrap();
    assert_eq!(
        active.len(),
        1,
        "after flip, exactly 1 active pref (the new one); got: {:?}",
        active.iter().map(|m| &m.id).collect::<Vec<_>>()
    );
    assert_eq!(
        active[0].id, new_id,
        "active pref must be the NEW one after flip"
    );

    // ── Phase 8: <preferences-flipped> surfaces the OLD pref ──────────────
    let cc_resp2 = handle_request(
        &mut state,
        Request::CompileContext {
            agent: None,
            project: None,
            static_only: None,
            excluded_layers: None,
            session_id: None,
            focus: None,
            cwd: None,
            dry_run: None,
        },
    );
    let context2 = match cc_resp2 {
        Response::Ok {
            data: ResponseData::CompiledContext { context, .. },
        } => context,
        other => panic!("expected CompiledContext, got: {other:?}"),
    };
    assert!(
        context2.contains("<preferences-flipped>"),
        "CompileContext must contain <preferences-flipped> section after flip"
    );
    assert!(
        context2.contains("prefer rust syntax"),
        "<preferences-flipped> must surface the old pref's title"
    );
}
