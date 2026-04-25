//! Forge-Isolation bench (2A-5) — domain-transfer isolation correctness.
//!
//! Spec: `docs/superpowers/specs/2026-04-25-domain-isolation-bench-design.md`
//! v2.1 LOCKED.
//!
//! Validates that project scoping prevents cross-project memory leakage at
//! both the `Request::Recall` API and the `compile_dynamic_suffix_with_inj`
//! context-assembly entrypoint.
//!
//! ## Architecture (spec §3.7)
//!
//! Single shared corpus per seed; all 6 dimensions read from the same
//! `Connection` to preserve cross-dim leakage signal. Per-dim isolated
//! `:memory:` databases (as in `forge_identity::run_dim_isolated`) would
//! actively HIDE cross-dim regression — wrong primitive for an isolation
//! bench.
//!
//! ## Dimensions (§3.1, §3.3)
//!
//! | Dim | Probe | Min | Weight |
//! |-----|-------|-----|--------|
//! | D1 cross_project_precision     | Recall query=`isolation_bench` per-project; foreign-token rate | 0.95 | 0.25 |
//! | D2 self_recall_completeness    | Recall query=`{P}_secret`, project=Some(P); recall@10        | 0.85 | 0.15 |
//! | D3 global_memory_visibility    | Globals appear in every project's recall                      | 0.90 | 0.10 |
//! | D4 unscoped_query_breadth      | bucket coverage with project=None                             | 0.85 | 0.10 |
//! | D5 edge_case_resilience        | 7 sub-probes (empty/special/long/SQLi/prefix/case/whitespace) | 0.85 | 0.15 |
//! | D6 compile_context_isolation   | `compile_dynamic_suffix_with_inj` foreign-token denominator=15 | 0.95 | 0.25 |
//!
//! Composite = weighted mean. Pass = composite ≥ 0.95 AND every dim ≥ min.

use rand_chacha::ChaCha20Rng;
use serde::Serialize;

use crate::bench::common::{deterministic_embedding, seeded_rng};
use crate::server::handler::DaemonState;

// ── Per-bench weights (§3.1, §3.3) ──────────────────────────────────────

/// Weights summing to 1.00 (spec §3.3).
const DIM_WEIGHTS: [f64; 6] = [0.25, 0.15, 0.10, 0.10, 0.15, 0.25];

/// Per-dimension minimum scores for pass (spec §3.1).
const DIM_MINIMUMS: [f64; 6] = [0.95, 0.85, 0.90, 0.85, 0.85, 0.95];

/// Composite pass threshold (spec §3.3).
const COMPOSITE_THRESHOLD: f64 = 0.95;

// ── Corpus parameters (spec §3.2) ───────────────────────────────────────

/// Main projects whose memories must remain isolated from each other.
pub const MAIN_PROJECTS: [&str; 5] = ["alpha", "beta", "gamma", "delta", "epsilon"];

/// Prefix-collision sentinel project. Exists only to drive D5 probe 5
/// (`alpha` query must not match `alphabet` memories).
pub const PREFIX_COLLISION_PROJECT: &str = "alphabet";

/// Memories per main project.
pub const MEMORIES_PER_MAIN_PROJECT: usize = 30;

/// Memories in the prefix-collision sentinel.
pub const PREFIX_COLLISION_MEMORIES: usize = 5;

/// Project=None global memories visible in every project's recall.
pub const GLOBAL_MEMORIES: usize = 10;

/// Total: 5×30 + 5 + 10 = 165.
pub const TOTAL_CORPUS_SIZE: usize =
    MAIN_PROJECTS.len() * MEMORIES_PER_MAIN_PROJECT + PREFIX_COLLISION_MEMORIES + GLOBAL_MEMORIES;

/// Shared tag on every bench memory; D1 + D4 use this as a broad query.
pub const SHARED_TAG: &str = "isolation_bench";

// ── Result structs ──────────────────────────────────────────────────────

/// One bench memory (corpus row).
#[derive(Debug, Clone)]
pub struct BenchMemory {
    pub id: String,
    pub memory_type: String,
    /// `Some("alpha")` for project-scoped; `None` for global.
    pub project: Option<String>,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub confidence: f32,
    pub embedding: Vec<f32>,
}

/// Generated dataset for one bench seed.
#[derive(Debug, Clone)]
pub struct Corpus {
    pub memories: Vec<BenchMemory>,
}

impl Corpus {
    pub fn count_by_project(&self, project: Option<&str>) -> usize {
        self.memories
            .iter()
            .filter(|m| m.project.as_deref() == project)
            .count()
    }
}

/// One dimension's score with pass/fail eval.
#[derive(Debug, Clone, Serialize)]
pub struct DimensionScore {
    pub name: &'static str,
    pub score: f64,
    pub min: f64,
    pub pass: bool,
}

/// One infrastructure assertion's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct InfrastructureCheck {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

/// Top-level summary.json contract — mirrors `forge_identity::IdentityScore`.
#[derive(Debug, Clone, Serialize)]
pub struct IsolationScore {
    pub seed: u64,
    pub composite: f64,
    pub dimensions: [DimensionScore; 6],
    pub infrastructure_checks: Vec<InfrastructureCheck>,
    pub pass: bool,
    pub wall_duration_ms: u64,
}

/// Bench-runner config knobs (mirrors forge_identity::BenchConfig).
#[derive(Debug, Clone)]
pub struct BenchConfig {
    pub seed: u64,
    pub output_dir: std::path::PathBuf,
    pub expected_composite: Option<f64>,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            output_dir: std::path::PathBuf::from("bench_results_forge_isolation"),
            expected_composite: None,
        }
    }
}

