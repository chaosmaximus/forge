use serde::{Deserialize, Serialize};
use crate::types::memory::MemoryType;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Remember {
        memory_type: MemoryType,
        title: String,
        content: String,
        confidence: Option<f64>,
        tags: Option<Vec<String>>,
        project: Option<String>,
    },
    Recall {
        query: String,
        memory_type: Option<MemoryType>,
        limit: Option<usize>,
    },
    Forget {
        id: String,
    },
    Health,
    Status,
    Doctor,
    Shutdown,
}
