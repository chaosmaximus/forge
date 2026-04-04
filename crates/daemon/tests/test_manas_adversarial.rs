//! Adversarial tests for Manas 8-layer memory system.
//! Tests SQL injection, data corruption, boundary conditions, and enum safety.

use forge_daemon::db::manas;
use forge_daemon::server::handler::{handle_request, DaemonState};
use forge_core::protocol::*;
use forge_core::types::manas::*;

/// Helper: create an in-memory DaemonState for testing.
fn test_state() -> DaemonState {
    forge_daemon::db::vec::init_sqlite_vec();
    DaemonState::new(":memory:").expect("DaemonState::new(:memory:) should succeed")
}

// ===========================================================================
// 1. SQL Injection Tests
// ===========================================================================

#[test]
fn test_platform_sql_injection_key() {
    let mut state = test_state();

    // Store a platform entry with SQL injection in the key
    let injection_key = "os'; DROP TABLE platform; --";
    let resp = handle_request(
        &mut state,
        Request::StorePlatform {
            key: injection_key.into(),
            value: "linux".into(),
        },
    );
    match &resp {
        Response::Ok { data: ResponseData::PlatformStored { key } } => {
            assert_eq!(key, injection_key);
        }
        other => panic!("expected PlatformStored, got: {:?}", other),
    }

    // Verify it's retrievable
    let resp = handle_request(&mut state, Request::ListPlatform);
    match resp {
        Response::Ok { data: ResponseData::PlatformList { entries } } => {
            let found = entries.iter().any(|e| e.key == injection_key && e.value == "linux");
            assert!(found, "injection key should be stored as-is, got: {:?}", entries);
        }
        other => panic!("expected PlatformList, got: {:?}", other),
    }

    // Verify the platform table still exists (not DROPped)
    let resp = handle_request(&mut state, Request::ManasHealth { project: None });
    match resp {
        Response::Ok { data: ResponseData::ManasHealthData { platform_count, .. } } => {
            assert!(platform_count > 0, "platform table should still exist with entries");
        }
        other => panic!("expected ManasHealthData, got: {:?}", other),
    }
}

#[test]
fn test_tool_sql_injection_name() {
    let mut state = test_state();

    let injection_name = "git'; DELETE FROM tool WHERE '1'='1";
    let tool = Tool {
        id: "t-inject-1".into(),
        name: injection_name.into(),
        kind: ToolKind::Cli,
        capabilities: vec!["commit".into()],
        config: None,
        health: ToolHealth::Healthy,
        last_used: None,
        use_count: 0,
        discovered_at: "2026-04-03 12:00:00".into(),
    };
    let resp = handle_request(&mut state, Request::StoreTool { tool });
    match &resp {
        Response::Ok { data: ResponseData::ToolStored { id } } => {
            assert_eq!(id, "t-inject-1");
        }
        other => panic!("expected ToolStored, got: {:?}", other),
    }

    // Verify tool is retrievable with the injection name intact
    let resp = handle_request(&mut state, Request::ListTools);
    match resp {
        Response::Ok { data: ResponseData::ToolList { tools, .. } } => {
            let our_tool = tools.iter().find(|t| t.name == injection_name);
            assert!(our_tool.is_some(), "should find tool with injection name");
        }
        other => panic!("expected ToolList, got: {:?}", other),
    }
}

#[test]
fn test_identity_sql_injection_description() {
    let mut state = test_state();

    let injection_desc = "Expert'; UPDATE identity SET strength=999 WHERE '1'='1";
    let facet = IdentityFacet {
        id: "if-inject-1".into(),
        agent: "forge-test".into(),
        facet: "role".into(),
        description: injection_desc.into(),
        strength: 0.5,
        source: "test".into(),
        active: true,
        created_at: "2026-04-03 12:00:00".into(),
    };
    let resp = handle_request(&mut state, Request::StoreIdentity { facet });
    match &resp {
        Response::Ok { data: ResponseData::IdentityStored { id } } => {
            assert_eq!(id, "if-inject-1");
        }
        other => panic!("expected IdentityStored, got: {:?}", other),
    }

    // Verify the identity is stored correctly and strength is NOT 999
    let resp = handle_request(
        &mut state,
        Request::ListIdentity { agent: "forge-test".into() },
    );
    match resp {
        Response::Ok { data: ResponseData::IdentityList { facets, count } } => {
            assert_eq!(count, 1);
            assert_eq!(facets[0].description, injection_desc);
            assert!(
                (facets[0].strength - 0.5).abs() < f64::EPSILON,
                "strength should be 0.5, not 999; got: {}",
                facets[0].strength
            );
        }
        other => panic!("expected IdentityList, got: {:?}", other),
    }
}