// ── Corpus generator (T3, spec §3.2) ────────────────────────────────────

/// Deterministic per-index confidence formula per spec §3.2 M4 fix:
/// `0.7 + (idx * 0.01).clamp(0.0, 0.29)` — produces values in [0.70, 0.99]
/// for `idx ∈ 0..30`; clamps for idx ≥ 30 (only relevant if a future
/// extension grows MEMORIES_PER_MAIN_PROJECT past 30).
fn deterministic_confidence(idx: usize) -> f32 {
    0.7 + (idx as f32 * 0.01).clamp(0.0, 0.29)
}

/// Generate the 165-memory corpus per spec §3.2.
///
/// The function takes a `ChaCha20Rng` for signature-consistency with other
/// bench harnesses but does not consume randomness from it — corpus content
/// is fully derived by formula from project name + index, and embeddings
/// come from [`deterministic_embedding`] which seeds its own internal RNG
/// from the seed_key string. This is intentional: removing the
/// `random_range` sampling edge eliminates one degree of cross-rustc-version
/// drift risk (per v1 review M4 fix).
///
/// Layout:
/// - 5 main projects × 30 each (20 lessons + 10 decisions) = 150
/// - 1 prefix-collision sentinel ("alphabet") × 5 lessons = 5
/// - 10 globals (project=None; 6 patterns + 4 decisions)
/// - Total: 165
pub fn generate_corpus(_rng: &mut ChaCha20Rng) -> Corpus {
    let mut memories = Vec::with_capacity(TOTAL_CORPUS_SIZE);

    // Main projects (5 × 30).
    for project in MAIN_PROJECTS {
        for idx in 0..MEMORIES_PER_MAIN_PROJECT {
            let memory_type = if idx < 20 { "lesson" } else { "decision" };
            let title = format!("{project}_secret_{idx}");
            memories.push(BenchMemory {
                id: format!("isolation_{project}_{idx}"),
                memory_type: memory_type.to_string(),
                project: Some(project.to_string()),
                content: format!(
                    "In project {project}, the topic_{idx} pattern uses \
                     {project}_secret_{idx} as the canonical detail."
                ),
                tags: vec![project.to_string(), SHARED_TAG.to_string()],
                confidence: deterministic_confidence(idx),
                embedding: deterministic_embedding(&title),
                title,
            });
        }
    }

    // Prefix-collision sentinel (alphabet × 5). Exists only to drive
    // D5 probe 5: a query for "alpha" must NOT return alphabet's memories.
    for idx in 0..PREFIX_COLLISION_MEMORIES {
        let title = format!("{PREFIX_COLLISION_PROJECT}_secret_{idx}");
        memories.push(BenchMemory {
            id: format!("isolation_{PREFIX_COLLISION_PROJECT}_{idx}"),
            memory_type: "lesson".to_string(),
            project: Some(PREFIX_COLLISION_PROJECT.to_string()),
            content: format!(
                "In project {PREFIX_COLLISION_PROJECT}, sentinel_{idx} \
                 pattern uses {PREFIX_COLLISION_PROJECT}_secret_{idx} \
                 as the canonical detail."
            ),
            tags: vec![PREFIX_COLLISION_PROJECT.to_string(), SHARED_TAG.to_string()],
            confidence: deterministic_confidence(idx),
            embedding: deterministic_embedding(&title),
            title,
        });
    }

    // Globals (project=None × 10).
    for idx in 0..GLOBAL_MEMORIES {
        let memory_type = if idx < 6 { "pattern" } else { "decision" };
        let title = format!("global_pattern_{idx}");
        memories.push(BenchMemory {
            id: format!("isolation_global_{idx}"),
            memory_type: memory_type.to_string(),
            project: None,
            content: format!(
                "Global pattern_{idx}: this knowledge applies across all \
                 projects regardless of scope."
            ),
            tags: vec!["global".to_string(), SHARED_TAG.to_string()],
            confidence: deterministic_confidence(idx),
            embedding: deterministic_embedding(&title),
            title,
        });
    }

    Corpus { memories }
}

/// Seed all 165 corpus memories into the bench DaemonState.
///
/// Each memory is INSERT'd directly via SQL (matches forge_identity's
/// pattern at `forge_identity.rs:168`) — bypassing the dedup +
/// source-priority branching in higher-level helpers so the corpus is
/// persisted byte-identical. After each row inserts, the embedding is
/// stored via [`crate::db::vec::store_embedding`] which the
/// `memory_vec` virtual table needs for KNN queries.
///
/// Returns the number of memories successfully seeded; abort the bench if
/// this is anything other than `TOTAL_CORPUS_SIZE` (infrastructure check
/// `corpus_size_matches_spec` catches it).
pub fn seed_corpus(state: &mut DaemonState, corpus: &Corpus) -> rusqlite::Result<usize> {
    const NOW_ISO: &str = "2026-04-25T00:00:00Z";

    for m in &corpus.memories {
        let tags_json = serde_json::to_string(&m.tags).unwrap_or_else(|_| "[]".to_string());
        state.conn.execute(
            "INSERT INTO memory
                (id, memory_type, title, content, confidence, status,
                 project, tags, created_at, accessed_at, organization_id)
             VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?8, 'default')",
            rusqlite::params![
                m.id,
                m.memory_type,
                m.title,
                m.content,
                f64::from(m.confidence),
                m.project,
                tags_json,
                NOW_ISO,
            ],
        )?;
        crate::db::vec::store_embedding(&state.conn, &m.id, &m.embedding)?;
    }
    Ok(corpus.memories.len())
}

// ── Dimension implementations (T4-T6) ───────────────────────────────────

