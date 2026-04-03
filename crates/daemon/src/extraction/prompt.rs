// extraction/prompt.rs — Extraction prompt template + output parser

use serde::Deserialize;

// ---------------------------------------------------------------------------
// System prompt
// ---------------------------------------------------------------------------

pub const EXTRACTION_SYSTEM_PROMPT: &str = r#"Extract structured memories from this conversation.
Return a JSON array of objects, each with:
- type: "decision" | "lesson" | "pattern" | "preference" | "skill"
- title: concise summary (under 80 chars)
- content: full rationale/context
- confidence: 0.0-1.0 (how certain this is a real decision/lesson)
- tags: array of relevant keywords
- affects: array of file paths or symbol names mentioned
- valence: "positive" | "negative" | "neutral" (emotional tone of this memory)
- intensity: 0.0-1.0 (how emotionally significant — production outage = 1.0, routine change = 0.1)

Type guidance:
- "decision": a strategic choice made (e.g., "Use JWT for auth")
- "lesson": something learned from experience (e.g., "Always run tests before push")
- "pattern": a recurring approach (e.g., "Error handling uses Result<T, AppError>")
- "preference": a user preference or working style (e.g., "Prefers TDD")
- "skill": a REUSABLE WORKFLOW successfully demonstrated in this conversation.
  Only extract skills when the conversation shows a COMPLETE, SUCCESSFUL workflow
  that could be replicated. The content should describe the steps clearly.
  Example: "Deploy Rust service: 1) cargo build --release 2) copy binary to server 3) restart systemd unit"

Only extract REAL decisions/lessons/skills. If unsure, skip it.
Return [] if nothing worth remembering.
Return ONLY the JSON array, no other text."#;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractedMemory {
    #[serde(rename = "type")]
    pub memory_type: String,
    pub title: String,
    pub content: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub affects: Vec<String>,
    #[serde(default = "default_valence")]
    pub valence: String,
    #[serde(default)]
    pub intensity: f64,
}

fn default_confidence() -> f64 {
    0.5
}

fn default_valence() -> String {
    "neutral".to_string()
}

impl ExtractedMemory {
    /// Check whether memory_type is one of the known valid types.
    pub fn is_valid_type(&self) -> bool {
        matches!(
            self.memory_type.as_str(),
            "decision" | "lesson" | "pattern" | "preference" | "skill"
        )
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse extraction output JSON. Handles:
/// - Clean JSON array
/// - JSON wrapped in ```json code fences
/// - JSON array embedded in surrounding text
/// - Empty responses
///
/// Filters out memories with confidence < 0.3.
pub fn parse_extraction_output(output: &str) -> Vec<ExtractedMemory> {
    let trimmed = output.trim();

    if trimmed.is_empty() {
        return Vec::new();
    }

    // Try parsing the whole thing as JSON first
    if let Ok(memories) = serde_json::from_str::<Vec<ExtractedMemory>>(trimmed) {
        return filter_low_confidence(memories);
    }

    // Try extracting from ```json ... ``` code fences
    if let Some(json_str) = extract_code_fence(trimmed) {
        if let Ok(memories) = serde_json::from_str::<Vec<ExtractedMemory>>(json_str) {
            return filter_low_confidence(memories);
        }
    }

    // Try finding a JSON array embedded in surrounding text
    if let Some(json_str) = extract_json_array(trimmed) {
        if let Ok(memories) = serde_json::from_str::<Vec<ExtractedMemory>>(json_str) {
            return filter_low_confidence(memories);
        }
    }

    // All parsing strategies failed — log so silent data loss is visible
    eprintln!(
        "[extraction] failed to parse LLM output as JSON array (len={})",
        trimmed.len()
    );
    Vec::new()
}

/// Extract content between ```json and ``` fences.
fn extract_code_fence(s: &str) -> Option<&str> {
    let start_marker = "```json";
    let end_marker = "```";

    let start = s.find(start_marker)?;
    let content_start = start + start_marker.len();
    let rest = &s[content_start..];
    let end = rest.find(end_marker)?;
    Some(rest[..end].trim())
}

/// Find the outermost JSON array `[...]` in the text.
fn extract_json_array(s: &str) -> Option<&str> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end > start {
        Some(&s[start..=end])
    } else {
        None
    }
}

/// Filter out memories with confidence < 0.3 or an invalid memory type.
fn filter_low_confidence(memories: Vec<ExtractedMemory>) -> Vec<ExtractedMemory> {
    memories
        .into_iter()
        .filter(|m| m.confidence >= 0.3 && m.is_valid_type())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_clean_json() {
        let input = r#"[
            {
                "type": "decision",
                "title": "Use Rust for the daemon",
                "content": "Rust gives us memory safety without GC overhead",
                "confidence": 0.95,
                "tags": ["rust", "architecture"],
                "affects": ["crates/daemon/src/main.rs"]
            },
            {
                "type": "lesson",
                "title": "Always pin dependency versions",
                "content": "Unpinned deps caused build failures in CI",
                "confidence": 0.8,
                "tags": ["ci", "dependencies"],
                "affects": ["Cargo.toml"]
            }
        ]"#;

        let result = parse_extraction_output(input);
        assert_eq!(result.len(), 2);

        assert_eq!(result[0].memory_type, "decision");
        assert_eq!(result[0].title, "Use Rust for the daemon");
        assert!((result[0].confidence - 0.95).abs() < f64::EPSILON);
        assert_eq!(result[0].tags, vec!["rust", "architecture"]);
        assert_eq!(result[0].affects, vec!["crates/daemon/src/main.rs"]);

        assert_eq!(result[1].memory_type, "lesson");
        assert_eq!(result[1].title, "Always pin dependency versions");
    }

