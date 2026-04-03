//! Comprehensive E2E lifecycle tests for the Forge daemon.
//!
//! These tests exercise the full request/response lifecycle through
//! `handle_request` (no socket), covering all major endpoints and their
//! interactions across remember, recall, forget, health, guardrails,
//! blast radius, export, import, vector storage, and edge cases.

use forge_daemon::db::{ops, vec};
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_core::protocol::*;
use forge_core::types::MemoryType;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a fresh in-memory DaemonState.
fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

/// Remember a memory and return its stored ID.
fn do_remember(
    state: &mut DaemonState,
    memory_type: MemoryType,
    title: &str,
    content: &str,
    confidence: Option<f64>,
    tags: Option<Vec<String>>,
    project: Option<String>,
) -> String {
    let resp = handle_request(
        state,
        Request::Remember {
            memory_type,
            title: title.into(),
            content: content.into(),
            confidence,
            tags,
            project,
        },
    );
    match resp {
        Response::Ok { data: ResponseData::Stored { id } } => {
            assert!(!id.is_empty());
            id
        }
        other => panic!("expected Stored, got: {:?}", other),
    }
}

/// Recall with optional project/type filters and return the results vec.
fn do_recall(
    state: &mut DaemonState,
    query: &str,
    memory_type: Option<MemoryType>,
    project: Option<String>,
    limit: Option<usize>,
) -> Vec<MemoryResult> {
    let resp = handle_request(
        state,
        Request::Recall {
            query: query.into(),
            memory_type,
            project,
            limit,
        },
    );
    match resp {
        Response::Ok { data: ResponseData::Memories { results, .. } } => results,
        other => panic!("expected Memories, got: {:?}", other),
    }
}

/// Forget a memory by id. Returns true if it was found and forgotten.
fn do_forget(state: &mut DaemonState, id: &str) -> bool {
    let resp = handle_request(state, Request::Forget { id: id.into() });
    matches!(resp, Response::Ok { data: ResponseData::Forgotten { .. } })
}

