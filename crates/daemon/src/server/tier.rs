//! License tier gating — checks whether the current license tier allows a given request.
//!
//! Tier hierarchy: Free < Pro < Team < Enterprise.
//! Each tier includes all features of the tiers below it.
//! Unknown requests default to Free (fail-open).

use forge_core::protocol::Request;

/// Subscription tier levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Free = 0,
    Pro = 1,
    Team = 2,
    Enterprise = 3,
}

impl Tier {
    /// Parse a tier string (case-insensitive). Unknown values default to Free.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "pro" => Tier::Pro,
            "team" => Tier::Team,
            "enterprise" => Tier::Enterprise,
            _ => Tier::Free,
        }
    }

    /// Returns true if this tier allows the given feature.
    pub fn allows(&self, feature: Feature) -> bool {
        *self >= feature.required_tier()
    }

    /// Human-readable tier name for error messages.
    pub fn name(&self) -> &'static str {
        match self {
            Tier::Free => "Free",
            Tier::Pro => "Pro",
            Tier::Team => "Team",
            Tier::Enterprise => "Enterprise",
        }
    }

    /// Pricing info for upgrade messages.
    pub fn pricing(&self) -> &'static str {
        match self {
            Tier::Free => "free",
            Tier::Pro => "$12/mo",
            Tier::Team => "$19/seat",
            Tier::Enterprise => "contact sales",
        }
    }
}

/// Features that may be tier-gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    // === Free tier ===
    BasicRecall,
    BasicRemember,
    Health,
    CompileContext,

    // === Pro tier ===
    DeviceSync,
    DecisionAutoWrite,

    // === Team tier ===
    TeamWorkspace,
    OrgInit,
    MeetingMinutes,
    AgentWorkspace,
    TeamMessaging,

    // === Enterprise tier ===
    CentralizedWorkspace,
    RbacDirectories,
    AuditTrail,
}

impl Feature {
    /// The minimum tier required for this feature.
    pub fn required_tier(&self) -> Tier {
        match self {
            // Free
            Feature::BasicRecall
            | Feature::BasicRemember
            | Feature::Health
            | Feature::CompileContext => Tier::Free,

            // Pro
            Feature::DeviceSync | Feature::DecisionAutoWrite => Tier::Pro,

            // Team
            Feature::TeamWorkspace
            | Feature::OrgInit
            | Feature::MeetingMinutes
            | Feature::AgentWorkspace
            | Feature::TeamMessaging => Tier::Team,

            // Enterprise
            Feature::CentralizedWorkspace
            | Feature::RbacDirectories
            | Feature::AuditTrail => Tier::Enterprise,
        }
    }

    /// Human-readable feature name.
    pub fn name(&self) -> &'static str {
        match self {
            Feature::BasicRecall => "basic recall",
            Feature::BasicRemember => "basic remember",
            Feature::Health => "health",
            Feature::CompileContext => "compile context",
            Feature::DeviceSync => "multi-device sync",
            Feature::DecisionAutoWrite => "decision auto-write",
            Feature::TeamWorkspace => "team workspace",
            Feature::OrgInit => "organization management",
            Feature::MeetingMinutes => "meeting minutes",
            Feature::AgentWorkspace => "agent workspace",
            Feature::TeamMessaging => "team messaging",
            Feature::CentralizedWorkspace => "centralized workspace",
            Feature::RbacDirectories => "RBAC directories",
            Feature::AuditTrail => "audit trail",
        }
    }
}

