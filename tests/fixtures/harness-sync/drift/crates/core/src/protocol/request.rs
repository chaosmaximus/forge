// Same Request enum as clean fixture — drift comes from skills/agents only.
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
