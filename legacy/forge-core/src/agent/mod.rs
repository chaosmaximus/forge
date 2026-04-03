//! Agent lifecycle module — reads hook payloads from stdin and dispatches
//! to start/track/stop handlers.
//!
//! Called as: `forge-core agent --state-dir .forge < hook_payload.json`
//! The payload is a JSON object from Claude Code's hook system.

pub mod start;
pub mod stop;
pub mod track;
pub mod validate;

use serde::Deserialize;
use std::io::Read;

/// Maximum payload size we'll accept from stdin (64 KB).
const MAX_PAYLOAD_SIZE: usize = 64 * 1024;

/// Hook payload structure from Claude Code.
/// All fields are optional — different hook events populate different fields.
#[derive(Deserialize, Default, Debug)]
#[serde(default)]
struct HookPayload {
    #[serde(alias = "hookEventName")]
    hook_event_name: Option<String>,
    #[serde(alias = "sessionId")]
    session_id: Option<String>,
    #[serde(alias = "agentId")]
    agent_id: Option<String>,
    #[serde(alias = "agentType")]
    agent_type: Option<String>,
    #[serde(alias = "toolName")]
    tool_name: Option<String>,
    #[serde(alias = "toolInput")]
    tool_input: Option<serde_json::Value>,
    #[serde(alias = "agentTranscriptPath")]
    agent_transcript_path: Option<String>,
    #[serde(alias = "lastAssistantMessage")]
    last_assistant_message: Option<String>,
}

/// Entry point for the agent subcommand.
///
/// 1. Reads stdin bounded to 64 KB
/// 2. Parses as HookPayload
/// 3. Validates agent_id (exits silently if absent/invalid)
/// 4. Dispatches based on hook_event_name
pub fn run(state_dir: &str) {
    // Read stdin bounded to MAX_PAYLOAD_SIZE
    let mut buf = vec![0u8; MAX_PAYLOAD_SIZE];
    let n = match std::io::stdin().read(&mut buf) {
        Ok(n) => n,
        Err(_) => return, // stdin error — exit silently
    };
    buf.truncate(n);

    // Parse payload
    let payload: HookPayload = match serde_json::from_slice(&buf) {
        Ok(p) => p,
        Err(_) => return, // invalid JSON — exit silently
    };

    // Extract and validate agent_id — required for all dispatch paths
    let agent_id = match payload.agent_id {
        Some(ref id) if validate::valid_agent_id(id) => id.clone(),
        _ => return, // no agent_id or invalid — exit silently
    };

    // Extract and validate agent_type — default to "unknown"
    let agent_type = match payload.agent_type {
        Some(ref t) if validate::valid_agent_type(t) => t.clone(),
        _ => "unknown".to_string(),
    };

    // Dispatch based on hook_event_name
    match payload.hook_event_name.as_deref() {
        Some("SubagentStart") => {
            start::run(state_dir, &agent_id, &agent_type);
        }
        Some("SubagentStop") => {
            stop::run(
                state_dir,
                &agent_id,
                &agent_type,
                payload.agent_transcript_path.as_deref(),
                payload.last_assistant_message.as_deref(),
            );
        }
        _ => {
            // Any other event WITH a tool_name → track
            if let Some(ref tool) = payload.tool_name {
                // Extract file from tool_input if present
                let file = payload
                    .tool_input
                    .as_ref()
                    .and_then(|v| {
                        // Try common field names for file paths
                        v.get("file_path")
                            .or_else(|| v.get("file"))
                            .or_else(|| v.get("path"))
                            .and_then(|f| f.as_str())
                    });

                track::run(state_dir, &agent_id, &agent_type, tool, file);
            }
            // No tool_name and not start/stop — exit silently
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_deserialize_camel_case() {
        let json = r#"{
            "hookEventName": "SubagentStart",
            "sessionId": "sess-123",
            "agentId": "forge-planner-001",
            "agentType": "planner"
        }"#;
        let payload: HookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.hook_event_name, Some("SubagentStart".to_string()));
        assert_eq!(payload.session_id, Some("sess-123".to_string()));
        assert_eq!(payload.agent_id, Some("forge-planner-001".to_string()));
        assert_eq!(payload.agent_type, Some("planner".to_string()));
    }

    #[test]
    fn test_payload_deserialize_snake_case() {
        let json = r#"{
            "hook_event_name": "SubagentStop",
            "agent_id": "test-agent",
            "agent_type": "generator",
            "agent_transcript_path": "/tmp/transcript.jsonl",
            "last_assistant_message": "done"
        }"#;
        let payload: HookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.hook_event_name, Some("SubagentStop".to_string()));
        assert_eq!(payload.agent_id, Some("test-agent".to_string()));
        assert_eq!(
            payload.agent_transcript_path,
            Some("/tmp/transcript.jsonl".to_string())
        );
        assert_eq!(payload.last_assistant_message, Some("done".to_string()));
    }

    #[test]
    fn test_payload_missing_fields_default() {
        let json = r#"{}"#;
        let payload: HookPayload = serde_json::from_str(json).unwrap();
        assert!(payload.hook_event_name.is_none());
        assert!(payload.agent_id.is_none());
        assert!(payload.tool_name.is_none());
    }

    #[test]
    fn test_payload_with_tool_input() {
        let json = r#"{
            "hookEventName": "PostToolUse",
            "agentId": "gen-001",
            "toolName": "Read",
            "toolInput": {"file_path": "/src/main.rs", "limit": 100}
        }"#;
        let payload: HookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.tool_name, Some("Read".to_string()));
        let input = payload.tool_input.unwrap();
        assert_eq!(input["file_path"], "/src/main.rs");
    }
}
