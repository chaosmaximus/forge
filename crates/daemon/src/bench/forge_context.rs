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

use std::collections::HashSet;

use forge_core::protocol::{Request, Response, ResponseData};
use forge_core::types::manas::{DomainDna, Skill, Tool, ToolHealth, ToolKind};
use forge_core::types::memory::MemoryType;
use rand::seq::SliceRandom;

use super::common::{seeded_rng, sha256_hex};

// ── Constants ──────────────────────────────────────────────────────

/// The 12 tool keywords hardcoded in `recall.rs` skill filtering.
const HARDCODED_TOOL_KEYWORDS: [&str; 12] = [
    "docker",
    "kubectl",
    "terraform",
    "npm",
    "cargo",
    "pip",
    "gcloud",
    "aws",
    "ssh",
    "make",
    "scp",
    "rsync",
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
/// Each must match one of the LIKE patterns in the CompletionCheck handler
/// (handler.rs:2185): `%testing%`, `%production-readiness%`, `%anti-pattern%`,
/// `%uat%`, `%deployment%`.
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
    pub seed: u64,
    pub present_tools: Vec<Tool>,
    pub absent_keywords: Vec<String>,
    pub skills: Vec<Skill>,
    pub memory_ids: Vec<(MemoryType, String, String)>, // (type, id, title)
    pub decision_file_map: Vec<(String, String)>,      // (memory_id, file_path)
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
            description: format!("Uses {tool_name} for {domain} operations. Token: {token}"),
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
            ..Default::default()
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
            description: format!("Requires {tool_kw} for {domain} automation. Token: {token}"),
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
            ..Default::default()
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
            description: format!("Domain-only guidance for {domain} layer. Token: {token}"),
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
            ..Default::default()
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
            // Full 64-char token in title to resist semantic dedup.
            // Truncating to 8 chars caused same-domain pairs to exceed
            // the 0.65 Jaccard threshold (adversarial review CRITICAL-1).
            title: format!("Decision: {domain} layer architecture ({token}) — affects {file}"),
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
            title: format!("Lesson: {domain} {completion_tag} insight ({token})"),
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
            title: format!("Pattern: {domain} convention ({token})"),
            content: format!("Observed recurring {domain} pattern. Unique token: {token}"),
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
pub fn seed_state(state: &mut DaemonState, seed: u64) -> Result<SeededDataset, String> {
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
    if !matches!(resp, Response::Ok { .. }) {
        return Err(format!("RegisterSession failed: {resp:?}"));
    }

    // 2. Generate and store tools; purge auto-detected tools that conflict
    //    with our "absent" set so the split is deterministic.
    let (present_tools, absent_keywords) = generate_tools(seed);

    // Assert the tool table is empty (in-memory DB should have no tools
    // unless DaemonState::new auto-detects them). If tools exist, clear
    // them for determinism. This only runs against in-memory DBs in the
    // bench harness, never against production file-backed DBs.
    let tool_count: usize = state
        .conn
        .query_row("SELECT COUNT(*) FROM tool", [], |r| r.get(0))
        .map_err(|e| format!("count tools: {e}"))?;
    if tool_count > 0 {
        state
            .conn
            .execute("DELETE FROM tool", [])
            .map_err(|e| format!("clear tool table: {e}"))?;
    }

    for tool in &present_tools {
        let resp = handle_request(state, Request::StoreTool { tool: tool.clone() });
        if !matches!(resp, Response::Ok { .. }) {
            return Err(format!("StoreTool failed: {resp:?}"));
        }
    }

    // 3. Generate and store skills
    let skills = generate_skills(seed, &present_tools, &absent_keywords);
    for skill in &skills {
        crate::db::manas::store_skill(&state.conn, skill)
            .map_err(|e| format!("store_skill failed: {e}"))?;
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
                valence: None,
                intensity: None,
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
                    let _ = crate::db::ops::store_edge(&state.conn, &id, &target, "affects", "{}");
                    decision_file_map.push((id, fp.clone()));
                }
            }
            other => return Err(format!("Remember failed: {other:?}")),
        }
    }

    // 5. Store domain DNA
    let dna_entries = generate_domain_dna(seed);
    let domain_dna_aspects: Vec<String> = dna_entries.iter().map(|d| d.aspect.clone()).collect();
    for dna in &dna_entries {
        crate::db::manas::store_domain_dna(&state.conn, dna)
            .map_err(|e| format!("store_domain_dna failed: {e}"))?;
    }

    Ok(SeededDataset {
        seed,
        present_tools,
        absent_keywords,
        skills,
        memory_ids,
        decision_file_map,
        domain_dna_aspects,
        file_paths: FILE_PATHS.iter().map(|s| s.to_string()).collect(),
        session_id,
    })
}

// ── Query bank types ──────────────────────────────────────────────

/// Scoring dimensions for the Forge-Context benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    ContextAssembly,
    Guardrails,
    Completion,
    LayerRecall,
}

/// A single query with ground-truth expected results.
pub struct QueryCase {
    pub id: String,
    pub dimension: Dimension,
    pub request: Request,
    /// Expected items that should appear in the response.
    /// Expressed in the EXACT format the daemon produces.
    pub expected: HashSet<String>,
}

// ── Query bank generator ──────────────────────────────────────────