/// Map a Request variant to the Feature it requires.
/// Returns None for requests that are always allowed (Free tier / no gating needed).
fn request_to_feature(request: &Request) -> Option<Feature> {
    match request {
        // === Always allowed (Free tier) ===
        Request::Remember { .. }
        | Request::Recall { .. }
        | Request::Forget { .. }
        | Request::Supersede { .. }
        | Request::Health
        | Request::HealthByProject
        | Request::Status
        | Request::Doctor
        | Request::Export { .. }
        | Request::Import { .. }
        | Request::IngestClaude
        | Request::IngestDeclared { .. }
        | Request::Backfill { .. }
        | Request::Subscribe { .. }
        | Request::GuardrailsCheck { .. }
        | Request::PreBashCheck { .. }
        | Request::PostBashCheck { .. }
        | Request::PostEditCheck { .. }
        | Request::BlastRadius { .. }
        | Request::RegisterSession { .. }
        | Request::SessionHeartbeat { .. }
        | Request::ContextRefresh { .. }
        | Request::CompletionCheck { .. }
        | Request::TaskCompletionCheck { .. }
        | Request::ContextStats { .. }
        | Request::EndSession { .. }
        | Request::Sessions { .. }
        | Request::CleanupSessions { .. }
        | Request::LspStatus
        | Request::Verify { .. }
        | Request::GetDiagnostics { .. }
        | Request::StorePlatform { .. }
        | Request::ListPlatform
        | Request::StoreTool { .. }
        | Request::ListTools
        | Request::StorePerception { .. }
        | Request::ListPerceptions { .. }
        | Request::ConsumePerceptions { .. }
        | Request::StoreIdentity { .. }
        | Request::ListIdentity { .. }
        | Request::DeactivateIdentity { .. }
        | Request::ListDisposition { .. }
        | Request::ManasHealth { .. }
        | Request::CompileContext { .. }
        | Request::CompileContextTrace { .. }
        | Request::HlcBackfill
        | Request::StoreEvaluation { .. }
        | Request::Bootstrap { .. }
        | Request::ForceConsolidate
        | Request::ForceExtract
        | Request::ExtractWithProvider { .. }
        | Request::GetConfig
        | Request::SetConfig { .. }
        | Request::GetStats { .. }
        | Request::GetGraphData { .. }
        | Request::BatchRecall { .. }
        | Request::ListEntities { .. }
        | Request::GetEffectiveConfig { .. }
        | Request::SetScopedConfig { .. }
        | Request::DeleteScopedConfig { .. }
        | Request::ListScopedConfig { .. }
        | Request::DetectReality { .. }
        | Request::CrossEngineQuery { .. }
        | Request::FileMemoryMap { .. }
        | Request::CodeSearch { .. }
        | Request::ListRealities { .. }
        | Request::ForceIndex
        | Request::ListNotifications { .. }
        | Request::AckNotification { .. }
        | Request::DismissNotification { .. }
        | Request::ActOnNotification { .. }
        | Request::HealingStatus
        | Request::HealingRun
        | Request::HealingLog { .. }
        | Request::BackfillProject
        | Request::CleanupMemory
        | Request::SetCurrentTask { .. }
        | Request::Shutdown
        | Request::LicenseStatus
        | Request::SetLicense { .. }
        | Request::WorkspaceStatus => None,

        // === Pro tier: sync operations ===
        Request::SyncExport { .. }
        | Request::SyncImport { .. }
        | Request::SyncConflicts
        | Request::SyncResolve { .. } => Some(Feature::DeviceSync),

        // === Pro tier: A2A permissions (managed inter-session messaging) ===
        Request::GrantPermission { .. }
        | Request::RevokePermission { .. }
        | Request::ListPermissions => Some(Feature::DecisionAutoWrite),

        // === Team tier: organization hierarchy ===
        Request::CreateOrganization { .. }
        | Request::ListOrganizations
        | Request::CreateOrgFromTemplate { .. } => Some(Feature::OrgInit),

        // === Team tier: team management ===
        Request::CreateTeam { .. }
        | Request::ListTeamMembers { .. }
        | Request::SetTeamOrchestrator { .. }
        | Request::TeamStatus { .. }
        | Request::TeamTree { .. } => Some(Feature::TeamWorkspace),

        // === Team tier: team messaging ===
        Request::TeamSend { .. }
        | Request::SessionSend { .. }
        | Request::SessionRespond { .. }
        | Request::SessionMessages { .. }
        | Request::SessionAck { .. } => Some(Feature::TeamMessaging),

        // === Team tier: meetings ===
        Request::CreateMeeting { .. }
        | Request::MeetingStatus { .. }
        | Request::MeetingResponses { .. }
        | Request::MeetingSynthesize { .. }
        | Request::MeetingDecide { .. }
        | Request::ListMeetings { .. }
        | Request::MeetingTranscript { .. }
        | Request::RecordMeetingResponse { .. }
        | Request::MeetingVote { .. }
        | Request::MeetingResult { .. } => Some(Feature::MeetingMinutes),

        // === Team tier: agent teams ===
        Request::CreateAgentTemplate { .. }
        | Request::ListAgentTemplates { .. }
        | Request::GetAgentTemplate { .. }
        | Request::DeleteAgentTemplate { .. }
        | Request::UpdateAgentTemplate { .. }
        | Request::SpawnAgent { .. }
        | Request::ListAgents { .. }
        | Request::UpdateAgentStatus { .. }
        | Request::RetireAgent { .. } => Some(Feature::AgentWorkspace),

        // === Team tier: workspace init ===
        Request::WorkspaceInit { .. } => Some(Feature::TeamWorkspace),

        // === Team tier: team orchestration ===
        Request::RunTeam { .. }
        | Request::StopTeam { .. }
        | Request::ListTeamTemplates => Some(Feature::TeamWorkspace),

        // === Free tier: skills registry (browsing is free, install is free) ===
        Request::SkillsList { .. }
        | Request::SkillsInstall { .. }
        | Request::SkillsUninstall { .. }
        | Request::SkillsInfo { .. }
        | Request::SkillsRefresh
        | Request::RoutingStats => None,
    }
}

