use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_core::protocol::*;
use forge_core::types::MemoryType;

/// Helper: create an in-memory DaemonState for testing.
fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:) should succeed")
}

/// Helper: issue a Remember request and return the stored ID.
fn remember(
    state: &mut DaemonState,
    memory_type: MemoryType,
    title: &str,
    content: &str,
    confidence: Option<f64>,
    tags: Option<Vec<String>>,
    project: Option<String>,
) -> String {
    let req = Request::Remember {
        memory_type,
        title: title.to_string(),
        content: content.to_string(),
        confidence,
        tags,
        project,
    };
    let resp = handle_request(state, req);
    match resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => {
            assert!(!id.is_empty(), "stored id must be non-empty");
            id
        }
        other => panic!("expected Stored response, got: {:?}", other),
    }
}

/// Helper: issue a Recall request and return the results vec.
fn recall(state: &mut DaemonState, query: &str) -> Vec<MemoryResult> {
    let req = Request::Recall {
        query: query.to_string(),
        memory_type: None,
        project: None,
        limit: None,
        layer: None,
    };
    let resp = handle_request(state, req);
    match resp {
        Response::Ok {
            data: ResponseData::Memories { results, count },
        } => {
            assert_eq!(
                results.len(),
                count,
                "results.len() should equal count field"
            );
            results
        }
        other => panic!("expected Memories response, got: {:?}", other),
    }
}

/// Helper: issue a Forget request and return the response.
fn forget(state: &mut DaemonState, id: &str) -> Response {
    let req = Request::Forget {
        id: id.to_string(),
    };
    handle_request(state, req)
}

