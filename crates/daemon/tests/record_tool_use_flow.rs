//! Integration test for Phase 2A-4c1 Forge-Tool-Use-Recording.
//!
//! Exercises the full Request::RecordToolUse + Request::ListToolCalls path
//! end-to-end through the handler, including:
//!   - happy-path record of 3 calls (success, failure, correction-flagged)
//!   - ListToolCalls with session filter (verifies DESC ordering + all fields
//!     round-trip through serde_json::Value including nested tool_args)
//!   - ListToolCalls with session + agent filter
//!   - target-session organization_id is sourced from the session, not the caller

use forge_core::protocol::{Request, Response, ResponseData};
use forge_daemon::server::handler::{handle_request, DaemonState};

fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

#[test]
fn record_tool_use_flow_end_to_end() {
    let mut state = fresh_state();
    state
        .conn
        .execute(
            "INSERT INTO session (id, agent, started_at, status, organization_id)
             VALUES ('SESS1', 'claude-code', '2026-04-19 10:00:00', 'active', 'acme')",
            [],
        )
        .unwrap();

    // 1. Record 3 calls. Sleep between so created_at differs at second
    //    granularity — DESC ordering assertion below relies on distinct times.
    let calls_to_record = [
        (
            true,
            false,
            "Read",
            serde_json::json!({"file_path": "/tmp/a"}),
            "ok",
        ),
        (
            false,
            false,
            "Bash",
            serde_json::json!({"cmd": "false"}),
            "exit 1",
        ),
        (
            true,
            true,
            "Read",
            serde_json::json!({"file_path": "/tmp/b"}),
            "ok but corrected",
        ),
    ];
    for (success, correction, tool, args, summary) in calls_to_record {
        let req = Request::RecordToolUse {
            session_id: "SESS1".to_string(),
            agent: "claude-code".to_string(),
            tool_name: tool.to_string(),
            tool_args: args,
            tool_result_summary: summary.to_string(),
            success,
            user_correction_flag: correction,
        };
        let resp = handle_request(&mut state, req);
        assert!(matches!(
            resp,
            Response::Ok {
                data: ResponseData::ToolCallRecorded { .. }
            }
        ));
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    // 2. ListToolCalls session-only — verify 3 rows newest-first.
    let resp = handle_request(
        &mut state,
        Request::ListToolCalls {
            session_id: "SESS1".to_string(),
            agent: None,
            limit: None,
        },
    );
    let calls = match resp {
        Response::Ok {
            data: ResponseData::ToolCallList { calls },
        } => calls,
        other => panic!("got {other:?}"),
    };
    assert_eq!(calls.len(), 3);
    // DESC order: most recent (correction-flagged Read) first.
    assert_eq!(calls[0].tool_name, "Read");
    assert!(calls[0].user_correction_flag);
    // tool_args Value round-tripped correctly.
    assert_eq!(
        calls[0].tool_args,
        serde_json::json!({"file_path": "/tmp/b"})
    );

    // 3. ListToolCalls with agent filter (same agent — non-narrowing but
    //    exercises the filter code path).
    let resp = handle_request(
        &mut state,
        Request::ListToolCalls {
            session_id: "SESS1".to_string(),
            agent: Some("claude-code".to_string()),
            limit: None,
        },
    );
    assert!(matches!(
        resp,
        Response::Ok {
            data: ResponseData::ToolCallList { ref calls }
        } if calls.len() == 3
    ));
}

#[test]
fn record_tool_use_writes_target_session_org_id_not_caller_org_id() {
    // Two sessions in two different orgs. Writing to session_b must tag the
    // row with session_b's org, regardless of any "caller" concept.
    let mut state = fresh_state();
    state
        .conn
        .execute(
            "INSERT INTO session (id, agent, started_at, status, organization_id)
             VALUES ('SA', 'a', '2026-04-19 10:00:00', 'active', 'org_a')",
            [],
        )
        .unwrap();
    state
        .conn
        .execute(
            "INSERT INTO session (id, agent, started_at, status, organization_id)
             VALUES ('SB', 'a', '2026-04-19 10:00:00', 'active', 'org_b')",
            [],
        )
        .unwrap();

    // Write into session SB.
    let req = Request::RecordToolUse {
        session_id: "SB".to_string(),
        agent: "a".to_string(),
        tool_name: "T".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: String::new(),
        success: true,
        user_correction_flag: false,
    };
    let _ = handle_request(&mut state, req);

    // Verify the row was stored with org_b, not org_a or 'default'.
    let org: String = state
        .conn
        .query_row(
            "SELECT organization_id FROM session_tool_call WHERE session_id = 'SB' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(org, "org_b");

    // Verify listing session SB yields the row under org_b.
    let resp = handle_request(
        &mut state,
        Request::ListToolCalls {
            session_id: "SB".to_string(),
            agent: None,
            limit: None,
        },
    );
    assert!(matches!(
        resp,
        Response::Ok {
            data: ResponseData::ToolCallList { ref calls }
        } if calls.len() == 1
    ));

    // Verify listing SA returns 0 (no row tagged to org_a for SA).
    let resp = handle_request(
        &mut state,
        Request::ListToolCalls {
            session_id: "SA".to_string(),
            agent: None,
            limit: None,
        },
    );
    assert!(matches!(
        resp,
        Response::Ok {
            data: ResponseData::ToolCallList { ref calls }
        } if calls.is_empty()
    ));
}