// ===========================================================================
// 2. Boundary Value Tests
// ===========================================================================

#[test]
fn test_identity_strength_clamping() {
    let mut state = test_state();

    // Store identity with strength > 1.0 — handler should clamp to 1.0
    let facet_high = IdentityFacet {
        id: "if-clamp-high".into(),
        agent: "forge-test".into(),
        facet: "expertise".into(),
        description: "Too strong".into(),
        strength: 5.0,
        source: "test".into(),
        active: true,
        created_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreIdentity { facet: facet_high });

    // Store identity with negative strength — handler should clamp to 0.0
    let facet_neg = IdentityFacet {
        id: "if-clamp-neg".into(),
        agent: "forge-test".into(),
        facet: "weakness".into(),
        description: "Negative strength".into(),
        strength: -3.0,
        source: "test".into(),
        active: true,
        created_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreIdentity { facet: facet_neg });

    // Verify clamping occurred
    let resp = handle_request(
        &mut state,
        Request::ListIdentity { agent: "forge-test".into() },
    );
    match resp {
        Response::Ok { data: ResponseData::IdentityList { facets, .. } } => {
            let high = facets.iter().find(|f| f.id == "if-clamp-high").expect("high facet");
            assert!(
                (high.strength - 1.0).abs() < f64::EPSILON,
                "strength > 1.0 should be clamped to 1.0, got: {}",
                high.strength
            );
            let neg = facets.iter().find(|f| f.id == "if-clamp-neg").expect("neg facet");
            assert!(
                (neg.strength - 0.0).abs() < f64::EPSILON,
                "negative strength should be clamped to 0.0, got: {}",
                neg.strength
            );
        }
        other => panic!("expected IdentityList, got: {:?}", other),
    }
}

#[test]
fn test_disposition_value_extremes() {
    // Use direct DB ops since StoreDisposition is not a handler request
    forge_daemon::db::vec::init_sqlite_vec();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    forge_daemon::db::schema::create_schema(&conn).unwrap();

    // Store disposition with f64::MAX
    let d_max = Disposition {
        id: "dp-max".into(),
        agent: "forge-test".into(),
        disposition_trait: DispositionTrait::Caution,
        domain: None,
        value: f64::MAX,
        trend: Trend::Stable,
        updated_at: "2026-04-03 12:00:00".into(),
        evidence: vec![],
    };
    // Should not panic
    let result = manas::store_disposition(&conn, &d_max);
    assert!(result.is_ok(), "f64::MAX should store without panic: {:?}", result);

    // Store disposition with NEG_INFINITY
    let d_neg_inf = Disposition {
        id: "dp-neg-inf".into(),
        agent: "forge-test".into(),
        disposition_trait: DispositionTrait::Thoroughness,
        domain: None,
        value: f64::NEG_INFINITY,
        trend: Trend::Falling,
        updated_at: "2026-04-03 12:00:00".into(),
        evidence: vec![],
    };
    let result = manas::store_disposition(&conn, &d_neg_inf);
    assert!(result.is_ok(), "f64::NEG_INFINITY should store without panic: {:?}", result);

    // Store disposition with NaN — SQLite treats NaN as NULL, which violates NOT NULL constraint.
    // This is expected and correct behavior: the system should reject invalid values cleanly.
    let d_nan = Disposition {
        id: "dp-nan".into(),
        agent: "forge-test".into(),
        disposition_trait: DispositionTrait::Autonomy,
        domain: None,
        value: f64::NAN,
        trend: Trend::Rising,
        updated_at: "2026-04-03 12:00:00".into(),
        evidence: vec![],
    };
    let result = manas::store_disposition(&conn, &d_nan);
    // NaN is rejected by SQLite NOT NULL constraint — this is correct, not a panic
    assert!(result.is_err(), "f64::NAN should be rejected by NOT NULL constraint");

    // Verify we can list dispositions without panic (only the 2 valid ones)
    let dispositions = manas::list_dispositions(&conn, "forge-test").unwrap();
    assert_eq!(dispositions.len(), 2, "only MAX and NEG_INFINITY dispositions should be stored");
}

