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
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self { seed: 42 }
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

// ── Dimension stubs (T4-T6 will fill in) ────────────────────────────────

/// SKELETON — T4 implementation per spec §3.1 / §3.3.
fn dim_1_cross_project_precision(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "cross_project_precision",
        score: 0.0,
        min: DIM_MINIMUMS[0],
        pass: false,
    }
}

/// SKELETON — T4 implementation per spec §3.1 / §3.3.
fn dim_2_self_recall_completeness(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "self_recall_completeness",
        score: 0.0,
        min: DIM_MINIMUMS[1],
        pass: false,
    }
}

/// SKELETON — T5 implementation per spec §3.1 / §3.3.
fn dim_3_global_memory_visibility(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "global_memory_visibility",
        score: 0.0,
        min: DIM_MINIMUMS[2],
        pass: false,
    }
}

/// SKELETON — T5 implementation per spec §3.1 / §3.3.
fn dim_4_unscoped_query_breadth(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "unscoped_query_breadth",
        score: 0.0,
        min: DIM_MINIMUMS[3],
        pass: false,
    }
}

/// SKELETON — T6 implementation per spec §3.1a (7 sub-probes).
fn dim_5_edge_case_resilience(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "edge_case_resilience",
        score: 0.0,
        min: DIM_MINIMUMS[4],
        pass: false,
    }
}

/// SKELETON — T5 implementation per spec §3.1 / §3.3 + N1 fix
/// (max_possible = decisions_limit + lessons_limit = 15) + N3 fix (pinned
/// `ContextInjectionConfig { session_context: true, .. }` via
/// `compile_dynamic_suffix_with_inj`).
fn dim_6_compile_context_isolation(_state: &mut DaemonState, _corpus: &Corpus) -> DimensionScore {
    DimensionScore {
        name: "compile_context_isolation",
        score: 0.0,
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
fn run_infrastructure_checks(
    _state: &mut DaemonState,
    _corpus: &Corpus,
) -> Vec<InfrastructureCheck> {
    // T6 will populate these. Skeleton stubs all 8 as passed=false so the
    // run aborts loudly until T6 lands real implementations.
    vec![
        InfrastructureCheck {
            name: "memory_project_index_exists",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
        InfrastructureCheck {
            name: "memory_project_column_exists",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
        InfrastructureCheck {
            name: "recall_accepts_project_filter",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
        InfrastructureCheck {
            name: "seeded_rng_deterministic",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
        InfrastructureCheck {
            name: "corpus_size_matches_spec",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
        InfrastructureCheck {
            name: "project_distribution_correct",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
        InfrastructureCheck {
            name: "embedding_dim_matches_consolidation",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
        InfrastructureCheck {
            name: "compile_context_returns_xml",
            passed: false,
            detail: "stub — T6 to implement".into(),
        },
    ]
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

    let dims_raw = [
        dim_1_cross_project_precision(state, corpus),
        dim_2_self_recall_completeness(state, corpus),
        dim_3_global_memory_visibility(state, corpus),
        dim_4_unscoped_query_breadth(state, corpus),
        dim_5_edge_case_resilience(state, corpus),
        dim_6_compile_context_isolation(state, corpus),
    ];
    let dimensions: [DimensionScore; 6] = std::array::from_fn(|i| mark_pass(dims_raw[i].clone()));
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
/// Returns the [`IsolationScore`]; caller is responsible for serializing
/// summary.json.
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

    run_bench_in_state(&mut state, &corpus, config.seed)
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
    fn skeleton_run_returns_zeroed_failing_score() {
        let score = run_bench(&BenchConfig { seed: 42 });
        assert_eq!(score.seed, 42);
        // Skeleton dims all return 0.0; infra checks all return passed=false.
        // Composite gets short-circuited to 0.0 via the infra-fail branch.
        assert_eq!(score.composite, 0.0);
        assert!(!score.pass);
        assert_eq!(score.dimensions.len(), 6);
        assert_eq!(score.infrastructure_checks.len(), 8);
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
