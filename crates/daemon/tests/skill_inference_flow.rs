//! Integration test for Phase 2A-4c2 Forge-Behavioral-Skill-Inference.
//!
//! Exercises the full surface end-to-end through the Rust handler:
//! register_session × 3 → record_tool_use × 9 (matching fingerprint) →
//! force_consolidate → compile_context → verify <skill inferred_sessions="3">.

use forge_core::protocol::{Request, Response, ResponseData};
use forge_daemon::server::handler::{handle_request, DaemonState};

fn fresh_state() -> DaemonState {
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:)")
}

fn register(state: &mut DaemonState, id: &str, agent: &str, project: &str) {
    let resp = handle_request(
        state,
        Request::RegisterSession {
            id: id.to_string(),
            agent: agent.to_string(),
            project: Some(project.to_string()),
            cwd: Some("/tmp".to_string()),
            capabilities: None,
            current_task: None,
        },
    );
    assert!(
        matches!(resp, Response::Ok { .. }),
        "register failed: {resp:?}"
    );
}

fn record(state: &mut DaemonState, session: &str, tool: &str, args: serde_json::Value) {
    let resp = handle_request(
        state,
        Request::RecordToolUse {
            session_id: session.to_string(),
            agent: "claude-code".to_string(),
            tool_name: tool.to_string(),
            tool_args: args,
            tool_result_summary: String::new(),
            success: true,
            user_correction_flag: false,
        },
    );
    assert!(
        matches!(resp, Response::Ok { .. }),
        "record_tool_use failed: {resp:?}"
    );
}

fn force_consolidate(state: &mut DaemonState) {
    // ForceConsolidate is a unit variant — no braces.
    let resp = handle_request(state, Request::ForceConsolidate);
    assert!(
        matches!(resp, Response::Ok { .. }),
        "force_consolidate failed: {resp:?}"
    );
}

fn compile_context_xml(state: &mut DaemonState, project: &str) -> String {
    let resp = handle_request(
        state,
        Request::CompileContext {
            agent: None,
            project: Some(project.to_string()),
            static_only: None,
            excluded_layers: None,
            session_id: None,
            focus: None,
        },
    );
    match resp {
        Response::Ok {
            data: ResponseData::CompiledContext { context, .. },
        } => context,
        other => panic!("compile_context failed: {other:?}"),
    }
}

#[test]
fn skill_inference_end_to_end_via_protocol() {
    let mut state = fresh_state();
    for sid in ["SA", "SB", "SC"] {
        register(&mut state, sid, "claude-code", "proj");
        record(
            &mut state,
            sid,
            "Read",
            serde_json::json!({"file_path": "/a"}),
        );
        record(
            &mut state,
            sid,
            "Edit",
            serde_json::json!({"file_path": "/a", "old_string": "x", "new_string": "y"}),
        );
        record(
            &mut state,
            sid,
            "Bash",
            serde_json::json!({"cmd": "cargo test"}),
        );
    }

    force_consolidate(&mut state);

    let xml = compile_context_xml(&mut state, "proj");
    assert!(
        xml.contains("inferred_sessions=\"3\""),
        "<skills> must contain inferred_sessions=\"3\" after 3 matching sessions; got:\n{xml}"
    );
    assert!(
        xml.contains("Inferred: Bash+Edit+Read"),
        "inferred skill name missing from XML:\n{xml}"
    );
}

#[test]
fn skill_inference_does_not_emit_for_two_sessions() {
    let mut state = fresh_state();
    for sid in ["SA", "SB"] {
        register(&mut state, sid, "claude-code", "proj");
        record(
            &mut state,
            sid,
            "Read",
            serde_json::json!({"file_path": "/a"}),
        );
        record(
            &mut state,
            sid,
            "Edit",
            serde_json::json!({"file_path": "/a", "old_string": "x", "new_string": "y"}),
        );
        record(
            &mut state,
            sid,
            "Bash",
            serde_json::json!({"cmd": "cargo test"}),
        );
    }

    force_consolidate(&mut state);

    let xml = compile_context_xml(&mut state, "proj");
    assert!(
        !xml.contains("Inferred: Bash+Edit+Read"),
        "inferred skill must NOT be emitted at 2 sessions; got:\n{xml}"
    );
}