/// Helper: issue a Health request and return (decisions, lessons, patterns, preferences).
fn health(state: &mut DaemonState) -> (usize, usize, usize, usize) {
    let resp = handle_request(state, Request::Health);
    match resp {
        Response::Ok {
            data:
                ResponseData::Health {
                    decisions,
                    lessons,
                    patterns,
                    preferences,
                    ..
                },
        } => (decisions, lessons, patterns, preferences),
        other => panic!("expected Health response, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test 1: Full memory lifecycle — remember, recall, forget, verify
// ---------------------------------------------------------------------------
#[test]
fn test_full_memory_lifecycle() {
    let mut state = fresh_state();

    // --- Step 1: Remember 3 decisions ---
    let id_jwt = remember(
        &mut state,
        MemoryType::Decision,
        "Use JWT for auth",
        "JSON Web Tokens for stateless authentication across microservices",
        Some(0.9),
        Some(vec!["architecture".to_string()]),
        Some("forge".to_string()),
    );

    let id_pg = remember(
        &mut state,
        MemoryType::Decision,
        "PostgreSQL primary DB",
        "PostgreSQL as the primary relational database for all persistent storage",
        Some(0.9),
        Some(vec!["architecture".to_string()]),
        Some("forge".to_string()),
    );

    let id_redis = remember(
        &mut state,
        MemoryType::Decision,
        "Redis for caching",
        "Redis as the caching layer for frequently accessed data and session tokens",
        Some(0.9),
        Some(vec!["architecture".to_string()]),
        Some("forge".to_string()),
    );

    // All IDs must be unique
    assert_ne!(id_jwt, id_pg, "JWT and PostgreSQL IDs must differ");
    assert_ne!(id_pg, id_redis, "PostgreSQL and Redis IDs must differ");
    assert_ne!(id_jwt, id_redis, "JWT and Redis IDs must differ");

    // --- Step 2: Health check — 3 decisions ---
    let (decisions, lessons, patterns, preferences) = health(&mut state);
    assert_eq!(decisions, 3, "should have 3 decisions after remembering");
    assert_eq!(lessons, 0, "no lessons stored");
    assert_eq!(patterns, 0, "no patterns stored");
    assert_eq!(preferences, 0, "no preferences stored");

    // --- Step 3: Recall "authentication tokens" — should find JWT ---
    let results = recall(&mut state, "authentication tokens");
    assert!(
        !results.is_empty(),
        "recall 'authentication tokens' should return at least 1 result"
    );
    let has_jwt = results
        .iter()
        .any(|r| r.memory.title.contains("JWT"));
    assert!(has_jwt, "at least one result should contain 'JWT' in title");

    // --- Step 4: Recall "database" — should find PostgreSQL ---
    let results = recall(&mut state, "database");
    assert!(
        !results.is_empty(),
        "recall 'database' should return at least 1 result"
    );
    let has_pg = results
        .iter()
        .any(|r| r.memory.title.contains("PostgreSQL"));
    assert!(has_pg, "at least one result should contain 'PostgreSQL' in title");

    // --- Step 5: Forget the JWT decision ---
    // First recall "JWT" to confirm it exists and get the ID
    let jwt_results = recall(&mut state, "JWT");
    assert!(
        !jwt_results.is_empty(),
        "JWT should be recallable before forget"
    );
    let jwt_id = &jwt_results
        .iter()
        .find(|r| r.memory.title.contains("JWT"))
        .expect("should find JWT memory")
        .memory
        .id;
    assert_eq!(jwt_id, &id_jwt, "recalled JWT id should match stored id");

    let forget_resp = forget(&mut state, jwt_id);
    match forget_resp {
        Response::Ok {
            data: ResponseData::Forgotten { id },
        } => {
            assert_eq!(id, id_jwt, "forgotten id should match the JWT decision id");
        }
        other => panic!("expected Forgotten response, got: {:?}", other),
    }

    // --- Step 6: Recall "JWT authentication" — should be gone ---
    let results = recall(&mut state, "JWT authentication");
    let has_jwt_after = results
        .iter()
        .any(|r| r.memory.title.contains("JWT"));
    assert!(
        !has_jwt_after,
        "JWT should not appear in recall after forget (got {} results)",
        results.len()
    );

    // --- Step 7: Health check — 2 decisions remaining ---
    let (decisions, lessons, patterns, preferences) = health(&mut state);
    assert_eq!(decisions, 2, "should have 2 decisions after forgetting one");
    assert_eq!(lessons, 0);
    assert_eq!(patterns, 0);
    assert_eq!(preferences, 0);
}

// ---------------------------------------------------------------------------
// Test 2: Remember different types — decision, lesson, pattern, preference
// ---------------------------------------------------------------------------
#[test]
fn test_remember_different_types() {
    let mut state = fresh_state();

    // Remember one of each type
    let _id_decision = remember(
        &mut state,
        MemoryType::Decision,
        "Microservice architecture",
        "Split the monolith into domain-bounded microservices",
        Some(0.95),
        Some(vec!["architecture".to_string()]),
        None,
    );

    let _id_lesson = remember(
        &mut state,
        MemoryType::Lesson,
        "Always write tests first",
        "TDD prevents regressions and clarifies requirements before coding",
        Some(0.85),
        Some(vec!["testing".to_string()]),
        None,
    );

    let _id_pattern = remember(
        &mut state,
        MemoryType::Pattern,
        "Builder pattern for config",
        "Use the builder pattern for constructing complex configuration objects",
        Some(0.8),
        Some(vec!["design-patterns".to_string()]),
        None,
    );

    let _id_preference = remember(
        &mut state,
        MemoryType::Preference,
        "Prefer Rust for CLI tools",
        "User prefers Rust over Python for command-line tools due to performance",
        Some(0.9),
        Some(vec!["tooling".to_string()]),
        None,
    );

    // Health: verify 1 of each type
    let (decisions, lessons, patterns, preferences) = health(&mut state);
    assert_eq!(decisions, 1, "should have exactly 1 decision");
    assert_eq!(lessons, 1, "should have exactly 1 lesson");
    assert_eq!(patterns, 1, "should have exactly 1 pattern");
    assert_eq!(preferences, 1, "should have exactly 1 preference");

    // Recall each by relevant keyword and verify the correct type comes back
    let results = recall(&mut state, "microservice monolith");
    assert!(!results.is_empty(), "recall 'microservice monolith' should find results");
    let decision_result = results
        .iter()
        .find(|r| r.memory.title.contains("Microservice"));
    assert!(
        decision_result.is_some(),
        "should find the microservice decision"
    );
    assert_eq!(
        decision_result.unwrap().memory.memory_type,
        MemoryType::Decision,
        "microservice memory should be Decision type"
    );

    let results = recall(&mut state, "tests TDD regressions");
    assert!(!results.is_empty(), "recall 'tests TDD' should find results");
    let lesson_result = results
        .iter()
        .find(|r| r.memory.title.contains("tests first"));
    assert!(lesson_result.is_some(), "should find the TDD lesson");
    assert_eq!(
        lesson_result.unwrap().memory.memory_type,
        MemoryType::Lesson,
        "TDD memory should be Lesson type"
    );

    let results = recall(&mut state, "builder configuration objects");
    assert!(!results.is_empty(), "recall 'builder configuration' should find results");
    let pattern_result = results
        .iter()
        .find(|r| r.memory.title.contains("Builder pattern"));
    assert!(
        pattern_result.is_some(),
        "should find the builder pattern"
    );
    assert_eq!(
        pattern_result.unwrap().memory.memory_type,
        MemoryType::Pattern,
        "builder memory should be Pattern type"
    );

    let results = recall(&mut state, "Rust CLI tools performance");
    assert!(!results.is_empty(), "recall 'Rust CLI tools' should find results");
    let pref_result = results
        .iter()
        .find(|r| r.memory.title.contains("Rust"));
    assert!(pref_result.is_some(), "should find the Rust preference");
    assert_eq!(
        pref_result.unwrap().memory.memory_type,
        MemoryType::Preference,
        "Rust CLI memory should be Preference type"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Forget a nonexistent ID — should return Error
// ---------------------------------------------------------------------------
#[test]
fn test_forget_nonexistent() {
    let mut state = fresh_state();

    let fake_id = "NONEXISTENT_000000000000000";
    let resp = forget(&mut state, fake_id);

    match resp {
        Response::Error { message } => {
            assert!(
                message.contains("not found") || message.contains("already deleted"),
                "error message should indicate not found/already deleted, got: {message}"
            );
        }
        other => panic!(
            "expected Error response for nonexistent id, got: {:?}",
            other
        ),
    }
}