/// Check if the current license tier allows a given request.
///
/// Returns `Ok(())` if allowed, or `Err(upgrade_message)` with a human-readable
/// message indicating which tier is required and how to upgrade.
pub fn check_tier(tier_str: &str, request: &Request) -> Result<(), String> {
    let current_tier = Tier::parse(tier_str);

    let feature = match request_to_feature(request) {
        Some(f) => f,
        None => return Ok(()), // No gating — always allowed
    };

    if current_tier.allows(feature) {
        Ok(())
    } else {
        let required = feature.required_tier();
        Err(format!(
            "This feature ({}) requires {} tier ({}). Upgrade at https://forge.bhairavi.tech/pricing",
            feature.name(),
            required.name(),
            required.pricing(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::protocol::Request;

    #[test]
    fn test_tier_ordering() {
        assert!(Tier::Free < Tier::Pro);
        assert!(Tier::Pro < Tier::Team);
        assert!(Tier::Team < Tier::Enterprise);
    }

    #[test]
    fn test_tier_from_str() {
        assert_eq!(Tier::parse("free"), Tier::Free);
        assert_eq!(Tier::parse("pro"), Tier::Pro);
        assert_eq!(Tier::parse("team"), Tier::Team);
        assert_eq!(Tier::parse("enterprise"), Tier::Enterprise);
        assert_eq!(Tier::parse("FREE"), Tier::Free);
        assert_eq!(Tier::parse("Pro"), Tier::Pro);
        assert_eq!(Tier::parse("unknown"), Tier::Free);
        assert_eq!(Tier::parse(""), Tier::Free);
    }

    #[test]
    fn test_tier_allows() {
        assert!(Tier::Free.allows(Feature::BasicRecall));
        assert!(Tier::Free.allows(Feature::Health));
        assert!(!Tier::Free.allows(Feature::DeviceSync));
        assert!(!Tier::Free.allows(Feature::TeamWorkspace));
        assert!(!Tier::Free.allows(Feature::CentralizedWorkspace));

        assert!(Tier::Pro.allows(Feature::BasicRecall));
        assert!(Tier::Pro.allows(Feature::DeviceSync));
        assert!(!Tier::Pro.allows(Feature::TeamWorkspace));

        assert!(Tier::Team.allows(Feature::BasicRecall));
        assert!(Tier::Team.allows(Feature::DeviceSync));
        assert!(Tier::Team.allows(Feature::TeamWorkspace));
        assert!(Tier::Team.allows(Feature::MeetingMinutes));
        assert!(!Tier::Team.allows(Feature::CentralizedWorkspace));

        assert!(Tier::Enterprise.allows(Feature::BasicRecall));
        assert!(Tier::Enterprise.allows(Feature::DeviceSync));
        assert!(Tier::Enterprise.allows(Feature::TeamWorkspace));
        assert!(Tier::Enterprise.allows(Feature::CentralizedWorkspace));
    }

    #[test]
    fn test_free_tier_allows_basic_ops() {
        let ops = vec![
            Request::Recall {
                query: "test".into(),
                memory_type: None,
                project: None,
                limit: None,
                layer: None,
                since: None,
            },
            Request::Remember {
                memory_type: forge_core::types::MemoryType::Decision,
                title: "t".into(),
                content: "c".into(),
                confidence: None,
                tags: None,
                project: None,
                metadata: None,
            },
            Request::Health,
            Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
                excluded_layers: None,
                session_id: None,
                focus: None,
            },
        ];
        for op in ops {
            assert!(check_tier("free", &op).is_ok(), "Free tier should allow {:?}", op);
        }
    }

    #[test]
    fn test_free_tier_blocks_team_features() {
        let blocked = vec![
            Request::CreateOrganization {
                name: "test".into(),
                description: None,
            },
            Request::CreateTeam {
                name: "eng".into(),
                team_type: None,
                purpose: None,
                organization_id: None,
            },
            Request::WorkspaceInit {
                org_name: "test".into(),
                template: None,
            },
        ];
        for op in blocked {
            let result = check_tier("free", &op);
            assert!(result.is_err(), "Free tier should block {:?}", op);
            let msg = result.unwrap_err();
            assert!(msg.contains("Team tier"), "Error should mention Team tier: {msg}");
            assert!(msg.contains("$19/seat"), "Error should mention pricing: {msg}");
            assert!(msg.contains("https://forge.bhairavi.tech/pricing"), "Error should contain upgrade URL: {msg}");
        }
    }

    #[test]
    fn test_free_tier_blocks_pro_features() {
        let blocked = vec![
            Request::SyncExport {
                project: None,
                since: None,
            },
            Request::SyncImport {
                lines: vec![],
            },
        ];
        for op in blocked {
            let result = check_tier("free", &op);
            assert!(result.is_err(), "Free tier should block {:?}", op);
            let msg = result.unwrap_err();
            assert!(msg.contains("Pro tier"), "Error should mention Pro tier: {msg}");
            assert!(msg.contains("$12/mo"), "Error should mention pricing: {msg}");
        }
    }

    #[test]
    fn test_team_tier_allows_org_init() {
        let op = Request::CreateOrganization {
            name: "test".into(),
            description: None,
        };
        assert!(check_tier("team", &op).is_ok());
    }

    #[test]
    fn test_team_tier_allows_meetings() {
        let op = Request::ListMeetings {
            team_id: None,
            status: None,
            limit: None,
        };
        assert!(check_tier("team", &op).is_ok());
    }

    #[test]
    fn test_enterprise_allows_centralized_workspace() {
        // Enterprise features are gated by the Feature enum but currently
        // no Request variants map directly to CentralizedWorkspace.
        // This test verifies the tier comparison works.
        assert!(Tier::Enterprise.allows(Feature::CentralizedWorkspace));
        assert!(!Tier::Team.allows(Feature::CentralizedWorkspace));
    }

    #[test]
    fn test_check_tier_mapping_exhaustive() {
        // Verify that every Request variant is classified (the match is exhaustive).
        // This test just needs to compile — if a new Request variant is added
        // without updating request_to_feature, this will cause a compilation error
        // because the match is non-wildcard on all known variants.
        //
        // We test a representative sample from each tier group:
        let free_ops: Vec<Request> = vec![
            Request::Health,
            Request::Doctor,
            Request::LspStatus,
            Request::Shutdown,
        ];
        for op in &free_ops {
            assert!(check_tier("free", op).is_ok());
        }

        // Pro ops blocked on free
        assert!(check_tier("free", &Request::SyncConflicts).is_err());

        // Team ops blocked on pro
        assert!(check_tier("pro", &Request::ListOrganizations).is_err());
        assert!(check_tier("pro", &Request::ListMeetings { team_id: None, status: None, limit: None }).is_err());
    }

    #[test]
    fn test_pro_tier_allows_sync() {
        assert!(check_tier("pro", &Request::SyncExport { project: None, since: None }).is_ok());
        assert!(check_tier("pro", &Request::SyncConflicts).is_ok());
    }

    #[test]
    fn test_upgrade_message_format() {
        let result = check_tier("free", &Request::CreateOrganization {
            name: "test".into(),
            description: None,
        });
        let msg = result.unwrap_err();
        assert!(msg.starts_with("This feature (organization management) requires Team tier ($19/seat)."));
    }
}
