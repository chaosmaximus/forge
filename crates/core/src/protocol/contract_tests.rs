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
            ("force_index", Request::ForceIndex),
            ("list_permissions", Request::ListPermissions),
            ("shutdown", Request::Shutdown),
        ];

        for (expected_method, request) in &cases {
            let json = serde_json::to_string(request).unwrap();
            assert!(
                json.contains(&format!("\"method\":\"{}\"", expected_method)),
                "Unit variant should serialize to method='{}', got: {}",
                expected_method,
                json
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
                "Round-trip failed for unit variant '{}'",
                expected_method
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
                },
            ),
            (
                "pre_bash_check",
                Request::PreBashCheck {
                    command: "rm -rf /tmp/test".into(),
                },
            ),
            (
                "post_bash_check",
                Request::PostBashCheck {
                    command: "cargo test".into(),
                    exit_code: 1,
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
            (
                "end_session",
                Request::EndSession { id: "s1".into() },
            ),
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
                "compile_context",
                Request::CompileContext {
                    agent: Some("claude-code".into()),
                    project: Some("forge".into()),
                    static_only: None,
                    excluded_layers: Some(vec!["decisions".into()]),
                },
            ),
            (
                "compile_context_trace",
                Request::CompileContextTrace {
                    agent: Some("claude-code".into()),
                    project: Some("forge".into()),
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
                Request::RevokePermission { id: "perm-123".into() },
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
            (
                "get_stats",
                Request::GetStats {
                    hours: Some(24),
                },
            ),
        ];

        for (expected_method, request) in &cases {
            let json = serde_json::to_string(request).unwrap();
            assert!(
                json.contains(&format!("\"method\":\"{}\"", expected_method)),
                "Parameterized variant should serialize to method='{}', got: {}",
                expected_method,
                json
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
                "Round-trip failed for parameterized variant '{}'",
                expected_method
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
            (
                "recall",
                r#"{"method":"recall","params":{"query":"test"}}"#,
            ),
            (
                "recall with layer",
                r#"{"method":"recall","params":{"query":"test","layer":"experience","limit":10}}"#,
            ),
            (
                "forget",
                r#"{"method":"forget","params":{"id":"abc"}}"#,
            ),
            (
                "export",
                r#"{"method":"export","params":{"format":"json"}}"#,
            ),
            (
                "import",
                r#"{"method":"import","params":{"data":"{}"}}"#,
            ),
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
            (
                "sync_export",
                r#"{"method":"sync_export","params":{}}"#,
            ),
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
            ("force_index", r#"{"method":"force_index"}"#),
            ("list_permissions", r#"{"method":"list_permissions"}"#),
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

    // ────────────────────────────────────────────────────────
    // Completeness guard: count all variants
    // ────────────────────────────────────────────────────────

    /// Ensure we cover ALL Request variants.
    /// If a new variant is added without updating these tests,
    /// the count assertion will fail.
    #[test]
    fn test_variant_count_completeness() {
        // Unit variants: 16 (ManasHealth moved to parameterized, +ListPermissions, +ForceIndex)
        let unit_count = 16;
        // Parameterized variants: 57 (including ListEntities, A2A FISP, A2A permissions, Scoped Config, DetectReality, CrossEngineQuery, FileMemoryMap, CodeSearch, ListRealities, GetStats)
        let param_count = 57;
        // Total: 73
        let expected_total = 73;

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
                Request::ForceIndex,
                Request::GetConfig,
                Request::Shutdown,
                // Parameterized variants
                Request::Remember {
                    memory_type: MemoryType::Decision,
                    title: "t".into(),
                    content: "c".into(),
                    confidence: None,
                    tags: None,
                    project: None,
                },
                Request::Recall {
                    query: "q".into(),
                    memory_type: None,
                    project: None,
                    limit: None,
                    layer: None,
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
                Request::Subscribe { events: None },
                Request::GuardrailsCheck {
                    file: "f".into(),
                    action: "a".into(),
                },
                Request::PostEditCheck { file: "f".into() },
                Request::PreBashCheck { command: "ls".into() },
                Request::PostBashCheck { command: "cargo test".into(), exit_code: 1 },
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
                Request::CleanupSessions { prefix: Some("hook-test".into()) },
                Request::SessionSend {
                    to: "s2".into(),
                    kind: "notification".into(),
                    topic: "test".into(),
                    parts: vec![],
                    project: None,
                    timeout_secs: None,
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
                },
                Request::SessionAck {
                    message_ids: vec!["m1".into()],
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
                    },
                },
                Request::ListIdentity { agent: "a".into() },
                Request::DeactivateIdentity { id: "i".into() },
                Request::ListDisposition { agent: "a".into() },
                Request::CompileContext {
                    agent: None,
                    project: None,
                    static_only: None,
                    excluded_layers: None,
                },
                Request::CompileContextTrace {
                    agent: None,
                    project: None,
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
                Request::GetDiagnostics {
                    file: "f".into(),
                },
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
                Request::Bootstrap {
                    project: None,
                },
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
                Request::RevokePermission { id: "perm-1".into() },
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
                Request::GetStats {
                    hours: Some(24),
                },
            ]
        }

        let variants = all_variants();
        assert_eq!(
            variants.len(),
            expected_total,
            "all_variants() must return exactly {} variants",
            expected_total
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
}