/// Returns true if `title` starts with any foreign-project's `_secret_`
/// token relative to the probing project. Globals (titles starting with
/// `global_pattern_`) and the probing project's own titles are NOT
/// foreign (per spec §3.3 — foreign-token denominator excludes globals).
fn is_foreign_token(title: &str, probing_project: &str) -> bool {
    for other in MAIN_PROJECTS
        .iter()
        .copied()
        .chain(std::iter::once(PREFIX_COLLISION_PROJECT))
    {
        if other == probing_project {
            continue;
        }
        let prefix = format!("{other}_secret_");
        if title.starts_with(&prefix) {
            return true;
        }
    }
    false
}

/// **D1 — cross_project_precision** (T4, weight 0.25, min 0.95).
///
/// Per spec §3.1 + §3.3. For each main project P: recall the shared-tag
/// query `"isolation_bench"` scoped to P; assert no foreign-project
/// memories appear in the result. Foreign-token denominator excludes
/// globals (recallable from every project) and includes the alphabet
/// sentinel (prefix-collision audit).
///
/// Per project: `max_possible = (N-1) × 30 + 5 = 125` (4 other main projects'
/// memories + 5 alphabet-sentinel memories).
fn dim_1_cross_project_precision(state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    let mut sum_score = 0.0;

    for project in MAIN_PROJECTS {
        let results =
            crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, Some(project), 200)
                .unwrap_or_default();

        let foreign_count = results
            .iter()
            .filter(|r| is_foreign_token(&r.title, project))
            .count();

        let max_possible =
            (MAIN_PROJECTS.len() - 1) * MEMORIES_PER_MAIN_PROJECT + PREFIX_COLLISION_MEMORIES;
        let score_p = 1.0 - (foreign_count as f64 / max_possible as f64);
        sum_score += score_p;
    }

    let score = sum_score / MAIN_PROJECTS.len() as f64;
    DimensionScore {
        name: "cross_project_precision",
        score,
        min: DIM_MINIMUMS[0],
        pass: false,
    }
}

/// **D2 — self_recall_completeness** (T4, weight 0.15, min 0.85).
///
/// Per spec §3.1 + §3.3. For each main project P: recall the project's
/// secret prefix `"{P}_secret"` scoped to P; assert recall@10 covers the
/// project's 10 most relevant memories.
///
/// Note: BM25 ranks rows containing the term; with 30 candidates per project
/// each containing the prefix in title + content, recall@10 ≥ 10/10 = 1.0
/// is achievable when no leakage occurs. Cross-project leakage would push
/// some of the project's own memories OUT of the top 10.
fn dim_2_self_recall_completeness(state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    let mut sum_recall = 0.0;
    const TOP_K: usize = 10;

    for project in MAIN_PROJECTS {
        let query = format!("{project}_secret");
        let results = crate::db::ops::recall_bm25_project(&state.conn, &query, Some(project), 50)
            .unwrap_or_default();

        let project_id_prefix = format!("isolation_{project}_");
        let own_hits_in_top_k = results
            .iter()
            .take(TOP_K)
            .filter(|r| r.id.starts_with(&project_id_prefix))
            .count();

        let expected = TOP_K.min(MEMORIES_PER_MAIN_PROJECT);
        let recall_at_k = own_hits_in_top_k as f64 / expected as f64;
        sum_recall += recall_at_k;
    }

    let score = sum_recall / MAIN_PROJECTS.len() as f64;
    DimensionScore {
        name: "self_recall_completeness",
        score,
        min: DIM_MINIMUMS[1],
        pass: false,
    }
}

/// **D3 — global_memory_visibility** (T5, weight 0.10, min 0.90).
///
/// Per spec §3.1 + §3.3. Globals (project=None) must appear in every main
/// project's recall — they're meant to be visible cross-project. For each
/// project P: `Recall { query: "global_pattern", project: Some(P), limit: 50 }`;
/// score = (globals seen / total globals) averaged across projects.
fn dim_3_global_memory_visibility(state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    let mut sum_rate = 0.0;

    for project in MAIN_PROJECTS {
        let results =
            crate::db::ops::recall_bm25_project(&state.conn, "global_pattern", Some(project), 50)
                .unwrap_or_default();

        let globals_seen = results
            .iter()
            .filter(|r| r.id.starts_with("isolation_global_"))
            .count();

        let rate = globals_seen as f64 / GLOBAL_MEMORIES as f64;
        sum_rate += rate;
    }

    let score = sum_rate / MAIN_PROJECTS.len() as f64;
    DimensionScore {
        name: "global_memory_visibility",
        score,
        min: DIM_MINIMUMS[2],
        pass: false,
    }
}

/// **D4 — unscoped_query_breadth** (T5, weight 0.10, min 0.85).
///
/// Per spec §3.1 + §3.3. With `project=None`, the recall must span all 6
/// buckets (5 main projects + global pool — alphabet sentinel intentionally
/// excluded since it's a D5-only construct). Score = bucket_coverage / 6.
fn dim_4_unscoped_query_breadth(state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    let results =
        crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, None, 200).unwrap_or_default();

    let mut buckets: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    for r in &results {
        for project in MAIN_PROJECTS {
            let prefix = format!("isolation_{project}_");
            if r.id.starts_with(&prefix) {
                buckets.insert(project);
            }
        }
        if r.id.starts_with("isolation_global_") {
            buckets.insert("__global__");
        }
    }

    // Expected buckets: 5 main projects + global pool = 6.
    let expected = MAIN_PROJECTS.len() + 1; // global pool counts once
    let score = buckets.len() as f64 / expected as f64;

    DimensionScore {
        name: "unscoped_query_breadth",
        score,
        min: DIM_MINIMUMS[3],
        pass: false,
    }
}

