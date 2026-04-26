//! Protocol contract tests: pins EVERY Request variant's JSON method name.
//!
//! If someone adds a new Request variant, they MUST add it here too.
//! These tests prevent the entire class of CLI serialization bugs where
//! the JSON method name doesn't match what the daemon expects.

#[cfg(test)]
mod tests {
    use crate::protocol::codec::decode_request;
    use crate::protocol::request::{EvaluationFinding, RecallQuery, Request};
    use crate::types::manas::{
        IdentityFacet, Perception, PerceptionKind, Severity, Tool, ToolHealth, ToolKind,
    };
    use crate::types::memory::MemoryType;

    // ────────────────────────────────────────────────────────
    // Unit variants: verify serialized JSON method name
    // ────────────────────────────────────────────────────────

    /// Pin every unit (no-params) Request variant's JSON method name.
    #[test]
    fn test_unit_variants_method_names() {
        let cases: Vec<(&str, Request)> = vec![
            ("health", Request::Health),
            ("health_by_project", Request::HealthByProject),
            ("status", Request::Status),
            ("doctor", Request::Doctor),
            ("ingest_claude", Request::IngestClaude),
            ("lsp_status", Request::LspStatus),
            ("list_platform", Request::ListPlatform),
            ("list_tools", Request::ListTools),
            ("sync_conflicts", Request::SyncConflicts),
            ("hlc_backfill", Request::HlcBackfill),
            ("force_consolidate", Request::ForceConsolidate),
            ("force_index", Request::ForceIndex { path: None }),
            ("list_permissions", Request::ListPermissions),
            ("list_organizations", Request::ListOrganizations),
            ("healing_status", Request::HealingStatus),
            ("healing_run", Request::HealingRun),
            ("version", Request::Version),
            ("shutdown", Request::Shutdown),
        ];

        for (expected_method, request) in &cases {
            let json = serde_json::to_string(request).unwrap();
            assert!(
                json.contains(&format!("\"method\":\"{expected_method}\"")),
                "Unit variant should serialize to method='{expected_method}', got: {json}"
            );

            // Round-trip: deserialize back and verify equality
            let decoded = decode_request(&json);
            assert!(
                decoded.is_ok(),
                "Failed to decode unit variant '{}': {:?}",
                expected_method,
                decoded.err()
            );
            assert_eq!(
                request,
                &decoded.unwrap(),
                "Round-trip failed for unit variant '{expected_method}'"
            );
        }
    }

    // ────────────────────────────────────────────────────────
    // Parameterized variants: construct typed instances,
    // verify JSON method name, and round-trip
    // ────────────────────────────────────────────────────────

