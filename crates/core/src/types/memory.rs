use serde::{Deserialize, Serialize};

/// NEW-6: Produce ISO 8601 timestamps matching SQLite `datetime('now')` format.
/// Output: `"2026-04-02 23:15:30"` (no trailing Z — matches SQLite convention).
fn now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;

    // Date calculation (correct for 1970–2399 range)
    let mut year = 1970u64;
    let mut remaining_days = days_since_epoch;
    loop {
        let is_leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days_in_year = if is_leap { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }

    let is_leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let month_days: [u64; 12] = if is_leap {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 1u64;
    for &days in &month_days {
        if remaining_days < days {
            break;
        }
        remaining_days -= days;
        month += 1;
    }
    let day = remaining_days + 1;

    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds
    )
}

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
        self.confidence = confidence;
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
            .with_project("forge-v2");

        assert_eq!(m.memory_type, MemoryType::Lesson);
        assert!((m.confidence - 0.75).abs() < f64::EPSILON);
        assert_eq!(m.tags, vec!["tdd".to_string(), "testing".to_string()]);
        assert_eq!(m.project, Some("forge-v2".to_string()));
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