    #[test]
    fn test_parse_markdown_wrapped() {
        let input = r#"```json
[
    {
        "type": "pattern",
        "title": "Error handling with Result",
        "content": "We wrap all fallible ops in Result<T, E>",
        "confidence": 0.7,
        "tags": ["error-handling"],
        "affects": []
    }
]
```"#;

        let result = parse_extraction_output(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_type, "pattern");
        assert_eq!(result[0].title, "Error handling with Result");
        assert!((result[0].confidence - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_empty_array() {
        let result = parse_extraction_output("[]");
        assert!(result.is_empty());

        let result2 = parse_extraction_output("");
        assert!(result2.is_empty());

        let result3 = parse_extraction_output("   \n  ");
        assert!(result3.is_empty());
    }

    #[test]
    fn test_filter_low_confidence() {
        let input = r#"[
            {
                "type": "decision",
                "title": "High confidence item",
                "content": "Should be kept",
                "confidence": 0.9,
                "tags": [],
                "affects": []
            },
            {
                "type": "preference",
                "title": "Low confidence item",
                "content": "Should be filtered out",
                "confidence": 0.2,
                "tags": [],
                "affects": []
            },
            {
                "type": "lesson",
                "title": "Borderline item",
                "content": "Exactly at threshold — kept",
                "confidence": 0.3,
                "tags": [],
                "affects": []
            }
        ]"#;

        let result = parse_extraction_output(input);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].title, "High confidence item");
        assert_eq!(result[1].title, "Borderline item");
    }

    #[test]
    fn test_filter_invalid_type() {
        let output = r#"[{"type":"garbage","title":"Bad","content":"x","confidence":0.9,"tags":[],"affects":[]}]"#;
        let memories = parse_extraction_output(output);
        assert!(memories.is_empty(), "invalid memory_type should be filtered out");
    }

    #[test]
    fn test_skill_type_valid() {
        let em = ExtractedMemory {
            memory_type: "skill".to_string(),
            title: "Deploy Rust service".to_string(),
            content: "1) cargo build --release 2) scp binary 3) systemctl restart".to_string(),
            confidence: 0.85,
            tags: vec!["devops".to_string()],
            affects: vec![],
            valence: "neutral".to_string(),
            intensity: 0.0,
        };
        assert!(em.is_valid_type());
    }

    #[test]
    fn test_skill_parsed_from_json() {
        let json = r#"[{"type":"skill","title":"Run tests","content":"1) cargo test 2) check output","confidence":0.9,"tags":["testing"],"affects":[]}]"#;
        let result = parse_extraction_output(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_type, "skill");
    }

    #[test]
    fn test_extracted_memory_with_valence() {
        let json = r#"[{"type":"decision","title":"Rollback deploy","content":"Production was down","confidence":0.95,"tags":["incident"],"affects":[],"valence":"negative","intensity":0.9}]"#;
        let result = parse_extraction_output(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].valence, "negative");
        assert!((result[0].intensity - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn test_extracted_memory_valence_defaults() {
        let json = r#"[{"type":"lesson","title":"Use TDD","content":"Testing first","confidence":0.8,"tags":[],"affects":[]}]"#;
        let result = parse_extraction_output(json);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].valence, "neutral");
        assert!((result[0].intensity - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_with_surrounding_text() {
        let input = r#"Here are the extracted memories:

[
    {
        "type": "decision",
        "title": "Use SQLite for local storage",
        "content": "SQLite is lightweight and embedded",
        "confidence": 0.85,
        "tags": ["database"],
        "affects": ["crates/daemon/src/db/"]
    }
]

Done. I found 1 memory worth extracting."#;

        let result = parse_extraction_output(input);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_type, "decision");
        assert_eq!(result[0].title, "Use SQLite for local storage");
        assert!((result[0].confidence - 0.85).abs() < f64::EPSILON);
    }
}
