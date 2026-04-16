//! Dataset generator for the Forge-Context benchmark.
//!
//! Seeds a `DaemonState` with deterministic test data from a ChaCha20 seed:
//! 6 present tools (from the hardcoded-12), 6 absent keywords, 30 skills
//! (10 present-tool / 10 absent-tool / 10 no-tool), 30 memories (10 decisions
//! / 10 lessons / 10 patterns), 5 domain DNA entries, affects edges, and a
//! test session registration.
//!
//! The hardcoded-12 tool keywords are the same ones checked by the skill
//! filtering logic in `recall.rs:1077-1094`. "Absent" means NOT inserted
//! into the DB — the filter rejects skills mentioning a keyword for which
//! no matching tool row exists.

use forge_core::protocol::{Request, Response, ResponseData};
use forge_core::types::manas::{DomainDna, Skill, Tool, ToolHealth, ToolKind};
use forge_core::types::memory::MemoryType;
use rand::seq::SliceRandom;

use super::common::{seeded_rng, sha256_hex};

// ── Constants ──────────────────────────────────────────────────────

/// The 12 tool keywords hardcoded in `recall.rs` skill filtering.
const HARDCODED_TOOL_KEYWORDS: [&str; 12] = [
    "docker", "kubectl", "terraform", "npm", "cargo", "pip", "gcloud", "aws", "ssh", "make",
    "scp", "rsync",
];

/// Domain vocabulary used across generators.
const DOMAINS: [&str; 5] = ["auth", "database", "networking", "testing", "deployment"];

/// File paths corresponding to each domain.
const FILE_PATHS: [&str; 5] = [
    "src/auth/middleware.rs",
    "src/database/schema.rs",
    "src/networking/client.rs",
    "src/testing/harness.rs",
    "src/deployment/config.rs",
];

/// Tags relevant to CompletionCheck / TaskCompletionCheck queries.
const COMPLETION_TAGS: [&str; 5] = [
    "testing",
    "uat",
    "deployment",
    "anti-pattern",
    "production-readiness",
];

// ── Output type ────────────────────────────────────────────────────

/// Metadata from seeding, used by the query-bank generator.
pub struct SeededDataset {
    pub present_tools: Vec<Tool>,
    pub absent_keywords: Vec<String>,
    pub skills: Vec<Skill>,
    pub memory_ids: Vec<(MemoryType, String, String)>, // (type, id, title)
    pub decision_file_map: Vec<(String, String)>,       // (memory_id, file_path)
    pub domain_dna_aspects: Vec<String>,
    pub file_paths: Vec<String>,
    pub session_id: String,
}

// ── Generators ─────────────────────────────────────────────────────

/// Split the 12 hardcoded tool keywords into 6 "present" `Tool` structs and
/// 6 "absent" keyword strings.  The split is deterministic given `seed`.
pub fn generate_tools(seed: u64) -> (Vec<Tool>, Vec<String>) {
    let mut rng = seeded_rng(seed);
    let mut keywords: Vec<&str> = HARDCODED_TOOL_KEYWORDS.to_vec();
    keywords.shuffle(&mut rng);

    let present: Vec<Tool> = keywords[..6]
        .iter()
        .enumerate()
        .map(|(i, &kw)| {
            let token = sha256_hex(&format!("forge-context-{seed}-tool-{i}"));
            Tool {
                id: format!("bench-tool-{kw}"),
                name: kw.to_string(),
                kind: ToolKind::Cli,
                capabilities: vec![format!("{kw}-cap-{}", &token[..8])],
                config: None,
                health: ToolHealth::Healthy,
                last_used: None,
                use_count: 0,
                discovered_at: "2026-01-01T00:00:00Z".to_string(),
            }
        })
        .collect();

    let absent: Vec<String> = keywords[6..].iter().map(|kw| kw.to_string()).collect();

    (present, absent)
}

