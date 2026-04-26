//! Phase 2A-4a integration test: end-to-end flip preference flow.
//!
//! Exercises: Remember (seeded with positive valence via ops::remember) ->
//! FlipPreference -> ListFlipped ->
//! Recall (include_flipped=None, then Some(true)) -> CompileContext renders
//! `<preferences-flipped>` with correct old/new valence attributes.

use forge_core::protocol::*;
use forge_core::types::memory::{Memory, MemoryType};
use forge_daemon::db::ops;
use forge_daemon::server::handler::{handle_request, DaemonState};

/// Create a fresh in-memory DaemonState.
fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

#[test]
fn test_flip_preference_end_to_end_flow() {
    let mut state = fresh_state();

    // ── Step 1: Seed a preference with positive valence ────────────────────
    //
    // Request::Remember has no `valence` field — it defaults to "neutral".
    // We use ops::remember directly (the same technique used in handler unit
    // tests) so we can set valence = "positive" before seeding.
    let mut pref = Memory::new(
        MemoryType::Preference,
        "tabs over spaces",
        "prefer tabs for readability across the codebase",
    );
    pref.valence = "positive".to_string();
    pref.intensity = 0.7;
    pref.tags = vec!["formatting".to_string()];
    pref.project = Some("forge".to_string());
    ops::remember(&state.conn, &pref).expect("ops::remember should succeed");
    let old_id = pref.id.clone();

    // ── Step 2: Flip it to negative ────────────────────────────────────────
    let resp = handle_request(
        &mut state,
        Request::FlipPreference {
            memory_id: old_id.clone(),
            new_valence: "negative".into(),
            new_intensity: 0.8,
            reason: Some("team agreed on spaces for better diff readability".into()),
        },
    );
    let new_id = match resp {
        Response::Ok {
            data:
                ResponseData::PreferenceFlipped {
                    new_id,
                    new_valence,
                    old_id: returned_old_id,
                    ..
                },
        } => {
            assert_eq!(new_valence, "negative", "new valence must be negative");
            assert_eq!(returned_old_id, old_id, "returned old_id must match");
            new_id
        }
        other => panic!("flip failed: {other:?}"),
    };
    assert_ne!(new_id, old_id, "new_id must differ from old_id");

    // ── Step 3: ListFlipped — exactly one item, pointing old -> new ────────
    let resp = handle_request(
        &mut state,
        Request::ListFlipped {
            agent: None,
            limit: Some(10),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::FlippedList { items },
        } => {
            assert_eq!(items.len(), 1, "expected exactly 1 flipped item");
            assert_eq!(
                items[0].old.id, old_id,
                "flipped item's old.id must match old_id"
            );
            assert_eq!(
                items[0].flipped_to_id, new_id,
                "flipped_to_id must match new_id"
            );
        }
        other => panic!("list_flipped failed: {other:?}"),
    }

    // ── Step 4: Recall(include_flipped=None) — old NOT in results ──────────
    let resp = handle_request(
        &mut state,
        Request::Recall {
            query: "tabs readability formatting".into(),
            memory_type: None,
            project: None,
            limit: Some(20),
            layer: None,
            since: None,
            include_flipped: None,
            include_globals: None,
            query_embedding: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Memories { results, .. },
        } => {
            assert!(
                !results.iter().any(|m| m.memory.id == old_id),
                "old (flipped) memory should NOT appear in default Recall; got ids: {:?}",
                results.iter().map(|m| &m.memory.id).collect::<Vec<_>>()
            );
        }
        other => panic!("recall (include_flipped=None) failed: {other:?}"),
    }

    // ── Step 5: Recall(include_flipped=Some(true)) — old IS in results ─────
    let resp = handle_request(
        &mut state,
        Request::Recall {
            query: "tabs readability formatting".into(),
            memory_type: None,
            project: None,
            limit: Some(20),
            layer: None,
            since: None,
            include_flipped: Some(true),
            include_globals: None,
            query_embedding: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::Memories { results, .. },
        } => {
            assert!(
                results.iter().any(|m| m.memory.id == old_id),
                "old (flipped) memory SHOULD appear when include_flipped=true; got ids: {:?}",
                results.iter().map(|m| &m.memory.id).collect::<Vec<_>>()
            );
        }
        other => panic!("recall (include_flipped=Some(true)) failed: {other:?}"),
    }

    // ── Step 6: CompileContext — dynamic_suffix contains <preferences-flipped> ─
    //
    // The <preferences-flipped> block is rendered inside compile_dynamic_suffix
    // (see crates/daemon/src/recall.rs). We assert directly against
    // `dynamic_suffix` rather than the concatenated `context` string so a
    // future refactor that accidentally moves the section into `static_prefix`
    // is caught here instead of silently passing.
    let resp = handle_request(
        &mut state,
        Request::CompileContext {
            agent: Some("claude-code".into()),
            project: Some("forge".into()),
            static_only: None,
            excluded_layers: None,
            session_id: None,
            focus: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::CompiledContext { dynamic_suffix, .. },
        } => {
            assert!(
                dynamic_suffix.contains("<preferences-flipped>"),
                "<preferences-flipped> tag missing from dynamic_suffix (must not be in static_prefix).\n\
                 dynamic_suffix (first 600 chars): {}",
                dynamic_suffix.chars().take(600).collect::<String>()
            );
            assert!(
                dynamic_suffix.contains("old_valence=\"positive\""),
                "old_valence=\"positive\" missing from dynamic_suffix"
            );
            assert!(
                dynamic_suffix.contains("new_valence=\"negative\""),
                "new_valence=\"negative\" missing from dynamic_suffix"
            );
        }
        other => panic!("compile_context failed: {other:?}"),
    }
}
