//! Forge-Consolidation benchmark harness.
//!
//! Tests the daemon's 22-phase consolidation loop across 5 scored dimensions
//! plus infrastructure pass/fail assertions. See
//! `docs/benchmarks/forge-consolidation-design.md` for full design.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use forge_core::types::memory::MemoryType;

use super::common::sha256_hex;

// ── Configuration ────────────────────────────────────────────────

/// Configuration for a single Forge-Consolidation run.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsolidationBenchConfig {
    pub seed: u64,
    pub output_dir: PathBuf,
    /// Expected recall-improvement delta, set during calibration.
    /// `None` means "no threshold yet — first run will print the observed delta."
    pub expected_recall_delta: Option<f64>,
}

impl Default for ConsolidationBenchConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            output_dir: PathBuf::from("bench_results_consolidation"),
            expected_recall_delta: None,
        }
    }
}

// ── Ground truth enums ───────────────────────────────────────────

/// Dataset categories from design doc §4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    ExactDuplicates,     // Category 1
    SemanticDuplicates,  // Category 2
    EmbeddingDuplicates, // Category 3
    Contradictions,      // Category 4
    ReweaveEnrichment,   // Category 5
    LifecycleQuality,    // Category 6
    SelfHealing,         // Category 7
    Infrastructure,      // Category 8
}

/// Expected post-consolidation memory states for ground-truth assertions.
///
/// Most variants map 1-to-1 with the typed `MemoryStatus` enum
/// (`crates/core/src/types/memory.rs`). `Merged` is a **harness-only sentinel**
/// — `MemoryStatus` has no `Merged` variant; the consolidator writes the raw SQL
/// literal `'merged'` at `consolidator.rs:1035` (Phase 14 reweave). If you ever
/// deserialise that row through the typed enum, it falls through to
/// `MemoryStatus::Active`. Audit code (e.g. Task 3 `audit_reweave`) MUST
/// compare against the raw SQL string (`newer_status.as_deref() == Some("merged")`),
/// NOT against `MemoryStatus` enum variants.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedStatus {
    Active,     // memory should remain active
    Superseded, // marked superseded by Phases 1, 2, 5, 7, 12, 20
    Faded,      // marked faded by Phase 4 or Phase 21
    /// Harness-only sentinel. Corresponds to raw SQL status `'merged'` written
    /// by Phase 14 reweave (`consolidator.rs:1035`). `MemoryStatus` enum has no
    /// `Merged` variant — always compare via raw SQL string, never via the enum.
    Merged, // marked merged by Phase 14 (reweave)
    Deleted,    // DELETEd by Phase 1 (exact dedup)
}

// ── GroundTruth and dataset structures ───────────────────────────

/// Ground-truth annotation for a single seeded memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundTruth {
    pub memory_id: String,
    pub category: Category,
    pub expected_status: ExpectedStatus,
    /// For dedup pairs: the ID of the partner memory.
    pub duplicate_of: Option<String>,
    /// For contradiction pairs: the ID of the contradicting memory.
    pub contradicts: Option<String>,
    /// For reweave pairs: the ID of the newer memory that enriches this one.
    pub reweave_source: Option<String>,
    /// For quality scoring: expected post-consolidation quality_score (±0.01).
    pub expected_quality: Option<f64>,
    /// For decay/reconsolidation: expected post-consolidation confidence (±0.01).
    pub expected_confidence: Option<f64>,
    /// For activation decay: expected post-Phase-10 activation_level (±0.01).
    pub expected_activation: Option<f64>,
}

/// A recall query with expected ground-truth results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallQuery {
    pub id: String, // e.g., "RC-1"
    pub query: String,
    pub description: String,
    /// Titles that SHOULD appear in top-10 post-consolidation.
    pub expected_titles: HashSet<String>,
}

/// Full seeded dataset produced by `seed_state()`.
#[derive(Debug, Clone)]
pub struct SeededDataset {
    pub seed: u64,
    pub ground_truth: Vec<GroundTruth>,
    pub recall_queries: Vec<RecallQuery>,
    /// Expected count of new Pattern memories from Phase 5.
    pub expected_pattern_count: usize,
    /// Expected count of new Protocol memories from Phase 17.
    pub expected_protocol_count: usize,
    /// Expected count of new Resolution memories from Phase 12.
    pub expected_resolution_count: usize,
}

// ── Dataset generators: Category 1-4 ─────────────────────────────

