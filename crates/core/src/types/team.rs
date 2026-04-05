use serde::{Deserialize, Serialize};

// ── Agent Template ──

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub agent_type: String,
    pub organization_id: String,
    pub system_context: String,
    /// Pre-configured identity facets as JSON array
    pub identity_facets: String,
    /// Scoped config overrides as JSON object
    pub config_overrides: String,
    /// Knowledge domains as JSON array
    pub knowledge_domains: String,
    pub decision_style: String,
    pub created_at: String,
    pub updated_at: String,
}

// ── Agent Status ──

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Thinking,
    Responding,
    InMeeting,
    Error,
    Retired,
}

impl AgentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Thinking => "thinking",
            Self::Responding => "responding",
            Self::InMeeting => "in_meeting",
            Self::Error => "error",
            Self::Retired => "retired",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "idle" => Self::Idle,
            "thinking" => Self::Thinking,
            "responding" => Self::Responding,
            "in_meeting" => Self::InMeeting,
            "error" => Self::Error,
            "retired" => Self::Retired,
            _ => Self::Idle,
        }
    }
}

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Team Type ──

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TeamType {
    Human,
    Agent,
    Mixed,
}

impl TeamType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Mixed => "mixed",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "agent" => Self::Agent,
            "mixed" => Self::Mixed,
            _ => Self::Human,
        }
    }
}

// ── Meeting Status ──

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Open,
    Collecting,
    TimedOut,
    Synthesizing,
    Decided,
    Closed,
}

impl MeetingStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Collecting => "collecting",
            Self::TimedOut => "timed_out",
            Self::Synthesizing => "synthesizing",
            Self::Decided => "decided",
            Self::Closed => "closed",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "open" => Self::Open,
            "collecting" => Self::Collecting,
            "timed_out" => Self::TimedOut,
            "synthesizing" => Self::Synthesizing,
            "decided" => Self::Decided,
            "closed" => Self::Closed,
            _ => Self::Open,
        }
    }
}

// ── Meeting Participant Status ──

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantStatus {
    Pending,
    Thinking,
    Responded,
    TimedOut,
    Acknowledged,
}

impl ParticipantStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Thinking => "thinking",
            Self::Responded => "responded",
            Self::TimedOut => "timed_out",
            Self::Acknowledged => "acknowledged",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "thinking" => Self::Thinking,
            "responded" => Self::Responded,
            "timed_out" => Self::TimedOut,
            "acknowledged" => Self::Acknowledged,
            _ => Self::Pending,
        }
    }
}

// ── Meeting ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meeting {
    pub id: String,
    pub team_id: String,
    pub topic: String,
    pub context: Option<String>,
    pub status: String,
    pub orchestrator_session_id: String,
    pub synthesis: Option<String>,
    pub decision: Option<String>,
    pub decision_memory_id: Option<String>,
    pub created_at: String,
    pub decided_at: Option<String>,
}

// ── Meeting Participant ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingParticipant {
    pub id: String,
    pub meeting_id: String,
    pub session_id: String,
    pub template_name: String,
    pub status: String,
    pub response: Option<String>,
    pub responded_at: Option<String>,
    pub confidence: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_status_roundtrip() {
        for status in [AgentStatus::Idle, AgentStatus::Thinking, AgentStatus::Responding,
                       AgentStatus::InMeeting, AgentStatus::Error, AgentStatus::Retired] {
            assert_eq!(AgentStatus::from_str_lossy(status.as_str()), status);
        }
    }

    #[test]
    fn test_meeting_status_roundtrip() {
        for status in [MeetingStatus::Open, MeetingStatus::Collecting, MeetingStatus::TimedOut,
                       MeetingStatus::Synthesizing, MeetingStatus::Decided, MeetingStatus::Closed] {
            assert_eq!(MeetingStatus::from_str_lossy(status.as_str()), status);
        }
    }

    #[test]
    fn test_agent_template_serde() {
        let t = AgentTemplate {
            id: "t1".into(), name: "CTO".into(), description: "Tech lead".into(),
            agent_type: "claude-code".into(), organization_id: "default".into(),
            system_context: "You are the CTO".into(),
            identity_facets: "[]".into(), config_overrides: "{}".into(),
            knowledge_domains: "[]".into(), decision_style: "analytical".into(),
            created_at: "2026-04-05".into(), updated_at: "2026-04-05".into(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let restored: AgentTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(t, restored);
    }
}