/// **D5 — edge_case_resilience** (T6, weight 0.15, min 0.85).
///
/// Per spec §3.1a — 7 sub-probes; score = pass_count / 7.
///
/// 1. `empty_string_targets_global` — `Some("")` recall returns ONLY globals
/// 2. `special_chars_no_panic` — `Some("p@#$%")` does not panic
/// 3. `overlong_project_no_panic` — 256-char project does not panic
/// 4. `sql_injection_inert` — `Some("alpha'; DROP TABLE memory;--")` does
///    not drop OR mutate; sentinel-row hash check (N4 fix)
/// 5. `prefix_collision_isolated` — `Some("alpha")` excludes `alphabet`
/// 6. `case_sensitivity_strict` — `Some("ALPHA")` excludes `alpha` corpus
/// 7. `trailing_whitespace_strict` — `Some(" alpha")` excludes `alpha`
fn dim_5_edge_case_resilience(state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    let mut passes = 0u32;

    // Probe 1: empty-string targets global pool only.
    if let Ok(results) = crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, Some(""), 200)
    {
        // No project-scoped (`{P}_secret_*`) memories should appear.
        let foreign = results.iter().any(|r| {
            MAIN_PROJECTS
                .iter()
                .any(|p| r.title.starts_with(&format!("{p}_secret_")))
                || r.title
                    .starts_with(&format!("{PREFIX_COLLISION_PROJECT}_secret_"))
        });
        if !foreign {
            passes += 1;
        }
    }

    // Probe 2: special chars don't panic; result is Ok.
    if crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, Some("p@#$%"), 50).is_ok() {
        passes += 1;
    }

    // Probe 3: 256-char project doesn't panic.
    let long_proj: String = "x".repeat(256);
    if crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, Some(&long_proj), 50).is_ok() {
        passes += 1;
    }

    // Probe 4: SQL injection inert. N4 fix — sentinel-row hash check.
    // Per MED-3 fix: assert the call returns Ok (not just any result) AND
    // sentinel-row state unchanged. Catches DROP/DELETE (count delta) +
    // UPDATE-class corruption (hash delta) + upstream short-circuit (Ok
    // requirement: if a sanitizer ever rejects the dangerous string before
    // it reaches the bind layer, this probe still validates that the call
    // path returned a normal Ok rather than crashing or hanging).
    let pre_hash = sentinel_row_hash(state);
    let pre_count: i64 = state
        .conn
        .query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0))
        .unwrap_or(-1);

    let inj_call_ok = crate::db::ops::recall_bm25_project(
        &state.conn,
        SHARED_TAG,
        Some("alpha'; DROP TABLE memory;--"),
        50,
    )
    .is_ok();

    let post_hash = sentinel_row_hash(state);
    let post_count: i64 = state
        .conn
        .query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0))
        .unwrap_or(-2);
    if inj_call_ok && pre_count == post_count && pre_hash == post_hash && pre_hash.is_some() {
        passes += 1;
    }

    // Probe 5: prefix-collision — query=alpha excludes alphabet.
    if let Ok(results) =
        crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, Some("alpha"), 50)
    {
        let saw_alphabet = results.iter().any(|r| {
            r.title
                .starts_with(&format!("{PREFIX_COLLISION_PROJECT}_secret_"))
        });
        if !saw_alphabet {
            passes += 1;
        }
    }

    // Probe 6: case sensitivity — query="ALPHA" excludes "alpha" corpus.
    if let Ok(results) =
        crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, Some("ALPHA"), 50)
    {
        let saw_alpha = results.iter().any(|r| r.title.starts_with("alpha_secret_"));
        if !saw_alpha {
            passes += 1;
        }
    }

    // Probe 7: trailing whitespace strict.
    if let Ok(results) =
        crate::db::ops::recall_bm25_project(&state.conn, SHARED_TAG, Some(" alpha"), 50)
    {
        let saw_alpha = results.iter().any(|r| r.title.starts_with("alpha_secret_"));
        if !saw_alpha {
            passes += 1;
        }
    }

    let score = f64::from(passes) / 7.0;
    DimensionScore {
        name: "edge_case_resilience",
        score,
        min: DIM_MINIMUMS[4],
        pass: false,
    }
}