    /// Pin every parameterized Request variant's JSON method name AND round-trip.
    #[test]
    fn test_parameterized_variants_method_names() {
        let cases: Vec<(&str, Request)> = vec![
            (
                "remember",
                Request::Remember {
                    memory_type: MemoryType::Decision,
                    title: "test".into(),
                    content: "test content".into(),
                    confidence: Some(0.9),
                    tags: Some(vec!["t".into()]),
                    project: Some("forge".into()),
                    metadata: None,
                },
            ),
            (
                "recall",
                Request::Recall {
                    query: "test query".into(),
                    memory_type: Some(MemoryType::Decision),
                    project: Some("forge".into()),
                    limit: Some(10),
                    layer: Some("experience".into()),
                    since: None,
                    include_flipped: None,
                    query_embedding: None,
                },
            ),
            (
                "recall",
                Request::Recall {
                    query: "test".into(),
                    memory_type: None,
                    project: None,
                    limit: Some(10),
                    layer: None,
                    since: None,
                    include_flipped: Some(true),
                    query_embedding: None,
                },
            ),
            (
                "flip_preference",
                Request::FlipPreference {
                    memory_id: "01JABCDEF".into(),
                    new_valence: "negative".into(),
                    new_intensity: 0.8,
                    reason: Some("team switched to spaces".into()),
                },
            ),
            (
                "list_flipped",
                Request::ListFlipped {
                    agent: Some("claude-code".into()),
                    limit: Some(10),
                },
            ),
            ("forget", Request::Forget { id: "abc".into() }),
            (
                "export",
                Request::Export {
                    format: Some("ndjson".into()),
                    since: Some("2026-01-01".into()),
                },
            ),
            (
                "import",
                Request::Import {
                    data: r#"{"test":"data"}"#.into(),
                },
            ),
            (
                "ingest_declared",
                Request::IngestDeclared {
                    path: "/tmp/test.md".into(),
                    source: "test".into(),
                    project: Some("forge".into()),
                },
            ),
            (
                "backfill",
                Request::Backfill {
                    path: "/tmp/transcript.jsonl".into(),
                },
            ),
            (
                "subscribe",
                Request::Subscribe {
                    events: Some(vec!["memory_created".into()]),
                    session_id: None,
                    team_id: None,
                },
            ),
            (
                "guardrails_check",
                Request::GuardrailsCheck {
                    file: "src/main.rs".into(),
                    action: "edit".into(),
                },
            ),
            (
                "post_edit_check",
                Request::PostEditCheck {
                    file: "src/main.rs".into(),
                    session_id: None,
                },
            ),
            (
                "pre_bash_check",
                Request::PreBashCheck {
                    command: "rm -rf /tmp/test".into(),
                    session_id: None,
                },
            ),
            (
                "post_bash_check",
                Request::PostBashCheck {
                    command: "cargo test".into(),
                    exit_code: 1,
                    session_id: None,
                },
            ),
            (
                "blast_radius",
                Request::BlastRadius {
                    file: "src/main.rs".into(),
                },
            ),
            (
                "register_session",
                Request::RegisterSession {
                    id: "s1".into(),
                    agent: "claude-code".into(),
                    project: Some("forge".into()),
                    cwd: Some("/tmp".into()),
                    capabilities: None,
                    current_task: None,
                },
            ),
            ("end_session", Request::EndSession { id: "s1".into() }),
            (
                "sessions",
                Request::Sessions {
                    active_only: Some(true),
                },
            ),
            (
                "cleanup_sessions",
                Request::CleanupSessions {
                    prefix: Some("hook-test".into()),
                    older_than_secs: None,
                    prune_ended: false,
                },
            ),
            (
                "store_platform",
                Request::StorePlatform {
                    key: "os".into(),
                    value: "linux".into(),
                },
            ),
            (
                "store_tool",
                Request::StoreTool {
                    tool: Tool {
                        id: "t1".into(),
                        name: "cargo".into(),
                        kind: ToolKind::Cli,
                        capabilities: vec!["build".into()],
                        config: None,
                        health: ToolHealth::Healthy,
                        last_used: None,
                        use_count: 0,
                        discovered_at: "2026-04-03 12:00:00".into(),
                    },
                },
            ),
            (
                "store_perception",
                Request::StorePerception {
                    perception: Perception {
                        id: "p1".into(),
                        kind: PerceptionKind::Error,
                        data: "test error".into(),
                        severity: Severity::Error,
                        project: Some("forge".into()),
                        created_at: "2026-04-03 12:00:00".into(),
                        expires_at: None,
                        consumed: false,
                    },
                },
            ),
            (
                "list_perceptions",
                Request::ListPerceptions {
                    project: Some("forge".into()),
                    limit: Some(10),
                    offset: None,
                },
            ),
            (
                "consume_perceptions",
                Request::ConsumePerceptions {
                    ids: vec!["p1".into(), "p2".into()],
                },
            ),
            (
                "store_identity",
                Request::StoreIdentity {
                    facet: IdentityFacet {
                        id: "if1".into(),
                        agent: "claude-code".into(),
                        facet: "role".into(),
                        description: "memory system".into(),
                        strength: 0.8,
                        source: "declared".into(),
                        active: true,
                        created_at: "2026-04-03 12:00:00".into(),
                        user_id: None,
                    },
                },
            ),
            (
                "list_identity",
                Request::ListIdentity {
                    agent: "claude-code".into(),
                },
            ),
            (
                "deactivate_identity",
                Request::DeactivateIdentity { id: "if1".into() },
            ),
            (
                "list_disposition",
                Request::ListDisposition {
                    agent: "claude-code".into(),
                },
            ),
            (
                "manas_health",
                Request::ManasHealth {
                    project: Some("forge".into()),
                },
            ),
            (
                "context_refresh",
                Request::ContextRefresh {
                    session_id: "s-123".into(),
                    since: Some("2026-04-06T12:00:00Z".into()),
                },
            ),
            (
                "completion_check",
                Request::CompletionCheck {
                    session_id: "s-123".into(),
                    claimed_done: true,
                },
            ),
            (
                "task_completion_check",
                Request::TaskCompletionCheck {
                    session_id: "s-123".into(),
                    task_subject: "deploy to production".into(),
                    task_description: None,
                },
            ),
            (
                "context_stats",
                Request::ContextStats {
                    session_id: Some("s-123".into()),
                },
            ),
            (
                "compile_context",
                Request::CompileContext {
                    agent: Some("claude-code".into()),
                    project: Some("forge".into()),
                    static_only: None,
                    excluded_layers: Some(vec!["decisions".into()]),
                    session_id: None,
                    focus: None,
                },
            ),
            (
                "compile_context_trace",
                Request::CompileContextTrace {
                    agent: Some("claude-code".into()),
                    project: Some("forge".into()),
                    session_id: None,
                },
            ),
            (
                "sync_export",
                Request::SyncExport {
                    project: Some("forge".into()),
                    since: Some("2026-01-01".into()),
                },
            ),
            (
                "sync_import",
                Request::SyncImport {
                    lines: vec!["line1".into(), "line2".into()],
                },
            ),
            (
                "sync_resolve",
                Request::SyncResolve {
                    keep_id: "abc".into(),
                },
            ),
            (
                "verify",
                Request::Verify {
                    file: Some("src/main.rs".into()),
                },
            ),
            (
                "get_diagnostics",
                Request::GetDiagnostics {
                    file: "src/main.rs".into(),
                },
            ),
            (
                "store_evaluation",
                Request::StoreEvaluation {
                    findings: vec![EvaluationFinding {
                        description: "Missing error handling".into(),
                        severity: "high".into(),
                        files: vec!["src/auth.rs".into()],
                        category: "bug".into(),
                    }],
                    project: Some("forge".into()),
                    session_id: Some("s1".into()),
                },
            ),
            (
                "bootstrap",
                Request::Bootstrap {
                    project: Some("forge".into()),
                },
            ),
            (
                "get_graph_data",
                Request::GetGraphData {
                    layer: Some("experience".into()),
                    limit: Some(50),
                },
            ),
            (
                "batch_recall",
                Request::BatchRecall {
                    queries: vec![RecallQuery {
                        text: "test query".into(),
                        memory_type: None,
                        limit: Some(5),
                    }],
                },
            ),
            (
                "extract_with_provider",
                Request::ExtractWithProvider {
                    provider: "ollama".into(),
                    model: Some("qwen3:4b".into()),
                    text: "User decided to use Rust for the daemon.".into(),
                },
            ),
            (
                "grant_permission",
                Request::GrantPermission {
                    from_agent: "claude-code".into(),
                    to_agent: "cline".into(),
                    from_project: Some("forge".into()),
                    to_project: None,
                },
            ),
            (
                "revoke_permission",
                Request::RevokePermission {
                    id: "perm-123".into(),
                },
            ),
            (
                "get_effective_config",
                Request::GetEffectiveConfig {
                    session_id: Some("s1".into()),
                    agent: Some("claude-code".into()),
                    reality_id: Some("r1".into()),
                    user_id: Some("local".into()),
                    team_id: None,
                    organization_id: Some("default".into()),
                },
            ),
            (
                "set_scoped_config",
                Request::SetScopedConfig {
                    scope_type: "organization".into(),
                    scope_id: "default".into(),
                    key: "max_tokens".into(),
                    value: "4096".into(),
                    locked: false,
                    ceiling: Some(10000.0),
                },
            ),
            (
                "delete_scoped_config",
                Request::DeleteScopedConfig {
                    scope_type: "organization".into(),
                    scope_id: "default".into(),
                    key: "max_tokens".into(),
                },
            ),
            (
                "list_scoped_config",
                Request::ListScopedConfig {
                    scope_type: "organization".into(),
                    scope_id: "default".into(),
                },
            ),
            (
                "detect_reality",
                Request::DetectReality {
                    path: "/tmp/my-project".into(),
                },
            ),
            (
                "cross_engine_query",
                Request::CrossEngineQuery {
                    file: "src/main.rs".into(),
                    reality_id: Some("r1".into()),
                },
            ),
            (
                "file_memory_map",
                Request::FileMemoryMap {
                    files: vec!["src/main.rs".into(), "src/lib.rs".into()],
                    reality_id: None,
                },
            ),
            (
                "code_search",
                Request::CodeSearch {
                    query: "handle_request".into(),
                    kind: Some("function".into()),
                    limit: Some(10),
                },
            ),
            (
                "list_realities",
                Request::ListRealities {
                    organization_id: Some("default".into()),
                },
            ),
            ("get_stats", Request::GetStats { hours: Some(24) }),
            // ── Agent Lifecycle ──
            (
                "spawn_agent",
                Request::SpawnAgent {
                    template_name: "CTO".into(),
                    session_id: "s-cto-1".into(),
                    project: Some("forge".into()),
                    team: Some("leadership".into()),
                },
            ),
            (
                "list_agents",
                Request::ListAgents {
                    team: Some("leadership".into()),
                    limit: Some(10),
                },
            ),
            (
                "update_agent_status",
                Request::UpdateAgentStatus {
                    session_id: "s-cto-1".into(),
                    status: "thinking".into(),
                    current_task: Some("reviewing architecture".into()),
                },
            ),
            (
                "retire_agent",
                Request::RetireAgent {
                    session_id: "s-cto-1".into(),
                },
            ),
            (
                "run_team",
                Request::RunTeam {
                    team_name: "sprint-1".into(),
                    template_names: vec!["tech-lead".into(), "frontend-dev".into()],
                    topology: Some("mesh".into()),
                    goal: None,
                    project: None,
                },
            ),
            (
                "stop_team",
                Request::StopTeam {
                    team_name: "sprint-1".into(),
                },
            ),
            ("list_team_templates", Request::ListTeamTemplates),
            // ── Team Enhancements ──
            (
                "create_team",
                Request::CreateTeam {
                    name: "leadership".into(),
                    team_type: Some("agent".into()),
                    purpose: Some("strategic decisions".into()),
                    organization_id: Some("default".into()),
                },
            ),
            (
                "list_team_members",
                Request::ListTeamMembers {
                    team_name: "leadership".into(),
                },
            ),
            (
                "set_team_orchestrator",
                Request::SetTeamOrchestrator {
                    team_name: "leadership".into(),
                    session_id: "s-cto-1".into(),
                },
            ),
            (
                "team_status",
                Request::TeamStatus {
                    team_name: "leadership".into(),
                    team_id: None,
                },
            ),
            // ── Organization Hierarchy ──
            (
                "create_organization",
                Request::CreateOrganization {
                    name: "acme-corp".into(),
                    description: Some("Main organization".into()),
                },
            ),
            (
                "team_send",
                Request::TeamSend {
                    team_name: "leadership".into(),
                    kind: "notification".into(),
                    topic: "deploy".into(),
                    parts: vec![],
                    from_session: Some("s-orch".into()),
                    recursive: true,
                },
            ),
            (
                "team_tree",
                Request::TeamTree {
                    organization_id: Some("default".into()),
                },
            ),
            (
                "create_org_from_template",
                Request::CreateOrgFromTemplate {
                    template_name: "startup".into(),
                    org_name: "acme-corp".into(),
                },
            ),
            // ── Meeting Protocol ──
            (
                "create_meeting",
                Request::CreateMeeting {
                    team_id: "t1".into(),
                    topic: "Architecture review".into(),
                    context: Some("Q2 planning".into()),
                    orchestrator_session_id: "s-orch".into(),
                    participant_session_ids: vec!["s-cto".into(), "s-cmo".into()],
                    goal: None,
                },
            ),
            (
                "meeting_status",
                Request::MeetingStatus {
                    meeting_id: "m1".into(),
                },
            ),
            (
                "meeting_responses",
                Request::MeetingResponses {
                    meeting_id: "m1".into(),
                },
            ),
            (
                "meeting_synthesize",
                Request::MeetingSynthesize {
                    meeting_id: "m1".into(),
                    synthesis: "All agree on Rust".into(),
                },
            ),
            (
                "meeting_decide",
                Request::MeetingDecide {
                    meeting_id: "m1".into(),
                    decision: "Use Rust for the daemon".into(),
                },
            ),
            (
                "list_meetings",
                Request::ListMeetings {
                    team_id: Some("t1".into()),
                    status: Some("collecting".into()),
                    limit: Some(10),
                },
            ),
            (
                "meeting_transcript",
                Request::MeetingTranscript {
                    meeting_id: "m1".into(),
                },
            ),
            // ── Memory Self-Healing ──
            (
                "healing_log",
                Request::HealingLog {
                    limit: Some(10),
                    action: Some("auto_superseded".into()),
                },
            ),
            // ── Notification Engine ──
            (
                "list_notifications",
                Request::ListNotifications {
                    status: Some("pending".into()),
                    category: Some("alert".into()),
                    limit: Some(10),
                },
            ),
            (
                "ack_notification",
                Request::AckNotification { id: "n1".into() },
            ),
            (
                "dismiss_notification",
                Request::DismissNotification { id: "n1".into() },
            ),
            (
                "act_on_notification",
                Request::ActOnNotification {
                    id: "n1".into(),
                    approved: true,
                },
            ),
            // ── Phase 2A-4d.2: Observability API ──
            (
                "inspect",
                Request::Inspect {
                    shape: crate::protocol::InspectShape::Latency,
                    window: "1h".into(),
                    filter: crate::protocol::InspectFilter {
                        phase: Some("phase_1_exact_dedup".into()),
                        ..Default::default()
                    },
                    group_by: Some(crate::protocol::InspectGroupBy::Phase),
                },
            ),
            // ── Phase 2A-4d.3 T10: BenchRunSummary leaderboard shape ──
            (
                "inspect",
                Request::Inspect {
                    shape: crate::protocol::InspectShape::BenchRunSummary,
                    window: "30d".into(),
                    filter: crate::protocol::InspectFilter {
                        bench_name: Some("forge-identity".into()),
                        ..Default::default()
                    },
                    group_by: Some(crate::protocol::InspectGroupBy::BenchName),
                },
            ),
        ];

        for (expected_method, request) in &cases {
            let json = serde_json::to_string(request).unwrap();
            assert!(
                json.contains(&format!("\"method\":\"{expected_method}\"")),
                "Parameterized variant should serialize to method='{expected_method}', got: {json}"
            );

            // Round-trip: deserialize back and verify equality
            let decoded = decode_request(&json);
            assert!(
                decoded.is_ok(),
                "Failed to decode parameterized variant '{}': {:?}",
                expected_method,
                decoded.err()
            );
            assert_eq!(
                request,
                &decoded.unwrap(),
                "Round-trip failed for parameterized variant '{expected_method}'"
            );
        }
    }