// ===========================================================================
// Test 1: Full memory lifecycle
// ===========================================================================
#[test]
fn test_full_memory_lifecycle() {
    let mut state = fresh_state();

    // -- Remember 5 decisions across 2 projects + 1 global --
    let forge_id1 = do_remember(
        &mut state,
        MemoryType::Decision,
        "Use Rust for CLI",
        "Performance and safety",
        Some(0.95),
        Some(vec!["rust".into(), "cli".into()]),
        Some("forge".into()),
    );
    let forge_id2 = do_remember(
        &mut state,
        MemoryType::Lesson,
        "SQLite WAL mode is fast",
        "Use WAL for concurrent reads",
        Some(0.8),
        None,
        Some("forge".into()),
    );
    let backend_id1 = do_remember(
        &mut state,
        MemoryType::Decision,
        "Use PostgreSQL for backend",
        "Relational model fits our domain",
        Some(0.9),
        Some(vec!["database".into()]),
        Some("backend".into()),
    );
    let backend_id2 = do_remember(
        &mut state,
        MemoryType::Pattern,
        "Repository pattern for data access",
        "Abstract DB queries behind repos",
        Some(0.85),
        None,
        Some("backend".into()),
    );
    let global_id = do_remember(
        &mut state,
        MemoryType::Preference,
        "Always write tests first",
        "TDD approach for all projects",
        Some(0.99),
        Some(vec!["tdd".into()]),
        None, // global
    );

    // -- Recall with project="forge" -> should get forge + global, NOT backend --
    let forge_results = do_recall(
        &mut state,
        "Rust SQLite tests",
        None,
        Some("forge".into()),
        Some(50),
    );
    let forge_ids: Vec<&str> = forge_results.iter().map(|r| r.memory.id.as_str()).collect();
    // forge-specific memories should be present
    assert!(
        forge_ids.contains(&forge_id1.as_str()) || forge_ids.contains(&forge_id2.as_str()),
        "forge project recall should return at least one forge memory"
    );
    // Global memory should be visible in forge project
    assert!(
        forge_ids.contains(&global_id.as_str()),
        "global memory should be visible in forge project recall"
    );
    // Backend-only memories should NOT appear
    assert!(
        !forge_ids.contains(&backend_id1.as_str()),
        "backend decision should NOT appear in forge project recall"
    );
    assert!(
        !forge_ids.contains(&backend_id2.as_str()),
        "backend pattern should NOT appear in forge project recall"
    );

    // -- Recall with no project -> should get ALL --
    let all_results = do_recall(&mut state, "Rust PostgreSQL tests pattern", None, None, Some(50));
    assert!(
        all_results.len() >= 3,
        "no-project recall should return memories across all projects, got {}",
        all_results.len()
    );

    // -- Recall with type filter -> only that type --
    let decision_results = do_recall(
        &mut state,
        "Rust PostgreSQL CLI backend",
        Some(MemoryType::Decision),
        None,
        Some(50),
    );
    for r in &decision_results {
        assert_eq!(
            r.memory.memory_type,
            MemoryType::Decision,
            "type-filtered recall should only return Decisions, got {:?}",
            r.memory.memory_type
        );
    }

    // -- Forget one decision -> recall should not return it --
    assert!(do_forget(&mut state, &forge_id1), "forget should succeed");
    let after_forget = do_recall(&mut state, "Rust CLI", None, Some("forge".into()), Some(50));
    let after_forget_ids: Vec<&str> = after_forget.iter().map(|r| r.memory.id.as_str()).collect();
    assert!(
        !after_forget_ids.contains(&forge_id1.as_str()),
        "forgotten memory should not appear in recall"
    );

    // -- Health -> correct counts --
    let resp = handle_request(&mut state, Request::Health);
    match resp {
        Response::Ok {
            data: ResponseData::Health { decisions, lessons, patterns, preferences, .. },
        } => {
            // forge_id1 was forgotten, so 1 forge decision gone => 1 backend decision remains
            assert_eq!(decisions, 1, "should have 1 active decision after forget");
            assert_eq!(lessons, 1, "should have 1 lesson");
            assert_eq!(patterns, 1, "should have 1 pattern");
            assert_eq!(preferences, 1, "should have 1 preference");
        }
        other => panic!("expected Health, got: {:?}", other),
    }

    // -- HealthByProject -> correct per-project breakdown --
    let resp = handle_request(&mut state, Request::HealthByProject);
    match resp {
        Response::Ok { data: ResponseData::HealthByProject { projects } } => {
            // forge project: only the lesson remains (forge_id1 decision was forgotten)
            let forge_data = projects.get("forge").expect("forge project should exist");
            assert_eq!(forge_data.decisions, 0, "forge decisions after forget");
            assert_eq!(forge_data.lessons, 1, "forge lessons");

            // backend project: 1 decision + 1 pattern
            let backend_data = projects.get("backend").expect("backend project should exist");
            assert_eq!(backend_data.decisions, 1, "backend decisions");
            assert_eq!(backend_data.patterns, 1, "backend patterns");

            // global: 1 preference
            let global_data = projects.get("_global").expect("_global should exist");
            assert_eq!(global_data.preferences, 1, "global preferences");
        }
        other => panic!("expected HealthByProject, got: {:?}", other),
    }

    // -- Doctor -> all fields populated --
    let resp = handle_request(&mut state, Request::Doctor);
    match resp {
        Response::Ok {
            data: ResponseData::Doctor {
                daemon_up,
                memory_count,
                workers,
                uptime_secs,
                ..
            },
        } => {
            assert!(daemon_up);
            assert_eq!(memory_count, 4, "4 active memories after 1 forget");
            assert!(!workers.is_empty(), "workers list should not be empty");
            // uptime_secs should be very small in a test
            assert!(uptime_secs < 60, "uptime should be small in a test");
        }
        other => panic!("expected Doctor, got: {:?}", other),
    }

    // -- Export -> all active memories present --
    let resp = handle_request(&mut state, Request::Export { format: None, since: None });
    match resp {
        Response::Ok { data: ResponseData::Export { memories, .. } } => {
            assert_eq!(memories.len(), 4, "export should contain 4 active memories");
            // The forgotten memory should NOT be in the export
            let export_ids: Vec<&str> = memories.iter().map(|m| m.memory.id.as_str()).collect();
            assert!(!export_ids.contains(&forge_id1.as_str()));
        }
        other => panic!("expected Export, got: {:?}", other),
    }

    // -- Shutdown -> returns ok --
    let resp = handle_request(&mut state, Request::Shutdown);
    match resp {
        Response::Ok { data: ResponseData::Shutdown } => {}
        other => panic!("expected Shutdown, got: {:?}", other),
    }
}