/// Compute SHA-256 hash of a canary memory's `(title, content, project, tags)`
/// to detect mutation-class SQL-injection regressions (per spec §3.1a probe 4
/// + N4 fix). Returns `None` if the canary row is missing (which itself is
///   a regression — table was deleted).
fn sentinel_row_hash(state: &DaemonState) -> Option<String> {
    let canary_id = "isolation_alpha_0";
    let row: rusqlite::Result<(String, String, Option<String>, String)> = state.conn.query_row(
        "SELECT title, content, project, tags FROM memory WHERE id = ?1",
        [canary_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    );
    match row {
        Ok((title, content, project, tags)) => {
            let mut payload = String::new();
            payload.push_str(&title);
            payload.push('\0');
            payload.push_str(&content);
            payload.push('\0');
            payload.push_str(project.as_deref().unwrap_or(""));
            payload.push('\0');
            payload.push_str(&tags);
            Some(crate::bench::common::sha256_hex(&payload))
        }
        Err(_) => None,
    }
}

/// **D6 — compile_context_isolation** (T5, weight 0.25, min 0.95).
///
/// Per spec §3.1 + §3.3 + N1 fix (max_possible = 15 not 120) + N3 fix
/// (pinned ContextInjectionConfig avoids brittleness against future
/// Default::default() flips).
///
/// For each main project P: call `compile_dynamic_suffix_with_inj` with
/// `project=Some(P)` and a pinned `ContextInjectionConfig { session_context:
/// true, .. Default::default() }`. Scan the resulting XML for foreign-project
/// secret tokens. Score per project = `1 - (foreign_tokens_found / 15)`;
/// dimension score = mean across projects.
///
/// max_possible = 15 because compile_dynamic_suffix renders at most
/// `decisions_limit (10) + lessons_limit (5)` rows per project per config
/// defaults. Tight denominator means a 1-row regression scores 0.933 < 0.95
/// min and is CAUGHT.
fn dim_6_compile_context_isolation(state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    let ctx_config = crate::config::ContextConfig::default();
    let pinned_inj = crate::config::ContextInjectionConfig {
        session_context: true,
        ..Default::default()
    };
    // Per HIGH-1 fix: hardcode max_possible to spec §3.3 invariant value.
    // Don't read from runtime config — if a developer overrides
    // decisions_limit/lessons_limit, the spec invariant must hold regardless
    // (it's a property of the bench, not the live config). debug_assert
    // catches drift in tests but never relaxes the bench in release.
    const SPEC_MAX_POSSIBLE: f64 = 15.0;
    debug_assert!(
        (ctx_config.decisions_limit + ctx_config.lessons_limit) as f64 == SPEC_MAX_POSSIBLE,
        "spec §3.3 N1 fix invariant: decisions_limit + lessons_limit must equal 15 \
         (actual {}+{}={}); bench scoring would drift if config defaults change",
        ctx_config.decisions_limit,
        ctx_config.lessons_limit,
        ctx_config.decisions_limit + ctx_config.lessons_limit,
    );

    let mut sum_score = 0.0;

    for project in MAIN_PROJECTS {
        let (xml, _excluded) = crate::recall::compile_dynamic_suffix_with_inj(
            &state.conn,
            "isolation_bench_agent",
            Some(project),
            &ctx_config,
            &[],
            None,
            None,
            None,
            &pinned_inj,
        );

        // Per spec §3.3 D6 formula: foreign-token enumeration is over
        // `Q in projects if Q != P` — i.e. the 5 main projects.
        // The alphabet sentinel is a D5-only construct (prefix-collision
        // probe); spec §3.2 documents it does not participate in D6.
        // Per HIGH-1 fix: previous impl unconditionally added alphabet
        // foreign-tokens on top of the main-projects loop, which is a
        // spec deviation (would over-count if the SQL filter regressed).
        let mut foreign_tokens = 0usize;
        for other in MAIN_PROJECTS {
            if other == project {
                continue;
            }
            let needle = format!("{other}_secret_");
            foreign_tokens += xml.matches(&needle).count();
        }

        let score_p = (1.0 - foreign_tokens as f64 / SPEC_MAX_POSSIBLE).max(0.0);
        sum_score += score_p;
    }

    let score = sum_score / MAIN_PROJECTS.len() as f64;
    DimensionScore {
        name: "compile_context_isolation",
        score,
        min: DIM_MINIMUMS[5],
        pass: false,
    }
}

// ── Composite scorer (uses lifted bench::scoring) ───────────────────────

fn composite_score(dims: &[DimensionScore; 6]) -> f64 {
    let scores: [f64; 6] = std::array::from_fn(|i| dims[i].score);
    crate::bench::scoring::composite_score(&scores, &DIM_WEIGHTS)
}

fn mark_pass(d: DimensionScore) -> DimensionScore {
    let pass = d.score >= d.min;
    DimensionScore { pass, ..d }
}

// ── Infrastructure assertions (T6 will fill in) ─────────────────────────

/// Spec §3.4 — 8 fail-fast checks before dimensions run.
fn run_infrastructure_checks(state: &mut DaemonState, corpus: &Corpus) -> Vec<InfrastructureCheck> {
    let mut out = Vec::with_capacity(8);

    // 1. memory_project_index_exists
    let idx_exists: bool = state
        .conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='index' AND name='idx_memory_project'",
            [],
            |_r| Ok(true),
        )
        .unwrap_or(false);
    out.push(InfrastructureCheck {
        name: "memory_project_index_exists",
        passed: idx_exists,
        detail: if idx_exists {
            "idx_memory_project present in sqlite_master".into()
        } else {
            "idx_memory_project MISSING".into()
        },
    });

    // 2. memory_project_column_exists
    let col_exists: bool = state
        .conn
        .prepare("SELECT project FROM memory LIMIT 0")
        .is_ok();
    out.push(InfrastructureCheck {
        name: "memory_project_column_exists",
        passed: col_exists,
        detail: if col_exists {
            "memory.project column accessible".into()
        } else {
            "memory.project column MISSING".into()
        },
    });

    // 3. recall_accepts_project_filter — sanity call returns Ok.
    let probe =
        crate::db::ops::recall_bm25_project(&state.conn, "ping", Some("test_alpha"), 1).is_ok();
    out.push(InfrastructureCheck {
        name: "recall_accepts_project_filter",
        passed: probe,
        detail: if probe {
            "recall_bm25_project returned Ok with Some(project)".into()
        } else {
            "recall_bm25_project errored on project filter".into()
        },
    });

    // 4. seeded_rng_deterministic — same seed -> same first u64.
    use rand::RngExt;
    let mut a = seeded_rng(42);
    let mut b = seeded_rng(42);
    let v_a: u64 = a.random();
    let v_b: u64 = b.random();
    let det = v_a == v_b;
    out.push(InfrastructureCheck {
        name: "seeded_rng_deterministic",
        passed: det,
        detail: if det {
            "seeded_rng(42) produces same u64 twice".into()
        } else {
            format!("seeded_rng diverged: {v_a} != {v_b}")
        },
    });

    // 5. corpus_size_matches_spec
    let size_match = corpus.memories.len() == TOTAL_CORPUS_SIZE;
    out.push(InfrastructureCheck {
        name: "corpus_size_matches_spec",
        passed: size_match,
        detail: format!(
            "corpus has {} rows (expected {})",
            corpus.memories.len(),
            TOTAL_CORPUS_SIZE
        ),
    });

    // 6. project_distribution_correct
    let mut dist_ok = true;
    let mut dist_detail = String::new();
    for project in MAIN_PROJECTS {
        let n = corpus.count_by_project(Some(project));
        if n != MEMORIES_PER_MAIN_PROJECT {
            dist_ok = false;
            dist_detail.push_str(&format!("{project}={n}; "));
        }
    }
    if corpus.count_by_project(Some(PREFIX_COLLISION_PROJECT)) != PREFIX_COLLISION_MEMORIES {
        dist_ok = false;
        dist_detail.push_str(&format!(
            "{PREFIX_COLLISION_PROJECT}={}; ",
            corpus.count_by_project(Some(PREFIX_COLLISION_PROJECT))
        ));
    }
    if corpus.count_by_project(None) != GLOBAL_MEMORIES {
        dist_ok = false;
        dist_detail.push_str(&format!("globals={}", corpus.count_by_project(None)));
    }
    out.push(InfrastructureCheck {
        name: "project_distribution_correct",
        passed: dist_ok,
        detail: if dist_ok {
            "5×30 + 5 + 10 = 165 confirmed".into()
        } else {
            format!("distribution drift: {dist_detail}")
        },
    });

    // 7. embedding_dim_matches_consolidation
    let dim_ok = corpus
        .memories
        .first()
        .is_some_and(|m| m.embedding.len() == crate::bench::common::DETERMINISTIC_EMBEDDING_DIM);
    out.push(InfrastructureCheck {
        name: "embedding_dim_matches_consolidation",
        passed: dim_ok,
        detail: format!(
            "first memory embedding.len() = {}, expected {}",
            corpus
                .memories
                .first()
                .map(|m| m.embedding.len())
                .unwrap_or(0),
            crate::bench::common::DETERMINISTIC_EMBEDDING_DIM,
        ),
    });

    // 8. compile_context_returns_xml — non-empty result + contains the
    // expected `<forge-dynamic>` root tag. Per HIGH-2 fix: spec §3.4
    // originally said "containing `<context>`" but the actual tag emitted
    // by `compile_dynamic_suffix_with_inj` is `<forge-dynamic>`. The
    // assertion now matches reality (catches both empty-string regressions
    // AND a future helper rename that would change the root tag).
    let ctx_config = crate::config::ContextConfig::default();
    let pinned_inj = crate::config::ContextInjectionConfig {
        session_context: true,
        ..Default::default()
    };
    let (xml, _excluded) = crate::recall::compile_dynamic_suffix_with_inj(
        &state.conn,
        "isolation_bench_agent",
        Some("alpha"),
        &ctx_config,
        &[],
        None,
        None,
        None,
        &pinned_inj,
    );
    let xml_non_empty = !xml.is_empty();
    let xml_has_root = xml.contains("<forge-dynamic>");
    let xml_ok = xml_non_empty && xml_has_root;
    out.push(InfrastructureCheck {
        name: "compile_context_returns_xml",
        passed: xml_ok,
        detail: if xml_ok {
            format!(
                "compile_dynamic_suffix_with_inj returned {} chars containing <forge-dynamic>",
                xml.len()
            )
        } else if !xml_non_empty {
            "compile_dynamic_suffix_with_inj returned empty string".into()
        } else {
            format!(
                "compile_dynamic_suffix_with_inj returned {} chars but no <forge-dynamic> root tag",
                xml.len()
            )
        },
    });

    out
}

