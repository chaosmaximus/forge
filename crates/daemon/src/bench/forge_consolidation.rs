//! Forge-Consolidation benchmark harness.
//!
//! Tests the daemon's 22-phase consolidation loop across 5 scored dimensions
//! plus infrastructure pass/fail assertions. See
//! `docs/benchmarks/forge-consolidation-design.md` for full design.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ── Configuration ────────────────────────────────────────────────

/// Configuration for a single Forge-Consolidation run.
#[derive(Debug, Clone)]
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

/// Expected memory status after consolidation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExpectedStatus {
    Active,     // memory should remain active
    Superseded, // marked superseded by Phases 1, 2, 5, 7, 12, 20
    Faded,      // marked faded by Phase 4 or Phase 21
    Merged,     // marked merged by Phase 14 (reweave)
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
        let mut set = std::collections::HashSet::new();
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
}
