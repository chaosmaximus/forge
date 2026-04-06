// extraction/prompt.rs — Extraction prompt template + output parser

use serde::Deserialize;

// ---------------------------------------------------------------------------
// System prompt
// ---------------------------------------------------------------------------

pub const EXTRACTION_SYSTEM_PROMPT: &str = r#"Extract structured memories from this conversation.
Return a JSON array of objects. EXACT field names required (do NOT rename fields):
- "type": MUST be one of: "decision", "lesson", "pattern", "preference", "skill", "identity"
- "title": concise summary string (under 80 chars)
- "content": full rationale/context string
- "confidence": number between 0.0 and 1.0 (how certain this is a real memory)
- tags: array of relevant keywords
- affects: array of file paths or symbol names mentioned
- valence: "positive" | "negative" | "neutral" (emotional tone of this memory)
- intensity: 0.0-1.0 (how emotionally significant — production outage = 1.0, routine change = 0.1)
- motivated_by: optional — title of a previous decision/lesson that motivated this one
- alternatives: optional array of alternatives that were considered but rejected (e.g., ["MongoDB — rejected for lack of ACID", "Redis — rejected as too volatile"]). Only include when the conversation explicitly discusses alternatives.
- participants: optional array of people mentioned as involved (e.g., ["Alice — suggested this approach", "Bob — reviewed"]). Only include when specific people are mentioned.

Type guidance:
- "decision": a strategic choice made (e.g., "Use JWT for auth")
- "lesson": something learned from experience (e.g., "Always run tests before push")
- "pattern": a recurring approach (e.g., "Error handling uses Result<T, AppError>")
- "preference": a user preference or working style (e.g., "Prefers TDD")
- "identity": a signal about the HUMAN USER's role, expertise, or working context.
  Extract ONLY when the human user reveals WHO THEY ARE or WHAT THEY DO.
  NEVER extract identity about the AI assistant/agent — only the human.
  Look for: "I'm a...", "I work on...", "my company...", "I want to...", "we're building..."
  IMPORTANT: always include a tag indicating the facet type: "role", "expertise", "domain", "values", "goals", or "constraints".
  The title should be the identity signal, content should be the evidence from the user's own words.
  Examples:
    {"type": "identity", "title": "Senior Rust developer", "content": "User demonstrated deep Rust knowledge and mentioned years of experience", "confidence": 0.9, "tags": ["expertise"], "affects": []}
    {"type": "identity", "title": "Building a fintech platform", "content": "User is working on a financial technology product with payment processing", "confidence": 0.85, "tags": ["domain"], "affects": []}
    {"type": "identity", "title": "Security-first approach", "content": "User explicitly prioritizes security in all design decisions", "confidence": 0.8, "tags": ["values"], "affects": []}
    {"type": "identity", "title": "Tech lead at startup", "content": "User mentioned leading a small engineering team", "confidence": 0.9, "tags": ["role"], "affects": []}
    {"type": "identity", "title": "Ship weekly releases", "content": "User wants to maintain a weekly release cadence", "confidence": 0.7, "tags": ["goals"], "affects": []}
- "skill": a reusable pattern. TWO forms:

  A) PROCEDURAL: an explicit workflow with DISCRETE, NUMBERED STEPS.
     ONLY extract as a procedural skill if ALL of these are true:
     1. The workflow was SUCCESSFULLY completed in this conversation
     2. The workflow has at least 2 discrete steps that could be followed again
     3. The workflow is GENERALIZABLE (not specific to one file/bug/task)
     4. The title describes WHAT the workflow does, not a task status
     The content MUST contain numbered steps (1. 2. 3.) or bullet points (- step1 - step2).
     Include tag "procedural" plus a domain tag.
     Examples:
       {"type": "skill", "title": "Deploy Rust Service", "content": "1) cargo build --release 2) scp binary 3) systemctl restart", "confidence": 0.9, "tags": ["procedural", "deployment"], "affects": []}
       {"type": "skill", "title": "Add New Protocol Endpoint", "content": "1) Add Request variant 2) Add Response variant 3) Add handler arm 4) Add contract test", "confidence": 0.85, "tags": ["procedural", "protocol"], "affects": []}

  B) BEHAVIORAL: a pattern in HOW the user works — their debugging heuristic,
     architecture approach, quality standard, or decision-making style.
     Extract when you observe a REPEATED PATTERN in the user's behavior.
     The content MUST be a meaningful description (at least 100 chars) of the pattern.
     Include tag "behavioral" plus a domain tag.
     Examples:
       {"type": "skill", "title": "Debug by tracing to system failure", "content": "When encountering a bug, the user first asks 'why didn't the system catch this?' — traces the root cause to infrastructure design, not just the symptom.", "confidence": 0.85, "tags": ["behavioral", "debugging"], "affects": []}
       {"type": "skill", "title": "Wave-based parallel architecture", "content": "The user breaks large tasks into independent waves, builds in parallel with agents, runs adversarial review per wave, then merges.", "confidence": 0.9, "tags": ["behavioral", "architecture"], "affects": []}
       {"type": "skill", "title": "Fail-loud quality standard", "content": "The user insists on no silent error swallowing. Every error must be logged, every failure visible. Operations must truly succeed or fail visibly.", "confidence": 0.95, "tags": ["behavioral", "quality"], "affects": []}

  BAD (do NOT extract as skill):
  - "All 17 Tasks Complete" (status update, not a workflow)
  - "Fix the remaining 4 failures" (task-specific, not reusable)
  - "Cleanup Legacy Swift App" (one-off task)
  If you can't identify at least 2 discrete steps AND it's not a behavioral pattern, extract as a "lesson" instead.

