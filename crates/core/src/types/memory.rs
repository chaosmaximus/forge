use serde::{Deserialize, Serialize};

use crate::time::now_iso;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Decision,
    Lesson,
    Pattern,
    Preference,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Superseded,
    Reverted,
    Faded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub confidence: f64,
    pub status: MemoryStatus,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub embedding: Option<Vec<f32>>,
    pub created_at: String,
    pub accessed_at: String,
}

impl Memory {
    pub fn new(memory_type: MemoryType, title: impl Into<String>, content: impl Into<String>) -> Self {
        let now = now_iso();
        Self {
            id: ulid::Ulid::new().to_string(),
            memory_type,
            title: title.into(),
            content: content.into(),
            confidence: 0.9,
            status: MemoryStatus::Active,
            project: None,
            tags: Vec::new(),
            embedding: None,
            created_at: now.clone(),
            accessed_at: now,
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_project(mut self, project: impl Into<String>) -> Self {
        self.project = Some(project.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_new() {
        let m = Memory::new(MemoryType::Decision, "Use NDJSON", "Newline-delimited JSON for IPC");
        assert_eq!(m.memory_type, MemoryType::Decision);
        assert_eq!(m.title, "Use NDJSON");
        assert_eq!(m.content, "Newline-delimited JSON for IPC");
        assert!((m.confidence - 0.9).abs() < f64::EPSILON);
        assert_eq!(m.status, MemoryStatus::Active);
        assert!(m.project.is_none());
        assert!(m.tags.is_empty());
        assert!(m.embedding.is_none());
        assert!(!m.id.is_empty());
        assert!(!m.created_at.is_empty());
        assert!(!m.accessed_at.is_empty());
    }

    #[test]
    fn test_memory_builder() {
        let m = Memory::new(MemoryType::Lesson, "TDD first", "Write tests before impl")
            .with_confidence(0.75)
            .with_tags(vec!["tdd".to_string(), "testing".to_string()])
            .with_project("forge");

        assert_eq!(m.memory_type, MemoryType::Lesson);
        assert!((m.confidence - 0.75).abs() < f64::EPSILON);
        assert_eq!(m.tags, vec!["tdd".to_string(), "testing".to_string()]);
        assert_eq!(m.project, Some("forge".to_string()));
    }

    #[test]
    fn test_memory_serde_roundtrip() {
        let original = Memory::new(MemoryType::Pattern, "Builder pattern", "Use fluent builders")
            .with_confidence(0.85)
            .with_tags(vec!["rust".to_string()])
            .with_project("core");

        let json = serde_json::to_string(&original).expect("serialize");
        let restored: Memory = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(original.id, restored.id);
        assert_eq!(original.memory_type, restored.memory_type);
        assert_eq!(original.title, restored.title);
        assert_eq!(original.content, restored.content);
        assert!((original.confidence - restored.confidence).abs() < f64::EPSILON);
        assert_eq!(original.status, restored.status);
        assert_eq!(original.tags, restored.tags);
        assert_eq!(original.project, restored.project);
        assert_eq!(original.created_at, restored.created_at);
    }
}