// ── Orchestrator (single shared DaemonState per spec §3.7) ──────────────

/// Run the bench against a pre-seeded `Connection`. T3 builds the corpus
/// and seeds the connection; this fn runs the 6 dims + infra checks.
///
/// Per §3.7, all dims share the SAME connection — per-dim isolation is
/// the wrong primitive for an isolation bench because it would HIDE
/// cross-dim leakage.
pub fn run_bench_in_state(state: &mut DaemonState, corpus: &Corpus, seed: u64) -> IsolationScore {
    let start = std::time::Instant::now();

    let infra = run_infrastructure_checks(state, corpus);
    let infra_pass = infra.iter().all(|c| c.passed);

    // Per MED-4 fix: when infra checks fail, ALL dimensions are zeroed
    // (not just the composite). Avoids inconsistent summary.json where
    // composite=0.0 but per-dim scores are populated — confusing for
    // downstream readers. Mirrors forge_identity::run_bench precedent.
    let dimensions: [DimensionScore; 6] = if infra_pass {
        let dims_raw = [
            dim_1_cross_project_precision(state, corpus),
            dim_2_self_recall_completeness(state, corpus),
            dim_3_global_memory_visibility(state, corpus),
            dim_4_unscoped_query_breadth(state, corpus),
            dim_5_edge_case_resilience(state, corpus),
            dim_6_compile_context_isolation(state, corpus),
        ];
        std::array::from_fn(|i| mark_pass(dims_raw[i].clone()))
    } else {
        // Match the exact dim names + min thresholds the live path uses.
        const ZEROED_DIM_NAMES: [&str; 6] = [
            "cross_project_precision",
            "self_recall_completeness",
            "global_memory_visibility",
            "unscoped_query_breadth",
            "edge_case_resilience",
            "compile_context_isolation",
        ];
        std::array::from_fn(|i| DimensionScore {
            name: ZEROED_DIM_NAMES[i],
            score: 0.0,
            min: DIM_MINIMUMS[i],
            pass: false,
        })
    };
    let composite = if infra_pass {
        composite_score(&dimensions)
    } else {
        0.0 // Abort with failure if infra checks fail.
    };

    let dims_pass = dimensions.iter().all(|d| d.pass);
    let pass = infra_pass && dims_pass && composite >= COMPOSITE_THRESHOLD;

    IsolationScore {
        seed,
        composite,
        dimensions,
        infrastructure_checks: infra,
        pass,
        wall_duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// Top-level entry point used by the `forge-bench forge-isolation` CLI
/// (T7) and integration tests.
///
/// Builds a fresh `DaemonState::new(":memory:")` (which sets up the full
/// schema + FTS triggers + memory_vec virtual table), seeds the corpus via
/// [`seed_corpus`], then dispatches to [`run_bench_in_state`] for the 6
/// dimension probes + 8 infrastructure checks.
///
/// Writes `summary.json` to `config.output_dir` (mirrors forge-identity).
/// Returns the [`IsolationScore`].
pub fn run_bench(config: &BenchConfig) -> IsolationScore {
    let mut rng = seeded_rng(config.seed);
    let corpus = generate_corpus(&mut rng);

    let mut state =
        DaemonState::new(":memory:").expect("DaemonState::new(:memory:) for forge-isolation");
    let seeded = seed_corpus(&mut state, &corpus).expect("seed_corpus for forge-isolation");
    debug_assert_eq!(
        seeded, TOTAL_CORPUS_SIZE,
        "seed_corpus should insert exactly TOTAL_CORPUS_SIZE rows"
    );

    let score = run_bench_in_state(&mut state, &corpus, config.seed);

    // Best-effort: write summary.json. Don't panic on failure — bench
    // is informational; CLI also captures stderr summary.
    if let Err(e) = std::fs::create_dir_all(&config.output_dir) {
        tracing::warn!(error = %e, dir = %config.output_dir.display(),
            "failed to create forge-isolation output_dir");
    } else {
        let path = config.output_dir.join("summary.json");
        match serde_json::to_string_pretty(&score) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!(error = %e, path = %path.display(),
                        "failed to write forge-isolation summary.json");
                }
            }
            Err(e) => tracing::warn!(error = %e, "summary.json serialization failed"),
        }
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_weights_sum_to_one() {
        let sum: f64 = DIM_WEIGHTS.iter().sum();
        assert!((sum - 1.0).abs() < 1e-9, "DIM_WEIGHTS sum = {sum}");
    }

    #[test]
    fn corpus_size_constant_matches_spec() {
        assert_eq!(TOTAL_CORPUS_SIZE, 165, "5×30 + 5 + 10 = 165 per spec §3.2");
    }

    #[test]
    fn dim_minimums_match_spec() {
        assert_eq!(DIM_MINIMUMS, [0.95, 0.85, 0.90, 0.85, 0.85, 0.95]);
    }

    #[test]
    fn composite_threshold_is_0_95() {
        assert!((COMPOSITE_THRESHOLD - 0.95).abs() < 1e-9);
    }

    #[test]
    fn end_to_end_run_passes_on_seed_42() {
        // Post-T6: real impl. Composite should hit ≥ 0.95 with all dims passing.
        let score = run_bench(&BenchConfig {
            seed: 42,
            output_dir: std::path::PathBuf::from("/tmp"),
            expected_composite: None,
        });
        assert_eq!(score.seed, 42);
        assert_eq!(score.dimensions.len(), 6);
        assert_eq!(score.infrastructure_checks.len(), 8);
        assert!(
            score.infrastructure_checks.iter().all(|c| c.passed),
            "all 8 infra checks must pass; failing: {:?}",
            score
                .infrastructure_checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| (c.name, &c.detail))
                .collect::<Vec<_>>()
        );
        assert!(
            score.composite >= 0.95,
            "composite must be >= 0.95 on seed=42 clean corpus, got {}",
            score.composite
        );
        assert!(
            score.pass,
            "score.pass must be true; per-dim scores: {:?}",
            score
                .dimensions
                .iter()
                .map(|d| (d.name, d.score, d.pass))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn deterministic_embedding_call_in_skeleton() {
        // Sanity: lifted helper is callable from this module.
        let v = deterministic_embedding("forge-isolation-skeleton-test");
        assert_eq!(v.len(), 768);
    }

    #[test]
    fn corpus_generator_produces_165_memories() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        assert_eq!(corpus.memories.len(), TOTAL_CORPUS_SIZE);
        assert_eq!(corpus.memories.len(), 165);
    }

    #[test]
    fn corpus_distribution_matches_spec_3_2() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        // 5 main projects × 30 each.
        for project in MAIN_PROJECTS {
            assert_eq!(
                corpus.count_by_project(Some(project)),
                MEMORIES_PER_MAIN_PROJECT,
                "main project {project} should have 30 memories"
            );
        }
        // alphabet sentinel × 5.
        assert_eq!(
            corpus.count_by_project(Some(PREFIX_COLLISION_PROJECT)),
            PREFIX_COLLISION_MEMORIES
        );
        // Globals (project=None) × 10.
        assert_eq!(corpus.count_by_project(None), GLOBAL_MEMORIES);
    }

    #[test]
    fn corpus_titles_carry_project_token() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        // Every alpha-scoped memory's title contains "alpha_secret_".
        for m in corpus
            .memories
            .iter()
            .filter(|m| m.project.as_deref() == Some("alpha"))
        {
            assert!(
                m.title.starts_with("alpha_secret_"),
                "alpha memory title should start with `alpha_secret_`, got {}",
                m.title
            );
        }
        // alphabet-scoped memories' titles do NOT start with "alpha_secret_"
        // (they start with "alphabet_secret_") — critical for D5 prefix-collision probe.
        for m in corpus
            .memories
            .iter()
            .filter(|m| m.project.as_deref() == Some(PREFIX_COLLISION_PROJECT))
        {
            assert!(m.title.starts_with("alphabet_secret_"));
            assert!(!m.title.starts_with("alpha_secret_"));
        }
    }

    #[test]
    fn corpus_confidence_is_deterministic_and_in_range() {
        let mut rng_a = seeded_rng(42);
        let mut rng_b = seeded_rng(42);
        let corpus_a = generate_corpus(&mut rng_a);
        let corpus_b = generate_corpus(&mut rng_b);
        // Same seed → same corpus (byte-identical confidences).
        assert_eq!(corpus_a.memories.len(), corpus_b.memories.len());
        for (a, b) in corpus_a.memories.iter().zip(corpus_b.memories.iter()) {
            assert_eq!(a.confidence, b.confidence);
            // All confidences in [0.70, 0.99].
            assert!((0.70..=0.99).contains(&a.confidence));
        }
    }

    #[test]
    fn corpus_embedding_dim_is_768() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        for m in &corpus.memories {
            assert_eq!(m.embedding.len(), 768);
        }
    }

    #[test]
    fn d1_perfect_isolation_on_seeded_corpus() {
        // With the corpus correctly seeded and project scoping working,
        // D1 score should be 1.0 (no foreign tokens in any project's recall).
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let mut state = DaemonState::new(":memory:").expect("daemonstate");
        seed_corpus(&mut state, &corpus).expect("seed");
        let dim = dim_1_cross_project_precision(&mut state, &corpus);
        assert_eq!(dim.name, "cross_project_precision");
        assert!(
            dim.score >= 0.95,
            "D1 should be at least min 0.95 on a clean corpus, got {}",
            dim.score
        );
    }

    #[test]
    fn d3_global_visibility_on_seeded_corpus() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let mut state = DaemonState::new(":memory:").expect("daemonstate");
        seed_corpus(&mut state, &corpus).expect("seed");
        let dim = dim_3_global_memory_visibility(&mut state, &corpus);
        assert_eq!(dim.name, "global_memory_visibility");
        assert!(
            dim.score >= 0.90,
            "D3 should be at least min 0.90 on a clean corpus, got {}",
            dim.score
        );
    }

    #[test]
    fn d4_unscoped_breadth_on_seeded_corpus() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let mut state = DaemonState::new(":memory:").expect("daemonstate");
        seed_corpus(&mut state, &corpus).expect("seed");
        let dim = dim_4_unscoped_query_breadth(&mut state, &corpus);
        assert_eq!(dim.name, "unscoped_query_breadth");
        assert!(
            dim.score >= 0.85,
            "D4 should be at least min 0.85 on a clean corpus, got {}",
            dim.score
        );
    }

    #[test]
    fn d6_compile_context_isolation_on_seeded_corpus() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let mut state = DaemonState::new(":memory:").expect("daemonstate");
        seed_corpus(&mut state, &corpus).expect("seed");
        let dim = dim_6_compile_context_isolation(&mut state, &corpus);
        assert_eq!(dim.name, "compile_context_isolation");
        assert!(
            dim.score >= 0.95,
            "D6 should be at least min 0.95 on a clean corpus, got {}",
            dim.score
        );
    }

    #[test]
    fn d2_self_recall_completeness_on_seeded_corpus() {
        // With each project's 30 memories carrying the `_secret` prefix in
        // title + content, recall@10 with the per-project prefix query and
        // project=Some(P) should saturate at 1.0 (top 10 are all own memories).
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let mut state = DaemonState::new(":memory:").expect("daemonstate");
        seed_corpus(&mut state, &corpus).expect("seed");
        let dim = dim_2_self_recall_completeness(&mut state, &corpus);
        assert_eq!(dim.name, "self_recall_completeness");
        assert!(
            dim.score >= 0.85,
            "D2 should be at least min 0.85 on a clean corpus, got {}",
            dim.score
        );
    }

    #[test]
    fn seed_corpus_inserts_all_165_rows() {
        let mut rng = seeded_rng(42);
        let corpus = generate_corpus(&mut rng);
        let mut state = DaemonState::new(":memory:").expect("daemonstate :memory:");
        let seeded = seed_corpus(&mut state, &corpus).expect("seed");
        assert_eq!(seeded, TOTAL_CORPUS_SIZE);

        // Verify rows landed in the memory table.
        let row_count: i64 = state
            .conn
            .query_row("SELECT COUNT(*) FROM memory", [], |r| r.get(0))
            .expect("count");
        assert_eq!(row_count as usize, TOTAL_CORPUS_SIZE);

        // Verify per-project distribution survived the INSERT.
        for project in MAIN_PROJECTS {
            let n: i64 = state
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM memory WHERE project = ?1",
                    [project],
                    |r| r.get(0),
                )
                .expect("count by project");
            assert_eq!(n as usize, MEMORIES_PER_MAIN_PROJECT);
        }

        // Globals (project IS NULL) — 10.
        let globals: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM memory WHERE project IS NULL",
                [],
                |r| r.get(0),
            )
            .expect("count globals");
        assert_eq!(globals as usize, GLOBAL_MEMORIES);
    }
}