// ===========================================================================
// Test 2: Guardrails full lifecycle
// ===========================================================================
#[test]
fn test_guardrails_full_lifecycle() {
    let mut state = fresh_state();

    // -- Remember 3 decisions --
    let d1_id = do_remember(
        &mut state,
        MemoryType::Decision,
        "JWT authentication strategy",
        "Use JWT for all API endpoints",
        Some(0.95),
        None,
        None,
    );
    let d2_id = do_remember(
        &mut state,
        MemoryType::Decision,
        "Rate limiting middleware",
        "Apply rate limits to all endpoints",
        Some(0.9),
        None,
        None,
    );
    let d3_id = do_remember(
        &mut state,
        MemoryType::Decision,
        "Database connection pooling",
        "Use connection pool for all DB access",
        Some(0.85),
        None,
        None,
    );

    // -- Link d1+d2 to file:src/auth.rs via store_edge (affects) --
    ops::store_edge(&state.conn, &d1_id, "file:src/auth.rs", "affects", "{}").unwrap();
    ops::store_edge(&state.conn, &d2_id, "file:src/auth.rs", "affects", "{}").unwrap();

    // -- Link d1+d3 to file:src/db.rs via store_edge (affects) --
    ops::store_edge(&state.conn, &d1_id, "file:src/db.rs", "affects", "{}").unwrap();
    ops::store_edge(&state.conn, &d3_id, "file:src/db.rs", "affects", "{}").unwrap();

    // -- GuardrailsCheck on src/auth.rs -> safe=false, 2 decisions --
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/auth.rs".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, decisions_affected, .. },
        } => {
            assert!(!safe, "src/auth.rs has 2 linked decisions => not safe");
            assert_eq!(decisions_affected.len(), 2);
            assert!(decisions_affected.contains(&d1_id));
            assert!(decisions_affected.contains(&d2_id));
        }
        other => panic!("expected GuardrailsCheck, got: {:?}", other),
    }

    // -- GuardrailsCheck on src/main.rs -> safe=true (no edges) --
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/main.rs".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, decisions_affected, .. },
        } => {
            assert!(safe, "src/main.rs has no linked decisions => safe");
            assert!(decisions_affected.is_empty());
        }
        other => panic!("expected GuardrailsCheck, got: {:?}", other),
    }

    // -- BlastRadius on src/auth.rs -> 2 decisions, files_affected includes src/db.rs --
    let resp = handle_request(
        &mut state,
        Request::BlastRadius { file: "src/auth.rs".into() },
    );
    match resp {
        Response::Ok {
            data: ResponseData::BlastRadius { decisions, files_affected, .. },
        } => {
            assert_eq!(decisions.len(), 2, "blast radius should show 2 decisions for auth.rs");
            let decision_ids: Vec<&str> = decisions.iter().map(|d| d.id.as_str()).collect();
            assert!(decision_ids.contains(&d1_id.as_str()));
            assert!(decision_ids.contains(&d2_id.as_str()));

            // d1 also affects src/db.rs, so db.rs should be in files_affected
            assert!(
                files_affected.contains(&"src/db.rs".to_string()),
                "files_affected should include src/db.rs (co-affected via d1), got: {:?}",
                files_affected
            );
            // src/auth.rs itself should NOT be in files_affected
            assert!(
                !files_affected.contains(&"src/auth.rs".to_string()),
                "target file should not appear in its own files_affected"
            );
        }
        other => panic!("expected BlastRadius, got: {:?}", other),
    }

    // -- Forget d1 -> GuardrailsCheck on src/auth.rs -> now only 1 decision (d2) --
    assert!(do_forget(&mut state, &d1_id));
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/auth.rs".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, decisions_affected, .. },
        } => {
            assert!(!safe, "still 1 linked decision after forgetting d1");
            assert_eq!(decisions_affected.len(), 1);
            assert_eq!(decisions_affected[0], d2_id);
        }
        other => panic!("expected GuardrailsCheck, got: {:?}", other),
    }

    // -- Forget d2 -> GuardrailsCheck on src/auth.rs -> safe=true --
    assert!(do_forget(&mut state, &d2_id));
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/auth.rs".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, decisions_affected, .. },
        } => {
            assert!(safe, "all decisions forgotten => safe");
            assert!(decisions_affected.is_empty());
        }
        other => panic!("expected GuardrailsCheck, got: {:?}", other),
    }
}