#[test]
fn test_platform_empty_key() {
    let mut state = test_state();

    // Store platform with empty key ""
    let resp = handle_request(
        &mut state,
        Request::StorePlatform {
            key: "".into(),
            value: "some_value".into(),
        },
    );
    // Should either store or error, but NOT panic
    match resp {
        Response::Ok { data: ResponseData::PlatformStored { key } } => {
            assert_eq!(key, "", "empty key should be accepted");
        }
        Response::Error { message } => {
            // An error is also acceptable — no panic is the key requirement
            assert!(!message.is_empty(), "error message should be non-empty: {}", message);
        }
        other => panic!("unexpected response for empty key: {:?}", other),
    }
}

#[test]
fn test_tool_empty_capabilities() {
    let mut state = test_state();

    let tool = Tool {
        id: "t-empty-caps".into(),
        name: "empty-tool".into(),
        kind: ToolKind::Cli,
        capabilities: vec![],
        config: None,
        health: ToolHealth::Unknown,
        last_used: None,
        use_count: 0,
        discovered_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreTool { tool });

    // List tools and verify our tool with empty capabilities is present
    let resp = handle_request(&mut state, Request::ListTools);
    match resp {
        Response::Ok { data: ResponseData::ToolList { tools, .. } } => {
            let our_tool = tools.iter().find(|t| t.id == "t-empty-caps");
            assert!(our_tool.is_some(), "should find our stored tool");
            assert!(our_tool.unwrap().capabilities.is_empty(), "empty capabilities should round-trip");
        }
        other => panic!("expected ToolList, got: {:?}", other),
    }
}

#[test]
fn test_perception_huge_data_payload() {
    let mut state = test_state();

    // Create a 100KB data payload
    let huge_data = "X".repeat(100 * 1024);
    let perception = Perception {
        id: "p-huge".into(),
        kind: PerceptionKind::Error,
        data: huge_data.clone(),
        severity: Severity::Warning,
        project: Some("test".into()),
        created_at: "2026-04-03 12:00:00".into(),
        expires_at: None,
        consumed: false,
    };
    let resp = handle_request(
        &mut state,
        Request::StorePerception { perception },
    );
    match &resp {
        Response::Ok { data: ResponseData::PerceptionStored { id } } => {
            assert_eq!(id, "p-huge");
        }
        other => panic!("expected PerceptionStored for 100KB data, got: {:?}", other),
    }

    // Verify it retrieves correctly
    let resp = handle_request(
        &mut state,
        Request::ListPerceptions { project: Some("test".into()), limit: Some(10) },
    );
    match resp {
        Response::Ok { data: ResponseData::PerceptionList { perceptions, count } } => {
            assert_eq!(count, 1);
            assert_eq!(perceptions[0].data.len(), 100 * 1024, "100KB data should be preserved");
        }
        other => panic!("expected PerceptionList, got: {:?}", other),
    }
}

// ===========================================================================
// 3. Unicode and Special Character Tests
// ===========================================================================

#[test]
fn test_identity_unicode_description() {
    let mut state = test_state();

    let unicode_desc = "\u{0420}\u{0430}\u{0437}\u{0440}\u{0430}\u{0431}\u{043E}\u{0442}\u{0447}\u{0438}\u{043A} \u{1F980} \u{9AD8}\u{7EA7}";
    let facet = IdentityFacet {
        id: "if-unicode".into(),
        agent: "forge-test".into(),
        facet: "expertise".into(),
        description: unicode_desc.into(),
        strength: 0.8,
        source: "test".into(),
        active: true,
        created_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreIdentity { facet });

    // Retrieve and verify exact match
    let resp = handle_request(
        &mut state,
        Request::ListIdentity { agent: "forge-test".into() },
    );
    match resp {
        Response::Ok { data: ResponseData::IdentityList { facets, count } } => {
            assert_eq!(count, 1);
            assert_eq!(
                facets[0].description, unicode_desc,
                "unicode description should round-trip exactly"
            );
        }
        other => panic!("expected IdentityList, got: {:?}", other),
    }
}

#[test]
fn test_declared_binary_content() {
    // Use direct DB ops since StoreDeclared is not a handler request
    forge_daemon::db::vec::init_sqlite_vec();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    forge_daemon::db::schema::create_schema(&conn).unwrap();

    // Null bytes are not valid in Rust &str, but control chars are.
    // Test with low-range control characters that might cause issues.
    let content_with_controls = "hello\x01\x02\x03world";
    let d = Declared {
        id: "dk-binary".into(),
        source: "test".into(),
        path: None,
        content: content_with_controls.into(),
        hash: "hash-binary".into(),
        project: None,
        ingested_at: "2026-04-03 12:00:00".into(),
    };

    // Should store without panic
    let result = manas::store_declared(&conn, &d);
    assert!(result.is_ok(), "control chars in content should store: {:?}", result);

    // Should retrieve without panic
    let entries = manas::list_declared(&conn, None).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].content, content_with_controls, "control chars should round-trip");
}