/// Generate 30 skills in 3 tiers of 10:
///   - Tier A (0..9):  mention a *present* tool keyword — should survive filtering.
///   - Tier B (10..19): mention an *absent* tool keyword — should be filtered out.
///   - Tier C (20..29): no tool keywords at all — should survive filtering.
///
/// All skills have `success_count > 0` to satisfy the SQL filter in
/// `find_applicable_skills`.
pub fn generate_skills(
    seed: u64,
    present_tools: &[Tool],
    absent_keywords: &[String],
) -> Vec<Skill> {
    let mut skills = Vec::with_capacity(30);

    // Tier A: present-tool skills
    for i in 0..10 {
        let tool_name = &present_tools[i % present_tools.len()].name;
        let domain = DOMAINS[i % DOMAINS.len()];
        let token = sha256_hex(&format!("forge-context-{seed}-skill-a-{i}"));
        skills.push(Skill {
            id: format!("bench-skill-a-{i}"),
            name: format!("{tool_name} {domain} workflow {}", &token[..8]),
            domain: domain.to_string(),
            description: format!(
                "Uses {tool_name} for {domain} operations. Token: {token}"
            ),
            steps: vec![
                format!("step-1: run {tool_name}"),
                format!("step-2: verify {domain}"),
            ],
            success_count: (i as u64) + 1,
            fail_count: 0,
            last_used: Some("2026-01-01T00:00:00Z".to_string()),
            source: "bench".to_string(),
            version: 1,
            project: None,
            skill_type: "procedural".to_string(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        });
    }

    // Tier B: absent-tool skills
    for i in 0..10 {
        let tool_kw = &absent_keywords[i % absent_keywords.len()];
        let domain = DOMAINS[i % DOMAINS.len()];
        let token = sha256_hex(&format!("forge-context-{seed}-skill-b-{i}"));
        skills.push(Skill {
            id: format!("bench-skill-b-{i}"),
            name: format!("{tool_kw} {domain} pipeline {}", &token[..8]),
            domain: domain.to_string(),
            description: format!(
                "Requires {tool_kw} for {domain} automation. Token: {token}"
            ),
            steps: vec![
                format!("step-1: install {tool_kw}"),
                format!("step-2: configure {domain}"),
            ],
            success_count: (i as u64) + 1,
            fail_count: 0,
            last_used: Some("2026-01-01T00:00:00Z".to_string()),
            source: "bench".to_string(),
            version: 1,
            project: None,
            skill_type: "procedural".to_string(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        });
    }

    // Tier C: no-tool skills
    for i in 0..10 {
        let domain = DOMAINS[i % DOMAINS.len()];
        let token = sha256_hex(&format!("forge-context-{seed}-skill-c-{i}"));
        skills.push(Skill {
            id: format!("bench-skill-c-{i}"),
            name: format!("{domain} best practice {}", &token[..8]),
            domain: domain.to_string(),
            description: format!(
                "Domain-only guidance for {domain} layer. Token: {token}"
            ),
            steps: vec![
                format!("step-1: review {domain} standards"),
                format!("step-2: apply pattern"),
            ],
            success_count: (i as u64) + 1,
            fail_count: 0,
            last_used: Some("2026-01-01T00:00:00Z".to_string()),
            source: "bench".to_string(),
            version: 1,
            project: None,
            skill_type: "procedural".to_string(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        });
    }

    skills
}

/// Memory spec produced by `generate_memories`, not yet stored.
pub struct MemorySpec {
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub confidence: f64,
    pub tags: Vec<String>,
    /// For decisions: the file path this decision "affects".
    pub file_path: Option<String>,
}

/// Generate 30 memory specifications split 10/10/10 (Decision / Lesson / Pattern).
///
/// - Decisions get domain tags + a file path for an affects edge.
/// - Lessons get completion-relevant tags (`COMPLETION_TAGS`).
/// - Patterns get domain tags only.
///
/// Content includes a SHA-256 token to avoid semantic dedup.
pub fn generate_memories(seed: u64) -> Vec<MemorySpec> {
    let mut specs = Vec::with_capacity(30);

    // 10 Decisions
    for i in 0..10 {
        let domain = DOMAINS[i % DOMAINS.len()];
        let file = FILE_PATHS[i % FILE_PATHS.len()];
        let token = sha256_hex(&format!("forge-context-{seed}-decision-{i}"));
        specs.push(MemorySpec {
            memory_type: MemoryType::Decision,
            title: format!(
                "Decision: {domain} layer architecture ({}) — affects {file}",
                &token[..8]
            ),
            content: format!(
                "Decided to refactor {domain} layer via {file}. Unique token: {token}"
            ),
            confidence: 0.85 + (i as f64) * 0.01,
            tags: vec![domain.to_string()],
            file_path: Some(file.to_string()),
        });
    }

    // 10 Lessons
    for i in 0..10 {
        let domain = DOMAINS[i % DOMAINS.len()];
        let completion_tag = COMPLETION_TAGS[i % COMPLETION_TAGS.len()];
        let token = sha256_hex(&format!("forge-context-{seed}-lesson-{i}"));
        specs.push(MemorySpec {
            memory_type: MemoryType::Lesson,
            title: format!(
                "Lesson: {domain} {completion_tag} insight ({})",
                &token[..8]
            ),
            content: format!(
                "Learned about {completion_tag} in {domain} context. Unique token: {token}"
            ),
            confidence: 0.75 + (i as f64) * 0.02,
            tags: vec![domain.to_string(), completion_tag.to_string()],
            file_path: None,
        });
    }

    // 10 Patterns
    for i in 0..10 {
        let domain = DOMAINS[i % DOMAINS.len()];
        let token = sha256_hex(&format!("forge-context-{seed}-pattern-{i}"));
        specs.push(MemorySpec {
            memory_type: MemoryType::Pattern,
            title: format!("Pattern: {domain} convention ({})", &token[..8]),
            content: format!(
                "Observed recurring {domain} pattern. Unique token: {token}"
            ),
            confidence: 0.70 + (i as f64) * 0.02,
            tags: vec![domain.to_string()],
            file_path: None,
        });
    }

    specs
}

/// Generate 5 domain DNA entries, one per domain.
fn generate_domain_dna(seed: u64) -> Vec<DomainDna> {
    DOMAINS
        .iter()
        .enumerate()
        .map(|(i, &domain)| {
            let token = sha256_hex(&format!("forge-context-{seed}-dna-{i}"));
            DomainDna {
                id: format!("bench-dna-{domain}"),
                project: "forge-context-bench".to_string(),
                aspect: format!("{domain}-conventions"),
                pattern: format!("{domain} uses standard patterns. Token: {token}"),
                confidence: 0.9,
                evidence: vec![FILE_PATHS[i].to_string()],
                detected_at: "2026-01-01T00:00:00Z".to_string(),
            }
        })
        .collect()
}

// ── Seeder ─────────────────────────────────────────────────────────

use crate::server::handler::{handle_request, DaemonState};

/// Seed a fresh in-memory `DaemonState` with all benchmark data.
///
/// Returns the `SeededDataset` metadata that the query-bank generator
/// needs to construct ground-truth expectations.
pub fn seed_state(state: &mut DaemonState, seed: u64) -> SeededDataset {
    let session_id = format!("forge-context-bench-{seed}");

    // 1. Register a test session
    let resp = handle_request(
        state,
        Request::RegisterSession {
            id: session_id.clone(),
            agent: "bench".to_string(),
            project: None,
            cwd: None,
            capabilities: None,
            current_task: None,
        },
    );
    assert!(
        matches!(resp, Response::Ok { .. }),
        "RegisterSession failed: {resp:?}"
    );

    // 2. Generate and store tools; purge auto-detected tools that conflict
    //    with our "absent" set so the split is deterministic.
    let (present_tools, absent_keywords) = generate_tools(seed);

    // Delete all auto-detected tools first so we have a clean slate, then
    // insert only our present tools.
    state
        .conn
        .execute("DELETE FROM tool", [])
        .expect("clear tool table");

    for tool in &present_tools {
        let resp = handle_request(
            state,
            Request::StoreTool {
                tool: tool.clone(),
            },
        );
        assert!(
            matches!(resp, Response::Ok { .. }),
            "StoreTool failed: {resp:?}"
        );
    }

    // 3. Generate and store skills
    let skills = generate_skills(seed, &present_tools, &absent_keywords);
    for skill in &skills {
        crate::db::manas::store_skill(&state.conn, skill)
            .expect("store_skill failed");
    }

    // 4. Generate and store memories; collect IDs
    let memory_specs = generate_memories(seed);
    let mut memory_ids: Vec<(MemoryType, String, String)> = Vec::with_capacity(30);
    let mut decision_file_map: Vec<(String, String)> = Vec::new();

    for spec in &memory_specs {
        let resp = handle_request(
            state,
            Request::Remember {
                memory_type: spec.memory_type.clone(),
                title: spec.title.clone(),
                content: spec.content.clone(),
                confidence: Some(spec.confidence),
                tags: Some(spec.tags.clone()),
                project: None,
                metadata: None,
            },
        );
        match resp {
            Response::Ok {
                data: ResponseData::Stored { id },
            } => {
                memory_ids.push((spec.memory_type.clone(), id.clone(), spec.title.clone()));

                // For decisions: create an explicit affects edge to the file path.
                // The handler may already create one if the content matches its regex,
                // but we add one unconditionally to guarantee the edge exists.
                if let Some(ref fp) = spec.file_path {
                    let target = format!("file:{fp}");
                    let _ = crate::db::ops::store_edge(
                        &state.conn,
                        &id,
                        &target,
                        "affects",
                        "{}",
                    );
                    decision_file_map.push((id, fp.clone()));
                }
            }
            other => panic!("Remember failed: {other:?}"),
        }
    }

    // 5. Store domain DNA
    let dna_entries = generate_domain_dna(seed);
    let domain_dna_aspects: Vec<String> = dna_entries.iter().map(|d| d.aspect.clone()).collect();
    for dna in &dna_entries {
        crate::db::manas::store_domain_dna(&state.conn, dna)
            .expect("store_domain_dna failed");
    }

    SeededDataset {
        present_tools,
        absent_keywords,
        skills,
        memory_ids,
        decision_file_map,
        domain_dna_aspects,
        file_paths: FILE_PATHS.iter().map(|s| s.to_string()).collect(),
        session_id,
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_generate_tools_splits_present_and_absent() {
        let (present, absent) = generate_tools(42);

        // Correct counts
        assert_eq!(present.len(), 6, "expected 6 present tools");
        assert_eq!(absent.len(), 6, "expected 6 absent keywords");

        // All from the hardcoded-12
        let all_kw: HashSet<&str> = HARDCODED_TOOL_KEYWORDS.iter().copied().collect();
        for tool in &present {
            assert!(
                all_kw.contains(tool.name.as_str()),
                "present tool '{}' not in hardcoded-12",
                tool.name
            );
        }
        for kw in &absent {
            assert!(
                all_kw.contains(kw.as_str()),
                "absent keyword '{kw}' not in hardcoded-12"
            );
        }

        // No overlap between present names and absent keywords
        let present_names: HashSet<&str> = present.iter().map(|t| t.name.as_str()).collect();
        let absent_set: HashSet<&str> = absent.iter().map(|s| s.as_str()).collect();
        assert!(
            present_names.is_disjoint(&absent_set),
            "present and absent must not overlap"
        );

        // Together they cover all 12
        let union: HashSet<&str> = present_names.union(&absent_set).copied().collect();
        assert_eq!(union.len(), 12, "present + absent must cover all 12");

        // Deterministic: same seed produces same split
        let (present2, absent2) = generate_tools(42);
        assert_eq!(
            present.iter().map(|t| &t.name).collect::<Vec<_>>(),
            present2.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
        assert_eq!(absent, absent2);
    }

    #[test]
    fn test_generate_skills_tags_tool_dependencies() {
        let (present, absent) = generate_tools(42);
        let skills = generate_skills(42, &present, &absent);

        assert_eq!(skills.len(), 30, "expected 30 skills");

        // Tier A (0..10): each mentions a present tool keyword
        let present_names: HashSet<&str> = present.iter().map(|t| t.name.as_str()).collect();
        for skill in &skills[..10] {
            let text = format!("{} {} {}", skill.name, skill.description, skill.domain)
                .to_lowercase();
            let mentions_present = present_names.iter().any(|kw| text.contains(kw));
            assert!(
                mentions_present,
                "tier-A skill '{}' should mention a present tool",
                skill.id
            );
        }

        // Tier B (10..20): each mentions an absent tool keyword
        let absent_set: HashSet<&str> = absent.iter().map(|s| s.as_str()).collect();
        for skill in &skills[10..20] {
            let text = format!("{} {} {}", skill.name, skill.description, skill.domain)
                .to_lowercase();
            let mentions_absent = absent_set.iter().any(|kw| text.contains(kw));
            assert!(
                mentions_absent,
                "tier-B skill '{}' should mention an absent tool",
                skill.id
            );
        }

        // Tier C (20..30): must NOT mention any hardcoded keyword
        let all_kw: HashSet<&str> = HARDCODED_TOOL_KEYWORDS.iter().copied().collect();
        for skill in &skills[20..30] {
            let text = format!("{} {} {}", skill.name, skill.description, skill.domain)
                .to_lowercase();
            for kw in &all_kw {
                assert!(
                    !text.contains(kw),
                    "tier-C skill '{}' must not mention hardcoded keyword '{kw}'",
                    skill.id
                );
            }
        }

        // All skills have success_count > 0
        for skill in &skills {
            assert!(
                skill.success_count > 0,
                "skill '{}' must have success_count > 0",
                skill.id
            );
        }
    }

    #[test]
    fn test_generate_memories_creates_decisions_lessons_patterns() {
        let specs = generate_memories(42);
        assert_eq!(specs.len(), 30, "expected 30 memory specs");

        let decisions: Vec<_> = specs
            .iter()
            .filter(|s| matches!(s.memory_type, MemoryType::Decision))
            .collect();
        let lessons: Vec<_> = specs
            .iter()
            .filter(|s| matches!(s.memory_type, MemoryType::Lesson))
            .collect();
        let patterns: Vec<_> = specs
            .iter()
            .filter(|s| matches!(s.memory_type, MemoryType::Pattern))
            .collect();

        assert_eq!(decisions.len(), 10, "expected 10 decisions");
        assert_eq!(lessons.len(), 10, "expected 10 lessons");
        assert_eq!(patterns.len(), 10, "expected 10 patterns");

        // Decisions have file_path set
        for d in &decisions {
            assert!(
                d.file_path.is_some(),
                "decision '{}' must have a file_path",
                d.title
            );
        }

        // Lessons have at least one completion-relevant tag
        let completion_set: HashSet<&str> = COMPLETION_TAGS.iter().copied().collect();
        for l in &lessons {
            let has_completion_tag = l.tags.iter().any(|t| completion_set.contains(t.as_str()));
            assert!(
                has_completion_tag,
                "lesson '{}' must have a completion tag from {:?}, got {:?}",
                l.title, COMPLETION_TAGS, l.tags
            );
        }

        // All content tokens are unique (SHA-256 hex in content)
        let contents: HashSet<&str> = specs.iter().map(|s| s.content.as_str()).collect();
        assert_eq!(contents.len(), 30, "all memory contents must be unique");
    }

    #[test]
    fn test_seed_state_populates_all_categories() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let dataset = seed_state(&mut state, 42);

        // Tools
        assert_eq!(dataset.present_tools.len(), 6, "6 present tools");
        assert_eq!(dataset.absent_keywords.len(), 6, "6 absent keywords");

        // Verify only present tools are in DB
        let db_tools = crate::db::manas::list_tools(&state.conn, None)
            .expect("list_tools");
        let db_names: HashSet<String> = db_tools.into_iter().map(|t| t.name).collect();
        for tool in &dataset.present_tools {
            assert!(
                db_names.contains(&tool.name),
                "present tool '{}' should be in DB",
                tool.name
            );
        }
        for kw in &dataset.absent_keywords {
            assert!(
                !db_names.contains(kw),
                "absent keyword '{kw}' should NOT be in DB"
            );
        }

        // Skills
        assert_eq!(dataset.skills.len(), 30, "30 skills");
        let db_skills = crate::db::manas::list_skills(&state.conn, None)
            .expect("list_skills");
        let bench_skills: Vec<_> = db_skills
            .iter()
            .filter(|s| s.id.starts_with("bench-skill-"))
            .collect();
        assert_eq!(bench_skills.len(), 30, "30 bench skills in DB");

        // Memories
        assert_eq!(dataset.memory_ids.len(), 30, "30 memory IDs");
        let decisions: Vec<_> = dataset
            .memory_ids
            .iter()
            .filter(|(t, _, _)| matches!(t, MemoryType::Decision))
            .collect();
        let lessons: Vec<_> = dataset
            .memory_ids
            .iter()
            .filter(|(t, _, _)| matches!(t, MemoryType::Lesson))
            .collect();
        let patterns: Vec<_> = dataset
            .memory_ids
            .iter()
            .filter(|(t, _, _)| matches!(t, MemoryType::Pattern))
            .collect();
        assert_eq!(decisions.len(), 10);
        assert_eq!(lessons.len(), 10);
        assert_eq!(patterns.len(), 10);

        // Affects edges
        assert_eq!(
            dataset.decision_file_map.len(),
            10,
            "10 decision-file edges"
        );

        // Domain DNA
        assert_eq!(dataset.domain_dna_aspects.len(), 5, "5 domain DNA aspects");
        let db_dna = crate::db::manas::list_domain_dna(&state.conn, Some("forge-context-bench"))
            .expect("list_domain_dna");
        assert_eq!(db_dna.len(), 5, "5 domain DNA entries in DB");

        // File paths
        assert_eq!(dataset.file_paths.len(), 5, "5 file paths");

        // Session
        assert_eq!(dataset.session_id, format!("forge-context-bench-42"));
    }
}
