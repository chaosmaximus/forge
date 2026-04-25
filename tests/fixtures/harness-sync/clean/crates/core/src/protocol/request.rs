// Synthetic Request enum for harness-sync fixture tests.
// The serde rename_all = "snake_case" produces these JSON method names:
//   health, health_by_project, recall, remember, record_tool_use, list_tool_calls
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Health,
    HealthByProject,
    Recall { query: String },
    Remember { content: String },
    RecordToolUse { name: String },
    ListToolCalls,
}