// ===========================================================================
// Test 3: Import/Export roundtrip
// ===========================================================================
#[test]
fn test_import_export_roundtrip() {
    let mut state = fresh_state();

    // -- Remember 3 memories with different types, tags, projects --
    let _id1 = do_remember(
        &mut state,
        MemoryType::Decision,
        "Use NDJSON for IPC",
        "Newline-delimited JSON for daemon communication",
        Some(0.95),
        Some(vec!["ipc".into(), "protocol".into()]),
        Some("forge".into()),
    );
    let _id2 = do_remember(
        &mut state,
        MemoryType::Lesson,
        "WAL mode prevents lock contention",
        "SQLite WAL allows concurrent readers",
        Some(0.8),
        Some(vec!["sqlite".into()]),
        Some("backend".into()),
    );
    let _id3 = do_remember(
        &mut state,
        MemoryType::Pattern,
        "Builder pattern for config",
        "Use builder pattern for complex configuration objects",
        Some(0.7),
        None,
        None, // global
    );

    // -- Export -> get JSON --
    let export_resp = handle_request(&mut state, Request::Export { format: None, since: None });
    let export_json = match &export_resp {
        Response::Ok { data: ResponseData::Export { memories, files, symbols, edges } } => {
            assert_eq!(memories.len(), 3);
            assert!(files.is_empty());
            assert!(symbols.is_empty());
            assert!(edges.is_empty());

            // Build the import payload from exported data
            let mem_values: Vec<serde_json::Value> = memories
                .iter()
                .map(|mr| serde_json::to_value(&mr.memory).unwrap())
                .collect();
            serde_json::json!({
                "memories": mem_values,
                "files": [],
                "symbols": []
            })
            .to_string()
        }
        other => panic!("expected Export, got: {:?}", other),
    };

    // -- Create a fresh DaemonState --
    let mut state2 = fresh_state();

    // Verify fresh state is empty
    let resp = handle_request(&mut state2, Request::Health);
    match resp {
        Response::Ok { data: ResponseData::Health { decisions, lessons, patterns, preferences, .. } } => {
            assert_eq!(decisions + lessons + patterns + preferences, 0);
        }
        other => panic!("expected Health, got: {:?}", other),
    }

    // -- Import the exported data into the fresh state --
    let resp = handle_request(&mut state2, Request::Import { data: export_json });
    match resp {
        Response::Ok {
            data: ResponseData::Import { memories_imported, skipped, .. },
        } => {
            assert_eq!(memories_imported, 3, "all 3 memories should be imported");
            assert_eq!(skipped, 0, "no records should be skipped");
        }
        other => panic!("expected Import, got: {:?}", other),
    }

    // -- Recall on the fresh state -> all 3 memories present --
    let all = do_recall(&mut state2, "NDJSON WAL builder pattern config", None, None, Some(50));
    assert!(
        all.len() >= 3,
        "imported state should have all 3 memories recallable, got {}",
        all.len()
    );

    // Verify types are preserved
    let has_decision = all.iter().any(|r| r.memory.memory_type == MemoryType::Decision);
    let has_lesson = all.iter().any(|r| r.memory.memory_type == MemoryType::Lesson);
    let has_pattern = all.iter().any(|r| r.memory.memory_type == MemoryType::Pattern);
    assert!(has_decision, "imported data should contain a Decision");
    assert!(has_lesson, "imported data should contain a Lesson");
    assert!(has_pattern, "imported data should contain a Pattern");

    // Verify health counts match
    let resp = handle_request(&mut state2, Request::Health);
    match resp {
        Response::Ok { data: ResponseData::Health { decisions, lessons, patterns, .. } } => {
            assert_eq!(decisions, 1);
            assert_eq!(lessons, 1);
            assert_eq!(patterns, 1);
        }
        other => panic!("expected Health, got: {:?}", other),
    }
}

