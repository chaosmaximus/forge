use forge_core::protocol::*;
use forge_core::types::MemoryType;
use forge_daemon::db::{ops, vec};
use forge_daemon::server::handler::{handle_request, DaemonState};

fn make_state() -> DaemonState {
    vec::init_sqlite_vec();
    DaemonState::new(":memory:").unwrap()
}

#[test]
fn test_guardrail_check_safe_on_fresh_db() {
    let mut state = make_state();
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/main.rs".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, .. },
        } => {
            assert!(safe);
        }
        _ => panic!("expected GuardrailsCheck"),
    }
}

#[test]
fn test_guardrail_check_with_linked_decision() {
    let mut state = make_state();

    let resp = handle_request(
        &mut state,
        Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Use JWT for authentication".into(),
            content: "Always use JWT tokens, never session cookies".into(),
            confidence: Some(0.95),
            tags: Some(vec!["auth".into()]),
            project: Some("forge".into()),
            metadata: None,
        },
    );
    let decision_id = match resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => id,
        _ => panic!("expected Stored"),
    };

    ops::store_edge(
        &state.conn,
        &decision_id,
        "file:src/auth/middleware.rs",
        "affects",
        "{}",
    )
    .unwrap();

    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/auth/middleware.rs".into(),
            action: "edit".into(),
        },
    );
    match resp {
        Response::Ok {
            data:
                ResponseData::GuardrailsCheck {
                    safe,
                    warnings,
                    decisions_affected,
                    ..
                },
        } => {
            assert!(!safe, "should not be safe");
            assert_eq!(decisions_affected.len(), 1);
            assert_eq!(decisions_affected[0], decision_id);
            assert!(warnings[0].contains("Use JWT"));
            assert!(warnings[0].contains("[edit]"));
        }
        _ => panic!("expected GuardrailsCheck"),
    }
}

#[test]
fn test_blast_radius_with_co_affected_files() {
    let mut state = make_state();

    let resp = handle_request(
        &mut state,
        Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Token validation".into(),
            content: "Validate tokens in middleware and router".into(),
            confidence: Some(0.9),
            tags: None,
            project: None,
            metadata: None,
        },
    );
    let decision_id = match resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => id,
        _ => panic!("expected Stored"),
    };

    ops::store_edge(
        &state.conn,
        &decision_id,
        "file:src/middleware.rs",
        "affects",
        "{}",
    )
    .unwrap();
    ops::store_edge(
        &state.conn,
        &decision_id,
        "file:src/router.rs",
        "affects",
        "{}",
    )
    .unwrap();

    let resp = handle_request(
        &mut state,
        Request::BlastRadius {
            file: "src/middleware.rs".into(),
        },
    );
    match resp {
        Response::Ok {
            data:
                ResponseData::BlastRadius {
                    decisions,
                    files_affected,
                    ..
                },
        } => {
            assert_eq!(decisions.len(), 1);
            assert_eq!(decisions[0].title, "Token validation");
            assert_eq!(files_affected.len(), 1);
            assert_eq!(files_affected[0], "src/router.rs");
        }
        _ => panic!("expected BlastRadius"),
    }
}

#[test]
fn test_guardrail_after_forget() {
    let mut state = make_state();

    let resp = handle_request(
        &mut state,
        Request::Remember {
            memory_type: MemoryType::Decision,
            title: "Old approach".into(),
            content: "Deprecated".into(),
            confidence: None,
            tags: None,
            project: None,
            metadata: None,
        },
    );
    let id = match resp {
        Response::Ok {
            data: ResponseData::Stored { id },
        } => id,
        _ => panic!("expected Stored"),
    };

    ops::store_edge(&state.conn, &id, "file:src/old.rs", "affects", "{}").unwrap();

    // Forget the decision
    handle_request(&mut state, Request::Forget { id });

    // Should now be safe
    let resp = handle_request(
        &mut state,
        Request::GuardrailsCheck {
            file: "src/old.rs".into(),
            action: "delete".into(),
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::GuardrailsCheck { safe, .. },
        } => {
            assert!(safe, "forgotten decisions should not block");
        }
        _ => panic!("expected GuardrailsCheck"),
    }
}