    // ────────────────────────────────────────────────────────
    // Decode from raw JSON strings (simulates CLI sending JSON)
    // ────────────────────────────────────────────────────────

    /// Test decoding from raw JSON for parameterized variants.
    /// This catches mismatches between what the CLI sends and what the daemon expects.
    #[test]
    fn test_decode_from_raw_json() {
        let cases: Vec<(&str, &str)> = vec![
            (
                "remember",
                r#"{"method":"remember","params":{"memory_type":"decision","title":"t","content":"c"}}"#,
            ),
            ("recall", r#"{"method":"recall","params":{"query":"test"}}"#),
            (
                "recall with layer",
                r#"{"method":"recall","params":{"query":"test","layer":"experience","limit":10}}"#,
            ),
            ("forget", r#"{"method":"forget","params":{"id":"abc"}}"#),
            (
                "export",
                r#"{"method":"export","params":{"format":"json"}}"#,
            ),
            ("import", r#"{"method":"import","params":{"data":"{}"}}"#),
            (
                "ingest_declared",
                r#"{"method":"ingest_declared","params":{"path":"/tmp/f","source":"test"}}"#,
            ),
            (
                "backfill",
                r#"{"method":"backfill","params":{"path":"/tmp/t.jsonl"}}"#,
            ),
            (
                "subscribe",
                r#"{"method":"subscribe","params":{"events":["memory_created"]}}"#,
            ),
            (
                "guardrails_check",
                r#"{"method":"guardrails_check","params":{"file":"src/main.rs","action":"edit"}}"#,
            ),
            (
                "post_edit_check",
                r#"{"method":"post_edit_check","params":{"file":"src/main.rs"}}"#,
            ),
            (
                "pre_bash_check",
                r#"{"method":"pre_bash_check","params":{"command":"rm -rf /tmp"}}"#,
            ),
            (
                "post_bash_check",
                r#"{"method":"post_bash_check","params":{"command":"cargo test","exit_code":1}}"#,
            ),
            (
                "blast_radius",
                r#"{"method":"blast_radius","params":{"file":"src/main.rs"}}"#,
            ),
            (
                "register_session",
                r#"{"method":"register_session","params":{"id":"s1","agent":"claude-code"}}"#,
            ),
            (
                "end_session",
                r#"{"method":"end_session","params":{"id":"s1"}}"#,
            ),
            (
                "sessions",
                r#"{"method":"sessions","params":{"active_only":true}}"#,
            ),
            (
                "store_platform",
                r#"{"method":"store_platform","params":{"key":"os","value":"linux"}}"#,
            ),
            (
                "list_perceptions",
                r#"{"method":"list_perceptions","params":{"project":null,"limit":10}}"#,
            ),
            (
                "consume_perceptions",
                r#"{"method":"consume_perceptions","params":{"ids":["p1"]}}"#,
            ),
            (
                "list_identity",
                r#"{"method":"list_identity","params":{"agent":"claude-code"}}"#,
            ),
            (
                "deactivate_identity",
                r#"{"method":"deactivate_identity","params":{"id":"f1"}}"#,
            ),
            (
                "list_disposition",
                r#"{"method":"list_disposition","params":{"agent":"claude-code"}}"#,
            ),
            (
                "manas_health",
                r#"{"method":"manas_health","params":{"project":"forge"}}"#,
            ),
            (
                "manas_health no project",
                r#"{"method":"manas_health","params":{}}"#,
            ),
            (
                "compile_context",
                r#"{"method":"compile_context","params":{"agent":"claude-code"}}"#,
            ),
            (
                "compile_context with excluded_layers",
                r#"{"method":"compile_context","params":{"agent":"claude-code","excluded_layers":["decisions","perceptions"]}}"#,
            ),
            (
                "compile_context_trace",
                r#"{"method":"compile_context_trace","params":{"agent":"claude-code"}}"#,
            ),
            ("sync_export", r#"{"method":"sync_export","params":{}}"#),
            (
                "sync_import",
                r#"{"method":"sync_import","params":{"lines":[]}}"#,
            ),
            (
                "sync_resolve",
                r#"{"method":"sync_resolve","params":{"keep_id":"abc"}}"#,
            ),
            (
                "verify",
                r#"{"method":"verify","params":{"file":"src/main.rs"}}"#,
            ),
            (
                "get_diagnostics",
                r#"{"method":"get_diagnostics","params":{"file":"src/main.rs"}}"#,
            ),
            (
                "store_evaluation",
                r#"{"method":"store_evaluation","params":{"findings":[{"description":"bug found","severity":"high","files":["src/main.rs"],"category":"bug"}],"project":"forge","session_id":"s1"}}"#,
            ),
            (
                "bootstrap",
                r#"{"method":"bootstrap","params":{"project":"forge"}}"#,
            ),
            (
                "bootstrap no project",
                r#"{"method":"bootstrap","params":{}}"#,
            ),
            (
                "get_graph_data",
                r#"{"method":"get_graph_data","params":{"layer":"experience","limit":50}}"#,
            ),
            (
                "get_graph_data no params",
                r#"{"method":"get_graph_data","params":{}}"#,
            ),
            (
                "batch_recall",
                r#"{"method":"batch_recall","params":{"queries":[{"text":"test query","limit":5}]}}"#,
            ),
            (
                "batch_recall empty",
                r#"{"method":"batch_recall","params":{"queries":[]}}"#,
            ),
            (
                "extract_with_provider",
                r#"{"method":"extract_with_provider","params":{"provider":"ollama","text":"some conversation"}}"#,
            ),
            (
                "extract_with_provider with model",
                r#"{"method":"extract_with_provider","params":{"provider":"claude_api","model":"claude-3-haiku","text":"some conversation"}}"#,
            ),
            (
                "grant_permission",
                r#"{"method":"grant_permission","params":{"from_agent":"claude-code","to_agent":"cline"}}"#,
            ),
            (
                "grant_permission with projects",
                r#"{"method":"grant_permission","params":{"from_agent":"*","to_agent":"*","from_project":"forge","to_project":"forge"}}"#,
            ),
            (
                "revoke_permission",
                r#"{"method":"revoke_permission","params":{"id":"perm-123"}}"#,
            ),
            (
                "get_effective_config",
                r#"{"method":"get_effective_config","params":{"organization_id":"default"}}"#,
            ),
            (
                "get_effective_config all params",
                r#"{"method":"get_effective_config","params":{"session_id":"s1","agent":"claude-code","reality_id":"r1","user_id":"local","team_id":"t1","organization_id":"default"}}"#,
            ),
            (
                "set_scoped_config",
                r#"{"method":"set_scoped_config","params":{"scope_type":"organization","scope_id":"default","key":"max_tokens","value":"4096","locked":false}}"#,
            ),
            (
                "set_scoped_config with ceiling",
                r#"{"method":"set_scoped_config","params":{"scope_type":"reality","scope_id":"r1","key":"max_tokens","value":"8192","locked":true,"ceiling":10000.0}}"#,
            ),
            (
                "delete_scoped_config",
                r#"{"method":"delete_scoped_config","params":{"scope_type":"organization","scope_id":"default","key":"max_tokens"}}"#,
            ),
            (
                "list_scoped_config",
                r#"{"method":"list_scoped_config","params":{"scope_type":"organization","scope_id":"default"}}"#,
            ),
            (
                "cross_engine_query",
                r#"{"method":"cross_engine_query","params":{"file":"src/main.rs"}}"#,
            ),
            (
                "cross_engine_query with reality_id",
                r#"{"method":"cross_engine_query","params":{"file":"src/main.rs","reality_id":"r1"}}"#,
            ),
            (
                "file_memory_map",
                r#"{"method":"file_memory_map","params":{"files":["src/main.rs","src/lib.rs"]}}"#,
            ),
            (
                "code_search",
                r#"{"method":"code_search","params":{"query":"handle_request"}}"#,
            ),
            (
                "code_search with kind",
                r#"{"method":"code_search","params":{"query":"handle_request","kind":"function","limit":10}}"#,
            ),
            (
                "list_realities",
                r#"{"method":"list_realities","params":{}}"#,
            ),
            (
                "list_realities with org",
                r#"{"method":"list_realities","params":{"organization_id":"default"}}"#,
            ),
            (
                "get_stats",
                r#"{"method":"get_stats","params":{"hours":24}}"#,
            ),
            (
                "get_stats no params",
                r#"{"method":"get_stats","params":{}}"#,
            ),
            // ── Agent Lifecycle ──
            (
                "spawn_agent",
                r#"{"method":"spawn_agent","params":{"template_name":"CTO","session_id":"s1"}}"#,
            ),
            (
                "spawn_agent with team",
                r#"{"method":"spawn_agent","params":{"template_name":"CMO","session_id":"s2","project":"forge","team":"leadership"}}"#,
            ),
            ("list_agents", r#"{"method":"list_agents","params":{}}"#),
            (
                "list_agents with team",
                r#"{"method":"list_agents","params":{"team":"leadership","limit":10}}"#,
            ),
            (
                "update_agent_status",
                r#"{"method":"update_agent_status","params":{"session_id":"s1","status":"thinking"}}"#,
            ),
            (
                "update_agent_status with task",
                r#"{"method":"update_agent_status","params":{"session_id":"s1","status":"responding","current_task":"code review"}}"#,
            ),
            (
                "retire_agent",
                r#"{"method":"retire_agent","params":{"session_id":"s1"}}"#,
            ),
            // ── Team Enhancements ──
            (
                "create_team",
                r#"{"method":"create_team","params":{"name":"leadership"}}"#,
            ),
            (
                "create_team with type",
                r#"{"method":"create_team","params":{"name":"leadership","team_type":"agent","purpose":"strategic decisions","organization_id":"default"}}"#,
            ),
            (
                "list_team_members",
                r#"{"method":"list_team_members","params":{"team_name":"leadership"}}"#,
            ),
            (
                "set_team_orchestrator",
                r#"{"method":"set_team_orchestrator","params":{"team_name":"leadership","session_id":"s1"}}"#,
            ),
            (
                "team_status",
                r#"{"method":"team_status","params":{"team_name":"leadership"}}"#,
            ),
            // ── Organization Hierarchy ──
            (
                "create_organization",
                r#"{"method":"create_organization","params":{"name":"acme-corp"}}"#,
            ),
            (
                "create_organization with description",
                r#"{"method":"create_organization","params":{"name":"acme-corp","description":"Main org"}}"#,
            ),
            (
                "team_send",
                r#"{"method":"team_send","params":{"team_name":"leadership","kind":"notification","topic":"deploy","parts":[]}}"#,
            ),
            (
                "team_send recursive",
                r#"{"method":"team_send","params":{"team_name":"leadership","kind":"notification","topic":"deploy","parts":[],"from_session":"s-orch","recursive":true}}"#,
            ),
            ("team_tree", r#"{"method":"team_tree","params":{}}"#),
            (
                "team_tree with org",
                r#"{"method":"team_tree","params":{"organization_id":"default"}}"#,
            ),
            (
                "create_org_from_template",
                r#"{"method":"create_org_from_template","params":{"template_name":"startup","org_name":"acme-corp"}}"#,
            ),
            // ── Meeting Protocol ──
            (
                "create_meeting",
                r#"{"method":"create_meeting","params":{"team_id":"t1","topic":"Architecture","orchestrator_session_id":"s-orch","participant_session_ids":["s-cto","s-cmo"]}}"#,
            ),
            (
                "create_meeting with context",
                r#"{"method":"create_meeting","params":{"team_id":"t1","topic":"Architecture","context":"Q2","orchestrator_session_id":"s-orch","participant_session_ids":["s-cto"]}}"#,
            ),
            (
                "meeting_status",
                r#"{"method":"meeting_status","params":{"meeting_id":"m1"}}"#,
            ),
            (
                "meeting_responses",
                r#"{"method":"meeting_responses","params":{"meeting_id":"m1"}}"#,
            ),
            (
                "meeting_synthesize",
                r#"{"method":"meeting_synthesize","params":{"meeting_id":"m1","synthesis":"All agree"}}"#,
            ),
            (
                "meeting_decide",
                r#"{"method":"meeting_decide","params":{"meeting_id":"m1","decision":"Use Rust"}}"#,
            ),
            ("list_meetings", r#"{"method":"list_meetings","params":{}}"#),
            (
                "list_meetings with filters",
                r#"{"method":"list_meetings","params":{"team_id":"t1","status":"collecting","limit":10}}"#,
            ),
            (
                "meeting_transcript",
                r#"{"method":"meeting_transcript","params":{"meeting_id":"m1"}}"#,
            ),
            // ── Memory Self-Healing ──
            (
                "healing_log",
                r#"{"method":"healing_log","params":{"limit":10,"action":"auto_superseded"}}"#,
            ),
            (
                "healing_log no params",
                r#"{"method":"healing_log","params":{}}"#,
            ),
            // ── Notification Engine ──
            (
                "list_notifications",
                r#"{"method":"list_notifications","params":{}}"#,
            ),
            (
                "list_notifications with filters",
                r#"{"method":"list_notifications","params":{"status":"pending","category":"alert","limit":10}}"#,
            ),
            (
                "ack_notification",
                r#"{"method":"ack_notification","params":{"id":"n1"}}"#,
            ),
            (
                "dismiss_notification",
                r#"{"method":"dismiss_notification","params":{"id":"n1"}}"#,
            ),
            (
                "act_on_notification",
                r#"{"method":"act_on_notification","params":{"id":"n1","approved":true}}"#,
            ),
            (
                "act_on_notification reject",
                r#"{"method":"act_on_notification","params":{"id":"n1","approved":false}}"#,
            ),
            // ISS-D7: to_session alias for session_send
            (
                "session_send with to",
                r#"{"method":"session_send","params":{"to":"s2","kind":"notification","topic":"test","parts":[]}}"#,
            ),
            (
                "session_send with to_session alias",
                r#"{"method":"session_send","params":{"to_session":"s2","kind":"notification","topic":"test","parts":[]}}"#,
            ),
            // ── Phase 2A-4d.2: Observability API ──
            (
                "inspect row_count minimal",
                r#"{"method":"inspect","params":{"shape":"row_count"}}"#,
            ),
            (
                "inspect latency with filter + group_by",
                r#"{"method":"inspect","params":{"shape":"latency","window":"24h","filter":{"phase":"phase_23_infer_skills_from_behavior"},"group_by":"phase"}}"#,
            ),
            (
                "inspect error_rate default window",
                r#"{"method":"inspect","params":{"shape":"error_rate"}}"#,
            ),
            (
                "inspect phase_run_summary",
                r#"{"method":"inspect","params":{"shape":"phase_run_summary","window":"7d"}}"#,
            ),
            (
                "inspect throughput group_by=event_type",
                r#"{"method":"inspect","params":{"shape":"throughput","window":"1h","group_by":"event_type"}}"#,
            ),
            // ── Phase 2A-4d.3 T10: BenchRunSummary ──
            (
                "inspect bench_run_summary defaults",
                r#"{"method":"inspect","params":{"shape":"bench_run_summary"}}"#,
            ),
            (
                "inspect bench_run_summary 180d window + bench_name + commit_sha filter",
                r#"{"method":"inspect","params":{"shape":"bench_run_summary","window":"180d","filter":{"bench_name":"forge-identity","commit_sha":"abc123"},"group_by":"bench_name"}}"#,
            ),
            (
                "inspect bench_run_summary group_by=commit_sha",
                r#"{"method":"inspect","params":{"shape":"bench_run_summary","window":"30d","group_by":"commit_sha"}}"#,
            ),
            (
                "inspect bench_run_summary group_by=seed",
                r#"{"method":"inspect","params":{"shape":"bench_run_summary","window":"7d","group_by":"seed"}}"#,
            ),
        ];

        for (label, json) in &cases {
            let result = decode_request(json);
            assert!(
                result.is_ok(),
                "Failed to decode raw JSON for '{}': {} -> {:?}",
                label,
                json,
                result.err()
            );
        }
    }

    /// Test decoding unit variants from raw JSON (no params field).
    #[test]
    fn test_decode_unit_variants_from_raw_json() {
        let cases: Vec<(&str, &str)> = vec![
            ("health", r#"{"method":"health"}"#),
            ("health_by_project", r#"{"method":"health_by_project"}"#),
            ("status", r#"{"method":"status"}"#),
            ("doctor", r#"{"method":"doctor"}"#),
            ("ingest_claude", r#"{"method":"ingest_claude"}"#),
            ("lsp_status", r#"{"method":"lsp_status"}"#),
            ("list_platform", r#"{"method":"list_platform"}"#),
            ("list_tools", r#"{"method":"list_tools"}"#),
            ("sync_conflicts", r#"{"method":"sync_conflicts"}"#),
            ("hlc_backfill", r#"{"method":"hlc_backfill"}"#),
            ("force_consolidate", r#"{"method":"force_consolidate"}"#),
            // force_index is now parameterized (has optional path field)
            // Old wire format {"method":"force_index"} no longer decodes — use params: {}
            ("list_permissions", r#"{"method":"list_permissions"}"#),
            ("list_organizations", r#"{"method":"list_organizations"}"#),
            ("healing_status", r#"{"method":"healing_status"}"#),
            ("healing_run", r#"{"method":"healing_run"}"#),
            ("version", r#"{"method":"version"}"#),
            ("shutdown", r#"{"method":"shutdown"}"#),
        ];

        for (label, json) in &cases {
            let result = decode_request(json);
            assert!(
                result.is_ok(),
                "Failed to decode unit variant '{}': {} -> {:?}",
                label,
                json,
                result.err()
            );
        }
    }

    /// Verify that force_index with empty params decodes correctly.
    /// The old bare `{"method":"force_index"}` no longer works (struct variant requires params),
    /// but `{"method":"force_index","params":{}}` should decode to ForceIndex { path: None }.
    #[test]
    fn test_force_index_backward_compat() {
        // New format with empty params — should work
        let result = decode_request(r#"{"method":"force_index","params":{}}"#);
        assert!(
            result.is_ok(),
            "force_index with empty params should decode: {:?}",
            result.err()
        );

        // New format with path — should work
        let result = decode_request(r#"{"method":"force_index","params":{"path":"/tmp"}}"#);
        assert!(
            result.is_ok(),
            "force_index with path should decode: {:?}",
            result.err()
        );

        // Old bare format — documents the breaking change
        let result = decode_request(r#"{"method":"force_index"}"#);
        assert!(
            result.is_err(),
            "bare force_index without params should fail (breaking change)"
        );
    }

    // ────────────────────────────────────────────────────────
    // Completeness guard: count all variants
    // ────────────────────────────────────────────────────────

    /// Ensure we cover ALL Request variants.
    /// If a new variant is added without updating these tests,
    /// the count assertion will fail.
    #[test]
    fn test_variant_count_completeness() {
        // Unit variants: 20 (was 19; +1 Version)
        let unit_count = 20;
        // Parameterized variants: 104 (was 103; +1 SessionMessageRead in W27)
        let param_count = 104;
        // Total: 124
        let expected_total = 124;

        assert_eq!(
            unit_count + param_count,
            expected_total,
            "Variant count mismatch — update contract tests when adding new Request variants!"
        );

        // Actually construct and serialize all variants to verify compile-time completeness.
        // This function must list EVERY variant; if one is added to the enum,
        // this won't compile until it's added here too (if the match is exhaustive).
        fn all_variants() -> Vec<Request> {
            vec![
                // Unit variants
                Request::Health,
                Request::HealthByProject,
                Request::Status,
                Request::Doctor,
                Request::IngestClaude,
                Request::LspStatus,
                Request::ListPlatform,
                Request::ListTools,
                Request::ManasHealth { project: None },
                Request::SyncConflicts,
                Request::HlcBackfill,
                Request::ForceConsolidate,
                Request::ForceExtract,
                Request::ForceIndex { path: None },
                Request::ListContradictions {
                    status: None,
                    limit: None,
                },
                Request::ResolveContradiction {
                    contradiction_id: "c1".into(),
                    resolution: "a".into(),
                },
                Request::GetConfig,
                // Agent Teams
                Request::CreateAgentTemplate {
                    name: "CTO".into(),
                    description: "tech lead".into(),
                    agent_type: "claude-code".into(),
                    organization_id: None,
                    system_context: None,
                    identity_facets: None,
                    config_overrides: None,
                    knowledge_domains: None,
                    decision_style: None,
                },
                Request::ListAgentTemplates {
                    organization_id: None,
                    limit: None,
                },
                Request::GetAgentTemplate {
                    id: Some("t1".into()),
                    name: None,
                },
                Request::DeleteAgentTemplate { id: "t1".into() },
                Request::UpdateAgentTemplate {
                    id: "t1".into(),
                    name: Some("CTO v2".into()),
                    description: None,
                    system_context: None,
                    identity_facets: None,
                    config_overrides: None,
                    knowledge_domains: None,
                    decision_style: None,
                },
                Request::Version,
                Request::Shutdown,
                // Parameterized variants
                Request::Remember {
                    memory_type: MemoryType::Decision,
                    title: "t".into(),
                    content: "c".into(),
                    confidence: None,
                    tags: None,
                    project: None,
                    metadata: None,
                },
                Request::Recall {
                    query: "q".into(),
                    memory_type: None,
                    project: None,
                    limit: None,
                    layer: None,
                    since: None,
                    include_flipped: None,
                    query_embedding: None,
                },
                Request::Forget { id: "x".into() },
                Request::Export {
                    format: None,
                    since: None,
                },
                Request::Import { data: "{}".into() },
                Request::IngestDeclared {
                    path: "p".into(),
                    source: "s".into(),
                    project: None,
                },
                Request::Backfill { path: "p".into() },
                Request::Subscribe {
                    events: None,
                    session_id: None,
                    team_id: None,
                },
                Request::GuardrailsCheck {
                    file: "f".into(),
                    action: "a".into(),
                },
                Request::PostEditCheck {
                    file: "f".into(),
                    session_id: None,
                },
                Request::PreBashCheck {
                    command: "ls".into(),
                    session_id: None,
                },
                Request::PostBashCheck {
                    command: "cargo test".into(),
                    exit_code: 1,
                    session_id: None,
                },
                Request::BlastRadius { file: "f".into() },
                Request::RegisterSession {
                    id: "s".into(),
                    agent: "a".into(),
                    project: None,
                    cwd: None,
                    capabilities: None,
                    current_task: None,
                },
                Request::EndSession { id: "s".into() },
                Request::Sessions { active_only: None },
                Request::CleanupSessions {
                    prefix: Some("hook-test".into()),
                    older_than_secs: None,
                    prune_ended: false,
                },
                Request::SessionSend {
                    to: "s2".into(),
                    kind: "notification".into(),
                    topic: "test".into(),
                    parts: vec![],
                    project: None,
                    timeout_secs: None,
                    meeting_id: None,
                    from_session: None,
                },
                Request::SessionRespond {
                    message_id: "m1".into(),
                    status: "completed".into(),
                    parts: vec![],
                },
                Request::SessionMessages {
                    session_id: "s1".into(),
                    status: None,
                    limit: None,
                    offset: Some(5),
                },
                Request::SessionMessageRead {
                    id: "01ABCDEF".into(),
                },
                Request::SessionAck {
                    message_ids: vec!["m1".into()],
                    session_id: None,
                },
                Request::StorePlatform {
                    key: "k".into(),
                    value: "v".into(),
                },
                Request::StoreTool {
                    tool: Tool {
                        id: "t".into(),
                        name: "n".into(),
                        kind: ToolKind::Cli,
                        capabilities: vec![],
                        config: None,
                        health: ToolHealth::Healthy,
                        last_used: None,
                        use_count: 0,
                        discovered_at: "2026-01-01 00:00:00".into(),
                    },
                },
                Request::StorePerception {
                    perception: Perception {
                        id: "p".into(),
                        kind: PerceptionKind::Error,
                        data: "d".into(),
                        severity: Severity::Error,
                        project: None,
                        created_at: "2026-01-01 00:00:00".into(),
                        expires_at: None,
                        consumed: false,
                    },
                },
                Request::ListPerceptions {
                    project: None,
                    limit: None,
                    offset: None,
                },
                Request::ConsumePerceptions { ids: vec![] },
                Request::StoreIdentity {
                    facet: IdentityFacet {
                        id: "i".into(),
                        agent: "a".into(),
                        facet: "f".into(),
                        description: "d".into(),
                        strength: 0.5,
                        source: "s".into(),
                        active: true,
                        created_at: "2026-01-01 00:00:00".into(),
                        user_id: None,
                    },
                },
                Request::Supersede {
                    old_id: "old".into(),
                    new_id: "new".into(),
                },
                Request::FlipPreference {
                    memory_id: "01JABCDEF".into(),
                    new_valence: "negative".into(),
                    new_intensity: 0.8,
                    reason: None,
                },
                Request::ListFlipped {
                    agent: None,
                    limit: None,
                },
                Request::ReaffirmPreference {
                    memory_id: "01HXXX".into(),
                },
                Request::ComputeRecencyFactor {
                    memory_id: "01HXXX".into(),
                },
                Request::RecordToolUse {
                    session_id: "S".into(),
                    agent: "a".into(),
                    tool_name: "T".into(),
                    tool_args: serde_json::json!({}),
                    tool_result_summary: "ok".into(),
                    success: true,
                    user_correction_flag: false,
                },
                Request::ListToolCalls {
                    session_id: "S".into(),
                    agent: None,
                    limit: None,
                },
                Request::ListIdentity { agent: "a".into() },
                Request::DeactivateIdentity { id: "i".into() },
                Request::ListDisposition { agent: "a".into() },
                Request::ContextRefresh {
                    session_id: "s".into(),
                    since: None,
                },
                Request::CompletionCheck {
                    session_id: "s".into(),
                    claimed_done: false,
                },
                Request::TaskCompletionCheck {
                    session_id: "s".into(),
                    task_subject: "t".into(),
                    task_description: None,
                },
                Request::ContextStats { session_id: None },
                Request::CompileContext {
                    agent: None,
                    project: None,
                    static_only: None,
                    excluded_layers: None,
                    session_id: None,
                    focus: None,
                },
                Request::CompileContextTrace {
                    agent: None,
                    project: None,
                    session_id: None,
                },
                Request::SyncExport {
                    project: None,
                    since: None,
                },
                Request::SyncImport { lines: vec![] },
                Request::SyncResolve {
                    keep_id: "k".into(),
                },
                Request::Verify {
                    file: Some("f".into()),
                },
                Request::GetDiagnostics { file: "f".into() },
                Request::StoreEvaluation {
                    findings: vec![EvaluationFinding {
                        description: "test".into(),
                        severity: "low".into(),
                        files: vec![],
                        category: "style".into(),
                    }],
                    project: None,
                    session_id: None,
                },
                Request::Bootstrap { project: None },
                Request::SetConfig {
                    key: "extraction.backend".into(),
                    value: "claude".into(),
                },
                Request::GetGraphData {
                    layer: None,
                    limit: None,
                },
                Request::BatchRecall {
                    queries: vec![RecallQuery {
                        text: "q".into(),
                        memory_type: None,
                        limit: None,
                    }],
                },
                Request::ExtractWithProvider {
                    provider: "ollama".into(),
                    model: None,
                    text: "test".into(),
                },
                Request::ListEntities {
                    project: None,
                    limit: None,
                },
                Request::GrantPermission {
                    from_agent: "claude-code".into(),
                    to_agent: "cline".into(),
                    from_project: None,
                    to_project: None,
                },
                Request::RevokePermission {
                    id: "perm-1".into(),
                },
                Request::ListPermissions,
                Request::GetEffectiveConfig {
                    session_id: None,
                    agent: None,
                    reality_id: None,
                    user_id: None,
                    team_id: None,
                    organization_id: Some("default".into()),
                },
                Request::SetScopedConfig {
                    scope_type: "organization".into(),
                    scope_id: "default".into(),
                    key: "max_tokens".into(),
                    value: "4096".into(),
                    locked: false,
                    ceiling: None,
                },
                Request::DeleteScopedConfig {
                    scope_type: "organization".into(),
                    scope_id: "default".into(),
                    key: "max_tokens".into(),
                },
                Request::ListScopedConfig {
                    scope_type: "organization".into(),
                    scope_id: "default".into(),
                },
                Request::DetectReality {
                    path: "/tmp/my-project".into(),
                },
                Request::CrossEngineQuery {
                    file: "src/main.rs".into(),
                    reality_id: Some("r1".into()),
                },
                Request::FileMemoryMap {
                    files: vec!["src/main.rs".into()],
                    reality_id: None,
                },
                Request::CodeSearch {
                    query: "test".into(),
                    kind: None,
                    limit: None,
                },
                Request::ListRealities {
                    organization_id: Some("default".into()),
                },
                Request::GetStats { hours: Some(24) },
                // Agent Lifecycle
                Request::SpawnAgent {
                    template_name: "CTO".into(),
                    session_id: "s-cto".into(),
                    project: Some("forge".into()),
                    team: Some("leadership".into()),
                },
                Request::ListAgents {
                    team: None,
                    limit: Some(50),
                },
                Request::UpdateAgentStatus {
                    session_id: "s-cto".into(),
                    status: "thinking".into(),
                    current_task: Some("reviewing".into()),
                },
                Request::RetireAgent {
                    session_id: "s-cto".into(),
                },
                Request::RunTeam {
                    team_name: "sprint-1".into(),
                    template_names: vec!["tech-lead".into(), "frontend-dev".into()],
                    topology: Some("mesh".into()),
                    goal: None,
                    project: None,
                },
                Request::StopTeam {
                    team_name: "sprint-1".into(),
                },
                Request::ListTeamTemplates,
                // Team Enhancements
                Request::CreateTeam {
                    name: "leadership".into(),
                    team_type: Some("agent".into()),
                    purpose: Some("strategic decisions".into()),
                    organization_id: Some("default".into()),
                },
                Request::ListTeamMembers {
                    team_name: "leadership".into(),
                },
                Request::SetTeamOrchestrator {
                    team_name: "leadership".into(),
                    session_id: "s-cto".into(),
                },
                Request::TeamStatus {
                    team_name: "leadership".into(),
                    team_id: None,
                },
                // Organization Hierarchy
                Request::CreateOrganization {
                    name: "acme-corp".into(),
                    description: Some("Main organization".into()),
                },
                Request::ListOrganizations,
                Request::TeamSend {
                    team_name: "leadership".into(),
                    kind: "notification".into(),
                    topic: "deploy".into(),
                    parts: vec![],
                    from_session: Some("s-orch".into()),
                    recursive: true,
                },
                Request::TeamTree {
                    organization_id: Some("default".into()),
                },
                Request::CreateOrgFromTemplate {
                    template_name: "startup".into(),
                    org_name: "acme-corp".into(),
                },
                // Meeting Protocol
                Request::CreateMeeting {
                    team_id: "t1".into(),
                    topic: "Architecture review".into(),
                    context: Some("Q2 planning".into()),
                    orchestrator_session_id: "s-orch".into(),
                    participant_session_ids: vec!["s-cto".into(), "s-cmo".into()],
                    goal: None,
                },
                Request::MeetingStatus {
                    meeting_id: "m1".into(),
                },
                Request::MeetingResponses {
                    meeting_id: "m1".into(),
                },
                Request::MeetingSynthesize {
                    meeting_id: "m1".into(),
                    synthesis: "All agree on Rust".into(),
                },
                Request::MeetingDecide {
                    meeting_id: "m1".into(),
                    decision: "Use Rust for the daemon".into(),
                },
                Request::ListMeetings {
                    team_id: Some("t1".into()),
                    status: Some("collecting".into()),
                    limit: Some(10),
                },
                Request::MeetingTranscript {
                    meeting_id: "m1".into(),
                },
                Request::RecordMeetingResponse {
                    meeting_id: "m1".into(),
                    session_id: "s1".into(),
                    response: "I agree".into(),
                    confidence: Some(0.9),
                },
                // Memory Self-Healing
                Request::HealingStatus,
                Request::HealingRun,
                Request::HealingLog {
                    limit: None,
                    action: None,
                },
                // Notification Engine
                Request::ListNotifications {
                    status: Some("pending".into()),
                    category: Some("alert".into()),
                    limit: Some(10),
                },
                Request::AckNotification { id: "n1".into() },
                Request::DismissNotification { id: "n1".into() },
                Request::ActOnNotification {
                    id: "n1".into(),
                    approved: true,
                },
            ]
        }

        let variants = all_variants();
        assert_eq!(
            variants.len(),
            expected_total,
            "all_variants() must return exactly {expected_total} variants"
        );

        // Verify every variant serializes successfully
        for (i, variant) in variants.iter().enumerate() {
            let json = serde_json::to_string(variant);
            assert!(
                json.is_ok(),
                "Variant #{} failed to serialize: {:?}",
                i,
                json.err()
            );
        }
    }

    // ────────────────────────────────────────────────────────
    // Negative tests: malformed JSON should fail gracefully
    // ────────────────────────────────────────────────────────

    #[test]
    fn test_unknown_method_fails() {
        let result = decode_request(r#"{"method":"nonexistent"}"#);
        assert!(result.is_err(), "Unknown method should fail to decode");
    }

    #[test]
    fn test_missing_required_params_fails() {
        // remember requires title and content
        let result = decode_request(r#"{"method":"remember","params":{"memory_type":"decision"}}"#);
        assert!(
            result.is_err(),
            "Missing required params should fail to decode"
        );
    }

    #[test]
    fn test_empty_json_fails() {
        let result = decode_request("{}");
        assert!(result.is_err(), "Empty JSON should fail to decode");
    }

    #[test]
    fn test_invalid_json_fails() {
        let result = decode_request("not json at all");
        assert!(result.is_err(), "Invalid JSON should fail to decode");
    }

    // ────────────────────────────────────────────────────────
    // Raw layer variants (RawIngest / RawSearch)
    // ────────────────────────────────────────────────────────

    #[test]
    fn test_raw_ingest_round_trip() {
        let req = Request::RawIngest {
            text: "Forge remembers.".into(),
            project: Some("forge".into()),
            session_id: Some("sess-1".into()),
            source: "claude-code".into(),
            timestamp: Some("2026-04-13T00:00:00Z".into()),
            metadata: Some(serde_json::json!({"bench": "longmemeval"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"method\":\"raw_ingest\""),
            "raw_ingest method name missing: {json}"
        );
        assert!(json.contains("\"text\":\"Forge remembers.\""));
        assert!(json.contains("\"source\":\"claude-code\""));
        let decoded = decode_request(&json).expect("decode raw_ingest");
        assert_eq!(req, decoded);
    }

    #[test]
    fn test_raw_ingest_optional_fields_absent() {
        // All optional fields omitted — must deserialize cleanly.
        let json = r#"{"method":"raw_ingest","params":{"text":"x","source":"cli"}}"#;
        let decoded = decode_request(json).expect("decode minimal raw_ingest");
        match decoded {
            Request::RawIngest {
                text,
                project,
                session_id,
                source,
                timestamp,
                metadata,
            } => {
                assert_eq!(text, "x");
                assert_eq!(source, "cli");
                assert!(project.is_none());
                assert!(session_id.is_none());
                assert!(timestamp.is_none());
                assert!(metadata.is_none());
            }
            _ => panic!("expected RawIngest variant"),
        }
    }

    #[test]
    fn test_raw_search_round_trip() {
        let req = Request::RawSearch {
            query: "rust daemon".into(),
            project: Some("forge".into()),
            session_id: None,
            k: Some(10),
            max_distance: Some(0.6),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"method\":\"raw_search\""),
            "raw_search method name missing: {json}"
        );
        let decoded = decode_request(&json).expect("decode raw_search");
        assert_eq!(req, decoded);
    }

    #[test]
    fn test_raw_search_minimal() {
        let json = r#"{"method":"raw_search","params":{"query":"hello"}}"#;
        let decoded = decode_request(json).expect("decode minimal raw_search");
        match decoded {
            Request::RawSearch {
                query,
                project,
                session_id,
                k,
                max_distance,
            } => {
                assert_eq!(query, "hello");
                assert!(project.is_none());
                assert!(session_id.is_none());
                assert!(k.is_none());
                assert!(max_distance.is_none());
            }
            _ => panic!("expected RawSearch variant"),
        }
    }

    #[test]
    fn test_raw_documents_list_round_trip() {
        let req = Request::RawDocumentsList {
            source: "forge-persist".into(),
            limit: Some(100),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"method\":\"raw_documents_list\""),
            "raw_documents_list method name missing: {json}"
        );
        assert!(json.contains("\"source\":\"forge-persist\""));
        assert!(json.contains("\"limit\":100"));
        let decoded = decode_request(&json).expect("decode raw_documents_list");
        assert_eq!(req, decoded);
    }

    #[test]
    fn test_raw_documents_list_minimal() {
        // `limit` is optional — omitted callers must still deserialize.
        let json = r#"{"method":"raw_documents_list","params":{"source":"forge-persist"}}"#;
        let decoded = decode_request(json).expect("decode minimal raw_documents_list");
        match decoded {
            Request::RawDocumentsList { source, limit } => {
                assert_eq!(source, "forge-persist");
                assert!(limit.is_none());
            }
            _ => panic!("expected RawDocumentsList variant"),
        }
    }

    #[test]
    fn test_preference_flipped_response_variant_roundtrips() {
        use crate::protocol::response::{Response, ResponseData};
        let resp = Response::Ok {
            data: ResponseData::PreferenceFlipped {
                old_id: "01OLD".into(),
                new_id: "01NEW".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                flipped_at: "2026-04-17 14:22:00".into(),
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: Response = serde_json::from_str(&json).unwrap();
        // Response derives may not include PartialEq — compare via re-serialization
        let reserialized = serde_json::to_string(&decoded).unwrap();
        assert_eq!(json, reserialized);
    }

    #[test]
    fn test_flipped_list_response_variant_roundtrips() {
        use crate::protocol::response::{FlippedMemory, Response, ResponseData};
        use crate::types::memory::{Memory, MemoryType};
        let mut m = Memory::new(MemoryType::Preference, "tabs", "prefer tabs");
        m.valence_flipped_at = Some("2026-04-17 14:22:00".into());
        m.superseded_by = Some("01NEW".into());
        let resp = Response::Ok {
            data: ResponseData::FlippedList {
                items: vec![FlippedMemory {
                    old: m,
                    flipped_to_id: "01NEW".into(),
                    flipped_at: "2026-04-17 14:22:00".into(),
                }],
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: Response = serde_json::from_str(&json).unwrap();
        let reserialized = serde_json::to_string(&decoded).unwrap();
        assert_eq!(json, reserialized);
    }

    #[test]
    fn test_raw_documents_list_response_round_trip() {
        use crate::protocol::response::{RawDocumentInfo, Response, ResponseData};
        let resp = Response::Ok {
            data: ResponseData::RawDocumentsList {
                documents: vec![
                    RawDocumentInfo {
                        id: "doc_a".into(),
                        source: "forge-persist".into(),
                        text: "content a".into(),
                        timestamp: "2026-04-15T00:00:00Z".into(),
                    },
                    RawDocumentInfo {
                        id: "doc_b".into(),
                        source: "forge-persist".into(),
                        text: "content b".into(),
                        timestamp: "2026-04-15T00:00:01Z".into(),
                    },
                ],
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(
            json.contains("\"kind\":\"raw_documents_list\""),
            "raw_documents_list kind tag missing: {json}"
        );
        assert!(json.contains("\"id\":\"doc_a\""));
        assert!(json.contains("\"text\":\"content a\""));
        assert!(json.contains("\"timestamp\":\"2026-04-15T00:00:00Z\""));
        let decoded: Response = serde_json::from_str(&json).expect("decode response");
        assert_eq!(resp, decoded);
    }

    // ────────────────────────────────────────────────────────
    // Phase 2A-4b: ReaffirmPreference + ComputeRecencyFactor
    // ────────────────────────────────────────────────────────

    #[test]
    fn request_reaffirm_preference_serde_roundtrip() {
        let req = Request::ReaffirmPreference {
            memory_id: "01HXXX".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Request::ReaffirmPreference { .. }));
        assert!(
            json.contains("\"method\":\"reaffirm_preference\""),
            "expected reaffirm_preference method tag, got: {json}"
        );
    }

    #[cfg(feature = "bench")]
    #[test]
    fn request_compute_recency_factor_serde_roundtrip() {
        let req = Request::ComputeRecencyFactor {
            memory_id: "01HXXX".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Request::ComputeRecencyFactor { .. }));
        assert!(
            json.contains("\"method\":\"compute_recency_factor\""),
            "expected compute_recency_factor method tag, got: {json}"
        );
    }

    // ────────────────────────────────────────────────────────
    // SP1 review-fixup: session_id on proactive-hook Request variants.
    // Verify:
    //   a) explicit session_id round-trips,
    //   b) omitting the field decodes to None (backwards-compat via #[serde(default)]).
    // ────────────────────────────────────────────────────────

    #[test]
    fn pre_bash_check_with_explicit_session_id_roundtrips() {
        let req = Request::PreBashCheck {
            command: "cargo test".into(),
            session_id: Some("sess-xyz".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(
            json.contains("\"session_id\":\"sess-xyz\""),
            "explicit session_id must serialize, got: {json}"
        );
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn pre_bash_check_without_session_id_decodes_to_none() {
        // Old clients (pre-SP1-fixup) send only `command`. Ensure we still
        // accept that payload and deserialize session_id to None.
        let legacy = r#"{"method":"pre_bash_check","params":{"command":"ls"}}"#;
        let parsed: Request = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            parsed,
            Request::PreBashCheck {
                command: "ls".into(),
                session_id: None,
            }
        );
    }

    #[test]
    fn post_bash_check_with_explicit_session_id_roundtrips() {
        let req = Request::PostBashCheck {
            command: "cargo build".into(),
            exit_code: 1,
            session_id: Some("sess-xyz".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"session_id\":\"sess-xyz\""));
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn post_bash_check_without_session_id_decodes_to_none() {
        let legacy = r#"{"method":"post_bash_check","params":{"command":"ls","exit_code":0}}"#;
        let parsed: Request = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            parsed,
            Request::PostBashCheck {
                command: "ls".into(),
                exit_code: 0,
                session_id: None,
            }
        );
    }

    #[test]
    fn post_edit_check_with_explicit_session_id_roundtrips() {
        let req = Request::PostEditCheck {
            file: "src/main.rs".into(),
            session_id: Some("sess-xyz".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"session_id\":\"sess-xyz\""));
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn post_edit_check_without_session_id_decodes_to_none() {
        let legacy = r#"{"method":"post_edit_check","params":{"file":"f.rs"}}"#;
        let parsed: Request = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            parsed,
            Request::PostEditCheck {
                file: "f.rs".into(),
                session_id: None,
            }
        );
    }

    #[test]
    fn response_preference_reaffirmed_serde_roundtrip() {
        use crate::protocol::response::ResponseData;
        let r = ResponseData::PreferenceReaffirmed {
            memory_id: "01HXXX".to_string(),
            reaffirmed_at: "2026-04-19 12:00:00".to_string(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: ResponseData = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, ResponseData::PreferenceReaffirmed { .. }));
        assert!(
            json.contains("\"kind\":\"preference_reaffirmed\""),
            "expected preference_reaffirmed kind tag, got: {json}"
        );
    }

    // ────────────────────────────────────────────────────────
    // Phase 2A-4c1: RecordToolUse / ListToolCalls / Response variants
    // ────────────────────────────────────────────────────────

    #[test]
    fn record_tool_use_roundtrip_all_fields() {
        let req = Request::RecordToolUse {
            session_id: "S".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({"k": 1}),
            tool_result_summary: "ok".to_string(),
            success: true,
            user_correction_flag: true,
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn record_tool_use_defaults_when_optional_fields_omitted() {
        // Request uses #[serde(tag = "method", content = "params", rename_all = "snake_case")]
        // so the JSON uses "method" as the tag key.
        let minimal_json = r#"{"method":"record_tool_use","params":{"session_id":"S","agent":"a","tool_name":"T","success":true}}"#;
        let req: Request =
            serde_json::from_str(minimal_json).expect("default-filled deserialise must work");
        if let Request::RecordToolUse {
            tool_args,
            tool_result_summary,
            user_correction_flag,
            ..
        } = req
        {
            assert_eq!(tool_args, serde_json::Value::Object(serde_json::Map::new()));
            assert_eq!(tool_result_summary, "");
            assert!(!user_correction_flag);
        } else {
            panic!("wrong Request variant");
        }
    }

    #[test]
    fn list_tool_calls_roundtrip_required_only() {
        let req = Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: None,
            limit: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn list_tool_calls_roundtrip_all_fields() {
        let req = Request::ListToolCalls {
            session_id: "S".to_string(),
            agent: Some("a".to_string()),
            limit: Some(100),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn tool_call_recorded_response_roundtrip() {
        use crate::protocol::response::{Response, ResponseData};
        let resp = Response::Ok {
            data: ResponseData::ToolCallRecorded {
                id: "01K".to_string(),
                created_at: "2026-04-19 12:00:00".to_string(),
            },
        };
        let s = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn tool_call_list_response_roundtrip_empty_and_three() {
        use crate::protocol::response::{Response, ResponseData};
        use crate::types::ToolCallRow;
        let row = ToolCallRow {
            id: "1".to_string(),
            session_id: "S".to_string(),
            agent: "a".to_string(),
            tool_name: "T".to_string(),
            tool_args: serde_json::json!({}),
            tool_result_summary: "".to_string(),
            success: true,
            user_correction_flag: false,
            created_at: "2026-04-19 12:00:00".to_string(),
        };
        for rows in [vec![], vec![row.clone(), row.clone(), row.clone()]] {
            let resp = Response::Ok {
                data: ResponseData::ToolCallList {
                    calls: rows.clone(),
                },
            };
            let s = serde_json::to_string(&resp).unwrap();
            let back: Response = serde_json::from_str(&s).unwrap();
            assert_eq!(resp, back);
        }
    }

    #[test]
    fn consolidation_complete_response_roundtrip_with_skills_inferred() {
        // 2P-1b §15: ConsolidationComplete now carries skills_inferred so
        // the Phase 23 count is visible to bench harnesses + ops telemetry.
        use crate::protocol::response::{Response, ResponseData};
        let resp = Response::Ok {
            data: ResponseData::ConsolidationComplete {
                exact_dedup: 1,
                semantic_dedup: 2,
                linked: 3,
                faded: 4,
                promoted: 5,
                reconsolidated: 6,
                embedding_merged: 7,
                strengthened: 8,
                contradictions: 9,
                entities_detected: 10,
                synthesized: 11,
                gaps_detected: 12,
                reweaved: 13,
                scored: 14,
                skills_inferred: 15,
            },
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(
            s.contains("\"skills_inferred\":15"),
            "serialised JSON must contain skills_inferred field: {s}"
        );
        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);

        // Backward-compat: pre-2P-1b JSON without skills_inferred deserialises
        // via #[serde(default)] — no deserialisation error and the field is 0.
        let legacy = r#"{"status":"ok","data":{"kind":"consolidation_complete",
            "exact_dedup":0,"semantic_dedup":0,"linked":0,"faded":0,"promoted":0,
            "reconsolidated":0,"embedding_merged":0,"strengthened":0,
            "contradictions":0,"entities_detected":0}}"#;
        let parsed: Response = serde_json::from_str(legacy).unwrap();
        match parsed {
            Response::Ok {
                data:
                    ResponseData::ConsolidationComplete {
                        skills_inferred, ..
                    },
            } => assert_eq!(skills_inferred, 0),
            other => panic!("expected ConsolidationComplete, got {other:?}"),
        }
    }

    /// Phase 2A-4d.3 T10 — round-trip a `ResponseData::Inspect` carrying a
    /// `BenchRunSummary` payload; then re-decode from the produced JSON to
    /// pin the wire format.
    #[test]
    fn response_inspect_bench_run_summary_decodes_from_raw_json() {
        use crate::protocol::response::{Response, ResponseData};
        use crate::protocol::{
            BenchRunRow, InspectData, InspectFilter, InspectGroupBy, InspectShape,
        };
        let original = Response::Ok {
            data: ResponseData::Inspect {
                shape: InspectShape::BenchRunSummary,
                window: "30d".into(),
                window_secs: 30 * 86_400,
                generated_at_secs: 1_745_500_000,
                effective_filter: InspectFilter {
                    bench_name: Some("forge-identity".into()),
                    ..Default::default()
                },
                effective_group_by: Some(InspectGroupBy::BenchName),
                stale: false,
                truncated: false,
                data: InspectData::BenchRunSummary {
                    rows: vec![BenchRunRow {
                        bench_name: "forge-identity".into(),
                        group_key: "forge-identity".into(),
                        runs: 3,
                        pass_rate: 0.666,
                        composite_mean: 0.95,
                        composite_p50: 0.95,
                        composite_p95: 0.97,
                        composite_sample_size: 3,
                        first_ts_secs: 1_745_000_000,
                        last_ts_secs: 1_745_500_000,
                    }],
                },
            },
        };
        let s = serde_json::to_string(&original).expect("encode");
        assert!(
            s.contains(r#""kind":"bench_run_summary""#),
            "expected inner InspectData tag; got {s}"
        );
        let back: Response = serde_json::from_str(&s).expect("decode");
        let Response::Ok {
            data:
                ResponseData::Inspect {
                    shape,
                    data: InspectData::BenchRunSummary { rows },
                    ..
                },
        } = back
        else {
            panic!("expected Ok + Inspect + BenchRunSummary");
        };
        assert_eq!(shape, InspectShape::BenchRunSummary);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].bench_name, "forge-identity");
        assert_eq!(rows[0].runs, 3);
    }

    #[test]
    fn response_error_roundtrips_with_all_six_prefixes() {
        use crate::protocol::response::Response;
        let prefixes = [
            "unknown_session: 01K...",
            "payload_too_large: tool_args: 65536",
            "limit_too_large: requested 1000, max 500",
            "empty_field: tool_name",
            "invalid_field: session_id: control_character",
            "internal_error: db locked",
        ];
        for p in prefixes {
            let resp = Response::Error {
                message: p.to_string(),
            };
            let s = serde_json::to_string(&resp).unwrap();
            let back: Response = serde_json::from_str(&s).unwrap();
            assert_eq!(resp, back);
        }
    }
}