/// Spec for a memory to be seeded into the corpus.
#[derive(Debug, Clone)]
pub struct MemorySpec {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub confidence: f64,
    pub valence: String,
    pub intensity: f64,
    pub tags: Vec<String>,
    pub project: String,
    pub access_count: u64,
    pub activation_level: f64,
    pub quality_score: Option<f64>,
    /// `created_at` as ISO-8601, or "NOW" / "NOW-Nd" shortcuts.
    pub created_at_spec: String,
    pub accessed_at_spec: String,
}

/// Category 1: 12 memories in 6 exact-duplicate pairs.
/// Phase 1 should keep higher-confidence copy and DELETE the other.
pub fn generate_category_1_exact_duplicates(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique_token = |idx: usize| sha256_hex(&format!("c1-{seed}-{idx}"));

    let types = [
        MemoryType::Decision,
        MemoryType::Decision,
        MemoryType::Lesson,
        MemoryType::Lesson,
        MemoryType::Pattern,
        MemoryType::Pattern,
    ];

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    for (pair_idx, mt) in types.iter().enumerate() {
        let token = unique_token(pair_idx);
        let title = format!("C1 exact duplicate pair {pair_idx} [{token}]");
        let keeper_id = format!("c1-{pair_idx}-keeper");
        let victim_id = format!("c1-{pair_idx}-victim");

        specs.push(MemorySpec {
            id: keeper_id.clone(),
            memory_type: mt.clone(),
            title: title.clone(),
            content: format!("Exact duplicate pair {pair_idx} keeper — content [{token}]"),
            confidence: 0.9,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-1".into(), format!("pair-{pair_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        specs.push(MemorySpec {
            id: victim_id.clone(),
            memory_type: mt.clone(),
            title: title.clone(), // SAME title triggers Phase 1 exact dedup
            content: format!("Exact duplicate pair {pair_idx} victim — content [{token}]"),
            confidence: 0.7, // LOWER confidence → victim
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-1".into(), format!("pair-{pair_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });

        truths.push(GroundTruth {
            memory_id: keeper_id.clone(),
            category: Category::ExactDuplicates,
            expected_status: ExpectedStatus::Active,
            duplicate_of: Some(victim_id.clone()),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: victim_id,
            category: Category::ExactDuplicates,
            expected_status: ExpectedStatus::Deleted, // Phase 1 DELETEs
            duplicate_of: Some(keeper_id),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    (specs, truths)
}

/// Category 2: 16 memories in 8 semantic near-duplicate pairs.
/// Titles share high word overlap via common anchor token.
pub fn generate_category_2_semantic_duplicates(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    // 8 distinct anchor tokens, one per pair
    let anchors: Vec<String> = (0..8)
        .map(|i| sha256_hex(&format!("c2-anchor-{seed}-{i}")))
        .collect();

    let types = [
        MemoryType::Decision,
        MemoryType::Decision,
        MemoryType::Decision,
        MemoryType::Lesson,
        MemoryType::Lesson,
        MemoryType::Lesson,
        MemoryType::Pattern,
        MemoryType::Pattern,
    ];

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    for (pair_idx, mt) in types.iter().enumerate() {
        let anchor = &anchors[pair_idx];
        // Two paraphrases sharing the anchor token produce >0.65 overlap
        let title_a = format!("Always enforce {anchor} on deployment boundaries");
        let title_b = format!("Enforce {anchor} deployment boundaries always");
        let content_a = format!("Policy: always enforce {anchor} validation before deployment");
        let content_b = format!("Deployment validation must always enforce {anchor}");

        let keeper_id = format!("c2-{pair_idx}-keeper");
        let victim_id = format!("c2-{pair_idx}-victim");

        specs.push(MemorySpec {
            id: keeper_id.clone(),
            memory_type: mt.clone(),
            title: title_a,
            content: content_a,
            confidence: 0.9,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-2".into(), format!("pair-{pair_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        specs.push(MemorySpec {
            id: victim_id.clone(),
            memory_type: mt.clone(),
            title: title_b,
            content: content_b,
            confidence: 0.75, // Lower — becomes victim
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-2".into(), format!("pair-{pair_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });

        truths.push(GroundTruth {
            memory_id: keeper_id.clone(),
            category: Category::SemanticDuplicates,
            expected_status: ExpectedStatus::Active,
            duplicate_of: Some(victim_id.clone()),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: victim_id,
            category: Category::SemanticDuplicates,
            expected_status: ExpectedStatus::Superseded,
            duplicate_of: Some(keeper_id),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    (specs, truths)
}

/// Category 3: 12 memories — 4 embedding-merge pairs + 2 embedding-control pairs.
/// Titles engineered to have LOW word overlap (<0.65) so Phase 2 does NOT catch them.
/// Phase 7 embedding merge catches them via cosine distance < 0.1 (synthetic embeddings added in Task 4).
pub fn generate_category_3_embedding_duplicates(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique = |label: &str, idx: usize| sha256_hex(&format!("c3-{seed}-{label}-{idx}"));

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    // 4 merge pairs — distance < 0.1 (Phase 7 merges lower-confidence victim)
    for pair_idx in 0..4 {
        let token_a = unique("A", pair_idx);
        let token_b = unique("B", pair_idx);
        let keeper_id = format!("c3-merge-{pair_idx}-keeper");
        let victim_id = format!("c3-merge-{pair_idx}-victim");

        specs.push(MemorySpec {
            id: keeper_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("Pattern {token_a}"),
            content: format!("Topic {token_a} rationale follows from context alpha."),
            confidence: 0.9,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-3-merge".into(), format!("pair-{pair_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        specs.push(MemorySpec {
            id: victim_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("Approach {token_b}"), // disjoint anchor → low word overlap
            content: format!("Rationale covers {token_b} derivation from stream beta."),
            confidence: 0.7,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-3-merge".into(), format!("pair-{pair_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });

        truths.push(GroundTruth {
            memory_id: keeper_id.clone(),
            category: Category::EmbeddingDuplicates,
            expected_status: ExpectedStatus::Active,
            duplicate_of: Some(victim_id.clone()),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: victim_id,
            category: Category::EmbeddingDuplicates,
            expected_status: ExpectedStatus::Superseded,
            duplicate_of: Some(keeper_id),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 2 CONTROL pairs — distance 0.15 (Phase 7 does NOT merge)
    for pair_idx in 0..2 {
        let token_a = unique("CA", pair_idx);
        let token_b = unique("CB", pair_idx);
        let a_id = format!("c3-control-{pair_idx}-a");
        let b_id = format!("c3-control-{pair_idx}-b");

        specs.push(MemorySpec {
            id: a_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("Control memory A {token_a}"),
            content: format!("Distinct topic {token_a} with unique content."),
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec![
                "category-3-control".into(),
                format!("control-pair-{pair_idx}"),
            ],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        specs.push(MemorySpec {
            id: b_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("Control memory B {token_b}"),
            content: format!("Separate topic {token_b} unrelated to A."),
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec![
                "category-3-control".into(),
                format!("control-pair-{pair_idx}"),
            ],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });

        // BOTH remain active — these are signal-preservation gate controls
        truths.push(GroundTruth {
            memory_id: a_id,
            category: Category::EmbeddingDuplicates,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: b_id,
            category: Category::EmbeddingDuplicates,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    (specs, truths)
}

/// Category 4: 16 memories in 4 valence + 4 content contradiction pairs.
/// Phase 9a detects valence pairs; Phase 12 synthesizes resolutions.
/// Phase 9b detects content pairs; NO synthesis. All pairs use decision/pattern/protocol
/// types (Phase 9b excludes lesson).
pub fn generate_category_4_contradictions(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique = |idx: usize| sha256_hex(&format!("c4-{seed}-{idx}"));

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    // 4 VALENCE pairs — opposite valence, ≥2 shared tags, intensity > 0.5
    for pair_idx in 0..4 {
        let token = unique(pair_idx);
        let shared_tags = vec![
            "category-4-valence".into(),
            format!("topic-{token}"),
            format!("valence-pair-{pair_idx}"),
        ];
        let pos_id = format!("c4-val-{pair_idx}-pos");
        let neg_id = format!("c4-val-{pair_idx}-neg");

        specs.push(MemorySpec {
            id: pos_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("We should adopt approach {token}"),
            content: format!("Approach {token} solves the problem cleanly."),
            confidence: 0.85,
            valence: "positive".into(),
            intensity: 0.8,
            tags: shared_tags.clone(),
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        specs.push(MemorySpec {
            id: neg_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("We should NOT adopt approach {token}"),
            content: format!("Approach {token} fails under load."),
            confidence: 0.9,
            valence: "negative".into(),
            intensity: 0.9,
            tags: shared_tags,
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });

        // Phase 12 synthesizes → BOTH superseded
        truths.push(GroundTruth {
            memory_id: pos_id.clone(),
            category: Category::Contradictions,
            expected_status: ExpectedStatus::Superseded,
            duplicate_of: None,
            contradicts: Some(neg_id.clone()),
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: neg_id,
            category: Category::Contradictions,
            expected_status: ExpectedStatus::Superseded,
            duplicate_of: None,
            contradicts: Some(pos_id),
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 4 CONTENT pairs — same type (decision), title Jaccard ≥0.5, content Jaccard <0.3
    // Titles use asymmetric lengths so Phase 2 intersection/max stays below 0.65
    // while Phase 9b intersection/union stays above 0.5.
    // Contents are substantially different so Phase 9b content Jaccard stays below 0.3.
    //
    // Title A: 10 core words + anchor = 11 Phase-2-meaningful-words total
    // Title B:  6 core words + anchor =  7 Phase-2-meaningful-words total (all B-core ⊆ A-core)
    //   Shared = 6 core + anchor = 7
    //   Phase 2 title_score = 7/max(11,7) = 7/11 ≈ 0.636  (< 0.65 ✓)
    //   Phase 9b title Jaccard = 7/(11+7-7) = 7/11 ≈ 0.636 (≥ 0.5 ✓)
    //
    // Content A and B use completely disjoint vocabulary (< 1 shared len≥3 word expected).
    //   Phase 2 content_score ≈ 0            (< 0.65 ✓)
    //   Phase 9b content Jaccard ≈ 0         (< 0.3  ✓)
    for pair_idx in 0..4 {
        let token_t = unique(100 + pair_idx); // shared title anchor (64-char hex)
        let a_id = format!("c4-content-{pair_idx}-a");
        let b_id = format!("c4-content-{pair_idx}-b");

        // Title A: 10 core words + anchor
        let title_a = format!(
            "Configure service timeout retry backoff interval policy limits monitoring alerts {token_t}"
        );
        // Title B: 6 core words (all ⊆ A's core words) + anchor
        let title_b = format!("Configure service retry interval limits alerts {token_t}");

        // Content A: specific vocabulary around long cooldown periods.
        // len≥3 words: {set, the, retry, backoff, thirty, seconds, upstream, apis, receive,
        //               mandatory, cooldown, calls, token_a_val} — disjoint from B.
        let token_a_val = unique(200 + pair_idx * 2);
        let content_a = format!(
            "Set the retry backoff to thirty seconds so upstream APIs receive mandatory cooldown between calls {token_a_val}"
        );

        // Content B: specific vocabulary around minimal delay / high throughput.
        // len≥3 words: {use, five, milliseconds, delay, attempts, maximize, throughput,
        //               avoid, queue, saturation, token_b_val} — disjoint from A.
        // "between" is the only potential overlap; it does NOT appear in B.
        let token_b_val = unique(200 + pair_idx * 2 + 1);
        let content_b = format!(
            "Use five milliseconds delay per attempt to maximize throughput and avoid queue saturation {token_b_val}"
        );

        specs.push(MemorySpec {
            id: a_id.clone(),
            memory_type: MemoryType::Decision,
            title: title_a,
            content: content_a,
            confidence: 0.9,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec![
                "category-4-content".into(),
                format!("content-pair-{pair_idx}"),
            ],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        specs.push(MemorySpec {
            id: b_id.clone(),
            memory_type: MemoryType::Decision,
            title: title_b,
            content: content_b,
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec![
                "category-4-content".into(),
                format!("content-pair-{pair_idx}"),
            ],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });

        // Phase 9b detects; Phase 12 does NOT synthesize → both stay ACTIVE with `contradicts` edge
        truths.push(GroundTruth {
            memory_id: a_id.clone(),
            category: Category::Contradictions,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: Some(b_id.clone()),
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: b_id,
            category: Category::Contradictions,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: Some(a_id),
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    (specs, truths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_config_default_values() {
        let cfg = ConsolidationBenchConfig::default();
        assert_eq!(cfg.seed, 42);
        assert_eq!(cfg.output_dir, PathBuf::from("bench_results_consolidation"));
        assert!(cfg.expected_recall_delta.is_none());
    }

    #[test]
    fn test_category_is_hashable() {
        let mut set = HashSet::new();
        set.insert(Category::ExactDuplicates);
        set.insert(Category::SemanticDuplicates);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_expected_status_equality() {
        assert_eq!(ExpectedStatus::Active, ExpectedStatus::Active);
        assert_ne!(ExpectedStatus::Active, ExpectedStatus::Superseded);
    }

    #[test]
    fn test_seeded_dataset_construction() {
        let ds = SeededDataset {
            seed: 42,
            ground_truth: vec![],
            recall_queries: vec![],
            expected_pattern_count: 4,
            expected_protocol_count: 7,
            expected_resolution_count: 4,
        };
        assert_eq!(ds.seed, 42);
        assert_eq!(ds.expected_pattern_count, 4);
    }

    // ── Category 1 tests ─────────────────────────────────────────

    #[test]
    fn test_category_1_produces_12_memories_in_6_pairs() {
        let (specs, truths) = generate_category_1_exact_duplicates(42);
        assert_eq!(specs.len(), 12);
        assert_eq!(truths.len(), 12);

        // Each pair has the SAME title
        for i in 0..6 {
            assert_eq!(specs[i * 2].title, specs[i * 2 + 1].title);
        }

        // Higher-confidence is keeper; lower is victim
        let keepers: Vec<_> = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Active)
            .collect();
        let victims: Vec<_> = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Deleted)
            .collect();
        assert_eq!(keepers.len(), 6);
        assert_eq!(victims.len(), 6);
    }

    #[test]
    fn test_category_1_deterministic() {
        let (specs_a, _) = generate_category_1_exact_duplicates(42);
        let (specs_b, _) = generate_category_1_exact_duplicates(42);
        for (a, b) in specs_a.iter().zip(specs_b.iter()) {
            assert_eq!(a.title, b.title);
            assert_eq!(a.content, b.content);
        }
    }

    // ── Category 2 tests ─────────────────────────────────────────

    #[test]
    fn test_category_2_produces_16_memories() {
        let (specs, truths) = generate_category_2_semantic_duplicates(42);
        assert_eq!(specs.len(), 16);
        assert_eq!(truths.len(), 16);

        let keepers = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Active)
            .count();
        let victims = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Superseded)
            .count();
        assert_eq!(keepers, 8);
        assert_eq!(victims, 8);
    }

    #[test]
    fn test_category_2_pairs_share_anchor_token() {
        let (specs, _) = generate_category_2_semantic_duplicates(42);
        // Each pair's titles should share a 64-char hex anchor
        for pair_idx in 0..8 {
            let a = &specs[pair_idx * 2].title;
            let b = &specs[pair_idx * 2 + 1].title;
            // Extract 64-hex token from a
            let token = a
                .split_whitespace()
                .find(|w| w.len() == 64 && w.chars().all(|c| c.is_ascii_hexdigit()))
                .expect("no 64-hex token in title A");
            assert!(
                b.contains(token),
                "title B doesn't share anchor: a={a}, b={b}"
            );
        }
    }

    // ── Category 3 tests ─────────────────────────────────────────

    #[test]
    fn test_category_3_produces_12_memories_4_merge_2_control() {
        let (specs, truths) = generate_category_3_embedding_duplicates(42);
        assert_eq!(specs.len(), 12);
        let merge_victims = truths
            .iter()
            .filter(|t| {
                t.category == Category::EmbeddingDuplicates
                    && t.expected_status == ExpectedStatus::Superseded
            })
            .count();
        let merge_keepers = truths
            .iter()
            .filter(|t| {
                t.category == Category::EmbeddingDuplicates
                    && t.expected_status == ExpectedStatus::Active
                    && t.duplicate_of.is_some()
            })
            .count();
        let controls = truths
            .iter()
            .filter(|t| {
                t.category == Category::EmbeddingDuplicates
                    && t.expected_status == ExpectedStatus::Active
                    && t.duplicate_of.is_none()
            })
            .count();
        assert_eq!(merge_victims, 4);
        assert_eq!(merge_keepers, 4);
        assert_eq!(controls, 4);
    }

    // ── Category 4 tests ─────────────────────────────────────────

    #[test]
    fn test_category_4_produces_16_memories() {
        let (specs, truths) = generate_category_4_contradictions(42);
        assert_eq!(specs.len(), 16);
        assert_eq!(truths.len(), 16);

        let valence_superseded = truths
            .iter()
            .filter(|t| {
                t.category == Category::Contradictions
                    && t.expected_status == ExpectedStatus::Superseded
            })
            .count();
        let content_active = truths
            .iter()
            .filter(|t| {
                t.category == Category::Contradictions
                    && t.expected_status == ExpectedStatus::Active
            })
            .count();
        assert_eq!(valence_superseded, 8); // all 4 valence pairs superseded
        assert_eq!(content_active, 8); // all 4 content pairs stay active
    }

    #[test]
    fn test_category_4_content_titles_not_exact_duplicates() {
        let (specs, _) = generate_category_4_contradictions(42);
        // Content pairs are specs 8-15 (after 8 valence specs)
        for pair_idx in 0..4 {
            let a = &specs[8 + pair_idx * 2].title;
            let b = &specs[8 + pair_idx * 2 + 1].title;
            assert_ne!(
                a, b,
                "content pair {pair_idx} has identical titles — would be caught by Phase 1"
            );
        }
    }

    #[test]
    fn test_category_4_content_pairs_avoid_phase_2() {
        use std::collections::HashSet;

        // Approximate Phase 2's meaningful_words: len > 1, exclude a small set of common stopwords
        // Phase 2's actual filter is more comprehensive; this test uses a minimal proxy to verify
        // the DESIGN, not exact Phase 2 compliance.
        fn mw(text: &str) -> HashSet<String> {
            let stop: HashSet<&str> = ["the", "to", "a", "an", "is", "are", "so", "that", "and"]
                .iter()
                .copied()
                .collect();
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 1 && !stop.contains(w))
                .map(String::from)
                .collect()
        }

        let (specs, _) = generate_category_4_contradictions(42);
        // Content pairs are specs 8-15 (8 valence first, then 8 content)
        for pair_idx in 0..4 {
            let a = &specs[8 + pair_idx * 2];
            let b = &specs[8 + pair_idx * 2 + 1];
            let title_a_words = mw(&a.title);
            let title_b_words = mw(&b.title);
            let content_a_words = mw(&a.content);
            let content_b_words = mw(&b.content);

            let title_shared = title_a_words.intersection(&title_b_words).count() as f64;
            let title_max = std::cmp::max(title_a_words.len(), title_b_words.len()) as f64;
            let title_score = if title_max == 0.0 {
                0.0
            } else {
                title_shared / title_max
            };

            let content_shared = content_a_words.intersection(&content_b_words).count() as f64;
            let content_max = std::cmp::max(content_a_words.len(), content_b_words.len()) as f64;
            let content_score = if content_max == 0.0 {
                0.0
            } else {
                content_shared / content_max
            };

            let weighted = 0.5 * title_score + 0.5 * content_score;
            let combined = weighted.max(title_score).max(content_score);

            assert!(
                combined < 0.65,
                "content pair {} would be caught by Phase 2 (combined={combined}, title_score={title_score}, content_score={content_score})",
                pair_idx
            );
        }
    }

    #[test]
    fn test_category_4_content_pairs_trigger_phase_9b() {
        use std::collections::HashSet;

        // Phase 9b word_set: len >= 3, no stopword filter
        fn ws(text: &str) -> HashSet<String> {
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() >= 3)
                .map(String::from)
                .collect()
        }

        let (specs, _) = generate_category_4_contradictions(42);
        for pair_idx in 0..4 {
            let a = &specs[8 + pair_idx * 2];
            let b = &specs[8 + pair_idx * 2 + 1];
            let title_a_ws = ws(&a.title);
            let title_b_ws = ws(&b.title);
            let content_a_ws = ws(&a.content);
            let content_b_ws = ws(&b.content);

            let t_shared = title_a_ws.intersection(&title_b_ws).count() as f64;
            let t_union = title_a_ws.union(&title_b_ws).count() as f64;
            let title_jaccard = if t_union == 0.0 {
                0.0
            } else {
                t_shared / t_union
            };

            let c_shared = content_a_ws.intersection(&content_b_ws).count() as f64;
            let c_union = content_a_ws.union(&content_b_ws).count() as f64;
            let content_jaccard = if c_union == 0.0 {
                0.0
            } else {
                c_shared / c_union
            };

            assert!(
                title_jaccard >= 0.5,
                "content pair {} title Jaccard too low for Phase 9b ({title_jaccard})",
                pair_idx
            );
            assert!(
                content_jaccard < 0.3,
                "content pair {} content Jaccard too high for Phase 9b ({content_jaccard})",
                pair_idx
            );
        }
    }
}