// ===========================================================================
// Test 4: Vector persistence across state
// ===========================================================================
#[test]
fn test_vector_persistence_across_state() {
    let mut state = fresh_state();

    // -- Remember a memory --
    let mem_id = do_remember(
        &mut state,
        MemoryType::Decision,
        "Use sqlite-vec for embeddings",
        "Vector search via sqlite-vec extension",
        Some(0.9),
        None,
        None,
    );

    // -- Store an embedding via vec::store_embedding --
    let embedding: Vec<f32> = (0..768).map(|j| (j as f32 * 0.001).sin()).collect();
    vec::store_embedding(&state.conn, &mem_id, &embedding).unwrap();

    // -- Verify vec::has_embedding returns true --
    assert!(
        vec::has_embedding(&state.conn, &mem_id).unwrap(),
        "embedding should exist after store"
    );

    // -- Verify vec::search_vectors finds it --
    let search_results = vec::search_vectors(&state.conn, &embedding, 5).unwrap();
    assert!(!search_results.is_empty(), "vector search should find the stored embedding");
    assert_eq!(search_results[0].0, mem_id, "nearest result should be the stored memory");
    assert!(
        search_results[0].1.abs() < 0.001,
        "self-distance should be ~0, got {}",
        search_results[0].1
    );

    // -- Verify Doctor shows correct counts --
    let resp = handle_request(&mut state, Request::Doctor);
    match resp {
        Response::Ok {
            data: ResponseData::Doctor { memory_count, daemon_up, .. },
        } => {
            assert!(daemon_up);
            assert_eq!(memory_count, 1, "doctor should report 1 memory");
        }
        other => panic!("expected Doctor, got: {:?}", other),
    }

    // Also verify embedding count directly
    let emb_count = vec::count_embeddings(&state.conn).unwrap();
    assert_eq!(emb_count, 1, "should have 1 embedding stored");
}

// ===========================================================================
// Test 5: Concurrent (rapid sequential) operations
// ===========================================================================
#[test]
fn test_concurrent_operations() {
    let mut state = fresh_state();

    // -- Remember 50 memories rapidly --
    let mut ids = Vec::with_capacity(50);
    for i in 0..50 {
        let id = do_remember(
            &mut state,
            if i % 4 == 0 {
                MemoryType::Decision
            } else if i % 4 == 1 {
                MemoryType::Lesson
            } else if i % 4 == 2 {
                MemoryType::Pattern
            } else {
                MemoryType::Preference
            },
            &format!("Memory number {} unique_token_{}", i, i),
            &format!("Content for memory {} with searchable text item_{}", i, i),
            Some(0.5 + (i as f64) * 0.01),
            Some(vec![format!("tag_{}", i)]),
            Some(format!("project_{}", i % 3)),
        );
        ids.push(id);
    }

    // -- Health -> counts match --
    let resp = handle_request(&mut state, Request::Health);
    match resp {
        Response::Ok {
            data: ResponseData::Health { decisions, lessons, patterns, preferences, .. },
        } => {
            let total = decisions + lessons + patterns + preferences;
            assert_eq!(total, 50, "should have 50 total memories, got {total}");
            // 50/4 = 12 or 13 per type
            assert_eq!(decisions, 13, "decisions: indices 0,4,8,...,48 = 13");
            assert_eq!(lessons, 13, "lessons: indices 1,5,9,...,49 = 13");
            assert_eq!(patterns, 12, "patterns: indices 2,6,10,...,46 = 12");
            assert_eq!(preferences, 12, "preferences: indices 3,7,11,...,47 = 12");
        }
        other => panic!("expected Health, got: {:?}", other),
    }

    // -- Recall -> spot check some are present --
    // Search for a specific unique token
    let results = do_recall(&mut state, "unique_token_25", None, None, Some(5));
    assert!(
        !results.is_empty(),
        "should find memory with unique_token_25"
    );

    // -- Forget all 50 --
    for id in &ids {
        let forgotten = do_forget(&mut state, id);
        assert!(forgotten, "forget should succeed for id {}", id);
    }

    // -- Health -> all zero --
    let resp = handle_request(&mut state, Request::Health);
    match resp {
        Response::Ok {
            data: ResponseData::Health { decisions, lessons, patterns, preferences, .. },
        } => {
            assert_eq!(decisions, 0);
            assert_eq!(lessons, 0);
            assert_eq!(patterns, 0);
            assert_eq!(preferences, 0);
        }
        other => panic!("expected Health, got: {:?}", other),
    }
}