/// Generate a bank of queries with ground-truth annotations across 4 dimensions.
///
/// The ground truth is intentionally conservative: only items that DEFINITELY
/// appear in the daemon response are included. BM25/FTS5 ranking can be
/// non-deterministic for ties, so we only assert on items whose ranking
/// position is unambiguous.
pub fn generate_query_bank(dataset: &SeededDataset) -> Vec<QueryCase> {
    let seed = dataset.seed;
    let mut cases = Vec::new();

    // ── Dimension 1: Context Assembly (CA-1..CA-6) ────────────────

    // CA-1..CA-3: CompileContext with focus for 3 domains.
    // Focus uses FTS5 MATCH to filter decisions only.
    // Expected: decision TITLES whose content/title match the focus domain.
    for (ca_idx, &domain) in DOMAINS[..3].iter().enumerate() {
        let id = format!("CA-{}", ca_idx + 1);

        // Decisions matching this domain: decisions with domain tag in title/content.
        // Decision titles contain the domain name, so FTS5 MATCH on the domain
        // will find them. Decisions cycle through DOMAINS[i % 5], so for a given
        // domain, indices i where i % 5 == domain_index will match.
        let domain_idx = ca_idx; // DOMAINS[0..3] = auth, database, networking
        let expected: HashSet<String> = (0..10)
            .filter(|&i| i % DOMAINS.len() == domain_idx)
            .map(|i| {
                let token = sha256_hex(&format!("forge-context-{seed}-decision-{i}"));
                let file = FILE_PATHS[i % FILE_PATHS.len()];
                format!("Decision: {domain} layer architecture ({token}) — affects {file}")
            })
            .collect();

        cases.push(QueryCase {
            id,
            dimension: Dimension::ContextAssembly,
            request: Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
                // Exclude skills: focus does NOT filter skills (they're
                // fetched independently via list_skills). Including them
                // would degrade precision since we only expect decisions.
                // Adversarial review CRITICAL-2.
                excluded_layers: Some(vec!["skills".to_string()]),
                session_id: Some(dataset.session_id.clone()),
                focus: Some(domain.to_string()),
                cwd: None,
                dry_run: None,
            },
            expected,
        });
    }

    // CA-4..CA-6: CompileContext without focus — check tool filtering on skills.
    // Skills are fetched independently (not filtered by focus). Present-tool skills
    // (Tier A) and no-tool skills (Tier C) should survive. Absent-tool skills
    // (Tier B) should be filtered out.
    //
    // The skill XML format is: <skill domain="{domain}" uses="{success_count}">{name}</skill>
    // We check that NO Tier B skill names appear in the output.
    // We can't predict exactly which 5 skills appear (ordering depends on list_skills
    // iteration order + take(5)), so we only assert on the ABSENCE of Tier B.
    for ca_idx in 0..3 {
        let id = format!("CA-{}", ca_idx + 4);

        // Expected: the Tier B skill names that must NOT appear.
        // We encode these as "ABSENT:{name}" — the scorer will check for absence.
        let expected: HashSet<String> = dataset.skills[10..20]
            .iter()
            .map(|s| format!("ABSENT:{}", s.name))
            .collect();

        cases.push(QueryCase {
            id,
            dimension: Dimension::ContextAssembly,
            request: Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
                excluded_layers: None,
                session_id: Some(dataset.session_id.clone()),
                focus: None,
                cwd: None,
                dry_run: None,
            },
            expected,
        });
    }

    // ── Dimension 2: Guardrails (GR-1..GR-10) ────────────────────

    // GR-1..GR-5: PostEditCheck for each file path.
    // Expected: decisions_to_review (decision TITLES linked via affects edges)
    //           and applicable_skills formatted as "Skill: {name} ({domain})".
    for (gr_idx, file) in FILE_PATHS.iter().enumerate() {
        let id = format!("GR-{}", gr_idx + 1);
        let mut expected = HashSet::new();

        // decisions_to_review: decision titles linked to this file via affects edges.
        // Each decision is linked to FILE_PATHS[i % 5]. So for file_path at index fp_idx,
        // decisions with i % 5 == fp_idx are linked.
        let fp_idx = gr_idx;
        for i in 0..10 {
            if i % FILE_PATHS.len() == fp_idx {
                let domain = DOMAINS[i % DOMAINS.len()];
                let token = sha256_hex(&format!("forge-context-{seed}-decision-{i}"));
                expected.insert(format!(
                    "Decision: {domain} layer architecture ({token}) — affects {file}"
                ));
            }
        }

        // applicable_skills: "Skill: {name} ({domain})" for skills matching file path.
        // find_applicable_skills parses search terms from the file path and matches
        // via LIKE on name/description/domain. The domain component (e.g., "auth")
        // matches all skills with that domain across all tiers (A, B, C).
        //
        // For domain_idx, matching skill indices are domain_idx and domain_idx+5
        // (i % 5 == domain_idx). Each tier has two such skills:
        //   - i=domain_idx:   success_count = domain_idx + 1
        //   - i=domain_idx+5: success_count = domain_idx + 6
        //
        // Across 3 tiers, there are 3 skills with success_count = domain_idx+6
        // (tied). ORDER BY success_count DESC LIMIT 2 picks the first two by
        // rowid (insertion order): Tier A i=domain_idx+5, then Tier B i=domain_idx+5.
        let domain_idx = gr_idx;
        let domain = DOMAINS[domain_idx];
        let skill_i = domain_idx + 5;

        // Tier A skill (present-tool): index skill_i in Tier A
        let skill_a = &dataset.skills[skill_i];
        expected.insert(format!("Skill: {} ({domain})", skill_a.name));

        // Tier B skill (absent-tool): index skill_i in Tier B (offset by 10)
        let skill_b = &dataset.skills[10 + skill_i];
        expected.insert(format!("Skill: {} ({domain})", skill_b.name));

        cases.push(QueryCase {
            id,
            dimension: Dimension::Guardrails,
            request: Request::PostEditCheck {
                file: file.to_string(),
                session_id: None,
            },
            expected,
        });
    }

    // GR-6..GR-8: PreBashCheck for commands.
    // Expected: relevant_skills formatted as "{name} ({domain})" (NO "Skill: " prefix).
    // The pre_bash_check searches for %{cmd_name}% in skill name/description/domain.
    let bash_commands = ["cargo test", "npm run build", "docker compose up"];
    for (gr_idx, cmd) in bash_commands.iter().enumerate() {
        let id = format!("GR-{}", gr_idx + 6);

        // Extract the first word of the command for matching.
        let cmd_name = cmd.split_whitespace().next().unwrap_or("");

        // Find skills that match %{cmd_name}% in name/description/domain.
        // Only Tier A skills mention tool names. Tier B also mentions tool names
        // (absent ones). But pre_bash_check doesn't filter by tool availability.
        // It uses: `WHERE success_count > 0 AND (description LIKE ?1 OR name LIKE ?1 OR domain LIKE ?1)`
        // ordered by success_count DESC LIMIT 2.
        let mut expected = HashSet::new();
        let mut matching_skills: Vec<&Skill> = dataset
            .skills
            .iter()
            .filter(|s| {
                let text = format!("{} {} {}", s.name, s.description, s.domain).to_lowercase();
                text.contains(cmd_name)
            })
            .collect();

        // Sort by success_count DESC (stable — preserves insertion order for ties),
        // then take top 2 to match the SQL LIMIT 2.
        matching_skills.sort_by(|a, b| b.success_count.cmp(&a.success_count));
        for skill in matching_skills.iter().take(2) {
            // PreBashCheck relevant_skills format: "{name} ({domain})"
            expected.insert(format!("{} ({})", skill.name, skill.domain));
        }

        cases.push(QueryCase {
            id,
            dimension: Dimension::Guardrails,
            request: Request::PreBashCheck {
                command: cmd.to_string(),
                session_id: None,
            },
            expected,
        });
    }

    // GR-9..GR-10: GuardrailsCheck for 2 files.
    // Expected: decisions_affected contains decision IDs (not titles!),
    //           and applicable_skills formatted as "Skill: {name} ({domain})".
    for (gr_idx, &file) in FILE_PATHS[..2].iter().enumerate() {
        let id = format!("GR-{}", gr_idx + 9);
        let mut expected = HashSet::new();

        // decisions_affected: decision IDs linked via affects edges.
        for (did, fpath) in &dataset.decision_file_map {
            if fpath == file {
                expected.insert(did.clone());
            }
        }

        // applicable_skills: same logic as GR-1..GR-5.
        // For FILE_PATHS[0] ("src/auth/middleware.rs") → domain_idx=0
        // For FILE_PATHS[1] ("src/database/schema.rs") → domain_idx=1
        let domain_idx = gr_idx;
        let domain = DOMAINS[domain_idx];
        let skill_i = domain_idx + 5;

        let skill_a = &dataset.skills[skill_i];
        expected.insert(format!("Skill: {} ({domain})", skill_a.name));

        let skill_b = &dataset.skills[10 + skill_i];
        expected.insert(format!("Skill: {} ({domain})", skill_b.name));

        cases.push(QueryCase {
            id,
            dimension: Dimension::Guardrails,
            request: Request::GuardrailsCheck {
                file: file.to_string(),
                action: "edit".to_string(),
            },
            expected,
        });
    }

    // ── Dimension 3: Completion (CO-1..CO-5) ──────────────────────

    // CO-1..CO-3: CompletionCheck with claimed_done=true.
    // Returns relevant_lessons as "title: content_substr(150)".
    // SQL: memory_type IN ('lesson', 'decision'), tags matching completion tags,
    //   (tags LIKE '%testing%' OR '%production-readiness%' OR '%anti-pattern%'
    //    OR '%uat%' OR '%deployment%')
    // ORDER BY quality_score DESC, confidence DESC LIMIT 3.
    //
    // All memories have quality_score=0.5 (default), so ordering is by confidence DESC.
    // We need to find the top-3 memories (lessons+decisions) that match the tag filter.
    //
    // Matching decisions: those with tag "testing" or "deployment" in their domain:
    //   Decision i=3: tag="testing", conf=0.88
    //   Decision i=4: tag="deployment", conf=0.89
    //   Decision i=8: tag="testing", conf=0.93
    //   Decision i=9: tag="deployment", conf=0.94
    //
    // Matching lessons: ALL 10 lessons have a completion tag.
    //   COMPLETION_TAGS = ["testing","uat","deployment","anti-pattern","production-readiness"]
    //   Lesson i=0: "testing" (0.75), i=1: "uat" (0.77), i=2: "deployment" (0.79),
    //   i=3: "anti-pattern" (0.81), ..., i=9: "production-readiness" (0.93)
    //
    // Combined and sorted by confidence DESC:
    //   Decision i=9: 0.94
    //   Decision i=8: 0.93, Lesson i=9: 0.93 (tie — decisions inserted first, lower rowid wins)
    //   Lesson i=8: 0.91
    //
    // Top 3: Decision i=9 (0.94), Decision i=8 (0.93), Lesson i=9 (0.93)
    for co_idx in 0..3 {
        let id = format!("CO-{}", co_idx + 1);
        let mut expected = HashSet::new();

        // Decision i=9: tag="deployment", conf=0.94 (highest)
        let token_d9 = sha256_hex(&format!("forge-context-{seed}-decision-9"));
        let domain_d9 = DOMAINS[9 % DOMAINS.len()]; // "deployment"
        let file_d9 = FILE_PATHS[9 % FILE_PATHS.len()]; // "src/deployment/config.rs"
        let title_d9 =
            format!("Decision: {domain_d9} layer architecture ({token_d9}) — affects {file_d9}");
        let content_d9 = format!(
            "Decided to refactor {domain_d9} layer via {file_d9}. Unique token: {token_d9}"
        );
        let content_substr_d9: String = content_d9.chars().take(150).collect();
        expected.insert(format!("{title_d9}: {content_substr_d9}"));

        // Decision i=8: tag="testing", conf=0.93 (ties with Lesson i=9 but lower rowid)
        let token_d8 = sha256_hex(&format!("forge-context-{seed}-decision-8"));
        let domain_d8 = DOMAINS[8 % DOMAINS.len()]; // "testing"
        let file_d8 = FILE_PATHS[8 % FILE_PATHS.len()]; // "src/testing/harness.rs"
        let title_d8 =
            format!("Decision: {domain_d8} layer architecture ({token_d8}) — affects {file_d8}");
        let content_d8 = format!(
            "Decided to refactor {domain_d8} layer via {file_d8}. Unique token: {token_d8}"
        );
        let content_substr_d8: String = content_d8.chars().take(150).collect();
        expected.insert(format!("{title_d8}: {content_substr_d8}"));

        // Lesson i=9: tag="production-readiness", conf=0.93 (loses tie to Decision i=8)
        let token_l9 = sha256_hex(&format!("forge-context-{seed}-lesson-9"));
        let domain_l9 = DOMAINS[9 % DOMAINS.len()]; // "deployment"
        let ctag_l9 = COMPLETION_TAGS[9 % COMPLETION_TAGS.len()]; // "production-readiness"
        let title_l9 = format!("Lesson: {domain_l9} {ctag_l9} insight ({token_l9})");
        let content_l9 =
            format!("Learned about {ctag_l9} in {domain_l9} context. Unique token: {token_l9}");
        let content_substr_l9: String = content_l9.chars().take(150).collect();
        expected.insert(format!("{title_l9}: {content_substr_l9}"));

        cases.push(QueryCase {
            id,
            dimension: Dimension::Completion,
            request: Request::CompletionCheck {
                session_id: dataset.session_id.clone(),
                claimed_done: true,
            },
            expected,
        });
    }

    // CO-4..CO-5: TaskCompletionCheck with shipping-related subjects.
    // Returns checklists as lesson TITLES (SELECT title).
    // SQL: memory_type = 'lesson', tags LIKE '%uat%' OR '%production%' OR '%deployment%',
    // ORDER BY quality_score DESC LIMIT 3.
    //
    // With COMPLETION_TAGS = ["testing","uat","deployment","anti-pattern","production-readiness"],
    // matching lessons (tag at i % 5):
    //   i=1: tag="uat" → matches %uat%
    //   i=2: tag="deployment" → matches %deployment%
    //   i=4: tag="production-readiness" → matches %production%
    //   i=6: tag="uat" → matches %uat%
    //   i=7: tag="deployment" → matches %deployment%
    //   i=9: tag="production-readiness" → matches %production%
    //
    // All quality_score=0.5. ORDER BY quality_score DESC gives them all equal.
    // SQLite returns by rowid in tie: i=1, i=2, i=4 (top 3 by insertion order).
    let task_subjects = ["deploy to production", "ship the release"];
    for (co_idx, subject) in task_subjects.iter().enumerate() {
        let id = format!("CO-{}", co_idx + 4);
        let mut expected = HashSet::new();

        // Include the top 3 matching lessons by insertion order.
        // Lesson i=1: tag="uat"
        let token_l1 = sha256_hex(&format!("forge-context-{seed}-lesson-1"));
        let domain_l1 = DOMAINS[1 % DOMAINS.len()]; // "database"
        let ctag_l1 = COMPLETION_TAGS[1 % COMPLETION_TAGS.len()]; // "uat"
        expected.insert(format!(
            "Lesson: {domain_l1} {ctag_l1} insight ({token_l1})"
        ));

        // Lesson i=2: tag="deployment"
        let token_l2 = sha256_hex(&format!("forge-context-{seed}-lesson-2"));
        let domain_l2 = DOMAINS[2 % DOMAINS.len()]; // "networking"
        let ctag_l2 = COMPLETION_TAGS[2 % COMPLETION_TAGS.len()]; // "deployment"
        expected.insert(format!(
            "Lesson: {domain_l2} {ctag_l2} insight ({token_l2})"
        ));

        // Lesson i=4: tag="production-readiness"
        let token_l4 = sha256_hex(&format!("forge-context-{seed}-lesson-4"));
        let domain_l4 = DOMAINS[4 % DOMAINS.len()]; // "deployment"
        let ctag_l4 = COMPLETION_TAGS[4 % COMPLETION_TAGS.len()]; // "production-readiness"
        expected.insert(format!(
            "Lesson: {domain_l4} {ctag_l4} insight ({token_l4})"
        ));

        cases.push(QueryCase {
            id,
            dimension: Dimension::Completion,
            request: Request::TaskCompletionCheck {
                session_id: dataset.session_id.clone(),
                task_subject: subject.to_string(),
                task_description: None,
            },
            expected,
        });
    }

    // ── Dimension 4: Layer Recall (LR-1..LR-8) ───────────────────

    // LR-1..LR-5: Recall with layer="skill", query="{domain} workflow".
    // search_skills does LIKE %{query}% on name/description/domain.
    // Tier A skill names: "{tool_name} {domain} workflow {token}" → matches "{domain} workflow".
    // Returns MemoryResult with title "[skill:{domain}] {name}".
    // Ordered by success_count DESC LIMIT 5.
    for (lr_idx, &domain) in DOMAINS.iter().enumerate() {
        let id = format!("LR-{}", lr_idx + 1);
        let query = format!("{domain} workflow");

        // Find Tier A skills matching this domain (domain in name as "{domain} workflow")
        let domain_idx = lr_idx;
        let mut expected = HashSet::new();

        for i in 0..10 {
            if i % DOMAINS.len() == domain_idx {
                let skill = &dataset.skills[i]; // Tier A: indices 0..9
                expected.insert(format!("[skill:{}] {}", skill.domain, skill.name));
            }
        }

        cases.push(QueryCase {
            id,
            dimension: Dimension::LayerRecall,
            request: Request::Recall {
                query,
                memory_type: None,
                project: None,
                limit: Some(10),
                layer: Some("skill".to_string()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
            expected,
        });
    }

    // LR-6..LR-8: Recall with layer="domain_dna", query="{domain}".
    // Filters by pattern.to_lowercase().contains(query_lower).
    // DNA patterns: "{domain} uses standard patterns. Token: {hash}"
    // Query "{domain}" matches patterns containing that domain.
    // Returns MemoryResult with title "[dna:{aspect}] {pattern}".
    for (lr_idx, &domain) in DOMAINS[..3].iter().enumerate() {
        let id = format!("LR-{}", lr_idx + 6);
        let query = domain.to_string();

        let dna_token = sha256_hex(&format!("forge-context-{seed}-dna-{lr_idx}"));
        let aspect = format!("{domain}-conventions");
        let pattern = format!("{domain} uses standard patterns. Token: {dna_token}");

        let mut expected = HashSet::new();
        expected.insert(format!("[dna:{aspect}] {pattern}"));

        cases.push(QueryCase {
            id,
            dimension: Dimension::LayerRecall,
            request: Request::Recall {
                query,
                memory_type: None,
                project: None,
                limit: Some(10),
                layer: Some("domain_dna".to_string()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
            expected,
        });
    }

    cases
}

// ── Result extractors (Task 4) ───────────────────────────────────

/// Normalize a daemon `Response` into a flat set of strings for scoring.
///
/// Each response variant produces items in the EXACT format the daemon
/// emits, so the scorer can intersect them with ground-truth expectations.
pub fn extract_result_items(response: &Response) -> Result<HashSet<String>, String> {
    match response {
        Response::Error { message } => Err(format!("daemon returned error: {message}")),
        Response::Ok { data } => match data {
            ResponseData::GuardrailsCheck {
                decisions_affected,
                relevant_lessons,
                applicable_skills,
                ..
            } => {
                let mut items = HashSet::new();
                for s in decisions_affected {
                    items.insert(s.clone());
                }
                for s in relevant_lessons {
                    items.insert(s.clone());
                }
                for s in applicable_skills {
                    items.insert(s.clone());
                }
                Ok(items)
            }
            ResponseData::PostEditChecked {
                applicable_skills,
                decisions_to_review,
                relevant_lessons,
                ..
            } => {
                let mut items = HashSet::new();
                for s in applicable_skills {
                    items.insert(s.clone());
                }
                for s in decisions_to_review {
                    items.insert(s.clone());
                }
                for s in relevant_lessons {
                    items.insert(s.clone());
                }
                Ok(items)
            }
            ResponseData::PreBashChecked {
                relevant_skills, ..
            } => {
                let mut items = HashSet::new();
                for s in relevant_skills {
                    items.insert(s.clone());
                }
                Ok(items)
            }
            ResponseData::CompletionCheckResult {
                relevant_lessons, ..
            } => {
                let mut items = HashSet::new();
                for s in relevant_lessons {
                    items.insert(s.clone());
                }
                Ok(items)
            }
            ResponseData::TaskCompletionCheckResult { checklists, .. } => {
                let mut items = HashSet::new();
                for s in checklists {
                    items.insert(s.clone());
                }
                Ok(items)
            }
            ResponseData::Memories { results, .. } => {
                let items: HashSet<String> =
                    results.iter().map(|mr| mr.memory.title.clone()).collect();
                Ok(items)
            }
            ResponseData::CompiledContext { context, .. } => {
                let items: HashSet<String> =
                    extract_from_compiled_context(context).into_iter().collect();
                Ok(items)
            }
            other => Err(format!(
                "unhandled response variant for extraction: {other:?}"
            )),
        },
    }
}

/// Parse CompileContext XML output and extract skill names and decision titles.
///
/// Skills appear as: `<skill domain="X" uses="N">name</skill>`
/// Decisions appear as: `<decision confidence="X.Y">title</decision>`
pub fn extract_from_compiled_context(context: &str) -> Vec<String> {
    let mut items = Vec::new();

    // Extract skill names from <skill ...>name</skill>
    for cap in context.split("<skill ") {
        // Each split piece after the first starts with attributes>content</skill>
        if let Some(close_bracket) = cap.find('>') {
            let after_bracket = &cap[close_bracket + 1..];
            if let Some(end_tag) = after_bracket.find("</skill>") {
                let name = &after_bracket[..end_tag];
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    items.push(trimmed.to_string());
                }
            }
        }
    }

    // Extract decision titles from <decision ...>title</decision>
    for cap in context.split("<decision ") {
        if let Some(close_bracket) = cap.find('>') {
            let after_bracket = &cap[close_bracket + 1..];
            if let Some(end_tag) = after_bracket.find("</decision>") {
                let title = &after_bracket[..end_tag];
                let trimmed = title.trim();
                if !trimmed.is_empty() {
                    items.push(trimmed.to_string());
                }
            }
        }
    }

    items
}

// ── Scoring (Task 5) ─────────────────────────────────────────────

/// Standard IR precision / recall / F1.
///
/// - Both empty: `(1.0, 1.0, 1.0)` — nothing expected, nothing returned = perfect.
/// - Actual empty but expected non-empty: `(0.0, 0.0, 0.0)`.
/// - Otherwise: `precision = |intersection| / |actual|`, `recall = |intersection| / |expected|`,
///   `F1 = 2 * P * R / (P + R)`.
///
/// **ABSENT: prefix convention**: expected items starting with `"ABSENT:"` are
/// *absence assertions*. For each such item, we strip the prefix and check that
/// the remainder is NOT present in `actual`. A satisfied absence = a "match"
/// for scoring purposes; a violated absence (item found in actual) = a mismatch.
pub fn precision_recall_f1(
    expected: &HashSet<String>,
    actual: &HashSet<String>,
) -> (f64, f64, f64) {
    if expected.is_empty() && actual.is_empty() {
        return (1.0, 1.0, 1.0);
    }

    // Split expected into presence and absence assertions.
    let mut presence_expected = HashSet::new();
    let mut absence_expected = HashSet::new();
    for item in expected {
        if let Some(stripped) = item.strip_prefix("ABSENT:") {
            absence_expected.insert(stripped.to_string());
        } else {
            presence_expected.insert(item.clone());
        }
    }

    // Count matches: presence items found in actual + absence items NOT found in actual.
    let presence_matches = presence_expected.intersection(actual).count();
    let absence_matches = absence_expected
        .iter()
        .filter(|item| !actual.contains(*item))
        .count();
    let matched = presence_matches + absence_matches;

    let total_expected = presence_expected.len() + absence_expected.len();

    // For precision denominator: actual items + absence assertions (since absence
    // assertions are "virtual" items in the actual set). If there are only absence
    // assertions and no actual items, use absence count as the denominator.
    let precision_denom = if presence_expected.is_empty() && !absence_expected.is_empty() {
        // Pure absence queries: precision = matched / total_expected
        total_expected
    } else {
        actual.len() + absence_expected.len()
    };

    if precision_denom == 0 && total_expected > 0 {
        return (0.0, 0.0, 0.0);
    }
    if precision_denom == 0 {
        return (1.0, 1.0, 1.0);
    }

    let precision = matched as f64 / precision_denom as f64;
    let recall = if total_expected == 0 {
        1.0
    } else {
        matched as f64 / total_expected as f64
    };

    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    (precision, recall, f1)
}

/// Per-query scoring result.
pub struct QueryResult {
    pub id: String,
    pub dimension: Dimension,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub expected_count: usize,
    pub actual_count: usize,
    pub matched_count: usize,
}

/// Aggregate benchmark score for a single seed run.
#[derive(Debug, serde::Serialize)]
pub struct ContextScore {
    pub seed: u64,
    pub context_assembly_f1: f64,
    pub guardrails_f1: f64,
    pub completion_f1: f64,
    pub layer_recall_f1: f64,
    pub tool_filter_accuracy: f64,
    pub composite: f64,
    pub total_queries: usize,
    pub pass: bool,
}

/// Aggregate per-dimension F1 into a composite score.
///
/// Weights: `0.30 * context_assembly + 0.30 * guardrails + 0.20 * completion + 0.20 * layer_recall`
///
/// `tool_filter_accuracy` is passed through to the output but does NOT factor into
/// the composite (it is covered indirectly by context_assembly absence assertions).
pub fn compute_composite(results: &[QueryResult], tool_filter_accuracy: f64) -> ContextScore {
    let avg_f1_for = |dim: Dimension| -> f64 {
        let matching: Vec<f64> = results
            .iter()
            .filter(|r| r.dimension == dim)
            .map(|r| r.f1)
            .collect();
        if matching.is_empty() {
            0.0
        } else {
            matching.iter().sum::<f64>() / matching.len() as f64
        }
    };

    let context_assembly_f1 = avg_f1_for(Dimension::ContextAssembly);
    let guardrails_f1 = avg_f1_for(Dimension::Guardrails);
    let completion_f1 = avg_f1_for(Dimension::Completion);
    let layer_recall_f1 = avg_f1_for(Dimension::LayerRecall);

    let composite = 0.30 * context_assembly_f1
        + 0.30 * guardrails_f1
        + 0.20 * completion_f1
        + 0.20 * layer_recall_f1;

    ContextScore {
        seed: 0, // caller sets this
        context_assembly_f1,
        guardrails_f1,
        completion_f1,
        layer_recall_f1,
        tool_filter_accuracy,
        composite,
        total_queries: results.len(),
        pass: composite >= 0.5,
    }
}

// ── Orchestrator (Task 6) ─────────────────────────────────────────

/// Configuration for a Forge-Context benchmark run.
pub struct ContextConfig {
    pub seed: u64,
    pub output_dir: Option<std::path::PathBuf>,
}

/// Compute tool-filter accuracy from CA-4..CA-6 query results.
///
/// These queries use the `ABSENT:` prefix convention — they assert that
/// absent-tool skills do NOT leak into the compiled context. If all three
/// score F1 = 1.0, tool filtering is exact (accuracy = 1.0). Otherwise,
/// accuracy = fraction of CA-4..CA-6 queries with F1 = 1.0.
fn compute_tool_filter_accuracy(results: &[QueryResult]) -> f64 {
    let tool_filter_results: Vec<&QueryResult> = results
        .iter()
        .filter(|r| {
            r.dimension == Dimension::ContextAssembly
                && (r.id == "CA-4" || r.id == "CA-5" || r.id == "CA-6")
        })
        .collect();

    if tool_filter_results.is_empty() {
        return 1.0; // no tool-filter queries → vacuously exact
    }

    let passed = tool_filter_results
        .iter()
        .filter(|r| (r.f1 - 1.0).abs() < f64::EPSILON)
        .count();

    passed as f64 / tool_filter_results.len() as f64
}

/// Run the Forge-Context benchmark and return composite scores.
pub fn run(config: ContextConfig) -> Result<ContextScore, String> {
    let mut state = DaemonState::new(":memory:").map_err(|e| format!("DaemonState: {e}"))?;

    // 1. Seed the dataset
    let dataset = seed_state(&mut state, config.seed)?;

    // 2. Generate query bank with ground truth
    let queries = generate_query_bank(&dataset);

    // 3. Execute each query, extract results, score
    let mut results = Vec::new();
    for query in &queries {
        let response = handle_request(&mut state, query.request.clone());
        let actual =
            extract_result_items(&response).map_err(|e| format!("query {}: {e}", query.id))?;

        let (p, r, f1) = precision_recall_f1(&query.expected, &actual);
        results.push(QueryResult {
            id: query.id.clone(),
            dimension: query.dimension,
            precision: p,
            recall: r,
            f1,
            expected_count: query.expected.len(),
            actual_count: actual.len(),
            matched_count: query
                .expected
                .iter()
                .filter(|item| {
                    if let Some(stripped) = item.strip_prefix("ABSENT:") {
                        !actual.contains(stripped)
                    } else {
                        actual.contains(*item)
                    }
                })
                .count(),
        });
    }

    // 4. Compute tool-filter accuracy from CA-4..CA-6 results
    let tool_filter_accuracy = compute_tool_filter_accuracy(&results);

    // 5. Compute composite
    let mut score = compute_composite(&results, tool_filter_accuracy);
    score.seed = config.seed;

    // 6. Write output if requested
    if let Some(dir) = &config.output_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
        let json =
            serde_json::to_string_pretty(&score).map_err(|e| format!("json serialize: {e}"))?;
        std::fs::write(dir.join("summary.json"), &json)
            .map_err(|e| format!("write summary: {e}"))?;
    }

    Ok(score)
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_generate_query_bank_covers_all_dimensions() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let dataset = seed_state(&mut state, 42).expect("seed");
        let queries = generate_query_bank(&dataset);

        assert!(!queries.is_empty(), "query bank must not be empty");

        let dims: HashSet<Dimension> = queries.iter().map(|q| q.dimension).collect();
        assert!(
            dims.contains(&Dimension::ContextAssembly),
            "missing ContextAssembly"
        );
        assert!(dims.contains(&Dimension::Guardrails), "missing Guardrails");
        assert!(dims.contains(&Dimension::Completion), "missing Completion");
        assert!(
            dims.contains(&Dimension::LayerRecall),
            "missing LayerRecall"
        );

        // Count queries per dimension
        let ca_count = queries
            .iter()
            .filter(|q| q.dimension == Dimension::ContextAssembly)
            .count();
        let gr_count = queries
            .iter()
            .filter(|q| q.dimension == Dimension::Guardrails)
            .count();
        let co_count = queries
            .iter()
            .filter(|q| q.dimension == Dimension::Completion)
            .count();
        let lr_count = queries
            .iter()
            .filter(|q| q.dimension == Dimension::LayerRecall)
            .count();

        assert_eq!(
            ca_count, 6,
            "expected 6 ContextAssembly queries (CA-1..CA-6)"
        );
        assert_eq!(gr_count, 10, "expected 10 Guardrails queries (GR-1..GR-10)");
        assert_eq!(co_count, 5, "expected 5 Completion queries (CO-1..CO-5)");
        assert_eq!(lr_count, 8, "expected 8 LayerRecall queries (LR-1..LR-8)");

        // All query IDs are unique
        let ids: HashSet<&str> = queries.iter().map(|q| q.id.as_str()).collect();
        assert_eq!(ids.len(), queries.len(), "all query IDs must be unique");

        // Every query has at least one expected item
        for q in &queries {
            assert!(
                !q.expected.is_empty(),
                "query {} must have expected items",
                q.id
            );
        }
    }

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
            let text =
                format!("{} {} {}", skill.name, skill.description, skill.domain).to_lowercase();
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
            let text =
                format!("{} {} {}", skill.name, skill.description, skill.domain).to_lowercase();
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
            let text =
                format!("{} {} {}", skill.name, skill.description, skill.domain).to_lowercase();
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
        let dataset = seed_state(&mut state, 42).expect("seed_state");

        // Tools
        assert_eq!(dataset.present_tools.len(), 6, "6 present tools");
        assert_eq!(dataset.absent_keywords.len(), 6, "6 absent keywords");

        // Verify only present tools are in DB
        let db_tools = crate::db::manas::list_tools(&state.conn, None).expect("list_tools");
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
        let db_skills = crate::db::manas::list_skills(&state.conn, None).expect("list_skills");
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

    // ── Task 4 tests: result extractors ──

    #[test]
    fn test_extract_result_items_from_guardrails_check() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let _dataset = seed_state(&mut state, 42).expect("seed");
        let resp = handle_request(
            &mut state,
            Request::GuardrailsCheck {
                file: "src/auth/middleware.rs".to_string(),
                action: "edit".to_string(),
            },
        );
        let items = extract_result_items(&resp);
        assert!(
            items.is_ok(),
            "extraction should not error on valid response"
        );
    }

    #[test]
    fn test_extract_result_items_from_post_edit_checked() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let _dataset = seed_state(&mut state, 42).expect("seed");
        let resp = handle_request(
            &mut state,
            Request::PostEditCheck {
                file: "src/auth/middleware.rs".to_string(),
                session_id: None,
            },
        );
        let items = extract_result_items(&resp);
        assert!(
            items.is_ok(),
            "extraction should not error on PostEditChecked"
        );
    }

    #[test]
    fn test_extract_result_items_from_pre_bash_checked() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let _dataset = seed_state(&mut state, 42).expect("seed");
        let resp = handle_request(
            &mut state,
            Request::PreBashCheck {
                command: "cargo test".to_string(),
                session_id: None,
            },
        );
        let items = extract_result_items(&resp);
        assert!(
            items.is_ok(),
            "extraction should not error on PreBashChecked"
        );
    }

    #[test]
    fn test_extract_result_items_from_completion_check() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let dataset = seed_state(&mut state, 42).expect("seed");
        let resp = handle_request(
            &mut state,
            Request::CompletionCheck {
                session_id: dataset.session_id.clone(),
                claimed_done: true,
            },
        );
        let items = extract_result_items(&resp);
        assert!(
            items.is_ok(),
            "extraction should not error on CompletionCheckResult"
        );
    }

    #[test]
    fn test_extract_result_items_from_task_completion_check() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let dataset = seed_state(&mut state, 42).expect("seed");
        let resp = handle_request(
            &mut state,
            Request::TaskCompletionCheck {
                session_id: dataset.session_id.clone(),
                task_subject: "deploy to production".to_string(),
                task_description: None,
            },
        );
        let items = extract_result_items(&resp);
        assert!(
            items.is_ok(),
            "extraction should not error on TaskCompletionCheckResult"
        );
    }

    #[test]
    fn test_extract_result_items_from_memories() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let _dataset = seed_state(&mut state, 42).expect("seed");
        let resp = handle_request(
            &mut state,
            Request::Recall {
                query: "auth workflow".to_string(),
                memory_type: None,
                project: None,
                limit: Some(5),
                layer: Some("skill".to_string()),
                since: None,
                include_flipped: None,
                include_globals: None,
                query_embedding: None,
            },
        );
        let items = extract_result_items(&resp);
        assert!(items.is_ok(), "extraction should not error on Memories");
        let items = items.unwrap();
        // Should find at least one skill mentioning auth
        assert!(
            !items.is_empty(),
            "recall for 'auth workflow' in skill layer should return items"
        );
    }

    #[test]
    fn test_extract_result_items_from_compiled_context() {
        let mut state = DaemonState::new(":memory:").expect("state");
        let dataset = seed_state(&mut state, 42).expect("seed");
        let resp = handle_request(
            &mut state,
            Request::CompileContext {
                agent: None,
                project: None,
                static_only: None,
                excluded_layers: None,
                session_id: Some(dataset.session_id.clone()),
                focus: None,
                cwd: None,
                dry_run: None,
            },
        );
        let items = extract_result_items(&resp);
        assert!(
            items.is_ok(),
            "extraction should not error on CompiledContext"
        );
        let items = items.unwrap();
        // Should contain at least some decisions or skills
        assert!(!items.is_empty(), "compiled context should produce items");
    }

    #[test]
    fn test_extract_result_items_error_response() {
        let resp = Response::Error {
            message: "test error".to_string(),
        };
        let items = extract_result_items(&resp);
        assert!(items.is_err(), "should error on Response::Error");
    }

    #[test]
    fn test_extract_from_compiled_context_skills() {
        let xml = r#"<skills hint="use ...">
  <skill domain="auth" uses="5">cargo auth workflow abc12345</skill>
  <skill domain="database" uses="3">npm database pipeline def67890</skill>
</skills>"#;
        let items = extract_from_compiled_context(xml);
        assert_eq!(items.len(), 2);
        assert!(items.contains(&"cargo auth workflow abc12345".to_string()));
        assert!(items.contains(&"npm database pipeline def67890".to_string()));
    }

    #[test]
    fn test_extract_from_compiled_context_decisions() {
        let xml = r#"<decisions>
  <decision confidence="0.9">Decision: auth layer architecture (token123)</decision>
  <decision confidence="0.8">Decision: database refactor (token456)</decision>
</decisions>"#;
        let items = extract_from_compiled_context(xml);
        assert_eq!(items.len(), 2);
        assert!(items.contains(&"Decision: auth layer architecture (token123)".to_string()));
        assert!(items.contains(&"Decision: database refactor (token456)".to_string()));
    }

    #[test]
    fn test_extract_from_compiled_context_mixed() {
        let xml = r#"<decisions>
  <decision confidence="0.9">My Decision</decision>
</decisions>
<skills>
  <skill domain="auth" uses="2">My Skill</skill>
</skills>"#;
        let items = extract_from_compiled_context(xml);
        assert_eq!(items.len(), 2);
        assert!(items.contains(&"My Decision".to_string()));
        assert!(items.contains(&"My Skill".to_string()));
    }

    #[test]
    fn test_extract_from_compiled_context_empty() {
        let xml = "<decisions/>\n<skills/>\n";
        let items = extract_from_compiled_context(xml);
        assert!(items.is_empty());
    }

    // ── Task 5 tests: scoring ──

    #[test]
    fn test_precision_recall_f1_basic() {
        let expected: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let actual: HashSet<String> = ["a", "b", "d"].iter().map(|s| s.to_string()).collect();
        let (p, r, f1) = precision_recall_f1(&expected, &actual);
        assert!((p - 2.0 / 3.0).abs() < 0.001);
        assert!((r - 2.0 / 3.0).abs() < 0.001);
        assert!((f1 - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_precision_recall_f1_empty_both() {
        let (p, r, f1) = precision_recall_f1(&HashSet::new(), &HashSet::new());
        assert_eq!(p, 1.0);
        assert_eq!(r, 1.0);
        assert_eq!(f1, 1.0);
    }

    #[test]
    fn test_precision_recall_f1_empty_actual() {
        let expected: HashSet<String> = ["a"].iter().map(|s| s.to_string()).collect();
        let (_, r, f1) = precision_recall_f1(&expected, &HashSet::new());
        assert_eq!(r, 0.0);
        assert_eq!(f1, 0.0);
    }

    #[test]
    fn test_precision_recall_f1_perfect() {
        let expected: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let actual: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let (p, r, f1) = precision_recall_f1(&expected, &actual);
        assert_eq!(p, 1.0);
        assert_eq!(r, 1.0);
        assert_eq!(f1, 1.0);
    }

    #[test]
    fn test_precision_recall_f1_absent_convention() {
        // ABSENT: items should be checked for absence in actual
        let expected: HashSet<String> = ["ABSENT:bad_skill_1", "ABSENT:bad_skill_2"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // Actual does NOT contain the absent items → perfect score
        let actual: HashSet<String> = ["good_skill"].iter().map(|s| s.to_string()).collect();
        let (p, r, _f1) = precision_recall_f1(&expected, &actual);
        assert_eq!(
            r, 1.0,
            "recall should be 1.0 when absent items are indeed absent"
        );
        assert!(p > 0.0, "precision should be positive");
    }

    #[test]
    fn test_precision_recall_f1_absent_violation() {
        // ABSENT: items that ARE present in actual → mismatch
        let expected: HashSet<String> =
            ["ABSENT:bad_skill"].iter().map(|s| s.to_string()).collect();
        let actual: HashSet<String> = ["bad_skill"].iter().map(|s| s.to_string()).collect();
        let (_p, r, _f1) = precision_recall_f1(&expected, &actual);
        assert_eq!(r, 0.0, "recall should be 0.0 when absent item is present");
    }

    #[test]
    fn test_composite_score_weights() {
        // Verify the 0.30/0.30/0.20/0.20 weighting
        let results = vec![
            QueryResult {
                id: "CA-1".into(),
                dimension: Dimension::ContextAssembly,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "GR-1".into(),
                dimension: Dimension::Guardrails,
                precision: 0.5,
                recall: 0.5,
                f1: 0.5,
                expected_count: 2,
                actual_count: 2,
                matched_count: 1,
            },
            QueryResult {
                id: "CO-1".into(),
                dimension: Dimension::Completion,
                precision: 0.0,
                recall: 0.0,
                f1: 0.0,
                expected_count: 1,
                actual_count: 0,
                matched_count: 0,
            },
            QueryResult {
                id: "LR-1".into(),
                dimension: Dimension::LayerRecall,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
        ];
        let score = compute_composite(&results, 1.0);
        // composite = 0.30*1.0 + 0.30*0.5 + 0.20*0.0 + 0.20*1.0 = 0.30 + 0.15 + 0.0 + 0.20 = 0.65
        assert!(
            (score.composite - 0.65).abs() < 0.001,
            "composite should be 0.65, got {}",
            score.composite
        );
        assert!(score.pass, "0.65 >= 0.5 should pass");
        assert_eq!(score.total_queries, 4);
        assert_eq!(score.tool_filter_accuracy, 1.0);
    }

    #[test]
    fn test_composite_score_all_perfect() {
        let results = vec![
            QueryResult {
                id: "CA-1".into(),
                dimension: Dimension::ContextAssembly,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "GR-1".into(),
                dimension: Dimension::Guardrails,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "CO-1".into(),
                dimension: Dimension::Completion,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "LR-1".into(),
                dimension: Dimension::LayerRecall,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
        ];
        let score = compute_composite(&results, 1.0);
        assert!((score.composite - 1.0).abs() < 0.001, "all perfect = 1.0");
    }

    #[test]
    fn test_composite_score_all_zero() {
        let results = vec![
            QueryResult {
                id: "CA-1".into(),
                dimension: Dimension::ContextAssembly,
                precision: 0.0,
                recall: 0.0,
                f1: 0.0,
                expected_count: 1,
                actual_count: 0,
                matched_count: 0,
            },
            QueryResult {
                id: "GR-1".into(),
                dimension: Dimension::Guardrails,
                precision: 0.0,
                recall: 0.0,
                f1: 0.0,
                expected_count: 1,
                actual_count: 0,
                matched_count: 0,
            },
            QueryResult {
                id: "CO-1".into(),
                dimension: Dimension::Completion,
                precision: 0.0,
                recall: 0.0,
                f1: 0.0,
                expected_count: 1,
                actual_count: 0,
                matched_count: 0,
            },
            QueryResult {
                id: "LR-1".into(),
                dimension: Dimension::LayerRecall,
                precision: 0.0,
                recall: 0.0,
                f1: 0.0,
                expected_count: 1,
                actual_count: 0,
                matched_count: 0,
            },
        ];
        let score = compute_composite(&results, 0.0);
        assert!((score.composite).abs() < 0.001, "all zero = 0.0");
        assert!(!score.pass, "0.0 < 0.5 should not pass");
    }

    #[test]
    fn test_composite_score_multiple_per_dimension() {
        // Two queries per dimension — F1 is averaged
        let results = vec![
            QueryResult {
                id: "CA-1".into(),
                dimension: Dimension::ContextAssembly,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "CA-2".into(),
                dimension: Dimension::ContextAssembly,
                precision: 0.0,
                recall: 0.0,
                f1: 0.0,
                expected_count: 1,
                actual_count: 0,
                matched_count: 0,
            },
            QueryResult {
                id: "GR-1".into(),
                dimension: Dimension::Guardrails,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "GR-2".into(),
                dimension: Dimension::Guardrails,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "CO-1".into(),
                dimension: Dimension::Completion,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
            QueryResult {
                id: "LR-1".into(),
                dimension: Dimension::LayerRecall,
                precision: 1.0,
                recall: 1.0,
                f1: 1.0,
                expected_count: 1,
                actual_count: 1,
                matched_count: 1,
            },
        ];
        let score = compute_composite(&results, 1.0);
        // CA avg = 0.5, GR avg = 1.0, CO = 1.0, LR = 1.0
        // composite = 0.30*0.5 + 0.30*1.0 + 0.20*1.0 + 0.20*1.0 = 0.15 + 0.30 + 0.20 + 0.20 = 0.85
        assert!(
            (score.composite - 0.85).abs() < 0.001,
            "composite should be 0.85, got {}",
            score.composite
        );
    }
}
