use serde::{Deserialize, Serialize};

// ──────────────────────────────────────────────
// Layer 0: Platform
// ──────────────────────────────────────────────

/// A key-value pair describing the host platform (OS, shell, arch, etc.)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlatformEntry {
    pub key: String,
    pub value: String,
    pub detected_at: String,
}

// ──────────────────────────────────────────────
// Layer 1: Tools
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Cli,
    Mcp,
    Builtin,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ToolHealth {
    Healthy,
    Degraded,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    pub id: String,
    pub name: String,
    pub kind: ToolKind,
    pub capabilities: Vec<String>,
    pub config: Option<String>,
    pub health: ToolHealth,
    pub last_used: Option<String>,
    pub use_count: u64,
    pub discovered_at: String,
}

// ──────────────────────────────────────────────
// Layer 2: Skills
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub description: String,
    pub steps: Vec<String>,
    pub success_count: u64,
    pub fail_count: u64,
    pub last_used: Option<String>,
    pub source: String,
    pub version: u64,
    pub project: Option<String>,
    /// "procedural" (numbered steps) or "behavioral" (observed user pattern)
    #[serde(default = "default_skill_type")]
    pub skill_type: String,
    /// Whether this skill is specific to the user's working style
    #[serde(default)]
    pub user_specific: bool,
    /// How many times this pattern has been independently observed
    #[serde(default = "default_observed_count")]
    pub observed_count: u32,
    /// IDs of correlated memories (identity facets, decisions, patterns)
    #[serde(default)]
    pub correlation_ids: Vec<String>,
}

fn default_skill_type() -> String {
    "procedural".to_string()
}

fn default_observed_count() -> u32 {
    1
}

impl Default for Skill {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            domain: "general".to_string(),
            description: String::new(),
            steps: Vec::new(),
            success_count: 0,
            fail_count: 0,
            last_used: None,
            source: "test".to_string(),
            version: 1,
            project: None,
            skill_type: "procedural".to_string(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: Vec::new(),
        }
    }
}

// ──────────────────────────────────────────────
// Layer 3: Domain DNA
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DomainDna {
    pub id: String,
    pub project: String,
    pub aspect: String,
    pub pattern: String,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub detected_at: String,
}

