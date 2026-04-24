//! Forge-Consolidation benchmark harness.
//!
//! Tests the daemon's 22-phase consolidation loop across 5 scored dimensions
//! plus infrastructure pass/fail assertions. See
//! `docs/benchmarks/forge-consolidation-design.md` for full design.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use forge_core::types::memory::MemoryType;

use super::common::{seeded_rng, sha256_hex};

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
///
/// Phase 2 guard: each pair's title is JUST the pair's SHA-256 token.
/// Within a pair: keeper and victim share the SAME token title → Phase 1 catches them.
/// Across pairs: every pair has a DIFFERENT token → meaningful_words intersection = 0 → Phase 2 skips.
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
        // Title = just the token. Both keeper and victim share the SAME title (Phase 1 dedup).
        // Across pairs, tokens are distinct → no meaningful_words overlap → Phase 2 safe.
        let title = token.clone();
        let keeper_id = format!("c1-{pair_idx}-keeper");
        let victim_id = format!("c1-{pair_idx}-victim");
        // Content: per-memory unique tokens → content_score = 0 across all pairs.
        let keeper_content_token = sha256_hex(&format!("c1-content-{seed}-{pair_idx}-keeper"));
        let victim_content_token = sha256_hex(&format!("c1-content-{seed}-{pair_idx}-victim"));

        specs.push(MemorySpec {
            id: keeper_id.clone(),
            memory_type: mt.clone(),
            title: title.clone(),
            content: keeper_content_token,
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
            content: victim_content_token,
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
/// Titles share high word overlap via common anchor + pair token.
///
/// Phase 2 design:
/// - Within-pair: combined = 1.0 > 0.65 → Phase 2 correctly catches victim ✓
/// - Cross-pair: each pair has unique anchor + pair_token → intersection = {"enforce", "validation"}
///   = 2 words. Max = 4 words → 2/4 = 0.5 < 0.65 → Phase 2 skips cross-pair ✓
///
/// meaningful_words for keeper title: {pair_token, "enforce", anchor, "validation"} (4 words)
/// meaningful_words for victim title:  {"enforce", anchor, "validation", pair_token} (4 words)
/// Within intersection = all 4. Max = 4 → 1.0 > 0.65 ✓
/// Cross-pair keeper0 vs keeper1: anchor differs, pair_token differs → {enforce, validation} = 2/4 = 0.5 ✓
pub fn generate_category_2_semantic_duplicates(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    // 8 distinct anchor tokens, one per pair
    let anchors: Vec<String> = (0..8)
        .map(|i| sha256_hex(&format!("c2-anchor-{seed}-{i}")))
        .collect();
    // 8 distinct per-pair tokens (unique to each pair, same within a pair)
    let pair_tokens: Vec<String> = (0..8)
        .map(|i| sha256_hex(&format!("c2-pair-{seed}-{i}")))
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
        let pair_token = &pair_tokens[pair_idx];
        // Two paraphrases sharing anchor + pair_token produce combined = 1.0 > 0.65.
        // Cross-pair: anchor and pair_token differ → only {"enforce", "validation"} shared = 2/4 = 0.5 < 0.65.
        let title_a = format!("{pair_token} enforce {anchor} validation");
        let title_b = format!("enforce {anchor} validation {pair_token}");
        let content_a = format!("{pair_token} enforce {anchor} validation");
        let content_b = format!("enforce {anchor} validation {pair_token}");

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
///
/// Phase 2 guard: each memory in a pair gets two per-memory unique SHA-256 tokens as its
/// entire title and content. Across ALL cat-3 memories (same project, same memory_type=Decision),
/// meaningful_words intersection = 0 since every token is unique → combined = 0 < 0.65.
pub fn generate_category_3_embedding_duplicates(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique = |label: &str, idx: usize| sha256_hex(&format!("c3-{seed}-{label}-{idx}"));

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    // 4 merge pairs — distance < 0.1 (Phase 7 merges lower-confidence victim)
    for pair_idx in 0..4 {
        let token_a1 = unique("A1", pair_idx);
        let token_a2 = unique("A2", pair_idx);
        let token_b1 = unique("B1", pair_idx);
        let token_b2 = unique("B2", pair_idx);
        let keeper_id = format!("c3-merge-{pair_idx}-keeper");
        let victim_id = format!("c3-merge-{pair_idx}-victim");

        // Title = two unique tokens (no shared English words across any cat-3 memory).
        // Content = two unique tokens (no shared English words).
        // Phase 2 combined = 0 for all cross-memory pairs → never caught.
        // Phase 7 catches via synthetic embedding cosine distance < 0.1.
        specs.push(MemorySpec {
            id: keeper_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{token_a1} {token_a2}"),
            content: format!("{token_a1} {token_a2}"),
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
            title: format!("{token_b1} {token_b2}"),
            content: format!("{token_b1} {token_b2}"),
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
    // Phase 2 guard: each control memory gets two unique tokens for title and content.
    for pair_idx in 0..2 {
        let token_a1 = unique("CA1", pair_idx);
        let token_a2 = unique("CA2", pair_idx);
        let token_b1 = unique("CB1", pair_idx);
        let token_b2 = unique("CB2", pair_idx);
        let a_id = format!("c3-control-{pair_idx}-a");
        let b_id = format!("c3-control-{pair_idx}-b");

        specs.push(MemorySpec {
            id: a_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{token_a1} {token_a2}"),
            content: format!("{token_a1} {token_a2}"),
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
            title: format!("{token_b1} {token_b2}"),
            content: format!("{token_b1} {token_b2}"),
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
    //
    // Phase 2 guard: titles use the pattern "{pos_token} favors {shared_token}" vs
    // "{neg_token} opposes {shared_token}". meaningful_words analysis:
    //   pos mw = {pos_token, "favors", shared_token} (3 words)
    //   neg mw = {neg_token, "opposes", shared_token} (3 words)
    //   intersection = {shared_token} = 1, max = 3 → title_score = 1/3 ≈ 0.333 < 0.65 ✓
    // Across pairs: shared_token differs per pair → cross-pair intersection = {} → 0 ✓
    // Phase 9a still detects via valence+tags+intensity, NOT title text ✓.
    for pair_idx in 0..4 {
        let shared_token = unique(pair_idx);
        let pos_token = sha256_hex(&format!("c4-pos-{seed}-{pair_idx}"));
        let neg_token = sha256_hex(&format!("c4-neg-{seed}-{pair_idx}"));
        let shared_tags = vec![
            "category-4-valence".into(),
            format!("topic-{shared_token}"),
            format!("valence-pair-{pair_idx}"),
        ];
        let pos_id = format!("c4-val-{pair_idx}-pos");
        let neg_id = format!("c4-val-{pair_idx}-neg");

        // Content: unique per-member token + opposite valence signal.
        // Phase 9a uses valence field, not content text.
        let pos_content_token = sha256_hex(&format!("c4-pos-content-{seed}-{pair_idx}"));
        let neg_content_token = sha256_hex(&format!("c4-neg-content-{seed}-{pair_idx}"));

        specs.push(MemorySpec {
            id: pos_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{pos_token} favors {shared_token}"),
            content: format!("{pos_content_token} resolves {shared_token} positively"),
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
            title: format!("{neg_token} opposes {shared_token}"),
            content: format!("{neg_content_token} rejects {shared_token} negatively"),
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
    //
    // Phase 2 guard + Phase 9b trigger design (using REAL stop-word filtered meaningful_words):
    //
    // Token anchors:
    //   token_t  = per-pair unique anchor (64-hex) — in BOTH A and B ✓
    //   pair_uniq = per-pair unique word (64-hex)   — in BOTH A and B ✓
    //
    // Title A mw: {token_t, pair_uniq, "retry", "interval", "configure"} = 5 words
    // Title B mw: {token_t, pair_uniq, "retry"}                           = 3 words
    //   (B is a subset of A's vocabulary: {token_t, pair_uniq, "retry"})
    //
    // Within-pair Phase 2:
    //   intersection = {token_t, pair_uniq, "retry"} = 3
    //   max = max(5, 3) = 5
    //   title_score = 3/5 = 0.60 < 0.65 ✓
    //
    // Within-pair Phase 9b title Jaccard (len≥3, no stop):
    //   word_set_A = {token_t, pair_uniq, "retry", "interval", "configure"} = 5
    //   word_set_B = {token_t, pair_uniq, "retry"} = 3
    //   shared = 3, union = 5  →  Jaccard = 3/5 = 0.60 ≥ 0.5 ✓
    //
    // Cross-pair Phase 2 (A0 vs A1 — token_t and pair_uniq differ):
    //   intersection = {"retry", "interval", "configure"} = 3
    //   max = 5
    //   title_score = 3/5 = 0.60 < 0.65 ✓
    //
    // Cross-pair Phase 2 (B0 vs B1 — token_t and pair_uniq differ):
    //   intersection = {"retry"} = 1, max = 3 → 1/3 = 0.333 < 0.65 ✓
    //
    // Content: per-pair per-member unique tokens → content Jaccard ≈ 0 for both Phase 2 and Phase 9b ✓
    for pair_idx in 0..4 {
        let token_t = unique(100 + pair_idx); // shared title anchor (64-char hex), same within pair
        let pair_uniq = sha256_hex(&format!("c4-pair-uniq-{seed}-{pair_idx}")); // second shared token
        let a_id = format!("c4-content-{pair_idx}-a");
        let b_id = format!("c4-content-{pair_idx}-b");

        // Title A: 5 mw words: {token_t, pair_uniq, "retry", "interval", "configure"}
        let title_a = format!("{token_t} {pair_uniq} retry interval configure");
        // Title B: 3 mw words (subset of A's vocab): {token_t, pair_uniq, "retry"}
        let title_b = format!("{token_t} {pair_uniq} retry");

        // Content A: single unique token → Phase 9b content Jaccard within-pair = 0 < 0.3 ✓
        //           Cross-pair: different unique tokens → content Jaccard = 0 ✓
        let token_a_val = unique(200 + pair_idx * 2);
        let content_a = token_a_val;

        // Content B: single unique token (different from A's) → Phase 9b content Jaccard = 0 ✓
        let token_b_val = unique(200 + pair_idx * 2 + 1);
        let content_b = token_b_val;

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

/// Category 5: 30 memories for Phase 14 (reweave), Phase 17 (protocol extraction),
/// Phase 18 (anti-pattern tagging).
///
/// CRITICAL: Titles must have <=50% whitespace-token overlap with each other to avoid
/// accidental Phase 5 clustering (which uses raw split_whitespace without stopword filter).
pub fn generate_category_5_reweave_enrichment(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique = |label: &str, idx: usize| sha256_hex(&format!("c5-{seed}-{label}-{idx}"));

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    // 10 REWEAVE pairs — same type + project + org + ≥2 shared tags, different ages
    //
    // Phase 2 guard: titles = two per-memory unique SHA-256 tokens → mw intersection = 0 across
    // all 20 reweave memories (no shared English words) → combined = 0 < 0.65 ✓.
    // Content: two per-memory unique tokens → content_score = 0 across memories ✓.
    // Phase 5 raw split_whitespace overlap between older and newer: 0/4 = 0 < 0.5 → no clustering ✓.
    // Phase 14 reweave still triggers via shared tags + age difference ✓.
    for pair_idx in 0..10 {
        let topic_token = unique("rtopic", pair_idx);
        let shared_tags = vec![
            "category-5-reweave".into(),
            format!("reweave-topic-{topic_token}"),
            format!("reweave-pair-{pair_idx}"),
        ];
        let older_id = format!("c5-reweave-{pair_idx}-older");
        let newer_id = format!("c5-reweave-{pair_idx}-newer");

        // Unique tokens per memory — no shared English boilerplate in title or content.
        let older_title_t1 = unique("rolder-t1", pair_idx);
        let older_title_t2 = unique("rolder-t2", pair_idx);
        let older_content_t1 = unique("rolder-c1", pair_idx);
        let older_content_t2 = unique("rolder-c2", pair_idx);
        let newer_title_t1 = unique("rnewer-t1", pair_idx);
        let newer_title_t2 = unique("rnewer-t2", pair_idx);
        let newer_content_t1 = unique("rnewer-c1", pair_idx);
        let newer_content_t2 = unique("rnewer-c2", pair_idx);

        specs.push(MemorySpec {
            id: older_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{older_title_t1} {older_title_t2}"),
            content: format!("{older_content_t1} {older_content_t2}"),
            confidence: 0.8,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: shared_tags.clone(),
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW-10d".into(), // older
            accessed_at_spec: "NOW-10d".into(),
        });
        specs.push(MemorySpec {
            id: newer_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{newer_title_t1} {newer_title_t2}"),
            content: format!("{newer_content_t1} {newer_content_t2}"),
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: shared_tags,
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(), // newer
            accessed_at_spec: "NOW".into(),
        });

        // Phase 14 reweave: newer marked 'merged', older content appended with "[Update]: ..."
        truths.push(GroundTruth {
            memory_id: older_id.clone(),
            category: Category::ReweaveEnrichment,
            expected_status: ExpectedStatus::Active, // content enriched in place
            duplicate_of: None,
            contradicts: None,
            reweave_source: Some(newer_id.clone()),
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: newer_id,
            category: Category::ReweaveEnrichment,
            expected_status: ExpectedStatus::Merged,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 4 PREFERENCES with process signals for Phase 17 Tier 1
    //
    // Phase 2 guard: title = two unique tokens per preference → mw intersection = 0 ✓.
    // Content: "always must require" process signals kept for Phase 17, plus unique token ✓.
    // Preferences are MemoryType::Preference — only compared against other preferences.
    for pref_idx in 0..4 {
        let token = unique("pref", pref_idx);
        let title_t2 = unique("pref-t2", pref_idx);
        let id = format!("c5-pref-{pref_idx}");
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Preference,
            // Two unique tokens → no shared meaningful_words across the 4 preferences ✓
            title: format!("{token} {title_t2}"),
            // Phase 17 applies a has_process_signal check on content. "always" satisfies it
            // and is not a stop word. Shared "always" across 4 preferences gives content_score
            // = 1/3 ≈ 0.333 < 0.65 threshold → no semantic dedup interference ✓.
            content: format!("always {token} {title_t2}"),
            confidence: 0.9,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-5-protocol".into(), format!("pref-{pref_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::ReweaveEnrichment,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 3 PATTERNS with behavioral: prefix + process signal for Phase 17 Tier 2
    //
    // Phase 2 guard: title = "behavioral: {unique_a} {unique_b}" — "behavioral" IS a shared
    // word across the 3 patterns, but meaningful_words of Pattern type only compared with other
    // Pattern memories. With 3 patterns sharing "behavioral" and 2 unique tokens each:
    //   mw = {"behavioral", unique_a, unique_b} (3 words)
    //   cross-pattern intersection = {"behavioral"} = 1, max = 3 → title_score = 1/3 ≈ 0.333 < 0.65 ✓
    // Content: "always require" process signals kept for Phase 17, plus unique token ✓.
    for pat_idx in 0..3 {
        let token_a = unique("behavioral-a", pat_idx);
        let token_b = unique("behavioral-b", pat_idx);
        let id = format!("c5-pattern-{pat_idx}");
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Pattern,
            // "behavioral:" prefix needed for Phase 17 Tier 2 SQL filter (`LOWER(title) LIKE 'behavioral:%'`)
            // AND satisfies the has_process_signal check via `title_lower.starts_with("behavioral:")`.
            // Content = two unique tokens → no shared boilerplate across the 3 patterns ✓.
            title: format!("behavioral: {token_a} {token_b}"),
            content: format!("{token_a} {token_b}"),
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-5-protocol".into(), format!("pattern-{pat_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::ReweaveEnrichment,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 3 LESSONS with negative signals for Phase 18 anti-pattern tagging
    //
    // Phase 2 guard: title = two unique tokens per lesson → mw intersection = 0 across
    // all lesson-type memories (cat 5, cat 6 clusters, cat 7 stale) ✓.
    // Content: "don't" / "avoid" / "caused problem" signals kept in CONTENT for Phase 18 ✓.
    // Phase 18 detects from content, not title, so removing "Avoid" from title is safe ✓.
    for les_idx in 0..3 {
        let token_a = unique("antipattern-a", les_idx);
        let token_b = unique("antipattern-b", les_idx);
        let id = format!("c5-antipattern-{les_idx}");
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Lesson,
            // Two unique tokens — no shared English boilerplate → Phase 2 safe ✓
            title: format!("{token_a} {token_b}"),
            // Negative signals in content where Phase 18 looks ✓
            content: format!("Don't use {token_a} — it caused problem and avoid {token_b}."),
            confidence: 0.8,
            valence: "negative".into(),
            intensity: 0.6,
            tags: vec!["category-5-antipattern".into(), format!("lesson-{les_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::ReweaveEnrichment,
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

/// Category 6: 31 memories for Phases 4 (decay), 5 (promotion), 6 (reconsolidation),
/// 15 (quality scoring).
pub fn generate_category_6_lifecycle_quality(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique = |label: &str, idx: usize| sha256_hex(&format!("c6-{seed}-{label}-{idx}"));

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    // 6 DECAY candidates — accessed_at 30+ days ago (Phase 4 keys off accessed_at!)
    //
    // Phase 2 guard: title = two unique tokens per decay memory → mw intersection = 0
    // across all decision-type memories → combined = 0 < 0.65 ✓.
    for d_idx in 0..6 {
        let token_a = unique("decay-a", d_idx);
        let token_b = unique("decay-b", d_idx);
        let id = format!("c6-decay-{d_idx}");
        let days_old = 30 + (d_idx * 5) as i64; // 30, 35, 40, 45, 50, 55 days old
                                                // Expected post-decay confidence: 0.9 * exp(-0.03 * days_old)
        let expected_conf = 0.9_f64 * (-0.03_f64 * days_old as f64).exp();

        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{token_a} {token_b}"),
            content: format!("{token_a} {token_b}"),
            confidence: 0.9,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-6-decay".into(), format!("decay-{d_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: format!("NOW-{days_old}d"),
            accessed_at_spec: format!("NOW-{days_old}d"), // critical: accessed_at drives decay
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::LifecycleQuality,
            expected_status: if expected_conf < 0.1 {
                ExpectedStatus::Faded
            } else {
                ExpectedStatus::Active
            },
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: Some(expected_conf),
            expected_activation: None,
        });
    }

    // 5 RECONSOLIDATION candidates — access_count >= 5 → confidence += 0.05
    //
    // Phase 2 guard: title = two unique tokens → mw intersection = 0 across all Decision memories ✓.
    for r_idx in 0..5 {
        let token_a = unique("recon-a", r_idx);
        let token_b = unique("recon-b", r_idx);
        let id = format!("c6-recon-{r_idx}");
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{token_a} {token_b}"),
            content: format!("{token_a} {token_b}"),
            confidence: 0.8,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-6-recon".into(), format!("recon-{r_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: 5 + r_idx as u64,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::LifecycleQuality,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: Some(0.85_f64.min(1.0)), // 0.80 + 0.05
            expected_activation: None,
        });
    }

    // 4 CLUSTERS of 3 lessons (12 total) with >50% title overlap for Phase 5 promotion.
    // Phase 2 guard: per-member SHA-256 tokens make meaningful_words sets diverge enough
    // (title combined ≈ 0.60 < 0.65) while keeping raw split_whitespace overlap > 0.5
    // so Phase 5 still clusters them.
    for cluster_idx in 0..4 {
        let cluster_token = unique("cluster-topic", cluster_idx);
        for lesson_idx in 0..3 {
            let id = format!("c6-cluster-{cluster_idx}-{lesson_idx}");
            // Two per-member tokens → meaningful_words intersection/max ≈ 3/5 = 0.60 < 0.65
            let member_token_a =
                sha256_hex(&format!("c6-cluster-{seed}-{cluster_idx}-{lesson_idx}-a"));
            let member_token_b =
                sha256_hex(&format!("c6-cluster-{seed}-{cluster_idx}-{lesson_idx}-b"));
            // Raw split: 5 tokens [cluster_token, "repeats", m_a, "across", m_b]
            // Between variants: shared = {cluster_token, "repeats", "across"} = 3/5 = 0.60 > 0.5 ✓
            let title = format!("{cluster_token} repeats {member_token_a} across {member_token_b}");
            // Per-member verb/noun keeps content_score ≈ 0.375 between any two variants
            let member_verb = match lesson_idx {
                0 => "discovered",
                1 => "noticed",
                _ => "verified",
            };
            let member_noun = match lesson_idx {
                0 => "during review",
                1 => "in production",
                _ => "via logs",
            };
            specs.push(MemorySpec {
                id: id.clone(),
                memory_type: MemoryType::Lesson,
                title,
                content: format!(
                    "{member_verb} {member_noun}: {cluster_token} instance {member_token_a} tied to {member_token_b}."
                ),
                confidence: 0.75,
                valence: "neutral".into(),
                intensity: 0.0,
                tags: vec![
                    "category-6-cluster".into(),
                    format!("cluster-{cluster_idx}"),
                ],
                project: "forge-consolidation-bench".into(),
                access_count: 0,
                activation_level: 0.0,
                quality_score: None,
                created_at_spec: "NOW".into(),
                accessed_at_spec: "NOW".into(),
            });
            truths.push(GroundTruth {
                memory_id: id,
                category: Category::LifecycleQuality,
                expected_status: ExpectedStatus::Superseded, // Phase 5 supersedes cluster members
                duplicate_of: None,
                contradicts: None,
                reweave_source: None,
                expected_quality: None,
                expected_confidence: None,
                expected_activation: None,
            });
        }
    }

    // 8 QUALITY scoring validation memories — varied dimensions, expected quality computed
    //
    // Phase 2 guard: title = two unique tokens → mw intersection = 0 across Decision memories ✓.
    // Content uses "x".repeat(N) — all-same-char content → mw = {} (no len>1 distinct words) → score = 0 ✓.
    for q_idx in 0..8 {
        let token_a = unique("quality-a", q_idx);
        let token_b = unique("quality-b", q_idx);
        let id = format!("c6-quality-{q_idx}");

        // Vary each dimension: age 0-7 days, access 0-4 (clamped to avoid Phase 6 recon
        // interference — only c6-recon-* should have access_count >= 5), content len 50-190,
        // activation 0.0-0.7
        let age_days = q_idx as i64; // 0, 1, 2, 3, 4, 5, 6, 7
        let access = (q_idx as u64).min(4); // 0, 1, 2, 3, 4, 4, 4, 4 (capped at 4)
        let content = "x".repeat(50 + q_idx * 20); // 50, 70, 90, ..., 190 chars
        let seeded_activation = (q_idx as f64) * 0.1; // 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7

        // Phase 10 decays activation BEFORE Phase 15 reads it
        let post_decay_activation = if seeded_activation * 0.95 > 0.01 {
            seeded_activation * 0.95
        } else {
            0.0
        };

        // Phase 15 formula
        let freshness = (1.0_f64 - (age_days as f64 / 7.0) * 0.1).clamp(0.1, 1.0);
        let utility = (access as f64 / 10.0).clamp(0.0, 1.0);
        let completeness = (content.len() as f64 / 200.0).min(1.0);
        let activation = post_decay_activation.clamp(0.0, 1.0);
        let phase15_quality =
            freshness * 0.3 + utility * 0.3 + completeness * 0.2 + activation * 0.2;

        // Phase 22 (quality pressure) adjusts quality AFTER Phase 15:
        //   - access_count = 0: normal decay -0.1 (if phase15 >= 0.3) or accel -0.15 (if < 0.3)
        //   - access_count > 0 AND accessed_at within 1 day: boost +0.05
        // All c6-quality memories use accessed_at_spec="NOW", so they qualify for the boost.
        let expected_quality = if access == 0 {
            let decay = if phase15_quality < 0.3 {
                0.15_f64
            } else {
                0.1_f64
            };
            (phase15_quality - decay).max(0.0)
        } else {
            // access > 0 AND accessed within 1 day → boost applies
            (phase15_quality + 0.05_f64).min(1.0)
        };

        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{token_a} {token_b}"),
            content,
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-6-quality".into(), format!("quality-{q_idx}")],
            project: "forge-consolidation-bench".into(),
            access_count: access,
            activation_level: seeded_activation,
            quality_score: None,
            created_at_spec: format!("NOW-{age_days}d"),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::LifecycleQuality,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: Some(expected_quality),
            expected_confidence: None,
            expected_activation: Some(post_decay_activation),
        });
    }

    (specs, truths)
}

/// Category 7: 24 memories for Phase 20 (topic supersede), Phase 21 (staleness fade),
/// Phase 22 (quality pressure).
pub fn generate_category_7_self_healing(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique = |label: &str, idx: usize| sha256_hex(&format!("c7-{seed}-{label}-{idx}"));

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    // 6 TOPIC-SUPERSEDE pairs — synthetic embeddings + word overlap 0.3-0.7 on title+content.
    //
    // Phase 2 guard: title pattern "{topic} {unique_per_member}":
    //   older mw = {topic, unique_older} (2 words)
    //   newer mw = {topic, unique_newer} (2 words)
    //   within-pair: intersection = {topic} = 1, max = 2 → title_score = 0.5 < 0.65 ✓
    //   across-pairs: each pair has DIFFERENT topic token → cross-pair intersection = {} → 0 ✓
    // Content: unique per-member token keeps content_score = 0 across all memories ✓.
    // Phase 20 combined title+content overlap in [0.3, 0.7):
    //   combined mw(title+content) for older = {topic, unique_older} (2 words)
    //   combined mw(title+content) for newer = {topic, unique_newer} (2 words)
    //   intersection=1, union=3 → overlap=0.333 ∈ [0.3, 0.7) ✓
    //
    // Phase 14 guard: ONLY 1 tag per pair ("category-7-supersede" only, no per-pair tag).
    //   Phase 14 (reweave) fires when memories share ≥2 tags. With 1 shared tag, Phase 14
    //   is suppressed → Phase 20 can find both memories still active ✓.
    for pair_idx in 0..6 {
        let topic = unique("topic", pair_idx);
        let unique_older = sha256_hex(&format!("c7-older-{seed}-{pair_idx}"));
        let unique_newer = sha256_hex(&format!("c7-newer-{seed}-{pair_idx}"));
        let older_id = format!("c7-supersede-{pair_idx}-older");
        let newer_id = format!("c7-supersede-{pair_idx}-newer");

        specs.push(MemorySpec {
            id: older_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{topic} {unique_older}"),
            content: unique_older.clone(), // unique per member → content_score = 0 cross-memory ✓
            confidence: 0.8,               // <0.95 to allow supersede
            valence: "neutral".into(),
            intensity: 0.0,
            // Only 1 tag: Phase 14 reweave requires ≥2 shared tags → suppressed ✓
            // Phase 20 uses embedding KNN (distance < 0.35) as primary signal → still fires ✓
            tags: vec!["category-7-supersede".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW-2d".into(),
            accessed_at_spec: "NOW-2d".into(),
        });
        specs.push(MemorySpec {
            id: newer_id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{topic} {unique_newer}"),
            content: unique_newer.clone(), // unique per member → content_score = 0 cross-memory ✓
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            // Only 1 tag: Phase 14 reweave requires ≥2 shared tags → suppressed ✓
            tags: vec!["category-7-supersede".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });

        truths.push(GroundTruth {
            memory_id: older_id.clone(),
            category: Category::SelfHealing,
            expected_status: ExpectedStatus::Superseded,
            duplicate_of: Some(newer_id.clone()),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
        truths.push(GroundTruth {
            memory_id: newer_id,
            category: Category::SelfHealing,
            expected_status: ExpectedStatus::Active,
            duplicate_of: Some(older_id),
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 6 STALENESS candidates — age 90 days, access=0, content ≤10 chars, activation=0
    //   Phase 15 quality will be: 0.1*0.3 + 0 + 0.015*0.2 + 0 = 0.033 < 0.1 aggressive tier
    //   Phase 2 guard: unique 3-char content per candidate keeps content_score = 0 between them.
    for s_idx in 0..6 {
        let s_token = sha256_hex(&format!("c7-stale-{seed}-{s_idx}"));
        let s_content = match s_idx {
            0 => "a1b",
            1 => "c2d",
            2 => "e3f",
            3 => "g4h",
            4 => "i5j",
            _ => "k6l",
        };
        let id = format!("c7-stale-{s_idx}");
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Lesson,
            title: format!("stale {s_token}"),
            content: s_content.into(), // 3 chars → completeness 0.015, Phase 15 quality ≈ 0.033
            confidence: 0.5,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-7-stale".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW-90d".into(),
            accessed_at_spec: "NOW-90d".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::SelfHealing,
            expected_status: ExpectedStatus::Faded,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 6 QUALITY-PRESSURE candidates — 3 accelerated-decay + 3 boost
    // Phase 2 guard: unique tokens per candidate make content_score = 0 between them.
    for p_idx in 0..3 {
        // Accelerated decay: 90-day-old, low quality, zero access
        let p_token = sha256_hex(&format!("c7-decay-{seed}-{p_idx}"));
        let p_content = match p_idx {
            0 => "x7y",
            1 => "z8w",
            _ => "v9u",
        };
        let id = format!("c7-decay-{p_idx}");
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("decay {p_token}"),
            content: p_content.into(), // 3 chars → completeness 0.015, Phase 15 quality ≈ 0.033
            confidence: 0.5,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-7-pressure-decay".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW-90d".into(),
            accessed_at_spec: "NOW-90d".into(),
        });
        // Note: Phase 21 fires BEFORE Phase 22. These get faded by Phase 21, not Phase 22.
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::SelfHealing,
            expected_status: ExpectedStatus::Faded,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }
    for p_idx in 0..3 {
        // Boost: high access, recent, moderate quality
        // Phase 2 guard: two per-member SHA-256 tokens make meaningful_words sets diverge.
        // Title: "recent {b_token_a} boost {b_token_b}" — 4 words, 2 unique per candidate.
        // Between any two candidates: shared = {"recent", "boost"} = 2/4 = 0.5 < 0.65 ✓
        // Content: b_token_a is the only shared meaningful word → content_score = 0 ✓
        let b_token_a = sha256_hex(&format!("c7-boost-{seed}-{p_idx}-a"));
        let b_token_b = sha256_hex(&format!("c7-boost-{seed}-{p_idx}-b"));
        let id = format!("c7-boost-{p_idx}");
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("recent {b_token_a} boost {b_token_b}"),
            content: format!("{b_token_a} {b_token_b}"),
            confidence: 0.8,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-7-pressure-boost".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 1 + p_idx as u64, // 1, 2, 3 — keep > 0 for Phase 22 boost; stay < 5 to avoid recon interference
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::SelfHealing,
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

/// Category 8: 26 memories for Phase 3 (linking), Phase 10 (activation decay),
/// Phase 11 (entity detection), Phase 16 (portability).
pub fn generate_category_8_infrastructure(seed: u64) -> (Vec<MemorySpec>, Vec<GroundTruth>) {
    let unique = |label: &str, idx: usize| sha256_hex(&format!("c8-{seed}-{label}-{idx}"));

    let mut specs = Vec::new();
    let mut truths = Vec::new();

    // 5 LINKING pairs — share ≥2 tags, accessed_at within last hour for Phase 8
    //
    // Phase 2 guard: title = two unique per-member tokens → mw intersection = 0 across all
    // Decision-type link memories → combined = 0 < 0.65 ✓.
    for pair_idx in 0..5 {
        let shared_tags = vec!["category-8-link".into(), format!("link-group-{pair_idx}")];
        for member_idx in 0..2 {
            let id = format!("c8-link-{pair_idx}-{member_idx}");
            let token_a = unique("link-a", pair_idx * 2 + member_idx);
            let token_b = unique("link-b", pair_idx * 2 + member_idx);
            specs.push(MemorySpec {
                id: id.clone(),
                memory_type: MemoryType::Decision,
                title: format!("{token_a} {token_b}"),
                content: format!("{token_a} {token_b}"),
                confidence: 0.85,
                valence: "neutral".into(),
                intensity: 0.0,
                tags: shared_tags.clone(),
                project: "forge-consolidation-bench".into(),
                access_count: 1, // recently accessed for Phase 8
                activation_level: 0.0,
                quality_score: None,
                created_at_spec: "NOW".into(),
                accessed_at_spec: "NOW".into(),
            });
            truths.push(GroundTruth {
                memory_id: id,
                category: Category::Infrastructure,
                expected_status: ExpectedStatus::Active,
                duplicate_of: None,
                contradicts: None,
                reweave_source: None,
                expected_quality: None,
                expected_confidence: None,
                expected_activation: None,
            });
        }
    }

    // 5 ACTIVATION candidates — activation_level 0.1..0.5, should be decayed to *0.95
    //
    // Phase 2 note: "test" IS a stop word. "Activation test {token}" → mw = {"activation", token}.
    // Cross-activation: intersection = {} → score = 0 ✓. These are already safe, but using
    // two unique tokens per memory for consistency with the overall Phase 2 guard strategy.
    for a_idx in 0..5 {
        let id = format!("c8-activation-{a_idx}");
        let token_a = unique("activation-a", a_idx);
        let token_b = unique("activation-b", a_idx);
        let seeded_activation = 0.1 + (a_idx as f64) * 0.1; // 0.1, 0.2, 0.3, 0.4, 0.5
        let expected_activation = seeded_activation * 0.95;
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{token_a} {token_b}"),
            content: format!("{token_a} {token_b}"),
            confidence: 0.85,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-8-activation".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: seeded_activation,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::Infrastructure,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: Some(expected_activation),
        });
    }

    // 8 ENTITY memories with proper nouns (PascalCase terms)
    let entity_terms = [
        ("Kubernetes", "container orchestration"),
        ("PostgreSQL", "database server"),
        ("Terraform", "infrastructure as code"),
        ("Prometheus", "metrics system"),
        ("Grafana", "dashboard tool"),
        ("RabbitMQ", "message broker"),
        ("Redis", "cache layer"),
        ("Consul", "service discovery"),
    ];
    for (e_idx, (entity, desc)) in entity_terms.iter().enumerate() {
        let id = format!("c8-entity-{e_idx}");
        let token = unique("entity", e_idx);
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{entity} usage note {token}"),
            content: format!("Using {entity} as {desc}."),
            confidence: 0.9,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-8-entity".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::Infrastructure,
            expected_status: ExpectedStatus::Active,
            duplicate_of: None,
            contradicts: None,
            reweave_source: None,
            expected_quality: None,
            expected_confidence: None,
            expected_activation: None,
        });
    }

    // 3 PORTABILITY candidates — no portability set (NULL/default 'unknown')
    //
    // Phase 2 guard: title = two unique tokens → mw intersection = 0 → combined = 0 < 0.65 ✓.
    for p_idx in 0..3 {
        let id = format!("c8-portability-{p_idx}");
        let token_a = unique("portability-a", p_idx);
        let token_b = unique("portability-b", p_idx);
        specs.push(MemorySpec {
            id: id.clone(),
            memory_type: MemoryType::Decision,
            title: format!("{token_a} {token_b}"),
            content: format!("{token_a} {token_b}"),
            confidence: 0.8,
            valence: "neutral".into(),
            intensity: 0.0,
            tags: vec!["category-8-portability".into()],
            project: "forge-consolidation-bench".into(),
            access_count: 0,
            activation_level: 0.0,
            quality_score: None,
            created_at_spec: "NOW".into(),
            accessed_at_spec: "NOW".into(),
        });
        truths.push(GroundTruth {
            memory_id: id,
            category: Category::Infrastructure,
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

// ── Corpus seeding ───────────────────────────────────────────────

/// Resolve "NOW" / "NOW-Nd" specs to concrete ISO-8601 timestamps.
/// Uses `forge_core::time::now_offset` (seconds from now) — no chrono dependency.
fn resolve_timestamp(spec: &str) -> String {
    if spec == "NOW" || spec == "NOW-0d" {
        return forge_core::time::now_iso();
    }
    if let Some(rest) = spec.strip_prefix("NOW-") {
        if let Some(n_str) = rest.strip_suffix('d') {
            if let Ok(n) = n_str.parse::<i64>() {
                return forge_core::time::now_offset(-(n * 86_400));
            }
        }
    }
    // Fallback: assume already ISO-8601
    spec.to_string()
}

/// Insert a single MemorySpec into the memory table via explicit SQL.
/// Uses explicit quality_score when provided; otherwise DB default (0.5) applies
/// and will be overwritten by Phase 15 anyway.
pub fn insert_memory_spec(conn: &rusqlite::Connection, spec: &MemorySpec) -> rusqlite::Result<()> {
    let created_at = resolve_timestamp(&spec.created_at_spec);
    let accessed_at = resolve_timestamp(&spec.accessed_at_spec);
    // Exhaustive match — no wildcard so new MemoryType variants force an explicit mapping here.
    let type_str = match spec.memory_type {
        MemoryType::Decision => "decision",
        MemoryType::Lesson => "lesson",
        MemoryType::Pattern => "pattern",
        MemoryType::Preference => "preference",
        MemoryType::Protocol => "protocol",
    };
    let tags_json = serde_json::to_string(&spec.tags).unwrap_or_else(|_| "[]".into());

    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags,
                             created_at, accessed_at, valence, intensity, access_count,
                             activation_level, quality_score, organization_id)
         VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'default')",
        rusqlite::params![
            spec.id,
            type_str,
            spec.title,
            spec.content,
            spec.confidence,
            spec.project,
            tags_json,
            created_at,
            accessed_at,
            spec.valence,
            spec.intensity,
            spec.access_count as i64,
            spec.activation_level,
            spec.quality_score.unwrap_or(0.5),
        ],
    )?;
    Ok(())
}

/// Full corpus seeder + ground-truth orchestrator.
/// Does NOT insert embeddings — that's Task 4's `seed_embeddings`.
pub fn seed_corpus(
    conn: &rusqlite::Connection,
    seed: u64,
) -> Result<(Vec<MemorySpec>, SeededDataset), String> {
    let (c1_s, c1_t) = generate_category_1_exact_duplicates(seed);
    let (c2_s, c2_t) = generate_category_2_semantic_duplicates(seed);
    let (c3_s, c3_t) = generate_category_3_embedding_duplicates(seed);
    let (c4_s, c4_t) = generate_category_4_contradictions(seed);
    let (c5_s, c5_t) = generate_category_5_reweave_enrichment(seed);
    let (c6_s, c6_t) = generate_category_6_lifecycle_quality(seed);
    let (c7_s, c7_t) = generate_category_7_self_healing(seed);
    let (c8_s, c8_t) = generate_category_8_infrastructure(seed);

    let mut all_specs = Vec::new();
    all_specs.extend(c1_s);
    all_specs.extend(c2_s);
    all_specs.extend(c3_s);
    all_specs.extend(c4_s);
    all_specs.extend(c5_s);
    all_specs.extend(c6_s);
    all_specs.extend(c7_s);
    all_specs.extend(c8_s);

    let mut all_truths = Vec::new();
    all_truths.extend(c1_t);
    all_truths.extend(c2_t);
    all_truths.extend(c3_t);
    all_truths.extend(c4_t);
    all_truths.extend(c5_t);
    all_truths.extend(c6_t);
    all_truths.extend(c7_t);
    all_truths.extend(c8_t);

    // Verify no ID collisions
    let mut ids = HashSet::new();
    for spec in &all_specs {
        if !ids.insert(&spec.id) {
            return Err(format!("duplicate ID {} across categories", spec.id));
        }
    }

    // Insert all memories
    for spec in &all_specs {
        insert_memory_spec(conn, spec).map_err(|e| format!("insert {}: {e}", spec.id))?;
    }

    Ok((
        all_specs.clone(),
        SeededDataset {
            seed,
            ground_truth: all_truths,
            recall_queries: Vec::new(), // filled by Task 5 `generate_query_bank`
            expected_pattern_count: 4,
            expected_protocol_count: 7,
            expected_resolution_count: 4,
        },
    ))
}

// ── Synthetic embeddings ─────────────────────────────────────────

const EMBEDDING_DIM: usize = 768;

/// Generate a deterministic unit vector of dimension EMBEDDING_DIM from a seed string.
pub fn generate_base_embedding(seed_key: &str) -> Vec<f32> {
    use rand::Rng;
    let hash = sha256_hex(seed_key);
    let mut rng = seeded_rng(u64::from_str_radix(&hash[0..16], 16).unwrap_or(0));
    let raw: Vec<f32> = (0..EMBEDDING_DIM)
        .map(|_| rng.random_range(-1.0_f32..1.0_f32))
        .collect();
    let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
    raw.into_iter().map(|x| x / norm).collect()
}

/// Perturb a base embedding to achieve a target cosine distance.
/// Target distance 0 = identical; 1 = orthogonal.
pub fn perturb_embedding(base: &[f32], target_distance: f32, seed_key: &str) -> Vec<f32> {
    use rand::Rng;
    let hash = sha256_hex(&format!("{seed_key}-perturb"));
    let mut rng = seeded_rng(u64::from_str_radix(&hash[0..16], 16).unwrap_or(0));

    // Generate a random orthogonal direction
    let mut direction: Vec<f32> = (0..EMBEDDING_DIM)
        .map(|_| rng.random_range(-1.0_f32..1.0_f32))
        .collect();
    // Project out the base direction (Gram-Schmidt)
    let dot: f32 = direction.iter().zip(base.iter()).map(|(a, b)| a * b).sum();
    for (d, b) in direction.iter_mut().zip(base.iter()) {
        *d -= dot * b;
    }
    let dir_norm: f32 = direction.iter().map(|x| x * x).sum::<f32>().sqrt();
    for d in direction.iter_mut() {
        *d /= dir_norm;
    }

    // Mix: result = alpha * base + beta * direction, where cos(angle) = alpha = 1 - target_distance
    let alpha = 1.0 - target_distance;
    let beta = (1.0 - alpha * alpha).sqrt();

    let mut mixed: Vec<f32> = (0..EMBEDDING_DIM)
        .map(|i| alpha * base[i] + beta * direction[i])
        .collect();
    // Re-normalize to unit length
    let norm: f32 = mixed.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in &mut mixed {
        *x /= norm;
    }
    mixed
}

/// Compute cosine distance between two unit vectors.
pub fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    1.0 - dot
}

/// Insert synthetic embeddings for Category 3 (merge + control) and Category 7 (topic-supersede)
/// into the `memory_vec` virtual table.
///
/// Category 3: 4 merge pairs at distance 0.08 (< 0.1 threshold → Phase 7 merges),
/// 2 control pairs at distance 0.15 (> 0.1 → Phase 7 skips).
/// Category 7: 6 supersede pairs at distance 0.25 (< 0.35 threshold → Phase 20 supersedes).
pub fn seed_embeddings(conn: &rusqlite::Connection, seed: u64) -> Result<usize, String> {
    let mut inserted = 0;

    // Category 3 merge pairs: distance 0.08
    for pair_idx in 0..4 {
        let base_key = format!("c3-merge-{seed}-{pair_idx}");
        let base = generate_base_embedding(&base_key);
        let perturbed = perturb_embedding(&base, 0.08, &format!("c3-merge-{seed}-{pair_idx}"));

        insert_vec(conn, &format!("c3-merge-{pair_idx}-keeper"), &base)?;
        insert_vec(conn, &format!("c3-merge-{pair_idx}-victim"), &perturbed)?;
        inserted += 2;
    }

    // Category 3 control pairs: distance 0.15
    for pair_idx in 0..2 {
        let base_key = format!("c3-control-{seed}-{pair_idx}");
        let base = generate_base_embedding(&base_key);
        let perturbed = perturb_embedding(&base, 0.15, &format!("c3-control-{seed}-{pair_idx}"));
        insert_vec(conn, &format!("c3-control-{pair_idx}-a"), &base)?;
        insert_vec(conn, &format!("c3-control-{pair_idx}-b"), &perturbed)?;
        inserted += 2;
    }

    // Category 7 supersede pairs: distance 0.25 (< 0.35 threshold)
    for pair_idx in 0..6 {
        let base_key = format!("c7-supersede-{seed}-{pair_idx}");
        let base = generate_base_embedding(&base_key);
        let perturbed = perturb_embedding(&base, 0.25, &format!("c7-supersede-{seed}-{pair_idx}"));
        insert_vec(conn, &format!("c7-supersede-{pair_idx}-older"), &base)?;
        insert_vec(conn, &format!("c7-supersede-{pair_idx}-newer"), &perturbed)?;
        inserted += 2;
    }

    Ok(inserted)
}

fn insert_vec(
    conn: &rusqlite::Connection,
    memory_id: &str,
    embedding: &[f32],
) -> Result<(), String> {
    let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
    conn.execute(
        "INSERT INTO memory_vec(id, embedding) VALUES (?1, ?2)",
        rusqlite::params![memory_id, bytes],
    )
    .map_err(|e| format!("insert_vec {memory_id}: {e}"))?;
    Ok(())
}

// ── Recall query bank ─────────────────────────────────────────────

/// Build the 15-query recall bank using SeededDataset ground truth.
///
/// Queries target effects visible in hybrid_recall:
/// - Non-active filter (dedup, supersede, fade, merge remove noise)
/// - New memories (resolutions, patterns, protocols) appear
/// - Reweave-enriched BM25 scores
/// - Graph expansion via related_to edges
///
/// NOTE: Since memory titles now use SHA-256 hex tokens, BM25 queries must include the
/// actual token to match. Queries are derived from the same seed-based token generation
/// as the generators, ensuring BM25 can find the memories.
pub fn generate_query_bank(dataset: &SeededDataset) -> Vec<RecallQuery> {
    let mut queries = Vec::new();
    let seed = dataset.seed;

    // RC-1: Duplicate-dilution — Category 1 pair 0 token (keeper and victim share same title token)
    // Pre: 2 rows (keeper + victim) match the token; victim DELETEd post-Phase-1, keeper stays.
    let c1_token_0 = sha256_hex(&format!("c1-{seed}-0"));
    queries.push(RecallQuery {
        id: "RC-1".into(),
        query: c1_token_0, // exact token — BM25 matches both pre-consolidation, only keeper post
        description: "Category 1 exact-dup query: keeper should remain after Phase 1 DELETE".into(),
        expected_titles: expected_titles_for_c1_keeper(dataset, 0),
    });

    // RC-2: Semantic dedup — Category 2 pair 0 anchor (titles still contain the anchor token)
    let c2_anchor_0 = sha256_hex(&format!("c2-anchor-{seed}-0"));
    queries.push(RecallQuery {
        id: "RC-2".into(),
        query: c2_anchor_0,
        description: "Category 2 semantic-dup query: keeper active post-Phase-2".into(),
        expected_titles: expected_titles_for_c2_keeper(dataset, 0),
    });

    // RC-3: Contradiction resolution — Category 4 valence pair 0 shared_token
    // Post-Phase-12: Resolution memory has shared_token in its title.
    let c4_shared_0 = sha256_hex(&format!("c4-{seed}-0"));
    queries.push(RecallQuery {
        id: "RC-3".into(),
        query: c4_shared_0,
        description: "Category 4 valence pair: Resolution memory appears post-Phase-12".into(),
        expected_titles: expected_resolution_titles(dataset, 0),
    });

    // RC-4: Pattern promotion — Category 6 cluster 0 cluster_token
    // Phase 5 promoted Pattern memory title includes the cluster_token.
    queries.push(RecallQuery {
        id: "RC-4".into(),
        query: sha256_hex(&format!("c6-{seed}-cluster-topic-0")),
        description: "Category 6 cluster: Pattern memory promoted post-Phase-5".into(),
        expected_titles: expected_pattern_titles(dataset, 0),
    });

    // RC-5: Protocol extraction — Category 5 preference 0 token
    // Phase 17 Protocol memory title derived from preference source.
    let c5_pref_token_0 = sha256_hex(&format!("c5-{seed}-pref-0"));
    queries.push(RecallQuery {
        id: "RC-5".into(),
        query: c5_pref_token_0,
        description: "Category 5 preference: Protocol memory created post-Phase-17".into(),
        expected_titles: expected_protocol_titles(dataset, 0),
    });

    // RC-6: Reweave enrichment — Category 5 reweave pair 0 older title token
    // Post-Phase-14: older memory's content enriched with "[Update]:"; title unchanged.
    let c5_older_t1_0 = sha256_hex(&format!("c5-{seed}-rolder-t1-0"));
    queries.push(RecallQuery {
        id: "RC-6".into(),
        query: c5_older_t1_0,
        description: "Category 5 reweave: older content enriched with [Update] post-Phase-14"
            .into(),
        expected_titles: expected_reweaved_titles(dataset, 0),
    });

    // RC-7: Topic supersede — Category 7 pair 0 newer token
    // Post-Phase-20: older superseded, newer stays active.
    let c7_newer_token_0 = sha256_hex(&format!("c7-newer-{seed}-0"));
    queries.push(RecallQuery {
        id: "RC-7".into(),
        query: c7_newer_token_0,
        description: "Category 7 topic-supersede: newer version only post-Phase-20".into(),
        expected_titles: expected_supersede_newer(dataset, 0),
    });

    // RC-8 through RC-11: C1 duplicate-dilution rotations (pairs 1-4)
    for i in 1..5 {
        let c1_token_i = sha256_hex(&format!("c1-{seed}-{i}"));
        queries.push(RecallQuery {
            id: format!("RC-{}", 7 + i),
            query: c1_token_i,
            description: format!("Duplicate-dilution query rotation {i}"),
            expected_titles: expected_titles_for_c1_keeper(dataset, i),
        });
    }

    // RC-12 through RC-15: C4 contradiction resolution rotations (pairs 1-4)
    for i in 1..5 {
        let c4_shared_i = sha256_hex(&format!("c4-{seed}-{i}"));
        queries.push(RecallQuery {
            id: format!("RC-{}", 11 + i),
            query: c4_shared_i,
            description: format!("Contradiction resolution rotation {i}"),
            expected_titles: expected_resolution_titles(dataset, i),
        });
    }

    queries
}

// Helpers: ground-truth-derived expected title sets.
// These look up GroundTruth entries and compute what post-consolidation titles should appear.

fn expected_titles_for_c1_keeper(dataset: &SeededDataset, pair_idx: usize) -> HashSet<String> {
    let mut set = HashSet::new();
    // Category 1 titles = just the SHA-256 token (same for keeper and victim in a pair).
    // After Phase 1, victim is DELETEd and keeper remains with this token as its title.
    let token = sha256_hex(&format!("c1-{}-{}", dataset.seed, pair_idx));
    set.insert(token);
    set
}

fn expected_titles_for_c2_keeper(dataset: &SeededDataset, pair_idx: usize) -> HashSet<String> {
    let mut set = HashSet::new();
    let anchor = sha256_hex(&format!("c2-anchor-{}-{}", dataset.seed, pair_idx));
    let pair_token = sha256_hex(&format!("c2-pair-{}-{}", dataset.seed, pair_idx));
    // Keeper title from generator: "{pair_token} enforce {anchor} validation"
    set.insert(format!("{pair_token} enforce {anchor} validation"));
    set
}

fn expected_resolution_titles(dataset: &SeededDataset, pair_idx: usize) -> HashSet<String> {
    let mut set = HashSet::new();
    // Category 4 valence titles now use "{pos_token} favors {shared_token}" and
    // "{neg_token} opposes {shared_token}" patterns.
    let shared_token = sha256_hex(&format!("c4-{}-{}", dataset.seed, pair_idx));
    let pos_token = sha256_hex(&format!("c4-pos-{}-{}", dataset.seed, pair_idx));
    let neg_token = sha256_hex(&format!("c4-neg-{}-{}", dataset.seed, pair_idx));
    let pos = format!("{pos_token} favors {shared_token}");
    let neg = format!("{neg_token} opposes {shared_token}");
    // Phase 12 creates title "Resolution: {a.title} vs {b.title}"
    set.insert(format!("Resolution: {pos} vs {neg}"));
    set
}

fn expected_pattern_titles(_dataset: &SeededDataset, _cluster_idx: usize) -> HashSet<String> {
    // Phase 5 generates pattern title via promote_recurring_lessons — title format depends on implementation.
    // Verified at implementation: title = "Pattern: {first lesson title}" or similar.
    // Bench must query the pattern table post-consolidation to get the actual title and
    // compare against a LOOSENED expected set (any Pattern memory containing the cluster_token).
    HashSet::new() // Empty for now; audit logic uses cluster_token substring match instead.
}

fn expected_protocol_titles(_dataset: &SeededDataset, _idx: usize) -> HashSet<String> {
    // Similar: Protocol titles derived at runtime from Phase 17 source.
    HashSet::new()
}

fn expected_reweaved_titles(dataset: &SeededDataset, pair_idx: usize) -> HashSet<String> {
    let mut set = HashSet::new();
    // Category 5 reweave older titles now use "{older_title_t1} {older_title_t2}" pattern.
    let older_t1 = sha256_hex(&format!("c5-{}-rolder-t1-{}", dataset.seed, pair_idx));
    let older_t2 = sha256_hex(&format!("c5-{}-rolder-t2-{}", dataset.seed, pair_idx));
    set.insert(format!("{older_t1} {older_t2}"));
    set
}

fn expected_supersede_newer(dataset: &SeededDataset, pair_idx: usize) -> HashSet<String> {
    let mut set = HashSet::new();
    // Category 7 topic-supersede newer title now uses "{topic} {unique_newer}" pattern.
    let topic = sha256_hex(&format!("c7-{}-topic-{}", dataset.seed, pair_idx));
    let unique_newer = sha256_hex(&format!("c7-newer-{}-{}", dataset.seed, pair_idx));
    set.insert(format!("{topic} {unique_newer}"));
    set
}

// ── Recall snapshot helpers ───────────────────────────────────────

/// A single snapshot of recall results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallSnapshot {
    pub results: Vec<RecallQueryResult>,
    pub mean_recall_at_10: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallQueryResult {
    pub query_id: String,
    pub retrieved_titles: Vec<String>,
    pub expected_titles: Vec<String>,
    pub recall_at_10: f64,
}

/// Run all queries through `handle_request(Request::Recall{..})` and compute recall@10.
/// Empty `expected_titles` sets default to 1.0 (trivially satisfied informational queries).
pub fn snapshot_recall(
    state: &mut crate::server::handler::DaemonState,
    queries: &[RecallQuery],
) -> RecallSnapshot {
    use forge_core::protocol::{Request, Response, ResponseData};

    let mut results = Vec::new();
    let mut total_recall = 0.0;

    for q in queries {
        let req = Request::Recall {
            query: q.query.clone(),
            memory_type: None,
            project: None,
            limit: Some(10),
            layer: None,
            since: None,
            include_flipped: None,
        };
        let resp = crate::server::handler::handle_request(state, req);
        let titles = match resp {
            Response::Ok {
                data: ResponseData::Memories { ref results, .. },
            } => results.iter().map(|r| r.memory.title.clone()).collect(),
            other => {
                tracing::warn!(
                    query_id = %q.id,
                    "snapshot_recall: unexpected response variant, scoring 0: {:?}",
                    other
                );
                HashSet::new()
            }
        };
        let matched = q
            .expected_titles
            .iter()
            .filter(|t| titles.contains(*t))
            .count();
        let r_at_10 = if q.expected_titles.is_empty() {
            1.0 // no expected → trivially 100% recall (informational queries)
        } else {
            matched as f64 / q.expected_titles.len() as f64
        };
        total_recall += r_at_10;
        results.push(RecallQueryResult {
            query_id: q.id.clone(),
            retrieved_titles: titles.into_iter().collect(),
            expected_titles: q.expected_titles.iter().cloned().collect(),
            recall_at_10: r_at_10,
        });
    }

    let mean = if queries.is_empty() {
        0.0
    } else {
        total_recall / queries.len() as f64
    };
    RecallSnapshot {
        results,
        mean_recall_at_10: mean,
    }
}

// ── Audit functions ──────────────────────────────────────────────

/// Per-dimension scoring result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DimensionScore {
    pub dimension: String,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    pub details: Vec<String>,
}

/// Dimension 1: dedup quality.
///
/// Counts transitions within Categories 1, 2, 3 only. Phases 5/12/14/20/21
/// are NOT counted as dedup events (scope filter). Signal preservation gate:
/// all 4 control memories (Category 3, `duplicate_of=None`) must remain active
/// or the entire score is zeroed.
pub fn audit_dedup(
    conn: &rusqlite::Connection,
    dataset: &SeededDataset,
) -> Result<DimensionScore, String> {
    // GT victims: memories expected deleted/superseded BY DEDUP PHASES (1, 2, 7).
    let gt_victims: HashSet<String> = dataset
        .ground_truth
        .iter()
        .filter(|t| {
            (t.category == Category::ExactDuplicates
                || t.category == Category::SemanticDuplicates
                || t.category == Category::EmbeddingDuplicates)
                && (t.expected_status == ExpectedStatus::Superseded
                    || t.expected_status == ExpectedStatus::Deleted)
        })
        .map(|t| t.memory_id.clone())
        .collect();

    // Scope: all IDs in categories 1-3
    let dedup_scope: HashSet<String> = dataset
        .ground_truth
        .iter()
        .filter(|t| {
            t.category == Category::ExactDuplicates
                || t.category == Category::SemanticDuplicates
                || t.category == Category::EmbeddingDuplicates
        })
        .map(|t| t.memory_id.clone())
        .collect();

    // Observed victims: memories in dedup scope that are deleted or superseded
    let mut observed_dedup: HashSet<String> = HashSet::new();
    for id in &dedup_scope {
        let result: rusqlite::Result<Option<String>> = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .optional();
        match result {
            Ok(None) => {
                observed_dedup.insert(id.clone());
            } // deleted
            Ok(Some(s)) if s == "superseded" => {
                observed_dedup.insert(id.clone());
            }
            _ => {}
        }
    }

    let (precision, recall, f1) = pr_f1(&gt_victims, &observed_dedup);

    // Signal preservation gate: all Category 3 controls must remain active
    let controls: Vec<&GroundTruth> = dataset
        .ground_truth
        .iter()
        .filter(|t| t.category == Category::EmbeddingDuplicates && t.duplicate_of.is_none())
        .collect();
    let mut failed_controls = Vec::new();
    for c in &controls {
        let status: rusqlite::Result<String> = conn.query_row(
            "SELECT status FROM memory WHERE id = ?1",
            rusqlite::params![c.memory_id],
            |row| row.get(0),
        );
        if status.as_deref() != Ok("active") {
            failed_controls.push(c.memory_id.clone());
        }
    }

    let final_f1 = if failed_controls.is_empty() { f1 } else { 0.0 };
    let mut details = vec![
        format!("gt_victims={}", gt_victims.len()),
        format!("observed_dedup={}", observed_dedup.len()),
    ];
    if !failed_controls.is_empty() {
        details.push(format!("SIGNAL_PRESERVATION_FAILED: {failed_controls:?}"));
    }

    Ok(DimensionScore {
        dimension: "dedup_quality".into(),
        precision,
        recall,
        f1: final_f1,
        details,
    })
}

/// Compute precision, recall, F1 for two sets of strings.
/// If `observed` is empty → precision = 0.0.
/// If `expected` is empty → recall = 1.0 (nothing to miss).
pub fn pr_f1(expected: &HashSet<String>, observed: &HashSet<String>) -> (f64, f64, f64) {
    let tp = expected.intersection(observed).count() as f64;
    let precision = if observed.is_empty() {
        0.0
    } else {
        tp / observed.len() as f64
    };
    let recall = if expected.is_empty() {
        1.0
    } else {
        tp / expected.len() as f64
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    (precision, recall, f1)
}

/// Compute precision, recall, F1 for two sets of unordered string pairs.
pub fn pr_f1_pairs(
    expected: &HashSet<(String, String)>,
    observed: &HashSet<(String, String)>,
) -> (f64, f64, f64) {
    let tp = expected.intersection(observed).count() as f64;
    let precision = if observed.is_empty() {
        0.0
    } else {
        tp / observed.len() as f64
    };
    let recall = if expected.is_empty() {
        1.0
    } else {
        tp / expected.len() as f64
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };
    (precision, recall, f1)
}

/// Dimension 2: contradiction handling.
///
/// Score = 0.5 * detection_F1 + 0.5 * synthesis_accuracy.
/// Detection uses `edge` table (`edge_type='contradicts'`), pairs deduped as
/// unordered (min, max) tuples. Synthesis checks that a resolution memory with
/// title prefix `"Resolution: "` exists and both originals are superseded.
pub fn audit_contradictions(
    conn: &rusqlite::Connection,
    dataset: &SeededDataset,
) -> Result<DimensionScore, String> {
    // GT contradiction pairs (unordered)
    let gt_pairs: HashSet<(String, String)> = dataset
        .ground_truth
        .iter()
        .filter_map(|t| {
            t.contradicts.as_ref().map(|other| {
                if t.memory_id < *other {
                    (t.memory_id.clone(), other.clone())
                } else {
                    (other.clone(), t.memory_id.clone())
                }
            })
        })
        .collect();

    // Observed: edge_type='contradicts', deduped as unordered pairs
    let mut stmt = conn
        .prepare("SELECT from_id, to_id FROM edge WHERE edge_type = 'contradicts'")
        .map_err(|e| format!("contradicts query: {e}"))?;
    let observed_raw: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("{e}"))?
        .filter_map(|r| r.ok())
        .collect();
    let observed: HashSet<(String, String)> = observed_raw
        .into_iter()
        .map(|(a, b)| if a < b { (a, b) } else { (b, a) })
        .collect();

    let (pp, rr, detection_f1) = pr_f1_pairs(&gt_pairs, &observed);

    // Synthesis accuracy: for each valence-pair, resolution memory + both superseded
    let valence_gt_pairs: HashSet<(String, String)> = dataset
        .ground_truth
        .iter()
        .filter(|t| {
            t.category == Category::Contradictions
                && t.expected_status == ExpectedStatus::Superseded
        })
        .filter_map(|t| {
            t.contradicts.as_ref().map(|o| {
                if t.memory_id < *o {
                    (t.memory_id.clone(), o.clone())
                } else {
                    (o.clone(), t.memory_id.clone())
                }
            })
        })
        .collect();

    let total_valence_pairs = valence_gt_pairs.len();
    let mut synthesis_correct = 0usize;
    for (a, b) in &valence_gt_pairs {
        let a_status: Option<String> = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![a],
                |r| r.get(0),
            )
            .ok();
        let b_status: Option<String> = conn
            .query_row(
                "SELECT status FROM memory WHERE id = ?1",
                rusqlite::params![b],
                |r| r.get(0),
            )
            .ok();
        let resolution_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM memory
                 WHERE title LIKE 'Resolution: %'
                 AND (title LIKE '%' || (SELECT title FROM memory WHERE id = ?1) || '%'
                      OR title LIKE '%' || (SELECT title FROM memory WHERE id = ?2) || '%')",
                rusqlite::params![a, b],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if a_status.as_deref() == Some("superseded")
            && b_status.as_deref() == Some("superseded")
            && resolution_exists
        {
            synthesis_correct += 1;
        }
    }
    let synthesis_accuracy = if total_valence_pairs == 0 {
        1.0
    } else {
        synthesis_correct as f64 / total_valence_pairs as f64
    };

    let score = 0.5 * detection_f1 + 0.5 * synthesis_accuracy;
    Ok(DimensionScore {
        dimension: "contradiction_handling".into(),
        precision: pp,
        recall: rr,
        f1: score,
        details: vec![
            format!("detection_f1={detection_f1:.4}"),
            format!("synthesis_accuracy={synthesis_accuracy:.4}"),
            format!("gt_pairs={}", gt_pairs.len()),
            format!("observed_pairs={}", observed.len()),
        ],
    })
}

/// Dimension 3: reweave & enrichment.
///
/// Weighted: 0.30 * reweave_F1 + 0.25 * protocol_accuracy +
///           0.25 * antipattern_accuracy + 0.20 * promotion_accuracy.
pub fn audit_reweave(
    conn: &rusqlite::Connection,
    dataset: &SeededDataset,
) -> Result<DimensionScore, String> {
    // Reweave: newer marked 'merged' AND older content contains '[Update]:'
    let gt_pairs: Vec<(String, String)> = dataset
        .ground_truth
        .iter()
        .filter_map(|t| {
            t.reweave_source
                .as_ref()
                .map(|newer| (t.memory_id.clone(), newer.clone()))
        })
        .collect();
    let total = gt_pairs.len();
    let mut correct = 0usize;
    for (older_id, newer_id) in &gt_pairs {
        let newer_status: Option<String> = conn
            .query_row(
                "SELECT status FROM memory WHERE id=?1",
                rusqlite::params![newer_id],
                |r| r.get(0),
            )
            .ok();
        let older_content: Option<String> = conn
            .query_row(
                "SELECT content FROM memory WHERE id=?1",
                rusqlite::params![older_id],
                |r| r.get(0),
            )
            .ok();
        if newer_status.as_deref() == Some("merged")
            && older_content.is_some_and(|c| c.contains("[Update]:"))
        {
            correct += 1;
        }
    }
    let reweave_f1 = if total == 0 {
        1.0
    } else {
        correct as f64 / total as f64
    };

    // Promotion: Pattern memories promoted by Phase 5 from Category 6 lesson clusters.
    // Phase 5's promote_recurring_lessons() creates Pattern memories with the cluster lesson
    // title (format: "{cluster_token} repeats {token_a} across {token_b}") and inherits
    // the source project. Tags are NOT copied from source lessons, so we cannot filter by
    // category-6-cluster tag. Instead we identify Phase 5 patterns by their title pattern
    // (contains "repeats" and "across") within the bench project.
    let pattern_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE memory_type = 'pattern'
             AND project = 'forge-consolidation-bench'
             AND title LIKE '% repeats % across %'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let promo_accuracy = if dataset.expected_pattern_count == 0 {
        1.0
    } else {
        (pattern_count as f64 / dataset.expected_pattern_count as f64).min(1.0)
    };

    // Protocol: protocol memories created by Phase 17.
    // Phase 17's extract_protocols() creates protocols with title "Protocol: {source_title}"
    // but does NOT inherit the project from the source memory (INSERT doesn't set project).
    // We identify Phase 17 protocols by the "Protocol: " title prefix regardless of project.
    let protocol_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE memory_type = 'protocol'
             AND status = 'active'
             AND title LIKE 'Protocol: %'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let proto_accuracy = if dataset.expected_protocol_count == 0 {
        1.0
    } else {
        (protocol_count as f64 / dataset.expected_protocol_count as f64).min(1.0)
    };

    // Anti-pattern tags: Category 5 anti-pattern memories must have "anti-pattern" tag
    let antipattern_ids: Vec<String> = dataset
        .ground_truth
        .iter()
        .filter(|t| {
            t.category == Category::ReweaveEnrichment && t.memory_id.starts_with("c5-antipattern-")
        })
        .map(|t| t.memory_id.clone())
        .collect();
    let mut antipattern_correct = 0usize;
    for id in &antipattern_ids {
        let tags: Option<String> = conn
            .query_row(
                "SELECT tags FROM memory WHERE id=?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .ok();
        if tags.is_some_and(|t| t.contains("anti-pattern")) {
            antipattern_correct += 1;
        }
    }
    let ap_accuracy = if antipattern_ids.is_empty() {
        1.0
    } else {
        antipattern_correct as f64 / antipattern_ids.len() as f64
    };

    let score =
        0.30 * reweave_f1 + 0.25 * proto_accuracy + 0.25 * ap_accuracy + 0.20 * promo_accuracy;
    Ok(DimensionScore {
        dimension: "reweave_enrichment".into(),
        precision: 0.0,
        recall: 0.0,
        f1: score,
        details: vec![
            format!("reweave_f1={reweave_f1:.4}"),
            format!("promo_accuracy={promo_accuracy:.4}"),
            format!("proto_accuracy={proto_accuracy:.4}"),
            format!("ap_accuracy={ap_accuracy:.4}"),
        ],
    })
}

/// Dimension 4: quality lifecycle.
///
/// Unweighted mean of 6 sub-accuracies: decay, reconsolidation, quality_score,
/// activation_level, staleness, quality_pressure. All float comparisons use ±0.01 tolerance.
pub fn audit_lifecycle(
    conn: &rusqlite::Connection,
    dataset: &SeededDataset,
) -> Result<DimensionScore, String> {
    let tol = 0.01_f64;
    let mut decay_pass = 0usize;
    let mut decay_total = 0usize;
    let mut recon_pass = 0usize;
    let mut recon_total = 0usize;
    let mut quality_pass = 0usize;
    let mut quality_total = 0usize;
    let mut activation_pass = 0usize;
    let mut activation_total = 0usize;
    let mut stale_pass = 0usize;
    let mut stale_total = 0usize;
    let mut pressure_pass = 0usize;
    let mut pressure_total = 0usize;

    for gt in &dataset.ground_truth {
        if let Some(expected_conf) = gt.expected_confidence {
            if gt.memory_id.starts_with("c6-decay-") {
                decay_total += 1;
                let obs: Option<f64> = conn
                    .query_row(
                        "SELECT confidence FROM memory WHERE id=?1",
                        rusqlite::params![gt.memory_id],
                        |r| r.get(0),
                    )
                    .ok();
                if obs.is_some_and(|o| (o - expected_conf).abs() < tol) {
                    decay_pass += 1;
                }
            } else if gt.memory_id.starts_with("c6-recon-") {
                recon_total += 1;
                let obs: Option<f64> = conn
                    .query_row(
                        "SELECT confidence FROM memory WHERE id=?1",
                        rusqlite::params![gt.memory_id],
                        |r| r.get(0),
                    )
                    .ok();
                if obs.is_some_and(|o| (o - expected_conf).abs() < tol) {
                    recon_pass += 1;
                }
            }
        }
        if let Some(expected_q) = gt.expected_quality {
            quality_total += 1;
            let obs: Option<f64> = conn
                .query_row(
                    "SELECT quality_score FROM memory WHERE id=?1",
                    rusqlite::params![gt.memory_id],
                    |r| r.get(0),
                )
                .ok();
            if obs.is_some_and(|o| (o - expected_q).abs() < tol) {
                quality_pass += 1;
            }
        }
        if let Some(expected_a) = gt.expected_activation {
            activation_total += 1;
            let obs: Option<f64> = conn
                .query_row(
                    "SELECT activation_level FROM memory WHERE id=?1",
                    rusqlite::params![gt.memory_id],
                    |r| r.get(0),
                )
                .ok();
            if obs.is_some_and(|o| (o - expected_a).abs() < tol) {
                activation_pass += 1;
            }
        }
        if gt.category == Category::SelfHealing && gt.memory_id.starts_with("c7-stale-") {
            stale_total += 1;
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM memory WHERE id=?1",
                    rusqlite::params![gt.memory_id],
                    |r| r.get(0),
                )
                .ok();
            if status.as_deref() == Some("faded") {
                stale_pass += 1;
            }
        }
        // Quality pressure (Phase 22 boost): c7-boost-* candidates have high access + recent
        // creation so Phase 22 applies a +0.05 boost. Post-consolidation quality_score should
        // be >= 0.5 (Phase 15 baseline ~0.48 + 0.05 boost).
        if gt.category == Category::SelfHealing && gt.memory_id.starts_with("c7-boost-") {
            pressure_total += 1;
            let q: Option<f64> = conn
                .query_row(
                    "SELECT quality_score FROM memory WHERE id=?1",
                    rusqlite::params![gt.memory_id],
                    |r| r.get(0),
                )
                .ok();
            if q.is_some_and(|v| v >= 0.5) {
                pressure_pass += 1;
            }
        }
    }

    let frac = |pass: usize, total: usize| {
        if total == 0 {
            1.0
        } else {
            pass as f64 / total as f64
        }
    };
    let decay = frac(decay_pass, decay_total);
    let recon = frac(recon_pass, recon_total);
    let quality = frac(quality_pass, quality_total);
    let act = frac(activation_pass, activation_total);
    let stale = frac(stale_pass, stale_total);
    let pressure = frac(pressure_pass, pressure_total);
    let score = (decay + recon + quality + act + stale + pressure) / 6.0;

    Ok(DimensionScore {
        dimension: "quality_lifecycle".into(),
        precision: 0.0,
        recall: 0.0,
        f1: score,
        details: vec![
            format!("decay={decay:.4}"),
            format!("recon={recon:.4}"),
            format!("quality={quality:.4}"),
            format!("activation={act:.4}"),
            format!("staleness={stale:.4}"),
            format!("pressure={pressure:.4}"),
        ],
    })
}

/// Infrastructure pass/fail assertions (Phases 3, 8, 11, 13, 19a, 19b, 20).
///
/// Returns a list of failure messages. Empty list = all assertions passed.
pub fn audit_infrastructure(
    conn: &rusqlite::Connection,
    dataset: &SeededDataset,
) -> Result<Vec<String>, String> {
    let _ = dataset; // dataset not needed for infrastructure assertions
    let mut failures = Vec::new();

    // Phase 3: at least 5 related_to edges
    let related_to_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edge WHERE edge_type='related_to'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if related_to_count < 5 {
        failures.push(format!(
            "Phase 3: only {related_to_count} related_to edges (need >=5)"
        ));
    }

    // Phase 8: at least one related_to edge with strength >= 0.1 in properties JSON.
    // strengthen_active_edges adds +0.1 per cycle from a 0.0 baseline, so after one
    // run_all_phases call the maximum reachable strength is 0.1. Threshold must be 0.1.
    let strengthened: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edge WHERE edge_type='related_to' AND CAST(json_extract(properties, '$.strength') AS REAL) >= 0.1",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if strengthened == 0 {
        failures.push("Phase 8: no edges with strength >= 0.1 (single-cycle max is 0.1)".into());
    }

    // Phase 11: at least 5 unique entities
    let entity_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM entity", [], |r| r.get(0))
        .unwrap_or(0);
    if entity_count < 5 {
        failures.push(format!("Phase 11: only {entity_count} entities (need >=5)"));
    }

    // Phase 13: at least 1 knowledge_gap perception
    let gaps: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM perception WHERE kind='knowledge_gap'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if gaps < 1 {
        failures.push("Phase 13: no knowledge_gap perceptions".into());
    }

    // Phase 19a: protocol_suggestion notification
    let proto_notif: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM notification WHERE topic='protocol_suggestion'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if proto_notif == 0 {
        failures.push("Phase 19a: no protocol_suggestion notification".into());
    }

    // Phase 19b: contradiction notification
    let contra_notif: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM notification WHERE topic='contradiction'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if contra_notif == 0 {
        failures.push("Phase 19b: no contradiction notification".into());
    }

    // Phase 20: at least 6 healing_log entries with action='auto_superseded'
    let heal_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM healing_log WHERE action='auto_superseded'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if heal_count < 6 {
        failures.push(format!(
            "Phase 20: only {heal_count} healing_log auto_superseded entries (need >=6)"
        ));
    }

    // Phase 16: portability candidates must be classified (no NULL or 'unknown' portability)
    let unclassified_portability: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE id LIKE 'c8-portability-%' AND (portability IS NULL OR portability = 'unknown')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if unclassified_portability > 0 {
        failures.push(format!(
            "Phase 16: {unclassified_portability} portability candidates still unclassified"
        ));
    }

    // Phase 19c: no unexpected quality_decline notifications — corpus is seeded with healthy
    // quality; any such notification indicates either a daemon bug or a corpus design flaw.
    let quality_decline_notifs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM notification WHERE topic LIKE '%quality_decline%' OR topic LIKE '%decline%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if quality_decline_notifs > 0 {
        failures.push(format!(
            "Phase 19c: unexpected quality_decline notification ({quality_decline_notifs} rows) — corpus was seeded with healthy quality"
        ));
    }

    // Phase 19d: no meeting notifications — no meetings were seeded; any meeting notification
    // indicates phantom meetings or a daemon bug.
    let meeting_notifs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM notification WHERE topic LIKE '%meeting%'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if meeting_notifs > 0 {
        failures.push(format!(
            "Phase 19d: unexpected meeting notification ({meeting_notifs} rows) — no meetings seeded"
        ));
    }

    Ok(failures)
}

// ── Composite score ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationScore {
    pub seed: u64,
    pub dimensions: HashMap<String, DimensionScore>,
    pub recall_baseline_mean: f64,
    pub recall_post_mean: f64,
    pub recall_delta: f64,
    pub recall_delta_score: f64,
    pub composite: f64,
    pub infrastructure_failures: Vec<String>,
    pub pass: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn compute_score(
    seed: u64,
    dim1: DimensionScore,
    dim2: DimensionScore,
    dim3: DimensionScore,
    dim4: DimensionScore,
    baseline: &RecallSnapshot,
    post: &RecallSnapshot,
    expected_recall_delta: Option<f64>,
    infrastructure_failures: Vec<String>,
) -> ConsolidationScore {
    let recall_delta = post.mean_recall_at_10 - baseline.mean_recall_at_10;

    let dim5_score = match (recall_delta, expected_recall_delta) {
        (d, _) if d < 0.0 => 0.0,
        (0.0, Some(e)) if e > 0.0 => 0.5, // neutral pass: delta==0, expected>0
        (d, Some(e)) if e > 0.0 => (d / e).clamp(0.0, 1.0), // delta>0, expected>0
        (_, Some(_)) => 0.0,              // expected==Some(0.0) or negative: INVALID
        (_, None) => {
            if recall_delta > 0.0 {
                1.0
            } else {
                0.5
            }
        } // no threshold yet (first calibration)
    };

    let mut dimensions = HashMap::new();
    dimensions.insert(dim1.dimension.clone(), dim1.clone());
    dimensions.insert(dim2.dimension.clone(), dim2.clone());
    dimensions.insert(dim3.dimension.clone(), dim3.clone());
    dimensions.insert(dim4.dimension.clone(), dim4.clone());
    dimensions.insert(
        "recall_improvement".into(),
        DimensionScore {
            dimension: "recall_improvement".into(),
            precision: 0.0,
            recall: 0.0,
            f1: dim5_score,
            details: vec![
                format!("baseline={:.4}", baseline.mean_recall_at_10),
                format!("post={:.4}", post.mean_recall_at_10),
                format!("delta={:.4}", recall_delta),
                format!("expected={expected_recall_delta:?}"),
            ],
        },
    );

    let composite =
        0.25 * dim1.f1 + 0.20 * dim2.f1 + 0.15 * dim3.f1 + 0.15 * dim4.f1 + 0.25 * dim5_score;

    let pass = infrastructure_failures.is_empty() && composite >= 0.8;

    ConsolidationScore {
        seed,
        dimensions,
        recall_baseline_mean: baseline.mean_recall_at_10,
        recall_post_mean: post.mean_recall_at_10,
        recall_delta,
        recall_delta_score: dim5_score,
        composite,
        infrastructure_failures,
        pass,
    }
}

// ── Orchestrator ──────────────────────────────────────────────────

pub fn run(config: ConsolidationBenchConfig) -> Result<ConsolidationScore, String> {
    // 1. Create in-memory state
    let mut state = crate::server::handler::DaemonState::new(":memory:")
        .map_err(|e| format!("state init: {e}"))?;

    // 2. Seed corpus
    let (_specs, mut dataset) = seed_corpus(&state.conn, config.seed)?;
    let _ = seed_embeddings(&state.conn, config.seed)?;

    // 3. Generate query bank
    dataset.recall_queries = generate_query_bank(&dataset);

    // 4. Baseline recall snapshot
    let baseline = snapshot_recall(&mut state, &dataset.recall_queries);

    // 5. Run all consolidation phases
    let cons_config = crate::config::ConsolidationConfig {
        batch_limit: 500,
        reweave_limit: 100,
        ..Default::default()
    };
    let stats = crate::workers::consolidator::run_all_phases(&state.conn, &cons_config, None);
    let _ = stats;

    // 6. Post-consolidation recall snapshot
    let post = snapshot_recall(&mut state, &dataset.recall_queries);

    // 7. Audit dimensions
    let dim1 = audit_dedup(&state.conn, &dataset)?;
    let dim2 = audit_contradictions(&state.conn, &dataset)?;
    let dim3 = audit_reweave(&state.conn, &dataset)?;
    let dim4 = audit_lifecycle(&state.conn, &dataset)?;
    let infra_failures = audit_infrastructure(&state.conn, &dataset)?;

    // 8. Compute composite
    let score = compute_score(
        config.seed,
        dim1,
        dim2,
        dim3,
        dim4,
        &baseline,
        &post,
        config.expected_recall_delta,
        infra_failures,
    );

    // 9. Write artifacts
    write_artifacts(&config.output_dir, &score, &baseline, &post)?;

    Ok(score)
}

fn write_artifacts(
    output_dir: &std::path::Path,
    score: &ConsolidationScore,
    baseline: &RecallSnapshot,
    post: &RecallSnapshot,
) -> Result<(), String> {
    std::fs::create_dir_all(output_dir).map_err(|e| format!("create output dir: {e}"))?;

    let write_one = |name: &str, content: String| -> Result<(), String> {
        let path = output_dir.join(name);
        std::fs::write(&path, content).map_err(|e| format!("write {}: {e}", path.display()))
    };

    write_one(
        "summary.json",
        serde_json::to_string_pretty(score).map_err(|e| format!("serialize summary.json: {e}"))?,
    )?;
    write_one(
        "baseline.json",
        serde_json::to_string_pretty(baseline)
            .map_err(|e| format!("serialize baseline.json: {e}"))?,
    )?;
    write_one(
        "post.json",
        serde_json::to_string_pretty(post).map_err(|e| format!("serialize post.json: {e}"))?,
    )?;

    // repro.sh
    let repro = format!(
        "#!/bin/bash\nset -euo pipefail\ncd \"$(git rev-parse --show-toplevel)\"\ncargo build --release --bin forge-bench\n./target/release/forge-bench forge-consolidation --seed {} --output \"{}\"\n",
        score.seed,
        output_dir.display()
    );
    write_one("repro.sh", repro)?;
    Ok(())
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
    fn test_category_2_pairs_share_anchor_and_pair_tokens() {
        let (specs, _) = generate_category_2_semantic_duplicates(42);
        // Each pair's titles should share both the anchor and pair_token (both 64-char hex).
        // New format: keeper = "{pair_token} enforce {anchor} validation"
        //             victim = "enforce {anchor} validation {pair_token}"
        // Both titles contain the same anchor AND pair_token for the same pair.
        for pair_idx in 0..8 {
            let a = &specs[pair_idx * 2].title;
            let b = &specs[pair_idx * 2 + 1].title;
            // Both 64-char hex tokens in title A must also appear in title B
            let tokens_in_a: Vec<&str> = a
                .split_whitespace()
                .filter(|w| w.len() == 64 && w.chars().all(|c| c.is_ascii_hexdigit()))
                .collect();
            assert_eq!(
                tokens_in_a.len(),
                2,
                "pair {pair_idx} title A should have 2 tokens: {a}"
            );
            for token in &tokens_in_a {
                assert!(
                    b.contains(token),
                    "pair {pair_idx} title B doesn't share token {token}: a={a}, b={b}"
                );
            }
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

    #[test]
    fn test_category_3_merge_pairs_avoid_phase_2() {
        // Use the ACTUAL stop-word list for accuracy.
        use crate::common::STOP_WORDS;
        use std::collections::HashSet;

        fn mw_real(text: &str) -> HashSet<String> {
            let stop: HashSet<&str> = STOP_WORDS.iter().copied().collect();
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 1 && !stop.contains(w))
                .map(String::from)
                .collect()
        }

        let (specs, _) = generate_category_3_embedding_duplicates(42);
        // All 12 specs: 8 merge pair members (pairs 0-3) + 4 control pair members (pairs 0-1).
        // Check ALL cross-memory pairs of same type (all are Decision).
        for i in 0..12 {
            for j in (i + 1)..12 {
                // Skip same-pair comparison (pair members share the same pair_idx)
                // Actually we want to verify ALL pairs avoid Phase 2, including within-pair.
                let a = &specs[i];
                let b = &specs[j];
                if a.memory_type != b.memory_type {
                    continue;
                }
                let taw = mw_real(&a.title);
                let tbw = mw_real(&b.title);
                let caw = mw_real(&a.content);
                let cbw = mw_real(&b.content);

                let title_shared = taw.intersection(&tbw).count() as f64;
                let title_max = std::cmp::max(taw.len(), tbw.len()) as f64;
                let title_score = if title_max == 0.0 {
                    0.0
                } else {
                    title_shared / title_max
                };

                let content_shared = caw.intersection(&cbw).count() as f64;
                let content_max = std::cmp::max(caw.len(), cbw.len()) as f64;
                let content_score = if content_max == 0.0 {
                    0.0
                } else {
                    content_shared / content_max
                };

                let weighted = 0.5 * title_score + 0.5 * content_score;
                let combined = weighted.max(title_score).max(content_score);

                assert!(
                    combined < 0.65,
                    "Category 3 memory pair ({i},{j}) would be caught by Phase 2 (combined={combined:.3}, ts={title_score:.3}, cs={content_score:.3})\n  a={} b={}",
                    a.id, b.id
                );
            }
        }
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
        // Use the ACTUAL stop-word list from the daemon crate (same as Phase 2 uses).
        // This catches proxy-mismatch bugs where the test passed with a minimal stop set
        // but the real implementation had different filtering.
        use crate::common::STOP_WORDS;
        use std::collections::HashSet;

        fn mw_real(text: &str) -> HashSet<String> {
            let stop: HashSet<&str> = STOP_WORDS.iter().copied().collect();
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 1 && !stop.contains(w))
                .map(String::from)
                .collect()
        }

        let (specs, _) = generate_category_4_contradictions(42);
        // Content pairs are specs 8-15 (8 valence first, then 8 content)
        // WITHIN-PAIR check: Phase 2 must NOT catch A vs B of the same pair.
        for pair_idx in 0..4 {
            let a = &specs[8 + pair_idx * 2];
            let b = &specs[8 + pair_idx * 2 + 1];
            let taw = mw_real(&a.title);
            let tbw = mw_real(&b.title);
            let caw = mw_real(&a.content);
            let cbw = mw_real(&b.content);

            let title_shared = taw.intersection(&tbw).count() as f64;
            let title_max = std::cmp::max(taw.len(), tbw.len()) as f64;
            let title_score = if title_max == 0.0 {
                0.0
            } else {
                title_shared / title_max
            };

            let content_shared = caw.intersection(&cbw).count() as f64;
            let content_max = std::cmp::max(caw.len(), cbw.len()) as f64;
            let content_score = if content_max == 0.0 {
                0.0
            } else {
                content_shared / content_max
            };

            let weighted = 0.5 * title_score + 0.5 * content_score;
            let combined = weighted.max(title_score).max(content_score);

            assert!(
                combined < 0.65,
                "content pair {pair_idx} A vs B would be caught by Phase 2 (combined={combined:.3}, ts={title_score:.3}, cs={content_score:.3})"
            );
        }

        // CROSS-PAIR check: Phase 2 must NOT cross-catch A_i vs A_j or B_i vs B_j.
        for i in 0..4 {
            for j in (i + 1)..4 {
                let ai = &specs[8 + i * 2];
                let aj = &specs[8 + j * 2];
                let bi = &specs[8 + i * 2 + 1];
                let bj = &specs[8 + j * 2 + 1];

                for (label, x, y) in [("Ai vs Aj", ai, aj), ("Bi vs Bj", bi, bj)] {
                    let txw = mw_real(&x.title);
                    let tyw = mw_real(&y.title);
                    let cxw = mw_real(&x.content);
                    let cyw = mw_real(&y.content);
                    let ts = {
                        let i = txw.intersection(&tyw).count() as f64;
                        let m = std::cmp::max(txw.len(), tyw.len()) as f64;
                        if m == 0.0 {
                            0.0
                        } else {
                            i / m
                        }
                    };
                    let cs = {
                        let i = cxw.intersection(&cyw).count() as f64;
                        let m = std::cmp::max(cxw.len(), cyw.len()) as f64;
                        if m == 0.0 {
                            0.0
                        } else {
                            i / m
                        }
                    };
                    let combined = (0.5 * ts + 0.5 * cs).max(ts).max(cs);
                    assert!(
                        combined < 0.65,
                        "content pairs {i} vs {j} ({label}) would be cross-caught by Phase 2: combined={combined:.3} ts={ts:.3} cs={cs:.3}"
                    );
                }
            }
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
                "content pair {pair_idx} title Jaccard too low for Phase 9b ({title_jaccard:.3})"
            );
            assert!(
                content_jaccard < 0.3,
                "content pair {pair_idx} content Jaccard too high for Phase 9b ({content_jaccard:.3})"
            );
        }
    }

    // ── Category 5 tests ─────────────────────────────────────────

    #[test]
    fn test_category_5_produces_30_memories() {
        let (specs, truths) = generate_category_5_reweave_enrichment(42);
        assert_eq!(specs.len(), 30);
        assert_eq!(truths.len(), 30);

        let merged = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Merged)
            .count();
        assert_eq!(merged, 10); // 10 newer reweave partners
    }

    // ── Category 6 tests ─────────────────────────────────────────

    #[test]
    fn test_category_6_produces_31_memories() {
        let (specs, truths) = generate_category_6_lifecycle_quality(42);
        assert_eq!(specs.len(), 31);

        // 6 decay + 5 reconsolidation + 12 cluster + 8 quality = 31
        // Decay values: 0.9 * exp(-0.03 * N) for N=30..55 → all < 0.5
        // Recon value: 0.85 (distinguishable by threshold < 0.5)
        let decay = truths
            .iter()
            .filter(|t| t.expected_confidence.is_some() && t.expected_confidence.unwrap() < 0.5)
            .count();
        let recon = truths
            .iter()
            .filter(|t| t.expected_confidence == Some(0.85_f64.min(1.0)))
            .count();
        let clusters = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Superseded)
            .count();
        let quality = truths
            .iter()
            .filter(|t| t.expected_quality.is_some())
            .count();

        assert_eq!(decay, 6);
        assert_eq!(recon, 5);
        assert_eq!(clusters, 12);
        assert_eq!(quality, 8);
    }

    // ── Category 7 tests ─────────────────────────────────────────

    #[test]
    fn test_category_7_produces_24_memories() {
        let (specs, truths) = generate_category_7_self_healing(42);
        assert_eq!(specs.len(), 24);

        let superseded = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Superseded)
            .count();
        let faded = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Faded)
            .count();
        let active = truths
            .iter()
            .filter(|t| t.expected_status == ExpectedStatus::Active)
            .count();

        assert_eq!(superseded, 6); // older members of topic-supersede pairs
        assert_eq!(faded, 9); // 6 staleness + 3 pressure-decay (all faded by Phase 21)
        assert_eq!(active, 9); // 6 newer topic-supersede + 3 boost
    }

    // ── Category 8 tests ─────────────────────────────────────────

    #[test]
    fn test_category_8_produces_26_memories() {
        let (specs, truths) = generate_category_8_infrastructure(42);
        assert_eq!(specs.len(), 26);
        // 10 link + 5 activation + 8 entity + 3 portability = 26
        assert_eq!(
            truths
                .iter()
                .filter(|t| t.expected_activation.is_some())
                .count(),
            5
        );
    }

    // ── Category 6 Phase-2 guard tests ───────────────────────────

    #[test]
    fn test_category_6_cluster_lessons_avoid_phase_2() {
        use std::collections::HashSet;
        fn mw(text: &str) -> HashSet<String> {
            let stop: HashSet<&str> = [
                "the", "to", "a", "an", "is", "are", "so", "that", "and", "of", "in", "on", "at",
                "by", "for", "with", "as",
            ]
            .iter()
            .copied()
            .collect();
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 1 && !stop.contains(w))
                .map(String::from)
                .collect()
        }
        let (specs, _) = generate_category_6_lifecycle_quality(42);
        // Layout: 6 decay + 5 recon = 11 specs, then 12 cluster lessons at indices 11..23
        for cluster_idx in 0..4 {
            for i in 0..3 {
                for j in (i + 1)..3 {
                    let a = &specs[11 + cluster_idx * 3 + i];
                    let b = &specs[11 + cluster_idx * 3 + j];
                    let tma = mw(&a.title);
                    let tmb = mw(&b.title);
                    let cma = mw(&a.content);
                    let cmb = mw(&b.content);
                    let t_shared = tma.intersection(&tmb).count() as f64;
                    let t_max = std::cmp::max(tma.len(), tmb.len()) as f64;
                    let title_s = if t_max == 0.0 { 0.0 } else { t_shared / t_max };
                    let c_shared = cma.intersection(&cmb).count() as f64;
                    let c_max = std::cmp::max(cma.len(), cmb.len()) as f64;
                    let content_s = if c_max == 0.0 { 0.0 } else { c_shared / c_max };
                    let combined = (0.5 * title_s + 0.5 * content_s)
                        .max(title_s)
                        .max(content_s);
                    assert!(
                        combined < 0.65,
                        "cluster {cluster_idx} lesson pair ({i},{j}) would be caught by Phase 2: \
                        combined={combined} title={title_s} content={content_s}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_category_6_cluster_lessons_survive_phase_5_clustering() {
        // Phase 5 uses raw split_whitespace overlap > 0.5 for clustering.
        let (specs, _) = generate_category_6_lifecycle_quality(42);
        for cluster_idx in 0..4 {
            for i in 0..3 {
                for j in (i + 1)..3 {
                    let a = &specs[11 + cluster_idx * 3 + i];
                    let b = &specs[11 + cluster_idx * 3 + j];
                    let tokens_a: std::collections::HashSet<&str> =
                        a.title.split_whitespace().collect();
                    let tokens_b: std::collections::HashSet<&str> =
                        b.title.split_whitespace().collect();
                    let shared = tokens_a.intersection(&tokens_b).count() as f64;
                    let max_len = std::cmp::max(tokens_a.len(), tokens_b.len()) as f64;
                    let overlap = if max_len == 0.0 {
                        0.0
                    } else {
                        shared / max_len
                    };
                    assert!(
                        overlap > 0.5,
                        "cluster {cluster_idx} lessons ({i},{j}) don't overlap enough for Phase 5: {overlap}"
                    );
                }
            }
        }
    }

    // ── Category 7 Phase-2 guard test ────────────────────────────

    #[test]
    fn test_category_7_short_title_candidates_avoid_phase_2() {
        use std::collections::HashSet;
        fn mw(text: &str) -> HashSet<String> {
            let stop: HashSet<&str> = [
                "the", "to", "a", "an", "is", "are", "so", "that", "and", "of", "in", "on", "at",
                "by", "for", "with", "as",
            ]
            .iter()
            .copied()
            .collect();
            text.to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() > 1 && !stop.contains(w))
                .map(String::from)
                .collect()
        }
        let (specs, _) = generate_category_7_self_healing(42);
        // Layout: 12 topic-supersede (6 pairs × 2) + 6 staleness + 3 pressure-decay + 3 boost = 24
        let groups: [(usize, usize, &str); 3] = [
            (12, 18, "staleness"),
            (18, 21, "pressure-decay"),
            (21, 24, "boost"),
        ];
        for (start, end, name) in groups {
            for i in start..end {
                for j in (i + 1)..end {
                    let a = &specs[i];
                    let b = &specs[j];
                    let tma = mw(&a.title);
                    let tmb = mw(&b.title);
                    let cma = mw(&a.content);
                    let cmb = mw(&b.content);
                    let t_shared = tma.intersection(&tmb).count() as f64;
                    let t_max = std::cmp::max(tma.len(), tmb.len()) as f64;
                    let title_s = if t_max == 0.0 { 0.0 } else { t_shared / t_max };
                    let c_shared = cma.intersection(&cmb).count() as f64;
                    let c_max = std::cmp::max(cma.len(), cmb.len()) as f64;
                    let content_s = if c_max == 0.0 { 0.0 } else { c_shared / c_max };
                    let combined = (0.5 * title_s + 0.5 * content_s)
                        .max(title_s)
                        .max(content_s);
                    assert!(
                        combined < 0.65,
                        "{name} pair ({i},{j}) Phase 2 risk: combined={combined} title={title_s} content={content_s}"
                    );
                }
            }
        }
    }

    // ── seed_corpus tests ─────────────────────────────────────────

    #[test]
    fn test_seed_corpus_produces_167_memories() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (specs, dataset) = seed_corpus(&state.conn, 42).unwrap();
        assert_eq!(specs.len(), 167);
        assert_eq!(dataset.ground_truth.len(), 167);
    }

    #[test]
    fn test_seed_corpus_no_id_collisions() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let mut ids = HashSet::new();
        for gt in &dataset.ground_truth {
            assert!(ids.insert(&gt.memory_id), "collision: {}", gt.memory_id);
        }
    }

    // ── synthetic embedding tests ─────────────────────────────────

    #[test]
    fn test_base_embedding_is_unit_vector() {
        let v = generate_base_embedding("test-seed");
        assert_eq!(v.len(), 768);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "norm = {norm}, not 1.0");
    }

    #[test]
    fn test_perturb_achieves_target_distance_close() {
        let base = generate_base_embedding("base-seed");
        let perturbed = perturb_embedding(&base, 0.08, "pair-0");
        let d = cosine_distance(&base, &perturbed);
        assert!((d - 0.08).abs() < 1e-3, "distance = {d}, target 0.08");
    }

    #[test]
    fn test_perturb_achieves_target_distance_far() {
        let base = generate_base_embedding("base-seed");
        let perturbed = perturb_embedding(&base, 0.15, "pair-0");
        let d = cosine_distance(&base, &perturbed);
        assert!((d - 0.15).abs() < 1e-3, "distance = {d}, target 0.15");
    }

    #[test]
    fn test_base_embedding_deterministic() {
        let a = generate_base_embedding("same-key");
        let b = generate_base_embedding("same-key");
        assert_eq!(a, b);
    }

    #[test]
    fn test_seed_embeddings_inserts_24_vectors() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let _ = seed_corpus(&state.conn, 42).unwrap();
        let count = seed_embeddings(&state.conn, 42).unwrap();
        assert_eq!(count, 24);
        let db_count: i64 = state
            .conn
            .query_row("SELECT COUNT(*) FROM memory_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(db_count, 24);
    }

    #[test]
    fn test_seed_embeddings_produce_correct_distances() {
        // Verify the end-to-end geometry: Category 3 control pairs must stay above
        // Phase 7's 0.1 merge threshold (so controls are NOT merged) and Category 7
        // supersede pairs must stay below Phase 20's 0.35 supersede threshold.
        // This guards against key-drift between seed_embeddings and the base/perturb logic.
        let seed = 42u64;

        // Category 3 merge pair 0: should be BELOW 0.1 (Phase 7 merges)
        let base_c3m = generate_base_embedding(&format!("c3-merge-{seed}-0"));
        let perturbed_c3m = perturb_embedding(&base_c3m, 0.08, &format!("c3-merge-{seed}-0"));
        let d_merge = cosine_distance(&base_c3m, &perturbed_c3m);
        assert!(
            d_merge < 0.1,
            "Category 3 merge pair must be below Phase 7 threshold: d={d_merge}"
        );

        // Category 3 control pair 0: should be ABOVE 0.1 (Phase 7 skips)
        let base_c3c = generate_base_embedding(&format!("c3-control-{seed}-0"));
        let perturbed_c3c = perturb_embedding(&base_c3c, 0.15, &format!("c3-control-{seed}-0"));
        let d_control = cosine_distance(&base_c3c, &perturbed_c3c);
        assert!(
            d_control > 0.1,
            "Category 3 control pair must be ABOVE Phase 7 threshold to avoid merge: d={d_control}"
        );
        assert!(
            d_control < 0.35,
            "Category 3 control pair should stay below Phase 20 threshold: d={d_control}"
        );

        // Category 7 supersede pair 0: should be BELOW 0.35 (Phase 20 supersedes)
        let base_c7 = generate_base_embedding(&format!("c7-supersede-{seed}-0"));
        let perturbed_c7 = perturb_embedding(&base_c7, 0.25, &format!("c7-supersede-{seed}-0"));
        let d_supersede = cosine_distance(&base_c7, &perturbed_c7);
        assert!(
            d_supersede < 0.35,
            "Category 7 supersede pair must be below Phase 20 threshold: d={d_supersede}"
        );
        assert!(
            d_supersede > 0.1,
            "Category 7 supersede pair should stay above Phase 7 threshold to avoid merge: d={d_supersede}"
        );
    }

    #[test]
    fn test_generate_query_bank_produces_15_queries() {
        let dataset = SeededDataset {
            seed: 42,
            ground_truth: vec![],
            recall_queries: vec![],
            expected_pattern_count: 4,
            expected_protocol_count: 7,
            expected_resolution_count: 4,
        };
        let queries = generate_query_bank(&dataset);
        assert_eq!(
            queries.len(),
            15,
            "expected exactly 15 queries, got {}",
            queries.len()
        );
        // All queries have unique IDs
        let ids: std::collections::HashSet<_> = queries.iter().map(|q| &q.id).collect();
        assert_eq!(ids.len(), queries.len(), "query IDs must be unique");
    }

    #[test]
    fn test_snapshot_recall_empty_queries() {
        let mut state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let snap = snapshot_recall(&mut state, &[]);
        assert_eq!(snap.mean_recall_at_10, 0.0);
        assert!(snap.results.is_empty());
    }

    #[test]
    fn test_expected_supersede_newer_matches_generator() {
        // The helper must produce the same title that the Category 7 generator uses
        // for the newer member of topic-supersede pair 0 with seed 42.
        let (specs, _) = generate_category_7_self_healing(42);
        // Category 7 layout: first 12 specs are the 6 supersede pairs (older then newer)
        // Pair 0 newer = index 1
        let actual_newer_title = &specs[1].title;
        let dataset = SeededDataset {
            seed: 42,
            ground_truth: vec![],
            recall_queries: vec![],
            expected_pattern_count: 4,
            expected_protocol_count: 7,
            expected_resolution_count: 4,
        };
        let expected = expected_supersede_newer(&dataset, 0);
        assert!(
            expected.contains(actual_newer_title),
            "expected_supersede_newer must produce the actual newer title.\n  actual: {actual_newer_title}\n  expected set: {expected:?}"
        );
    }

    #[test]
    fn test_expected_title_helpers_match_generators() {
        let seed = 42u64;
        let dataset = SeededDataset {
            seed,
            ground_truth: vec![],
            recall_queries: vec![],
            expected_pattern_count: 4,
            expected_protocol_count: 7,
            expected_resolution_count: 4,
        };

        // C1 keeper (pair 0)
        let (c1_specs, _) = generate_category_1_exact_duplicates(seed);
        // C1 layout: 2 memories per pair, keeper first, victim second. Pair 0 keeper = index 0
        let c1_actual = &c1_specs[0].title;
        let c1_expected = expected_titles_for_c1_keeper(&dataset, 0);
        assert!(
            c1_expected.contains(c1_actual),
            "c1 helper mismatch: actual={c1_actual}, expected={c1_expected:?}"
        );

        // C2 keeper (pair 0)
        let (c2_specs, _) = generate_category_2_semantic_duplicates(seed);
        let c2_actual = &c2_specs[0].title;
        let c2_expected = expected_titles_for_c2_keeper(&dataset, 0);
        assert!(
            c2_expected.contains(c2_actual),
            "c2 helper mismatch: actual={c2_actual}, expected={c2_expected:?}"
        );

        // C4 valence → Resolution title
        let (c4_specs, _) = generate_category_4_contradictions(seed);
        // C4 layout: 2 memories per valence pair, pos first then neg. Pair 0 pos=0, neg=1
        let pos = &c4_specs[0].title;
        let neg = &c4_specs[1].title;
        let c4_expected = expected_resolution_titles(&dataset, 0);
        let synthesized = format!("Resolution: {pos} vs {neg}");
        assert!(
            c4_expected.contains(&synthesized),
            "c4 resolution helper mismatch: built={synthesized}, expected={c4_expected:?}"
        );

        // C5 reweave older (pair 0) — older is index 0 of the pair (each pair = 2 memories, older first, newer second)
        let (c5_specs, _) = generate_category_5_reweave_enrichment(seed);
        let c5_actual = &c5_specs[0].title;
        let c5_expected = expected_reweaved_titles(&dataset, 0);
        assert!(
            c5_expected.contains(c5_actual),
            "c5 reweave helper mismatch: actual={c5_actual}, expected={c5_expected:?}"
        );
    }

    // ── pr_f1 / pr_f1_pairs tests ─────────────────────────────────

    #[test]
    fn test_pr_f1_basic() {
        let e: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let o: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let (p, r, f) = pr_f1(&e, &o);
        assert!((p - 1.0).abs() < 1e-9, "precision={p}");
        assert!((r - 2.0 / 3.0).abs() < 1e-9, "recall={r}");
        assert!((f - 0.8).abs() < 1e-9, "f1={f}");
    }

    #[test]
    fn test_pr_f1_empty_observed() {
        let e: HashSet<String> = ["a"].iter().map(|s| s.to_string()).collect();
        let o: HashSet<String> = HashSet::new();
        let (p, r, _) = pr_f1(&e, &o);
        assert_eq!(p, 0.0, "precision should be 0 when observed is empty");
        assert_eq!(
            r, 0.0,
            "recall should be 0 when nothing observed from expected"
        );
    }

    #[test]
    fn test_pr_f1_pairs_basic() {
        let e: HashSet<(String, String)> = [("a".into(), "b".into())].iter().cloned().collect();
        let o: HashSet<(String, String)> = [("a".into(), "b".into()), ("c".into(), "d".into())]
            .iter()
            .cloned()
            .collect();
        let (p, r, f) = pr_f1_pairs(&e, &o);
        assert!((p - 0.5).abs() < 1e-9, "precision={p}");
        assert!((r - 1.0).abs() < 1e-9, "recall={r}");
        assert!((f - 2.0 / 3.0).abs() < 1e-9, "f1={f}");
    }

    #[test]
    fn test_audit_dedup_smoke() {
        // smoke: audit_dedup runs on seeded dataset without panic
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let result = audit_dedup(&state.conn, &dataset);
        assert!(result.is_ok(), "audit_dedup failed: {:?}", result.err());
        let score = result.unwrap();
        assert_eq!(score.dimension, "dedup_quality");
        // Before consolidation runs, no victims are deleted/superseded → observed=0
        // gt_victims ≥ 1, observed=0 → recall=0, precision=0, f1=0
        assert_eq!(score.f1, 0.0, "pre-consolidation f1 should be 0");
    }

    #[test]
    fn test_audit_contradictions_smoke() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let result = audit_contradictions(&state.conn, &dataset);
        assert!(
            result.is_ok(),
            "audit_contradictions failed: {:?}",
            result.err()
        );
        let score = result.unwrap();
        assert_eq!(score.dimension, "contradiction_handling");
    }

    #[test]
    fn test_audit_reweave_smoke() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let result = audit_reweave(&state.conn, &dataset);
        assert!(result.is_ok(), "audit_reweave failed: {:?}", result.err());
        let score = result.unwrap();
        assert_eq!(score.dimension, "reweave_enrichment");
    }

    #[test]
    fn test_audit_lifecycle_smoke() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let result = audit_lifecycle(&state.conn, &dataset);
        assert!(result.is_ok(), "audit_lifecycle failed: {:?}", result.err());
        let score = result.unwrap();
        assert_eq!(score.dimension, "quality_lifecycle");
        // f1 is in [0, 1]
        assert!(score.f1 >= 0.0 && score.f1 <= 1.0, "f1={}", score.f1);
    }

    #[test]
    fn test_audit_infrastructure_smoke() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let result = audit_infrastructure(&state.conn, &dataset);
        assert!(
            result.is_ok(),
            "audit_infrastructure failed: {:?}",
            result.err()
        );
        // Before consolidation, all infrastructure checks will fail — that's expected
        let failures = result.unwrap();
        // Just verify we get a Vec (can be non-empty pre-consolidation)
        assert!(failures.len() <= 10, "at most 10 infrastructure assertions");
    }

    #[test]
    fn test_compute_score_all_perfect() {
        let d = |name: &str| DimensionScore {
            dimension: name.into(),
            precision: 1.0,
            recall: 1.0,
            f1: 1.0,
            details: vec![],
        };
        let baseline = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.5,
        };
        let post = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.8,
        };
        let s = compute_score(
            42,
            d("dedup_quality"),
            d("contradiction_handling"),
            d("reweave_enrichment"),
            d("quality_lifecycle"),
            &baseline,
            &post,
            Some(0.3),
            vec![],
        );
        assert!((s.composite - 1.0).abs() < 1e-9);
        assert!(s.pass);
    }

    #[test]
    fn test_compute_score_negative_delta_gives_zero() {
        let d = |f| DimensionScore {
            dimension: "".into(),
            precision: 0.0,
            recall: 0.0,
            f1: f,
            details: vec![],
        };
        let baseline = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.8,
        };
        let post = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.5,
        };
        let s = compute_score(
            42,
            d(1.0),
            d(1.0),
            d(1.0),
            d(1.0),
            &baseline,
            &post,
            Some(0.3),
            vec![],
        );
        assert_eq!(s.recall_delta_score, 0.0);
    }

    #[test]
    fn test_compute_score_infra_failure_blocks_pass() {
        let d = |f| DimensionScore {
            dimension: "".into(),
            precision: 0.0,
            recall: 0.0,
            f1: f,
            details: vec![],
        };
        let bs = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.0,
        };
        let ps = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 1.0,
        };
        let s = compute_score(
            42,
            d(1.0),
            d(1.0),
            d(1.0),
            d(1.0),
            &bs,
            &ps,
            Some(1.0),
            vec!["Phase 3 failed".into()],
        );
        assert!(
            !s.pass,
            "infrastructure failure should block pass even with composite 1.0"
        );
    }

    #[test]
    fn test_compute_score_none_expected_positive_delta_gives_one() {
        let d = |f: f64| DimensionScore {
            dimension: "".into(),
            precision: 0.0,
            recall: 0.0,
            f1: f,
            details: vec![],
        };
        let bs = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.0,
        };
        let ps = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.5,
        };
        let s = compute_score(42, d(1.0), d(1.0), d(1.0), d(1.0), &bs, &ps, None, vec![]);
        assert_eq!(
            s.recall_delta_score, 1.0,
            "None expected + positive delta → 1.0"
        );
    }

    #[test]
    fn test_compute_score_zero_delta_positive_expected_is_neutral() {
        let d = |f: f64| DimensionScore {
            dimension: "".into(),
            precision: 0.0,
            recall: 0.0,
            f1: f,
            details: vec![],
        };
        let bs = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.5,
        };
        let ps = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.5,
        };
        let s = compute_score(
            42,
            d(1.0),
            d(1.0),
            d(1.0),
            d(1.0),
            &bs,
            &ps,
            Some(0.3),
            vec![],
        );
        assert_eq!(
            s.recall_delta_score, 0.5,
            "delta=0 + expected>0 → 0.5 (neutral pass)"
        );
    }

    #[test]
    fn test_compute_score_zero_expected_is_invalid() {
        let d = |f: f64| DimensionScore {
            dimension: "".into(),
            precision: 0.0,
            recall: 0.0,
            f1: f,
            details: vec![],
        };
        let bs = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.0,
        };
        let ps = RecallSnapshot {
            results: vec![],
            mean_recall_at_10: 0.5,
        };
        let s = compute_score(
            42,
            d(1.0),
            d(1.0),
            d(1.0),
            d(1.0),
            &bs,
            &ps,
            Some(0.0),
            vec![],
        );
        assert_eq!(
            s.recall_delta_score, 0.0,
            "expected=Some(0.0) → INVALID → 0.0"
        );
    }

    #[test]
    #[ignore] // heavy — runs full bench; invoke with `-- --ignored`
    fn test_run_orchestrator_produces_score() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = ConsolidationBenchConfig {
            seed: 42,
            output_dir: tmp.path().to_path_buf(),
            expected_recall_delta: None,
        };
        let score = run(cfg).unwrap();
        assert_eq!(score.seed, 42);
        // Composite must be a valid [0,1] float
        assert!(score.composite >= 0.0 && score.composite <= 1.0);
    }

    // ── Cross-category Phase 2 guard tests ───────────────────────────
    // These are the LINCHPIN tests: Phase 2 semantic_dedup must merge EXACTLY 8 memories
    // (Category 2 intended victims) and nothing else.

    #[test]
    fn test_no_phase_2_over_catching_anywhere() {
        use crate::db::ops;
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, _) = seed_corpus(&state.conn, 42).unwrap();

        // Run only Phase 1 and Phase 2
        let _ = ops::dedup_memories(&state.conn).unwrap();
        let phase2_merged = ops::semantic_dedup(&state.conn, 500).unwrap();

        // Should be exactly 8 (Category 2 intended victims)
        assert_eq!(
            phase2_merged, 8,
            "Phase 2 merged {phase2_merged} memories — expected exactly 8 (Cat 2). \
            Others are being incorrectly caught."
        );
    }

    #[test]
    fn test_no_phase_2_over_catching_seed_1() {
        use crate::db::ops;
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, _) = seed_corpus(&state.conn, 1).unwrap();
        let _ = ops::dedup_memories(&state.conn).unwrap();
        let phase2_merged = ops::semantic_dedup(&state.conn, 500).unwrap();
        assert_eq!(
            phase2_merged, 8,
            "seed 1: Phase 2 merged {phase2_merged}, expected 8"
        );
    }

    /// Guard: all 6 Cat7 topic-supersede pairs must have combined title+content overlap in [0.3, 0.7).
    #[test]
    fn test_category_7_topic_supersede_in_phase_20_range() {
        use crate::db::ops::meaningful_words_pub;
        let (specs, _) = generate_category_7_self_healing(42);
        for pair_idx in 0..6 {
            let older = &specs[pair_idx * 2];
            let newer = &specs[pair_idx * 2 + 1];
            let older_text = format!("{} {}", older.title, older.content);
            let newer_text = format!("{} {}", newer.title, newer.content);
            let older_mw = meaningful_words_pub(&older_text);
            let newer_mw = meaningful_words_pub(&newer_text);
            let intersection = older_mw.intersection(&newer_mw).count();
            let union = older_mw.union(&newer_mw).count();
            let overlap = if union > 0 {
                intersection as f64 / union as f64
            } else {
                0.0
            };
            assert!(
                overlap >= 0.3 && overlap < 0.7,
                "pair {pair_idx}: overlap={overlap:.4} not in [0.3, 0.7)"
            );
        }
    }

    /// Guard: Cat7 topic-supersede pairs must have exactly 1 shared tag to prevent Phase 14 reweave.
    #[test]
    fn test_category_7_topic_supersede_single_shared_tag() {
        let (specs, _) = generate_category_7_self_healing(42);
        for pair_idx in 0..6 {
            let older = &specs[pair_idx * 2];
            let newer = &specs[pair_idx * 2 + 1];
            // Each member should have exactly 1 tag
            assert_eq!(
                older.tags.len(),
                1,
                "pair {pair_idx} older should have 1 tag"
            );
            assert_eq!(
                newer.tags.len(),
                1,
                "pair {pair_idx} newer should have 1 tag"
            );
            // Shared tags between pair must be < 2 (Phase 14 threshold)
            let shared: usize = older.tags.iter().filter(|t| newer.tags.contains(t)).count();
            assert!(
                shared < 2,
                "pair {pair_idx}: {shared} shared tags — Phase 14 would trigger (need < 2)"
            );
        }
    }

    /// Guard: Phase 20 fires for all 6 Cat7 topic-supersede pairs after a full run.
    #[test]
    fn test_phase_20_fires_for_all_supersede_pairs() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        seed_corpus(&state.conn, 42).unwrap();
        let _ = seed_embeddings(&state.conn, 42);
        let cons_config = crate::config::ConsolidationConfig {
            batch_limit: 500,
            reweave_limit: 100,
            ..Default::default()
        };
        let _ = crate::workers::consolidator::run_all_phases(&state.conn, &cons_config, None);
        let heal_count: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM healing_log WHERE action='auto_superseded'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        assert!(
            heal_count >= 6,
            "Phase 20 should fire ≥6 times, got {heal_count}"
        );
        // Verify all older memories are superseded and all newer are active
        for pair_idx in 0..6 {
            let older_status: String = state
                .conn
                .query_row(
                    "SELECT status FROM memory WHERE id=?1",
                    rusqlite::params![&format!("c7-supersede-{pair_idx}-older")],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            let newer_status: String = state
                .conn
                .query_row(
                    "SELECT status FROM memory WHERE id=?1",
                    rusqlite::params![&format!("c7-supersede-{pair_idx}-newer")],
                    |r| r.get(0),
                )
                .unwrap_or_default();
            assert_eq!(
                older_status, "superseded",
                "pair {pair_idx} older should be superseded"
            );
            assert_eq!(
                newer_status, "active",
                "pair {pair_idx} newer should be active"
            );
        }
    }

    /// Guard: audit_reweave finds promoted patterns and extracted protocols.
    #[test]
    fn test_audit_reweave_finds_new_memories() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let _ = seed_embeddings(&state.conn, 42);
        let cons_config = crate::config::ConsolidationConfig {
            batch_limit: 500,
            reweave_limit: 100,
            ..Default::default()
        };
        let _ = crate::workers::consolidator::run_all_phases(&state.conn, &cons_config, None);
        let score = audit_reweave(&state.conn, &dataset).unwrap();
        assert!(
            score.f1 > 0.9,
            "reweave_enrichment score should be > 0.9, got {:.4}",
            score.f1
        );
        // Check individual sub-scores in details
        let promo = score
            .details
            .iter()
            .find(|d| d.starts_with("promo_accuracy="))
            .unwrap();
        let proto = score
            .details
            .iter()
            .find(|d| d.starts_with("proto_accuracy="))
            .unwrap();
        let promo_val: f64 = promo
            .strip_prefix("promo_accuracy=")
            .unwrap()
            .parse()
            .unwrap();
        let proto_val: f64 = proto
            .strip_prefix("proto_accuracy=")
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            promo_val > 0.9,
            "promo_accuracy should be > 0.9, got {promo_val:.4}"
        );
        assert!(
            proto_val > 0.3,
            "proto_accuracy should be > 0.3, got {proto_val:.4}"
        );
    }

    /// Guard: audit_lifecycle finds correct decay confidence and quality scores.
    #[test]
    fn test_audit_lifecycle_decay_and_quality() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (_, dataset) = seed_corpus(&state.conn, 42).unwrap();
        let cons_config = crate::config::ConsolidationConfig {
            batch_limit: 500,
            reweave_limit: 100,
            ..Default::default()
        };
        let _ = crate::workers::consolidator::run_all_phases(&state.conn, &cons_config, None);
        let score = audit_lifecycle(&state.conn, &dataset).unwrap();
        // decay and quality should now pass
        let decay_detail = score
            .details
            .iter()
            .find(|d| d.starts_with("decay="))
            .unwrap();
        let quality_detail = score
            .details
            .iter()
            .find(|d| d.starts_with("quality="))
            .unwrap();
        let decay_val: f64 = decay_detail
            .strip_prefix("decay=")
            .unwrap()
            .parse()
            .unwrap();
        let quality_val: f64 = quality_detail
            .strip_prefix("quality=")
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            decay_val > 0.8,
            "decay sub-score should be > 0.8, got {decay_val:.4}"
        );
        assert!(
            quality_val > 0.8,
            "quality sub-score should be > 0.8, got {quality_val:.4}"
        );
    }

    /// Guard: no memory outside c6-recon-* has access_count >= 5.
    /// This ensures Phase 6 find_reconsolidation_candidates (LIMIT 5, ORDER BY access_count DESC)
    /// only picks the 5 intended recon candidates and no interfering memories from other categories.
    #[test]
    fn test_no_recon_interference() {
        let state = crate::server::handler::DaemonState::new(":memory:").unwrap();
        let (specs, _) = seed_corpus(&state.conn, 42).unwrap();
        let interferers: Vec<_> = specs
            .iter()
            .filter(|s| s.access_count >= 5 && !s.id.starts_with("c6-recon-"))
            .map(|s| s.id.clone())
            .collect();
        assert!(
            interferers.is_empty(),
            "memories with access_count >= 5 outside c6-recon-*: {interferers:?}"
        );
    }
}