Only extract REAL decisions/lessons/skills. If unsure, skip it.
Return [] if nothing worth remembering.
Return ONLY the JSON array, no other text."#;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ExtractedMemory {
    #[serde(alias = "type", alias = "memory_type", alias = "category")]
    pub memory_type: String,
    #[serde(alias = "title", alias = "summary", alias = "name")]
    pub title: String,
    #[serde(alias = "content", alias = "description", alias = "rationale", alias = "details")]
    pub content: String,
    #[serde(default = "default_confidence", alias = "score")]
    pub confidence: f64,
    #[serde(default, alias = "keywords", alias = "labels")]
    pub tags: Vec<String>,
    #[serde(default, alias = "files", alias = "affected_files")]
    pub affects: Vec<String>,
    #[serde(default = "default_valence")]
    pub valence: String,
    #[serde(default)]
    pub intensity: f64,
    /// Optional: ID or title of a previous decision/lesson that motivated this one (causal chain)
    #[serde(default)]
    pub motivated_by: Option<String>,
    /// Optional: alternatives considered but rejected (counterfactual memory)
    #[serde(default)]
    pub alternatives: Vec<String>,
    /// Optional: people involved in this decision/lesson (relational memory)
    #[serde(default)]
    pub participants: Vec<String>,
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
            "decision" | "lesson" | "pattern" | "preference" | "skill" | "identity"
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

    // Try finding a JSON array embedded anywhere in the text (most robust)
    // This handles: code fences, thinking blocks, prose around JSON, truncated fences
    if let Some(json_str) = extract_json_array(trimmed) {
        if let Ok(memories) = serde_json::from_str::<Vec<ExtractedMemory>>(json_str) {
            return filter_low_confidence(memories);
        }
        // JSON array found but parse failed — try fixing common issues
        // (e.g., trailing comma, truncated last element)
        let cleaned = json_str.trim_end_matches(',').trim();
        if !cleaned.ends_with(']') {
            // Truncated — try adding closing bracket
            let fixed = format!("{}]", cleaned.rsplit_once(',').map(|(before, _)| before).unwrap_or(cleaned));
            if let Ok(memories) = serde_json::from_str::<Vec<ExtractedMemory>>(&fixed) {
                eprintln!("[extraction] recovered truncated JSON array ({} memories)", memories.len());
                return filter_low_confidence(memories);
            }
        }
    }

    // Try extracting from ```json ... ``` code fences
    if let Some(json_str) = extract_code_fence(trimmed) {
        if let Ok(memories) = serde_json::from_str::<Vec<ExtractedMemory>>(json_str) {
            return filter_low_confidence(memories);
        }
    }

    // All parsing strategies failed — log content for debugging (fail-loud)
    let preview: String = trimmed.chars().take(300).collect();
    eprintln!(
        "[extraction] failed to parse LLM output as JSON array (len={}): {}",
        trimmed.len(), preview
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
            motivated_by: None,
            alternatives: vec![],
            participants: vec![],
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

    #[test]
    fn test_identity_type_parsed_with_facet_tags() {
        let json = r#"[
            {"type":"identity","title":"Senior Rust developer","content":"User demonstrated deep Rust knowledge","confidence":0.9,"tags":["expertise"],"affects":[]},
            {"type":"identity","title":"Building a fintech platform","content":"User is working on fintech","confidence":0.85,"tags":["domain"],"affects":[]},
            {"type":"identity","title":"Tech lead at startup","content":"User leads engineering team","confidence":0.9,"tags":["role"],"affects":[]}
        ]"#;
        let result = parse_extraction_output(json);
        assert_eq!(result.len(), 3, "all 3 identity memories should parse");
        assert_eq!(result[0].memory_type, "identity");
        assert_eq!(result[0].tags, vec!["expertise"]);
        assert_eq!(result[1].memory_type, "identity");
        assert_eq!(result[1].tags, vec!["domain"]);
        assert_eq!(result[2].memory_type, "identity");
        assert_eq!(result[2].tags, vec!["role"]);
    }

    #[test]
    fn test_identity_type_valid() {
        let em = ExtractedMemory {
            memory_type: "identity".to_string(),
            title: "Senior Rust developer".to_string(),
            content: "User demonstrated deep Rust knowledge".to_string(),
            confidence: 0.9,
            tags: vec!["expertise".to_string()],
            affects: vec![],
            valence: "neutral".to_string(),
            intensity: 0.0,
            motivated_by: None,
            alternatives: vec![],
            participants: vec![],
        };
        assert!(em.is_valid_type(), "'identity' should be a valid type");
    }
}