// ──────────────────────────────────────────────
// Layer 4: Perception
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionKind {
    FileChange,
    Error,
    BuildResult,
    TestResult,
    UserFeedback,
    MissingTool,
    CrossSessionDecision,
    ActionSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Perception {
    pub id: String,
    pub kind: PerceptionKind,
    pub data: String,
    pub severity: Severity,
    pub project: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub consumed: bool,
}

// ──────────────────────────────────────────────
// Layer 5: Declared Knowledge
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Declared {
    pub id: String,
    pub source: String,
    pub path: Option<String>,
    pub content: String,
    pub hash: String,
    pub project: Option<String>,
    pub ingested_at: String,
}

// ──────────────────────────────────────────────
// Layer 6: Identity
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentityFacet {
    pub id: String,
    pub agent: String,
    pub facet: String,
    pub description: String,
    pub strength: f64,
    pub source: String,
    pub active: bool,
    pub created_at: String,
}

// ──────────────────────────────────────────────
// Layer 7: Disposition
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DispositionTrait {
    Caution,
    Thoroughness,
    Autonomy,
    Verbosity,
    Creativity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    Rising,
    Stable,
    Falling,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Disposition {
    pub id: String,
    pub agent: String,
    pub disposition_trait: DispositionTrait,
    pub domain: Option<String>,
    pub value: f64,
    pub trend: Trend,
    pub updated_at: String,
    pub evidence: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_entry_serde() {
        let entry = PlatformEntry {
            key: "os".into(),
            value: "linux".into(),
            detected_at: "2026-04-03 12:00:00".into(),
        };
        let json = serde_json::to_string(&entry).expect("serialize PlatformEntry");
        let restored: PlatformEntry = serde_json::from_str(&json).expect("deserialize PlatformEntry");
        assert_eq!(entry, restored);
    }

    #[test]
    fn test_tool_serde() {
        let tool = Tool {
            id: "t1".into(),
            name: "cargo".into(),
            kind: ToolKind::Cli,
            capabilities: vec!["build".into(), "test".into()],
            config: None,
            health: ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03 12:00:00".into(),
        };
        let json = serde_json::to_string(&tool).expect("serialize Tool");
        let restored: Tool = serde_json::from_str(&json).expect("deserialize Tool");
        assert_eq!(tool, restored);
    }

    #[test]
    fn test_skill_serde() {
        let skill = Skill {
            id: "s1".into(),
            name: "TDD".into(),
            domain: "testing".into(),
            description: "Test-driven development".into(),
            steps: vec!["write test".into(), "make it pass".into()],
            success_count: 5,
            fail_count: 1,
            last_used: Some("2026-04-03 12:00:00".into()),
            source: "learned".into(),
            version: 1,
            project: None,
            skill_type: "procedural".into(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        };
        let json = serde_json::to_string(&skill).expect("serialize Skill");
        let restored: Skill = serde_json::from_str(&json).expect("deserialize Skill");
        assert_eq!(skill, restored);
    }

    #[test]
    fn test_skill_serde_defaults_backward_compat() {
        // Simulates deserializing a skill stored BEFORE behavioral fields were added
        let json = r#"{"id":"s1","name":"TDD","domain":"testing","description":"Test-driven development","steps":["write test"],"success_count":5,"fail_count":1,"last_used":null,"source":"learned","version":1,"project":null}"#;
        let skill: Skill = serde_json::from_str(json).expect("deserialize old Skill format");
        assert_eq!(skill.skill_type, "procedural");
        assert!(!skill.user_specific);
        assert_eq!(skill.observed_count, 1);
        assert!(skill.correlation_ids.is_empty());
    }

    #[test]
    fn test_behavioral_skill_serde() {
        let skill = Skill {
            id: "bs1".into(),
            name: "Debug by tracing to system failure".into(),
            domain: "debugging".into(),
            description: "When encountering a bug, the user first asks 'why didn't the system catch this?' — traces the root cause to infrastructure design, not just the symptom.".into(),
            steps: vec![],
            success_count: 1,
            fail_count: 0,
            last_used: None,
            source: "extracted".into(),
            version: 1,
            project: None,
            skill_type: "behavioral".into(),
            user_specific: true,
            observed_count: 3,
            correlation_ids: vec!["identity-abc".into(), "decision-xyz".into()],
        };
        let json = serde_json::to_string(&skill).expect("serialize behavioral Skill");
        let restored: Skill = serde_json::from_str(&json).expect("deserialize behavioral Skill");
        assert_eq!(skill, restored);
        assert_eq!(restored.skill_type, "behavioral");
        assert!(restored.user_specific);
        assert_eq!(restored.observed_count, 3);
        assert_eq!(restored.correlation_ids.len(), 2);
    }

    #[test]
    fn test_domain_dna_serde() {
        let dna = DomainDna {
            id: "d1".into(),
            project: "forge".into(),
            aspect: "naming".into(),
            pattern: "snake_case".into(),
            confidence: 0.9,
            evidence: vec!["src/main.rs".into()],
            detected_at: "2026-04-03 12:00:00".into(),
        };
        let json = serde_json::to_string(&dna).expect("serialize DomainDna");
        let restored: DomainDna = serde_json::from_str(&json).expect("deserialize DomainDna");
        assert_eq!(dna, restored);
    }

    #[test]
    fn test_perception_serde() {
        let p = Perception {
            id: "p1".into(),
            kind: PerceptionKind::Error,
            data: "compilation failed".into(),
            severity: Severity::Error,
            project: Some("forge".into()),
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        };
        let json = serde_json::to_string(&p).expect("serialize Perception");
        let restored: Perception = serde_json::from_str(&json).expect("deserialize Perception");
        assert_eq!(p, restored);
    }

    #[test]
    fn test_declared_serde() {
        let d = Declared {
            id: "dk1".into(),
            source: "CLAUDE.md".into(),
            path: Some("/project/CLAUDE.md".into()),
            content: "Use snake_case".into(),
            hash: "abc123".into(),
            project: Some("forge".into()),
            ingested_at: "2026-04-03 12:00:00".into(),
        };
        let json = serde_json::to_string(&d).expect("serialize Declared");
        let restored: Declared = serde_json::from_str(&json).expect("deserialize Declared");
        assert_eq!(d, restored);
    }

    #[test]
    fn test_identity_facet_serde() {
        let f = IdentityFacet {
            id: "if1".into(),
            agent: "forge".into(),
            facet: "role".into(),
            description: "memory system".into(),
            strength: 0.8,
            source: "declared".into(),
            active: true,
            created_at: "2026-04-03 12:00:00".into(),
        };
        let json = serde_json::to_string(&f).expect("serialize IdentityFacet");
        let restored: IdentityFacet = serde_json::from_str(&json).expect("deserialize IdentityFacet");
        assert_eq!(f, restored);
    }

    #[test]
    fn test_disposition_serde() {
        let d = Disposition {
            id: "dp1".into(),
            agent: "forge".into(),
            disposition_trait: DispositionTrait::Caution,
            domain: Some("security".into()),
            value: 0.7,
            trend: Trend::Rising,
            updated_at: "2026-04-03 12:00:00".into(),
            evidence: vec!["always runs clippy".into()],
        };
        let json = serde_json::to_string(&d).expect("serialize Disposition");
        let restored: Disposition = serde_json::from_str(&json).expect("deserialize Disposition");
        assert_eq!(d, restored);
    }

    #[test]
    fn test_tool_kind_variants() {
        let kinds = [ToolKind::Cli, ToolKind::Mcp, ToolKind::Builtin, ToolKind::Plugin];
        for kind in &kinds {
            let json = serde_json::to_string(kind).expect("serialize ToolKind");
            let restored: ToolKind = serde_json::from_str(&json).expect("deserialize ToolKind");
            assert_eq!(*kind, restored);
        }
    }

    #[test]
    fn test_severity_variants() {
        let severities = [Severity::Debug, Severity::Info, Severity::Warning, Severity::Error, Severity::Critical];
        for sev in &severities {
            let json = serde_json::to_string(sev).expect("serialize Severity");
            let restored: Severity = serde_json::from_str(&json).expect("deserialize Severity");
            assert_eq!(*sev, restored);
        }
    }

    #[test]
    fn test_trend_variants() {
        let trends = [Trend::Rising, Trend::Stable, Trend::Falling];
        for trend in &trends {
            let json = serde_json::to_string(trend).expect("serialize Trend");
            let restored: Trend = serde_json::from_str(&json).expect("deserialize Trend");
            assert_eq!(*trend, restored);
        }
    }
}