// ===========================================================================
// Test 6: Edge cases
// ===========================================================================
#[test]
fn test_edge_cases() {
    let mut state = fresh_state();

    // -- Remember with empty title -> should still store (dedup uses exact title match) --
    let empty_title_id = do_remember(
        &mut state,
        MemoryType::Decision,
        "",
        "Decision with empty title",
        Some(0.5),
        None,
        None,
    );
    assert!(!empty_title_id.is_empty(), "empty title should still produce an ID");

    // -- Remember with confidence 0.0 -> should be clamped and stored --
    let low_conf_id = do_remember(
        &mut state,
        MemoryType::Lesson,
        "Low confidence lesson",
        "This has zero confidence",
        Some(0.0),
        None,
        None,
    );
    // Verify the stored confidence via export
    let resp = handle_request(&mut state, Request::Export { format: None, since: None });
    match &resp {
        Response::Ok { data: ResponseData::Export { memories, .. } } => {
            let low_conf = memories.iter().find(|m| m.memory.id == low_conf_id);
            assert!(low_conf.is_some(), "low confidence memory should be in export");
            assert!(
                low_conf.unwrap().memory.confidence >= 0.0,
                "confidence should be >= 0.0"
            );
        }
        other => panic!("expected Export, got: {:?}", other),
    }

    // -- Remember with confidence 1.5 -> should be clamped to 1.0 --
    let high_conf_id = do_remember(
        &mut state,
        MemoryType::Pattern,
        "High confidence pattern",
        "This has over-max confidence",
        Some(1.5),
        None,
        None,
    );
    let resp = handle_request(&mut state, Request::Export { format: None, since: None });
    match &resp {
        Response::Ok { data: ResponseData::Export { memories, .. } } => {
            let high_conf = memories.iter().find(|m| m.memory.id == high_conf_id);
            assert!(high_conf.is_some(), "high confidence memory should be in export");
            assert!(
                high_conf.unwrap().memory.confidence <= 1.0,
                "confidence should be clamped to <= 1.0, got {}",
                high_conf.unwrap().memory.confidence
            );
        }
        other => panic!("expected Export, got: {:?}", other),
    }

    // -- Recall with empty query -> should not panic (may return empty) --
    let resp = handle_request(
        &mut state,
        Request::Recall {
            query: "".into(),
            memory_type: None,
            project: None,
            limit: None,
        },
    );
    match resp {
        Response::Ok { data: ResponseData::Memories { .. } } => {
            // Success: either empty or non-empty, but no panic
        }
        Response::Error { .. } => {
            // Also acceptable: an error message rather than a panic
        }
        other => panic!("unexpected response for empty query: {:?}", other),
    }

    // -- GuardrailsCheck with empty file -> safe=true (no edges) --
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, decisions_affected, .. },
        } => {
            assert!(safe, "empty file should be safe (no edges)");
            assert!(decisions_affected.is_empty());
        }
        other => panic!("expected GuardrailsCheck, got: {:?}", other),
    }

    // -- BlastRadius with nonexistent file -> empty result --
    let resp = handle_request(
        &mut state,
        Request::BlastRadius {
            file: "nonexistent/path/to/file.rs".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::BlastRadius { decisions, files_affected, importers, .. },
        } => {
            assert!(decisions.is_empty(), "nonexistent file should have no decisions");
            assert!(files_affected.is_empty(), "nonexistent file should have no co-affected files");
            assert!(importers.is_empty(), "nonexistent file should have no importers");
        }
        other => panic!("expected BlastRadius, got: {:?}", other),
    }

    // -- Forget a nonexistent ID -> should return Error, not panic --
    let resp = handle_request(
        &mut state,
        Request::Forget {
            id: "nonexistent-id-12345".into(),
        },
    );
    match resp {
        Response::Error { message } => {
            assert!(
                message.contains("not found") || message.contains("already deleted"),
                "error message should indicate not found, got: {}",
                message
            );
        }
        other => panic!("expected Error for nonexistent forget, got: {:?}", other),
    }

    // -- Status -> should return valid data --
    let resp = handle_request(&mut state, Request::Status);
    match resp {
        Response::Ok {
            data: ResponseData::Status { memory_count, uptime_secs, .. },
        } => {
            assert_eq!(memory_count, 3, "3 active memories in edge-case test");
            assert!(uptime_secs < 60);
        }
        other => panic!("expected Status, got: {:?}", other),
    }
}
