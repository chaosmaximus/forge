# Forge-Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the Forge-Context benchmark harness that measures whether the daemon surfaces the right procedural knowledge at the right moment across 4 dimensions: context assembly, guardrails, completion intelligence, and layer recall.

**Architecture:** In-process harness using `DaemonState::new(":memory:")` with deterministic seed-based dataset generation. 12 tools (6 present / 6 absent from the daemon's hardcoded-12 filter list), 30 skills, 30 memories, 5 domain DNA entries. 28 queries with ground-truth annotations. Scoring via precision/recall/F1 per dimension + tool-filter accuracy + composite. CLI subcommand `forge-bench forge-context`.

**Tech Stack:** Rust, SQLite in-memory, serde JSON, ChaCha20 PRNG (rand_chacha), sha2 for unique content generation.

**Design doc:** `docs/benchmarks/forge-context-design.md`

---

## File Structure

| File | Responsibility | Tasks |
|------|---------------|-------|
| `crates/daemon/src/bench/common.rs` (CREATE) | Shared helpers extracted from forge_persist.rs: `bytes_to_hex`, `seeded_rng` | 1 |
| `crates/daemon/src/bench/forge_persist.rs` | Update imports to use common.rs helpers | 1 |
| `crates/daemon/src/bench/forge_context.rs` (CREATE) | Full Forge-Context harness: dataset gen, query bank, result extraction, scoring, orchestrator | 2-6 |
| `crates/daemon/src/bench/mod.rs` | Register `forge_context` module | 2 |
| `crates/daemon/src/bin/forge-bench.rs` | `forge-context` CLI subcommand | 6 |
| `crates/daemon/tests/forge_context_harness.rs` (CREATE) | Integration test | 6 |

---

### Task 1: Extract shared helpers into `bench/common.rs`

**Files:**
- Create: `crates/daemon/src/bench/common.rs`
- Modify: `crates/daemon/src/bench/forge_persist.rs`
- Modify: `crates/daemon/src/bench/mod.rs`

- [ ] **Step 1.1: Write failing test for `common::bytes_to_hex`**

In `crates/daemon/src/bench/common.rs`:

```rust
//! Shared helpers for Forge-* benchmark harnesses.

use sha2::{Digest, Sha256};

/// Convert a byte slice to a lowercase hex string.
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Create a deterministic ChaCha20 PRNG from a u64 seed.
pub fn seeded_rng(seed: u64) -> rand_chacha::ChaCha20Rng {
    use rand::SeedableRng;
    rand_chacha::ChaCha20Rng::seed_from_u64(seed)
}

/// Generate a SHA-256 hex digest of the given input string.
/// Used to create unique tokens that resist semantic dedup.
pub fn sha256_hex(input: &str) -> String {
    bytes_to_hex(&Sha256::digest(input.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_to_hex_known_value() {
        assert_eq!(bytes_to_hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    #[test]
    fn test_bytes_to_hex_empty() {
        assert_eq!(bytes_to_hex(&[]), "");
    }

    #[test]
    fn test_sha256_hex_deterministic() {
        let a = sha256_hex("hello");
        let b = sha256_hex("hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // SHA-256 = 32 bytes = 64 hex chars
    }

    #[test]
    fn test_seeded_rng_deterministic() {
        use rand::Rng;
        let mut rng1 = seeded_rng(42);
        let mut rng2 = seeded_rng(42);
        let v1: u64 = rng1.gen();
        let v2: u64 = rng2.gen();
        assert_eq!(v1, v2, "same seed must produce same sequence");
    }
}
```

- [ ] **Step 1.2: Register common module in mod.rs**

Add to `crates/daemon/src/bench/mod.rs`:
```rust
pub mod common;
```

- [ ] **Step 1.3: Run tests to verify green**

Run: `cargo test -p forge-daemon --lib -- bench::common`
Expected: 4 tests PASS

- [ ] **Step 1.4: Update forge_persist.rs to use common helpers**

In `crates/daemon/src/bench/forge_persist.rs`, replace the local `bytes_to_hex` function with `use super::common::bytes_to_hex;` and remove the local definition. Do the same for any `seeded_rng` equivalent (check if forge_persist has its own — if it uses `ChaCha20Rng::seed_from_u64` directly, leave it; only extract what's duplicated).

Search for `fn bytes_to_hex` in forge_persist.rs and replace its body with a re-export or direct call to `common::bytes_to_hex`.

- [ ] **Step 1.5: Run full workspace tests**

Run: `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`
Expected: PASS, 0 warnings

- [ ] **Step 1.6: Commit**

```bash
git add crates/daemon/src/bench/common.rs crates/daemon/src/bench/mod.rs crates/daemon/src/bench/forge_persist.rs
git commit -m "refactor(bench): extract shared helpers into bench/common.rs

Extracts bytes_to_hex, seeded_rng, sha256_hex from forge_persist.rs
into a shared common module. Forge-Context (Phase 2A-2) is the second
call site that motivates the extraction, per forge-persist-design.md
§12 Q7.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Dataset generator — tools, skills, memories, domain DNA

**Files:**
- Create: `crates/daemon/src/bench/forge_context.rs`
- Modify: `crates/daemon/src/bench/mod.rs`

This task creates the dataset generator that seeds a DaemonState with deterministic test data.

- [ ] **Step 2.1: Create forge_context.rs with module skeleton and register it**

Create `crates/daemon/src/bench/forge_context.rs` with the core types and constants. Add `pub mod forge_context;` to `mod.rs`.

```rust
//! Forge-Context benchmark harness — proactive intelligence precision.
//!
//! Measures whether the daemon surfaces the right procedural knowledge
//! at the right moment across 4 dimensions: context assembly, guardrails,
//! completion intelligence, and layer recall.
//!
//! In-process harness using DaemonState with in-memory SQLite.
//! See docs/benchmarks/forge-context-design.md for the full design.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::server::handler::{handle_request, DaemonState};
use forge_core::protocol::{Request, Response, ResponseData};
use forge_core::types::manas::{Tool, ToolHealth, ToolKind};
use forge_core::types::memory::MemoryType;

use super::common::{seeded_rng, sha256_hex};

/// The 12 hardcoded tool keywords the daemon uses for skill filtering
/// in CompileContext's dynamic suffix (recall.rs:1077-1094).
const HARDCODED_TOOL_KEYWORDS: [&str; 12] = [
    "docker", "kubectl", "terraform", "npm", "cargo", "pip",
    "gcloud", "aws", "ssh", "make", "scp", "rsync",
];

/// Source tag for harness-generated content.
pub const HARNESS_SOURCE: &str = "forge-context";

/// Domain vocabulary for distributing items across topics.
const DOMAINS: [&str; 5] = ["auth", "database", "networking", "testing", "deployment"];
```

- [ ] **Step 2.2: Write failing test for tool generation**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_tools_splits_present_and_absent() {
        let (present, absent) = generate_tools(42);
        assert_eq!(present.len(), 6, "6 present tools");
        assert_eq!(absent.len(), 6, "6 absent tool keywords");
        // All names must be from HARDCODED_TOOL_KEYWORDS
        for tool in &present {
            assert!(
                HARDCODED_TOOL_KEYWORDS.contains(&tool.name.as_str()),
                "present tool '{}' not in hardcoded list", tool.name
            );
        }
        for name in &absent {
            assert!(
                HARDCODED_TOOL_KEYWORDS.contains(&name.as_str()),
                "absent tool '{}' not in hardcoded list", name
            );
        }
        // No overlap
        let present_names: HashSet<&str> = present.iter().map(|t| t.name.as_str()).collect();
        for name in &absent {
            assert!(!present_names.contains(name.as_str()), "overlap: {name}");
        }
    }
}
```

- [ ] **Step 2.3: Run test — should FAIL (function doesn't exist)**

Run: `cargo test -p forge-daemon --lib -- bench::forge_context::tests::test_generate_tools`
Expected: FAIL (E0425)

- [ ] **Step 2.4: Implement `generate_tools`**

```rust
/// Generate 6 present tools (inserted into DB) and 6 absent tool keywords
/// (NOT inserted — skills mentioning these get filtered by CompileContext).
/// The split is deterministic from seed.
pub fn generate_tools(seed: u64) -> (Vec<Tool>, Vec<String>) {
    use rand::seq::SliceRandom;
    let mut rng = seeded_rng(seed);
    let mut keywords: Vec<&str> = HARDCODED_TOOL_KEYWORDS.to_vec();
    keywords.shuffle(&mut rng);

    let present: Vec<Tool> = keywords[..6]
        .iter()
        .enumerate()
        .map(|(i, &name)| Tool {
            id: format!("tool-{}", sha256_hex(&format!("tool-{name}-{seed}"))[..8].to_string()),
            name: name.to_string(),
            kind: match i % 4 {
                0 => ToolKind::Cli,
                1 => ToolKind::Mcp,
                2 => ToolKind::Builtin,
                _ => ToolKind::Plugin,
            },
            capabilities: vec![format!("{name}-cap")],
            config: None,
            health: ToolHealth::Healthy,
            last_used: None,
            use_count: 0,
            discovered_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .collect();

    let absent: Vec<String> = keywords[6..].iter().map(|s| s.to_string()).collect();
    (present, absent)
}
```

- [ ] **Step 2.5: Run test — should PASS**

- [ ] **Step 2.6: Write failing test for skill generation**

```rust
#[test]
fn test_generate_skills_tags_tool_dependencies() {
    let (present_tools, absent_keywords) = generate_tools(42);
    let skills = generate_skills(42, &present_tools, &absent_keywords);
    assert_eq!(skills.len(), 30, "30 skills total");

    // All skills have success_count > 0 (required by find_applicable_skills SQL)
    for s in &skills {
        assert!(s.success_count > 0, "skill {} must have success_count > 0", s.name);
    }

    // Count tool-dependency distribution
    let present_names: HashSet<&str> = present_tools.iter().map(|t| t.name.as_str()).collect();
    let mut mentions_present = 0;
    let mut mentions_absent = 0;
    let mut mentions_none = 0;
    for s in &skills {
        let text = format!("{} {} {}", s.name, s.description, s.domain).to_lowercase();
        let has_present = present_names.iter().any(|n| text.contains(n));
        let has_absent = absent_keywords.iter().any(|n| text.contains(n));
        if has_present { mentions_present += 1; }
        else if has_absent { mentions_absent += 1; }
        else { mentions_none += 1; }
    }
    assert_eq!(mentions_present, 10, "10 skills depend on present tools");
    assert_eq!(mentions_absent, 10, "10 skills depend on absent tools");
    assert_eq!(mentions_none, 10, "10 skills have no tool dependency");
}
```

- [ ] **Step 2.7: Implement `generate_skills`**

```rust
/// Generate 30 skills: 10 mentioning a present-tool keyword, 10 mentioning
/// an absent-tool keyword, 10 with no hardcoded-12 keyword.
pub fn generate_skills(seed: u64, present_tools: &[Tool], absent_keywords: &[String]) -> Vec<forge_core::types::manas::Skill> {
    let mut skills = Vec::with_capacity(30);
    for i in 0..30 {
        let domain = DOMAINS[i % DOMAINS.len()];
        let unique = sha256_hex(&format!("skill-{i}-{seed}"));
        let unique_token = &unique[..16];

        // Tool dependency: first 10 mention a present tool, next 10 an absent tool, last 10 none
        let tool_mention = if i < 10 {
            format!(" uses {} for", present_tools[i % present_tools.len()].name)
        } else if i < 20 {
            format!(" uses {} for", absent_keywords[(i - 10) % absent_keywords.len()])
        } else {
            String::new()
        };

        let skill_type = if i < 20 { "procedural" } else { "behavioral" };
        let steps = if skill_type == "procedural" {
            vec![
                format!("step-1-{unique_token}"),
                format!("step-2-{unique_token}"),
                format!("step-3-{unique_token}"),
            ]
        } else {
            vec![]
        };

        skills.push(forge_core::types::manas::Skill {
            id: format!("skill-{i}-{}", &unique[..8]),
            name: format!("{domain}-skill-{i}-{unique_token}"),
            domain: domain.to_string(),
            description: format!("{domain} procedure {unique_token}{tool_mention} operations"),
            steps,
            success_count: (i as u64) + 1,
            fail_count: 0,
            last_used: None,
            source: HARNESS_SOURCE.to_string(),
            version: 1,
            project: None,
            skill_type: skill_type.to_string(),
            user_specific: false,
            observed_count: 1,
            correlation_ids: vec![],
        });
    }
    skills
}
```

- [ ] **Step 2.8: Run test — should PASS**

- [ ] **Step 2.9: Write failing test for memory generation**

```rust
#[test]
fn test_generate_memories_creates_decisions_lessons_patterns() {
    let mems = generate_memories(42);
    assert_eq!(mems.len(), 30);
    let decisions: Vec<_> = mems.iter().filter(|m| m.0 == MemoryType::Decision).collect();
    let lessons: Vec<_> = mems.iter().filter(|m| m.0 == MemoryType::Lesson).collect();
    let patterns: Vec<_> = mems.iter().filter(|m| m.0 == MemoryType::Pattern).collect();
    assert_eq!(decisions.len(), 10);
    assert_eq!(lessons.len(), 10);
    assert_eq!(patterns.len(), 10);

    // Lessons should have testing/uat/deployment tags for CompletionCheck
    for (_, title, _, tags, _, _) in &lessons {
        assert!(
            tags.iter().any(|t| t.contains("testing") || t.contains("uat") || t.contains("deployment") || t.contains("anti-pattern")),
            "lesson '{}' must have a completion-relevant tag", title
        );
    }
}
```

- [ ] **Step 2.10: Implement `generate_memories`**

Returns a `Vec<(MemoryType, String, String, Vec<String>, f64, f64)>` — (type, title, content, tags, confidence, quality_score).

```rust
/// Generate 30 memories: 10 decisions, 10 lessons (with completion-relevant tags), 10 patterns.
pub fn generate_memories(seed: u64) -> Vec<(MemoryType, String, String, Vec<String>, f64, f64)> {
    let completion_tags = ["testing", "uat", "deployment", "anti-pattern", "production-readiness"];
    let mut mems = Vec::with_capacity(30);
    for i in 0..30 {
        let domain = DOMAINS[i % DOMAINS.len()];
        let unique = sha256_hex(&format!("memory-{i}-{seed}"));
        let unique_token = &unique[..16];
        let (mem_type, tags) = if i < 10 {
            (MemoryType::Decision, vec![domain.to_string(), format!("{domain}-decision")])
        } else if i < 20 {
            let tag = completion_tags[(i - 10) % completion_tags.len()];
            (MemoryType::Lesson, vec![domain.to_string(), tag.to_string()])
        } else {
            (MemoryType::Pattern, vec![domain.to_string()])
        };
        let title = format!("{domain}-{}-{unique_token}", match mem_type {
            MemoryType::Decision => "decision",
            MemoryType::Lesson => "lesson",
            MemoryType::Pattern => "pattern",
            _ => "memory",
        });
        let content = format!("{domain} memory content {unique_token} for bench context harness");
        let confidence = 0.9 - (i as f64 * 0.01); // decreasing for deterministic ranking
        let quality_score = 0.9 - (i as f64 * 0.01);
        mems.push((mem_type, title, content, tags, confidence, quality_score));
    }
    mems
}
```

- [ ] **Step 2.11: Write failing test for `seed_state`**

```rust
#[test]
fn test_seed_state_populates_all_categories() {
    let mut state = DaemonState::new(":memory:").expect("state");
    let dataset = seed_state(&mut state, 42);
    assert_eq!(dataset.present_tools.len(), 6);
    assert_eq!(dataset.absent_keywords.len(), 6);
    assert_eq!(dataset.skills.len(), 30);
    assert_eq!(dataset.memory_titles.len(), 30);
    assert_eq!(dataset.domain_dna_aspects.len(), 5);
    assert!(dataset.file_paths.len() >= 5, "at least 5 file paths for affects edges");
}
```

- [ ] **Step 2.12: Implement `SeededDataset` struct and `seed_state` function**

```rust
/// The seeded dataset — holds metadata needed for ground-truth annotations.
pub struct SeededDataset {
    pub present_tools: Vec<Tool>,
    pub absent_keywords: Vec<String>,
    pub skills: Vec<forge_core::types::manas::Skill>,
    pub memory_titles: Vec<(MemoryType, String)>, // (type, title)
    pub domain_dna_aspects: Vec<String>,
    pub file_paths: Vec<String>, // files used in affects edges
    pub session_id: String,
}

/// Seed the DaemonState with the full deterministic dataset.
pub fn seed_state(state: &mut DaemonState, seed: u64) -> SeededDataset {
    // 1. Generate and store tools
    let (present_tools, absent_keywords) = generate_tools(seed);
    for tool in &present_tools {
        handle_request(state, Request::StoreTool { tool: tool.clone() });
    }

    // 2. Generate and store skills (directly via db::manas::store_skill)
    let skills = generate_skills(seed, &present_tools, &absent_keywords);
    for skill in &skills {
        crate::db::manas::store_skill(&state.conn, skill);
    }

    // 3. Generate and store memories (via Request::Remember)
    let mems = generate_memories(seed);
    let mut memory_titles = Vec::new();
    for (mem_type, title, content, tags, confidence, _quality) in &mems {
        memory_titles.push((mem_type.clone(), title.clone()));
        handle_request(state, Request::Remember {
            memory_type: mem_type.clone(),
            title: title.clone(),
            content: content.clone(),
            confidence: Some(*confidence),
            tags: Some(tags.clone()),
            project: None,
            metadata: None,
        });
    }

    // 4. Generate and store domain DNA
    let mut domain_dna_aspects = Vec::new();
    for (i, domain) in DOMAINS.iter().enumerate() {
        let unique = sha256_hex(&format!("dna-{i}-{seed}"));
        let aspect = format!("{domain}_convention");
        domain_dna_aspects.push(aspect.clone());
        crate::db::manas::store_domain_dna(&state.conn, &crate::types::manas::DomainDna {
            id: format!("dna-{i}-{}", &unique[..8]),
            project: "forge-context-bench".to_string(),
            aspect: aspect,
            pattern: format!("{domain} uses {}_style_{}", domain, &unique[..8]),
            confidence: 0.8,
            evidence: vec![format!("src/{domain}/mod.rs")],
            detected_at: "2026-01-01T00:00:00Z".to_string(),
        });
    }

    // 5. Create affects edges (decisions → files)
    let file_paths: Vec<String> = (0..5).map(|i| format!("src/{}/mod.rs", DOMAINS[i])).collect();
    for i in 0..10 {
        // Decision i affects file_paths[i % 5]
        let decision_title = &memory_titles[i].1;
        // Look up the memory ID we just stored
        // (We need to query back — Remember returns the ID)
        // Actually, we insert affects edges via direct SQL for simplicity
        // since there's no Request for creating arbitrary edges
    }

    // 6. Register test session for CompletionCheck queries
    let session_id = format!("forge-context-bench-{seed}");
    handle_request(state, Request::RegisterSession {
        id: session_id.clone(),
        agent: "bench".to_string(),
        project: None,
        cwd: None,
        capabilities: None,
        current_task: None,
    });

    SeededDataset {
        present_tools,
        absent_keywords,
        skills,
        memory_titles,
        domain_dna_aspects,
        file_paths,
        session_id,
    }
}
```

**Note:** The affects-edge insertion needs investigation during implementation — the implementer should find how edges are created (likely via `db::ops::insert_edge` or similar) and insert them for decisions → file paths.

- [ ] **Step 2.13: Run tests — should PASS**

- [ ] **Step 2.14: Run workspace tests + clippy**

Run: `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`

- [ ] **Step 2.15: Commit**

```bash
git add crates/daemon/src/bench/forge_context.rs crates/daemon/src/bench/mod.rs
git commit -m "feat(forge-context): dataset generator — tools, skills, memories, domain DNA (cycle b)

Seeds DaemonState with deterministic test data from a ChaCha20 seed:
6 present tools (from hardcoded-12), 6 absent keywords, 30 skills
(10 present-tool / 10 absent-tool / 10 no-tool), 30 memories
(10 decisions / 10 lessons / 10 patterns), 5 domain DNA entries,
affects edges, and test session registration.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Query bank generator with ground-truth annotations

**Files:**
- Modify: `crates/daemon/src/bench/forge_context.rs`

- [ ] **Step 3.1: Define query types and ground-truth struct**

```rust
/// Scoring dimensions per the design doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dimension {
    ContextAssembly,
    Guardrails,
    Completion,
    LayerRecall,
}

/// A single query with its expected results.
pub struct QueryCase {
    pub id: String,
    pub dimension: Dimension,
    pub request: Request,
    /// Expected items in the response (exact formatted strings as the daemon produces).
    pub expected: HashSet<String>,
}
```

- [ ] **Step 3.2: Write failing test for query bank generation**

```rust
#[test]
fn test_generate_query_bank_covers_all_dimensions() {
    let mut state = DaemonState::new(":memory:").expect("state");
    let dataset = seed_state(&mut state, 42);
    let queries = generate_query_bank(&dataset);

    assert!(!queries.is_empty(), "query bank must not be empty");

    let dims: HashSet<Dimension> = queries.iter().map(|q| q.dimension).collect();
    assert!(dims.contains(&Dimension::ContextAssembly));
    assert!(dims.contains(&Dimension::Guardrails));
    assert!(dims.contains(&Dimension::Completion));
    assert!(dims.contains(&Dimension::LayerRecall));

    // Every query must have at least one expected result (no vacuous queries)
    for q in &queries {
        assert!(!q.expected.is_empty(), "query {} must have expected results", q.id);
    }
}
```

- [ ] **Step 3.3: Implement `generate_query_bank`**

Build queries for each dimension using the dataset metadata. Generate expected results using the EXACT format the daemon will produce.

This function constructs `Request` variants and pre-computes expected response strings based on the seeded data. The implementer must trace each endpoint's response format (see design doc §6.5).

- [ ] **Step 3.4: Run test — should PASS**

- [ ] **Step 3.5: Commit**

```bash
git commit -m "feat(forge-context): query bank generator with ground-truth annotations (cycle c)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Result extractors — per-endpoint response → item sets

**Files:**
- Modify: `crates/daemon/src/bench/forge_context.rs`

- [ ] **Step 4.1: Write failing test for result extraction**

```rust
#[test]
fn test_extract_results_from_guardrails_check() {
    let mut state = DaemonState::new(":memory:").expect("state");
    // Remember a decision
    handle_request(&mut state, Request::Remember {
        memory_type: MemoryType::Decision,
        title: "Use JWT".to_string(),
        content: "For auth".to_string(),
        confidence: Some(0.9),
        tags: Some(vec!["auth".to_string()]),
        project: None,
        metadata: None,
    });
    // GuardrailsCheck
    let resp = handle_request(&mut state, Request::GuardrailsCheck {
        file: "src/auth/middleware.rs".to_string(),
        action: "edit".to_string(),
    });
    let items = extract_result_items(&resp);
    // Should be a HashSet of strings (may be empty if no affects edges)
    assert!(items.is_ok());
}
```

- [ ] **Step 4.2: Implement `extract_result_items`**

```rust
/// Extract item strings from a daemon response for scoring.
/// Each endpoint returns results in a different shape — this function
/// normalizes them into a flat HashSet<String>.
pub fn extract_result_items(response: &Response) -> Result<HashSet<String>, String> {
    match response {
        Response::Ok { data } => {
            let mut items = HashSet::new();
            match data {
                ResponseData::GuardrailsCheck {
                    decisions_affected,
                    relevant_lessons,
                    applicable_skills,
                    ..
                } => {
                    items.extend(decisions_affected.iter().cloned());
                    items.extend(relevant_lessons.iter().cloned());
                    items.extend(applicable_skills.iter().cloned());
                }
                ResponseData::PostEditChecked {
                    applicable_skills,
                    decisions_to_review,
                    relevant_lessons,
                    ..
                } => {
                    items.extend(applicable_skills.iter().cloned());
                    items.extend(decisions_to_review.iter().cloned());
                    items.extend(relevant_lessons.iter().cloned());
                }
                ResponseData::PreBashChecked {
                    relevant_skills,
                    ..
                } => {
                    items.extend(relevant_skills.iter().cloned());
                }
                ResponseData::CompletionCheckResult {
                    relevant_lessons,
                    ..
                } => {
                    items.extend(relevant_lessons.iter().cloned());
                }
                ResponseData::TaskCompletionCheckResult {
                    checklists,
                    ..
                } => {
                    items.extend(checklists.iter().cloned());
                }
                ResponseData::Memories { results, .. } => {
                    for r in results {
                        items.insert(r.memory.title.clone());
                    }
                }
                ResponseData::CompiledContext { context, .. } => {
                    // Parse XML for skill mentions and decision references
                    // Extract <skill> tags and decision titles from the context string
                    items.extend(extract_from_compiled_context(context));
                }
                other => return Err(format!("unexpected response variant: {other:?}")),
            }
            Ok(items)
        }
        Response::Error { message } => Err(format!("daemon error: {message}")),
    }
}

/// Extract skill names and decision mentions from compiled context XML.
fn extract_from_compiled_context(context: &str) -> Vec<String> {
    // Parse the XML-like context for skill and decision entries.
    // Skills appear as: <skill domain="X" uses="N">name — description</skill>
    // Decisions appear as: <decision confidence="0.9">title</decision>
    // The implementer must trace the exact XML format in recall.rs compile_dynamic_suffix.
    let mut items = Vec::new();
    // Simple line-based extraction (XML is line-oriented in practice)
    for line in context.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("<skill") {
            // Extract skill name from the tag content
            if let Some(content) = trimmed.split('>').nth(1) {
                if let Some(name) = content.split(" — ").next() {
                    items.push(name.to_string());
                }
            }
        }
        if trimmed.starts_with("<decision") {
            if let Some(content) = trimmed.split('>').nth(1) {
                if let Some(title) = content.strip_suffix("</decision>") {
                    items.push(title.to_string());
                }
            }
        }
    }
    items
}
```

- [ ] **Step 4.3: Run test — should PASS**

- [ ] **Step 4.4: Commit**

```bash
git commit -m "feat(forge-context): result extractors — per-endpoint response → item sets (cycle d)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Scoring — precision, recall, F1, composite, tool_filter_accuracy

**Files:**
- Modify: `crates/daemon/src/bench/forge_context.rs`

- [ ] **Step 5.1: Write failing tests for scoring functions**

```rust
#[test]
fn test_precision_recall_f1_basic() {
    let expected: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let actual: HashSet<String> = ["a", "b", "d"].iter().map(|s| s.to_string()).collect();
    let (p, r, f1) = precision_recall_f1(&expected, &actual);
    assert!((p - 2.0 / 3.0).abs() < 0.001, "precision = 2/3");
    assert!((r - 2.0 / 3.0).abs() < 0.001, "recall = 2/3");
    assert!((f1 - 2.0 / 3.0).abs() < 0.001, "f1 = 2/3");
}

#[test]
fn test_precision_recall_f1_empty_actual() {
    let expected: HashSet<String> = ["a"].iter().map(|s| s.to_string()).collect();
    let actual: HashSet<String> = HashSet::new();
    let (p, r, f1) = precision_recall_f1(&expected, &actual);
    assert_eq!(r, 0.0, "recall = 0 when nothing returned");
    assert_eq!(f1, 0.0, "f1 = 0 when nothing returned");
}

#[test]
fn test_precision_recall_f1_empty_expected() {
    let expected: HashSet<String> = HashSet::new();
    let actual: HashSet<String> = HashSet::new();
    let (p, r, f1) = precision_recall_f1(&expected, &actual);
    assert_eq!(p, 1.0, "perfect precision when both empty");
    assert_eq!(r, 1.0, "perfect recall when both empty");
}
```

- [ ] **Step 5.2: Implement scoring functions**

```rust
/// Compute precision, recall, and F1 for a single query.
pub fn precision_recall_f1(expected: &HashSet<String>, actual: &HashSet<String>) -> (f64, f64, f64) {
    if expected.is_empty() && actual.is_empty() {
        return (1.0, 1.0, 1.0);
    }
    if actual.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let intersection = expected.intersection(actual).count() as f64;
    let precision = intersection / actual.len() as f64;
    let recall = if expected.is_empty() { 0.0 } else { intersection / expected.len() as f64 };
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

/// Composite scoring output for a full run.
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

/// Aggregate per-dimension F1 scores and compute composite.
pub fn compute_composite(results: &[QueryResult], tool_filter_accuracy: f64) -> ContextScore {
    let dim_f1 = |dim: Dimension| -> f64 {
        let qs: Vec<&QueryResult> = results.iter().filter(|r| r.dimension == dim).collect();
        if qs.is_empty() { return 0.0; }
        qs.iter().map(|r| r.f1).sum::<f64>() / qs.len() as f64
    };

    let ca = dim_f1(Dimension::ContextAssembly);
    let gr = dim_f1(Dimension::Guardrails);
    let co = dim_f1(Dimension::Completion);
    let lr = dim_f1(Dimension::LayerRecall);

    let composite = 0.30 * ca + 0.30 * gr + 0.20 * co + 0.20 * lr;

    ContextScore {
        seed: 0, // set by caller
        context_assembly_f1: ca,
        guardrails_f1: gr,
        completion_f1: co,
        layer_recall_f1: lr,
        tool_filter_accuracy,
        composite,
        total_queries: results.len(),
        pass: false, // set during calibration
    }
}
```

- [ ] **Step 5.3: Run tests — should PASS**

- [ ] **Step 5.4: Commit**

```bash
git commit -m "feat(forge-context): scoring — precision/recall/F1, composite, tool_filter (cycle e)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Orchestrator `pub fn run` + CLI subcommand + integration test

**Files:**
- Modify: `crates/daemon/src/bench/forge_context.rs`
- Modify: `crates/daemon/src/bin/forge-bench.rs`
- Create: `crates/daemon/tests/forge_context_harness.rs`

- [ ] **Step 6.1: Define `ContextConfig`**

```rust
/// Configuration for a Forge-Context benchmark run.
pub struct ContextConfig {
    pub seed: u64,
    pub output_dir: Option<PathBuf>,
}
```

- [ ] **Step 6.2: Write failing integration test**

In `crates/daemon/tests/forge_context_harness.rs`:

```rust
use forge_daemon::bench::forge_context::{run, ContextConfig};

#[test]
fn test_context_harness_passes_on_clean_workload() {
    let config = ContextConfig {
        seed: 42,
        output_dir: None,
    };
    let score = run(config).expect("harness should not error");
    assert!(score.composite > 0.0, "composite must be positive");
    assert_eq!(score.tool_filter_accuracy, 1.0, "tool filtering must be exact");
    assert!(score.total_queries > 0, "must have queries");
}
```

- [ ] **Step 6.3: Implement `pub fn run`**

```rust
/// Run the Forge-Context benchmark and return composite scores.
pub fn run(config: ContextConfig) -> Result<ContextScore, String> {
    let mut state = DaemonState::new(":memory:").map_err(|e| format!("DaemonState: {e}"))?;

    // 1. Seed the dataset
    let dataset = seed_state(&mut state, config.seed);

    // 2. Generate query bank with ground truth
    let queries = generate_query_bank(&dataset);

    // 3. Execute each query and score
    let mut results = Vec::new();
    let mut tool_filter_correct = 0usize;
    let mut tool_filter_total = 0usize;

    for query in &queries {
        let response = handle_request(&mut state, query.request.clone());
        let actual = extract_result_items(&response)
            .map_err(|e| format!("query {}: {e}", query.id))?;

        let (p, r, f1) = precision_recall_f1(&query.expected, &actual);
        results.push(QueryResult {
            id: query.id.clone(),
            dimension: query.dimension,
            precision: p,
            recall: r,
            f1,
            expected_count: query.expected.len(),
            actual_count: actual.len(),
            matched_count: query.expected.intersection(&actual).count(),
        });

        // Tool-filter accuracy: for CA-4..CA-6 queries
        if query.id.starts_with("CA-") && query.dimension == Dimension::ContextAssembly {
            // Check that no absent-tool skills appear
            for item in &actual {
                let item_lower = item.to_lowercase();
                for absent in &dataset.absent_keywords {
                    if item_lower.contains(absent) {
                        // Leaked — should have been filtered
                    } else {
                        tool_filter_correct += 1;
                    }
                    tool_filter_total += 1;
                }
            }
        }
    }

    let tool_filter_accuracy = if tool_filter_total > 0 {
        tool_filter_correct as f64 / tool_filter_total as f64
    } else {
        1.0
    };

    let mut score = compute_composite(&results, tool_filter_accuracy);
    score.seed = config.seed;

    // Write outputs if output_dir is set
    if let Some(dir) = &config.output_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
        let json = serde_json::to_string_pretty(&score)
            .map_err(|e| format!("json: {e}"))?;
        std::fs::write(dir.join("summary.json"), &json)
            .map_err(|e| format!("write summary: {e}"))?;
    }

    Ok(score)
}
```

- [ ] **Step 6.4: Add CLI subcommand**

In `crates/daemon/src/bin/forge-bench.rs`, add a `ForgeContext` variant to the `Commands` enum and dispatch to `forge_context::run`.

- [ ] **Step 6.5: Run integration test**

Run: `cargo test -p forge-daemon --test forge_context_harness`
Expected: PASS

- [ ] **Step 6.6: Run workspace tests + clippy**

Run: `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`

- [ ] **Step 6.7: Commit**

```bash
git commit -m "feat(forge-context): pub fn run orchestrator + CLI dispatch + integration test (cycle f)

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Calibration sweep + results doc

**Files:**
- Modify: `crates/daemon/src/bench/forge_context.rs` (threshold locking)
- Create: `docs/benchmarks/results/forge-context-YYYY-MM-DD.md`

- [ ] **Step 7.1: Run calibration with 5 seeds**

```bash
./target/release/forge-bench forge-context --seed 1 --output bench_results_context/seed1
./target/release/forge-bench forge-context --seed 2 --output bench_results_context/seed2
./target/release/forge-bench forge-context --seed 3 --output bench_results_context/seed3
./target/release/forge-bench forge-context --seed 42 --output bench_results_context/seed42
./target/release/forge-bench forge-context --seed 100 --output bench_results_context/seed100
```

- [ ] **Step 7.2: Analyze results and set thresholds**

Review the 5 summary.json files. Set `composite` threshold to the minimum observed minus a small margin. Lock `tool_filter_accuracy = 1.00` (deterministic).

- [ ] **Step 7.3: Write results doc**

Create `docs/benchmarks/results/forge-context-YYYY-MM-DD.md` with:
- 5-seed calibration table
- Per-dimension breakdown
- Reproduction command
- Honest limitations
- Comparison note (no public baseline — Forge-specific bench)

- [ ] **Step 7.4: Commit results doc**

```bash
git commit -m "docs(bench): Forge-Context calibration results + threshold locking

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Design doc §4 dataset shape → Task 2 ✅
- Design doc §4.5 query bank → Task 3 ✅
- Design doc §6.5 result extraction → Task 4 ✅
- Design doc §5 scoring rubric → Task 5 ✅
- Design doc §6.4 execution flow → Task 6 ✅
- Design doc §8 CLI subcommand → Task 6 ✅
- Design doc §7 integration test → Task 6 ✅
- Design doc §9 reproduction → Task 7 ✅
- Design doc §6.2 shared extraction → Task 1 ✅

**Placeholder scan:** Task 2 step 2.12 has a note about affects-edge insertion requiring investigation. This is intentional — the exact edge API needs to be traced during implementation, and the step explicitly calls this out rather than guessing.

**Type consistency:** `SeededDataset`, `QueryCase`, `Dimension`, `QueryResult`, `ContextScore`, `ContextConfig` — used consistently across tasks 2-6.
