use serde::{Deserialize, Serialize};

use crate::time::now_iso;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Decision,
    Lesson,
    Pattern,
    Preference,
    /// Process-level meta-knowledge: HOW work should be done.
    /// Protocols evolve over time — user-declared or extracted from behavior.
    /// Injected into agent context as `<active-protocols>`.
    Protocol,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Superseded,
    Reverted,
    Faded,
    Conflict,
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
    #[serde(default = "default_valence")]
    pub valence: String, // "positive", "negative", "neutral"
    #[serde(default)]
    pub intensity: f64, // 0.0-1.0 how emotionally significant
    #[serde(default)]
    pub hlc_timestamp: String, // HLC: "{wall_ms}-{counter}-{node_id}"
    #[serde(default)]
    pub node_id: String, // 8-char hex node identifier
    #[serde(default)]
    pub session_id: String, // Session that created this memory
    #[serde(default)]
    pub access_count: u64, // How many times this memory has been accessed
    #[serde(default)]
    pub activation_level: f64, // 0.0-1.0, boosted on recall/context, decayed each consolidation
    #[serde(default)]
    pub alternatives: Vec<String>, // What was considered but rejected (counterfactual memory)
    #[serde(default)]
    pub participants: Vec<String>, // Who was involved (relational memory)
    #[serde(default)]
    pub organization_id: Option<String>, // Multi-tenant isolation: org that owns this memory
}

fn default_valence() -> String {
    "neutral".to_string()
}

impl Memory {
    pub fn new(
        memory_type: MemoryType,
        title: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
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
            valence: "neutral".to_string(),
            intensity: 0.0,
            hlc_timestamp: String::new(),
            node_id: String::new(),
            session_id: String::new(),
            access_count: 0,
            activation_level: 0.0,
            alternatives: Vec::new(),
            participants: Vec::new(),
            organization_id: None,
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

    pub fn with_organization(mut self, org_id: impl Into<String>) -> Self {
        self.organization_id = Some(org_id.into());
        self
    }

    pub fn with_valence(mut self, valence: &str, intensity: f64) -> Self {
        self.valence = valence.to_string();
        self.intensity = intensity.clamp(0.0, 1.0);
        self
    }

    pub fn with_alternatives(mut self, alternatives: Vec<String>) -> Self {
        self.alternatives = alternatives;
        self
    }

    pub fn with_participants(mut self, participants: Vec<String>) -> Self {
        self.participants = participants;
        self
    }

    /// Set HLC timestamp and node_id (called before storing to DB).
    pub fn set_hlc(&mut self, hlc_timestamp: String, node_id: String) {
        self.hlc_timestamp = hlc_timestamp;
        self.node_id = node_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_new() {
        let m = Memory::new(
            MemoryType::Decision,
            "Use NDJSON",
            "Newline-delimited JSON for IPC",
        );
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
    fn test_memory_with_valence() {
        let mem = Memory::new(MemoryType::Decision, "Broke prod", "Server went down")
            .with_valence("negative", 0.9);
        assert_eq!(mem.valence, "negative");
        assert!((mem.intensity - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn test_memory_valence_defaults() {
        let mem = Memory::new(MemoryType::Lesson, "Test first", "TDD works");
        assert_eq!(mem.valence, "neutral");
        assert!((mem.intensity - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_memory_valence_clamped() {
        let mem = Memory::new(MemoryType::Decision, "test", "test").with_valence("positive", 1.5);
        assert!(
            (mem.intensity - 1.0).abs() < f64::EPSILON,
            "intensity should be clamped to 1.0"
        );

        let mem2 = Memory::new(MemoryType::Decision, "test", "test").with_valence("negative", -0.5);
        assert!(
            (mem2.intensity - 0.0).abs() < f64::EPSILON,
            "intensity should be clamped to 0.0"
        );
    }

    #[test]
    fn test_memory_serde_roundtrip() {
        let original = Memory::new(
            MemoryType::Pattern,
            "Builder pattern",
            "Use fluent builders",
        )
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