#[test]
fn test_tool_name_special_chars() {
    let mut state = test_state();

    let xss_name = "git<script>alert(1)</script>";
    let tool = Tool {
        id: "t-xss".into(),
        name: xss_name.into(),
        kind: ToolKind::Cli,
        capabilities: vec!["<img onerror=alert(1)>".into()],
        config: None,
        health: ToolHealth::Healthy,
        last_used: None,
        use_count: 0,
        discovered_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreTool { tool });

    // Verify XSS payload is stored as-is and retrievable
    let resp = handle_request(&mut state, Request::ListTools);
    match resp {
        Response::Ok { data: ResponseData::ToolList { tools, .. } } => {
            let xss_tool = tools.iter().find(|t| t.id == "t-xss");
            assert!(xss_tool.is_some(), "should find XSS tool");
            let xss_tool = xss_tool.unwrap();
            assert_eq!(xss_tool.name, xss_name, "XSS payload should be stored as-is");
            assert_eq!(
                xss_tool.capabilities[0], "<img onerror=alert(1)>",
                "XSS in capabilities should be stored as-is"
            );
        }
        other => panic!("expected ToolList, got: {:?}", other),
    }
}

// ===========================================================================
// 4. Concurrency Safety Tests
// ===========================================================================

#[test]
fn test_manas_health_after_mass_insert() {
    let mut state = test_state();

    // Insert items across all 8 layers

    // Layer 0: Platform (10 entries)
    for i in 0..10 {
        handle_request(
            &mut state,
            Request::StorePlatform {
                key: format!("key_{}", i),
                value: format!("val_{}", i),
            },
        );
    }

    // Layer 1: Tools (10 entries)
    for i in 0..10 {
        let tool = Tool {
            id: format!("t-mass-{}", i),
            name: format!("tool_{}", i),
            kind: ToolKind::Cli,
            capabilities: vec![],
            config: None,
            health: ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-04-03 12:00:00".into(),
        };
        handle_request(&mut state, Request::StoreTool { tool });
    }

    // Layer 2: Skills (10 entries via direct DB ops)
    for i in 0..10 {
        let skill = Skill {
            id: format!("s-mass-{}", i),
            name: format!("skill_{}", i),
            domain: "testing".into(),
            description: format!("Skill number {}", i),
            steps: vec![],
            success_count: 0,
            fail_count: 0,
            last_used: None,
            source: "test".into(),
            version: 1,
            project: None,
            skill_type: "procedural".to_string(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        };
        manas::store_skill(&state.conn, &skill).unwrap();
    }

    // Layer 3: Domain DNA (10 entries via direct DB ops)
    for i in 0..10 {
        let dna = DomainDna {
            id: format!("dd-mass-{}", i),
            project: "test-project".into(),
            aspect: format!("aspect_{}", i),
            pattern: format!("pattern_{}", i),
            confidence: 0.8,
            evidence: vec!["evidence".into()],
            detected_at: "2026-04-03 12:00:00".into(),
        };
        manas::store_domain_dna(&state.conn, &dna).unwrap();
    }

    // Layer 4: Perceptions (10 entries)
    for i in 0..10 {
        let perception = Perception {
            id: format!("p-mass-{}", i),
            kind: PerceptionKind::Error,
            data: format!("error #{}", i),
            severity: Severity::Warning,
            project: None,
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        };
        handle_request(&mut state, Request::StorePerception { perception });
    }

    // Layer 5: Declared (10 entries via direct DB ops)
    for i in 0..10 {
        let d = Declared {
            id: format!("dk-mass-{}", i),
            source: "test".into(),
            path: Some(format!("/test/path_{}", i)),
            content: format!("content {}", i),
            hash: format!("hash_{}", i),
            project: None,
            ingested_at: "2026-04-03 12:00:00".into(),
        };
        manas::store_declared(&state.conn, &d).unwrap();
    }

    // Layer 6: Identity (10 entries)
    for i in 0..10 {
        let facet = IdentityFacet {
            id: format!("if-mass-{}", i),
            agent: "forge-mass".into(),
            facet: format!("facet_{}", i),
            description: format!("desc {}", i),
            strength: 0.5,
            source: "test".into(),
            active: true,
            created_at: "2026-04-03 12:00:00".into(),
        };
        handle_request(&mut state, Request::StoreIdentity { facet });
    }

    // Layer 7: Disposition (10 entries via direct DB ops)
    for i in 0..10 {
        let d = Disposition {
            id: format!("dp-mass-{}", i),
            agent: "forge-mass".into(),
            disposition_trait: DispositionTrait::Caution,
            domain: Some(format!("domain_{}", i)),
            value: 0.5,
            trend: Trend::Stable,
            updated_at: "2026-04-03 12:00:00".into(),
            evidence: vec![],
        };
        manas::store_disposition(&state.conn, &d).unwrap();
    }

    // Call manas_health and verify counts
    let resp = handle_request(&mut state, Request::ManasHealth { project: None });
    match resp {
        Response::Ok {
            data: ResponseData::ManasHealthData {
                platform_count,
                tool_count,
                skill_count,
                domain_dna_count,
                perception_unconsumed,
                declared_count,
                identity_facets,
                disposition_traits,
                ..
            },
        } => {
            // Platform may have auto-detected entries from DaemonState::new, so >= 10
            assert!(platform_count >= 10, "platform should have >= 10 entries, got: {}", platform_count);
            assert!(tool_count >= 10, "should have >= 10 tools (includes auto-detected), got: {}", tool_count);
            assert_eq!(skill_count, 10, "should have 10 skills");
            assert_eq!(domain_dna_count, 10, "should have 10 domain DNA entries");
            assert_eq!(perception_unconsumed, 10, "should have 10 unconsumed perceptions");
            assert_eq!(declared_count, 10, "should have 10 declared entries");
            assert_eq!(identity_facets, 10, "should have 10 active identity facets");
            assert_eq!(disposition_traits, 10, "should have 10 dispositions");
        }
        other => panic!("expected ManasHealthData, got: {:?}", other),
    }
}

#[test]
fn test_perception_expire_during_list() {
    let mut state = test_state();

    // Store 5 perceptions with expires_at in the past
    for i in 0..5 {
        let perception = Perception {
            id: format!("p-expired-{}", i),
            kind: PerceptionKind::Error,
            data: format!("expired error #{}", i),
            severity: Severity::Info,
            project: None,
            created_at: "2025-01-01 00:00:00".into(),
            expires_at: Some("2025-06-01 00:00:00".into()),
            consumed: false,
        };
        handle_request(&mut state, Request::StorePerception { perception });
    }

    // Store 5 perceptions without expiry (should persist)
    for i in 0..5 {
        let perception = Perception {
            id: format!("p-live-{}", i),
            kind: PerceptionKind::BuildResult,
            data: format!("live result #{}", i),
            severity: Severity::Info,
            project: None,
            created_at: "2026-04-03 12:00:00".into(),
            expires_at: None,
            consumed: false,
        };
        handle_request(&mut state, Request::StorePerception { perception });
    }

    // Consume the expired ones (simulating expiration via ConsumePerceptions)
    let expired_ids: Vec<String> = (0..5).map(|i| format!("p-expired-{}", i)).collect();
    let resp = handle_request(
        &mut state,
        Request::ConsumePerceptions { ids: expired_ids },
    );
    match &resp {
        Response::Ok { data: ResponseData::PerceptionsConsumed { count } } => {
            assert_eq!(*count, 5, "should consume 5 expired perceptions");
        }
        other => panic!("expected PerceptionsConsumed, got: {:?}", other),
    }

    // List unconsumed — should only have the 5 live ones
    let resp = handle_request(
        &mut state,
        Request::ListPerceptions { project: None, limit: Some(20) },
    );
    match resp {
        Response::Ok { data: ResponseData::PerceptionList { perceptions, count } } => {
            assert_eq!(count, 5, "should have 5 unconsumed perceptions after consuming expired");
            for p in &perceptions {
                assert!(
                    p.id.starts_with("p-live-"),
                    "only live perceptions should remain, got id: {}",
                    p.id
                );
            }
        }
        other => panic!("expected PerceptionList, got: {:?}", other),
    }
}

// ===========================================================================
// 5. Data Integrity Tests
// ===========================================================================

#[test]
fn test_identity_deactivate_nonexistent() {
    let mut state = test_state();

    // Deactivate an ID that doesn't exist
    let resp = handle_request(
        &mut state,
        Request::DeactivateIdentity { id: "nonexistent-id-12345".into() },
    );
    match resp {
        Response::Ok { data: ResponseData::IdentityDeactivated { id, found } } => {
            assert_eq!(id, "nonexistent-id-12345");
            assert!(!found, "should return found=false for nonexistent ID");
        }
        other => panic!("expected IdentityDeactivated, got: {:?}", other),
    }
}

#[test]
fn test_platform_overwrite() {
    let mut state = test_state();

    // Store "os" = "linux"
    handle_request(
        &mut state,
        Request::StorePlatform { key: "test_os".into(), value: "linux".into() },
    );

    // Store "os" = "macos" (should overwrite via INSERT OR REPLACE)
    handle_request(
        &mut state,
        Request::StorePlatform { key: "test_os".into(), value: "macos".into() },
    );

    // Verify only "macos" exists
    let resp = handle_request(&mut state, Request::ListPlatform);
    match resp {
        Response::Ok { data: ResponseData::PlatformList { entries } } => {
            let os_entries: Vec<&PlatformEntry> = entries.iter().filter(|e| e.key == "test_os").collect();
            assert_eq!(os_entries.len(), 1, "should have exactly 1 test_os entry after overwrite");
            assert_eq!(os_entries[0].value, "macos", "value should be 'macos' after overwrite");
        }
        other => panic!("expected PlatformList, got: {:?}", other),
    }
}

#[test]
fn test_skill_version_tracking() {
    // Use direct DB ops since StoreSkill is not a handler request
    forge_daemon::db::vec::init_sqlite_vec();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    forge_daemon::db::schema::create_schema(&conn).unwrap();

    // Store a skill with version 1
    let skill_v1 = Skill {
        id: "s-version".into(),
        name: "TDD".into(),
        domain: "testing".into(),
        description: "Version 1".into(),
        steps: vec!["write test".into()],
        success_count: 1,
        fail_count: 0,
        last_used: None,
        source: "test".into(),
        version: 1,
        project: None,
            skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
    };
    manas::store_skill(&conn, &skill_v1).unwrap();

    // Store again with version 2 (same ID)
    let skill_v2 = Skill {
        id: "s-version".into(),
        name: "TDD".into(),
        domain: "testing".into(),
        description: "Version 2 — improved".into(),
        steps: vec!["write test".into(), "refactor".into()],
        success_count: 5,
        fail_count: 1,
        last_used: None,
        source: "test".into(),
        version: 2,
        project: None,
        skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
    };
    manas::store_skill(&conn, &skill_v2).unwrap();

    // Verify version is updated to 2
    let skills = manas::list_skills(&conn, None).unwrap();
    assert_eq!(skills.len(), 1, "should have 1 skill after upsert");
    assert_eq!(skills[0].version, 2, "version should be updated to 2");
    assert_eq!(skills[0].description, "Version 2 — improved");
    assert_eq!(skills[0].steps.len(), 2);
}

#[test]
fn test_domain_dna_multiple_projects() {
    forge_daemon::db::vec::init_sqlite_vec();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    forge_daemon::db::schema::create_schema(&conn).unwrap();

    // Store DNA for project "A"
    let dna_a = DomainDna {
        id: "dd-a".into(),
        project: "project_a".into(),
        aspect: "naming".into(),
        pattern: "snake_case".into(),
        confidence: 0.9,
        evidence: vec!["file_a.rs".into()],
        detected_at: "2026-04-03 12:00:00".into(),
    };
    manas::store_domain_dna(&conn, &dna_a).unwrap();

    // Store DNA for project "B"
    let dna_b = DomainDna {
        id: "dd-b".into(),
        project: "project_b".into(),
        aspect: "naming".into(),
        pattern: "camelCase".into(),
        confidence: 0.85,
        evidence: vec!["file_b.ts".into()],
        detected_at: "2026-04-03 12:00:00".into(),
    };
    manas::store_domain_dna(&conn, &dna_b).unwrap();

    // List DNA for project "A" — should NOT include "B" patterns
    let dna_for_a = manas::list_domain_dna(&conn, Some("project_a")).unwrap();
    assert_eq!(dna_for_a.len(), 1, "project A should have exactly 1 DNA entry");
    assert_eq!(dna_for_a[0].pattern, "snake_case");
    assert_eq!(dna_for_a[0].project, "project_a");

    // List DNA for project "B" — should NOT include "A" patterns
    let dna_for_b = manas::list_domain_dna(&conn, Some("project_b")).unwrap();
    assert_eq!(dna_for_b.len(), 1, "project B should have exactly 1 DNA entry");
    assert_eq!(dna_for_b[0].pattern, "camelCase");

    // List all DNA — should include both
    let all_dna = manas::list_domain_dna(&conn, None).unwrap();
    assert_eq!(all_dna.len(), 2, "listing all should return 2 DNA entries");
}

#[test]
fn test_declared_hash_dedup() {
    forge_daemon::db::vec::init_sqlite_vec();
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    forge_daemon::db::schema::create_schema(&conn).unwrap();

    // Store declared with hash "abc123"
    let d1 = Declared {
        id: "dk-dedup-1".into(),
        source: "CLAUDE.md".into(),
        path: Some("/project/CLAUDE.md".into()),
        content: "Use snake_case".into(),
        hash: "abc123".into(),
        project: Some("forge".into()),
        ingested_at: "2026-04-03 12:00:00".into(),
    };
    manas::store_declared(&conn, &d1).unwrap();

    // get_declared_by_hash should return the entry
    let found = manas::get_declared_by_hash(&conn, "abc123").unwrap();
    assert!(found.is_some(), "should find declared by hash abc123");
    assert_eq!(found.unwrap().id, "dk-dedup-1");

    // Store again with same ID but updated content — should be idempotent (upsert)
    let d2 = Declared {
        id: "dk-dedup-1".into(),
        source: "CLAUDE.md".into(),
        path: Some("/project/CLAUDE.md".into()),
        content: "Use snake_case — updated".into(),
        hash: "abc123".into(),
        project: Some("forge".into()),
        ingested_at: "2026-04-03 13:00:00".into(),
    };
    manas::store_declared(&conn, &d2).unwrap();

    // Should still be 1 entry (upsert, not duplicate)
    let all = manas::list_declared(&conn, None).unwrap();
    assert_eq!(all.len(), 1, "upsert should not create duplicate");
    assert_eq!(all[0].content, "Use snake_case — updated", "content should be updated");

    // Store with a different ID but same hash — this is a new row
    let d3 = Declared {
        id: "dk-dedup-2".into(),
        source: "CLAUDE.md".into(),
        path: Some("/project/CLAUDE.md".into()),
        content: "Different entry same hash".into(),
        hash: "abc123".into(),
        project: Some("forge".into()),
        ingested_at: "2026-04-03 14:00:00".into(),
    };
    manas::store_declared(&conn, &d3).unwrap();

    // get_declared_by_hash now returns one of them (query returns first match)
    let found = manas::get_declared_by_hash(&conn, "abc123").unwrap();
    assert!(found.is_some(), "should still find declared by hash abc123");
}

// ===========================================================================
// 6. Handler-Level Tests
// ===========================================================================

#[test]
fn test_manas_health_includes_all_layers() {
    let mut state = test_state();

    // Layer 0: Platform — already has auto-detected entries from DaemonState::new
    // Add a custom one
    handle_request(
        &mut state,
        Request::StorePlatform { key: "custom".into(), value: "test".into() },
    );

    // Layer 1: Tool
    let tool = Tool {
        id: "t-health-test".into(),
        name: "health-tool".into(),
        kind: ToolKind::Mcp,
        capabilities: vec!["test".into()],
        config: None,
        health: ToolHealth::Healthy,
        last_used: None,
        use_count: 0,
        discovered_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreTool { tool });

    // Layer 2: Skill (direct DB)
    let skill = Skill {
        id: "s-health-test".into(),
        name: "Health Test Skill".into(),
        domain: "testing".into(),
        description: "test".into(),
        steps: vec![],
        success_count: 0,
        fail_count: 0,
        last_used: None,
        source: "test".into(),
        version: 1,
        project: None,
            skill_type: "procedural".to_string(),
        user_specific: false,
        observed_count: 1,
        correlation_ids: vec![],
    };
    manas::store_skill(&state.conn, &skill).unwrap();

    // Layer 3: Domain DNA (direct DB)
    let dna = DomainDna {
        id: "dd-health-test".into(),
        project: "test".into(),
        aspect: "naming".into(),
        pattern: "test_pattern".into(),
        confidence: 0.5,
        evidence: vec![],
        detected_at: "2026-04-03 12:00:00".into(),
    };
    manas::store_domain_dna(&state.conn, &dna).unwrap();

    // Layer 4: Perception
    let perception = Perception {
        id: "p-health-test".into(),
        kind: PerceptionKind::TestResult,
        data: "test passed".into(),
        severity: Severity::Info,
        project: None,
        created_at: "2026-04-03 12:00:00".into(),
        expires_at: None,
        consumed: false,
    };
    handle_request(&mut state, Request::StorePerception { perception });

    // Layer 5: Declared (direct DB)
    let declared = Declared {
        id: "dk-health-test".into(),
        source: "test".into(),
        path: None,
        content: "test content".into(),
        hash: "hash-health".into(),
        project: None,
        ingested_at: "2026-04-03 12:00:00".into(),
    };
    manas::store_declared(&state.conn, &declared).unwrap();

    // Layer 6: Identity
    let facet = IdentityFacet {
        id: "if-health-test".into(),
        agent: "forge-test".into(),
        facet: "health-test".into(),
        description: "test".into(),
        strength: 0.5,
        source: "test".into(),
        active: true,
        created_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreIdentity { facet });

    // Layer 7: Disposition (direct DB)
    let disposition = Disposition {
        id: "dp-health-test".into(),
        agent: "forge-test".into(),
        disposition_trait: DispositionTrait::Creativity,
        domain: None,
        value: 0.7,
        trend: Trend::Rising,
        updated_at: "2026-04-03 12:00:00".into(),
        evidence: vec![],
    };
    manas::store_disposition(&state.conn, &disposition).unwrap();

    // Verify ALL ManasHealth counts are > 0
    let resp = handle_request(&mut state, Request::ManasHealth { project: None });
    match resp {
        Response::Ok {
            data: ResponseData::ManasHealthData {
                platform_count,
                tool_count,
                skill_count,
                domain_dna_count,
                perception_unconsumed,
                declared_count,
                identity_facets,
                disposition_traits,
                ..
            },
        } => {
            assert!(platform_count > 0, "platform_count should be > 0, got: {}", platform_count);
            assert!(tool_count > 0, "tool_count should be > 0, got: {}", tool_count);
            assert!(skill_count > 0, "skill_count should be > 0, got: {}", skill_count);
            assert!(domain_dna_count > 0, "domain_dna_count should be > 0, got: {}", domain_dna_count);
            assert!(perception_unconsumed > 0, "perception_unconsumed should be > 0, got: {}", perception_unconsumed);
            assert!(declared_count > 0, "declared_count should be > 0, got: {}", declared_count);
            assert!(identity_facets > 0, "identity_facets should be > 0, got: {}", identity_facets);
            assert!(disposition_traits > 0, "disposition_traits should be > 0, got: {}", disposition_traits);
        }
        other => panic!("expected ManasHealthData, got: {:?}", other),
    }
}

#[test]
fn test_doctor_includes_manas_counts() {
    let mut state = test_state();

    // Store items in a few layers
    handle_request(
        &mut state,
        Request::StoreTool {
            tool: Tool {
                id: "t-doctor".into(),
                name: "doctor-tool".into(),
                kind: ToolKind::Builtin,
                capabilities: vec!["diagnose".into()],
                config: None,
                health: ToolHealth::Healthy,
                last_used: None,
                use_count: 0,
                discovered_at: "2026-04-03 12:00:00".into(),
            },
        },
    );

    let facet = IdentityFacet {
        id: "if-doctor".into(),
        agent: "forge-doctor".into(),
        facet: "role".into(),
        description: "doctor test".into(),
        strength: 0.6,
        source: "test".into(),
        active: true,
        created_at: "2026-04-03 12:00:00".into(),
    };
    handle_request(&mut state, Request::StoreIdentity { facet });

    // Call Doctor handler and verify Manas fields are present and correct
    let resp = handle_request(&mut state, Request::Doctor);
    match resp {
        Response::Ok {
            data: ResponseData::Doctor {
                daemon_up,
                tool_count,
                identity_count,
                platform_count,
                skill_count,
                domain_dna_count,
                perception_count,
                declared_count,
                disposition_count,
                ..
            },
        } => {
            assert!(daemon_up, "daemon should be up");
            assert!(tool_count >= 1, "should have >= 1 tool (auto-detected + stored), got: {}", tool_count);
            assert_eq!(identity_count, 1, "should have 1 identity facet");
            // platform_count may be > 0 from auto-detect; just check it exists
            let _ = platform_count;
            // The rest should be 0 (we didn't store skill/dna/perception/declared/disposition)
            assert_eq!(skill_count, 0, "should have 0 skills");
            assert_eq!(domain_dna_count, 0, "should have 0 domain DNA entries");
            assert_eq!(perception_count, 0, "should have 0 perceptions");
            assert_eq!(declared_count, 0, "should have 0 declared entries");
            assert_eq!(disposition_count, 0, "should have 0 dispositions");
        }
        other => panic!("expected Doctor response, got: {:?}", other),
    }
}
