// bench/scoring.rs — retrieval metrics for benchmark runs.
//
// Implementations mirror the reference Python code in MemPalace's
// `longmemeval_bench.py` (lines 53–80) so that any apples-to-apples
// comparison stays bit-identical at the metric level. The unit tests in this
// module pin the formulas against hand-computed expected values.

use std::collections::HashSet;

/// Recall@K — was at least one ground-truth ID found in the top-K results?
///
/// This is the "any-recall" variant used by every published memory benchmark
/// leaderboard. Returns 1.0 if any ground-truth ID is present in the first
/// `k` retrieved IDs, else 0.0. The numeric (not boolean) return type lets
/// callers average across questions for an aggregate score.
pub fn recall_any_at_k(retrieved: &[String], correct: &[String], k: usize) -> f64 {
    if correct.is_empty() {
        return 1.0;
    }
    let top_k = &retrieved[..retrieved.len().min(k)];
    let correct_set: HashSet<&str> = correct.iter().map(String::as_str).collect();
    if top_k.iter().any(|r| correct_set.contains(r.as_str())) {
        1.0
    } else {
        0.0
    }
}

/// Recall@K — strict: were ALL ground-truth IDs found in the top-K results?
///
/// Equivalent to MemPalace's `recall_all`. Useful when a question has multiple
/// answer sessions and you want to score whether the system retrieved every
/// piece of evidence, not just one.
pub fn recall_all_at_k(retrieved: &[String], correct: &[String], k: usize) -> f64 {
    if correct.is_empty() {
        return 1.0;
    }
    let top_k_set: HashSet<&str> = retrieved.iter().take(k).map(String::as_str).collect();
    if correct.iter().all(|c| top_k_set.contains(c.as_str())) {
        1.0
    } else {
        0.0
    }
}

/// NDCG@K — Normalized Discounted Cumulative Gain.
///
/// Standard formulation: `dcg = Σ rel_i / log2(i + 2)` where `rel_i` is 1 if
/// the i-th retrieved item is a ground-truth ID, else 0. `idcg` is the same
/// formula applied to the ideal (sorted-descending) ranking. Returns
/// `dcg / idcg`, or 0.0 if `idcg == 0`.
pub fn ndcg_at_k(retrieved: &[String], correct: &[String], k: usize) -> f64 {
    let correct_set: HashSet<&str> = correct.iter().map(String::as_str).collect();
    let rels: Vec<f64> = retrieved
        .iter()
        .take(k)
        .map(|r| {
            if correct_set.contains(r.as_str()) {
                1.0
            } else {
                0.0
            }
        })
        .collect();

    let dcg = dcg_from_rels(&rels);
    let mut ideal = rels.clone();
    ideal.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let idcg = dcg_from_rels(&ideal);

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

fn dcg_from_rels(rels: &[f64]) -> f64 {
    rels.iter()
        .enumerate()
        .map(|(i, r)| r / ((i + 2) as f64).log2())
        .sum()
}

/// Weighted-mean composite score for an N-dimensional bench evaluation.
/// Lifted from `bench/forge_identity.rs:1632` per 2A-5 spec §2 fact 12 / T2.2
/// so multi-bench harnesses (forge-identity, forge-isolation, future) share
/// one generalized scorer. Each bench supplies its own weight vector.
///
/// Returns `Σ weights[i] * scores[i]`. With weights summing to 1.0 this
/// produces a value in `[min(scores), max(scores)]`. Caller MUST pre-validate
/// score values are in `[0, 1]` if a bounded composite is required.
///
/// Debug-asserts `scores.len() == weights.len()` and `|sum(weights) − 1.0| < 1e-9`.
/// Both checks are zero-cost in release builds; production callers (none
/// today — bench-only) get release behavior.
pub fn composite_score(scores: &[f64], weights: &[f64]) -> f64 {
    debug_assert_eq!(
        scores.len(),
        weights.len(),
        "composite_score: scores ({}) vs weights ({}) length mismatch",
        scores.len(),
        weights.len(),
    );
    let weight_sum: f64 = weights.iter().sum();
    debug_assert!(
        (weight_sum - 1.0).abs() < 1e-9,
        "composite_score: weights must sum to 1.0 (got {weight_sum})",
    );
    scores.iter().zip(weights.iter()).map(|(s, w)| s * w).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn recall_any_perfect_top_1() {
        let retrieved = s(&["a", "b", "c"]);
        let correct = s(&["a"]);
        assert_eq!(recall_any_at_k(&retrieved, &correct, 5), 1.0);
    }

    #[test]
    fn recall_any_miss_returns_zero() {
        let retrieved = s(&["x", "y", "z"]);
        let correct = s(&["a"]);
        assert_eq!(recall_any_at_k(&retrieved, &correct, 5), 0.0);
    }

    #[test]
    fn recall_any_at_k_truncates() {
        let retrieved = s(&["x", "y", "z", "a"]);
        let correct = s(&["a"]);
        // a is at rank 4 — present at k=5, absent at k=3.
        assert_eq!(recall_any_at_k(&retrieved, &correct, 5), 1.0);
        assert_eq!(recall_any_at_k(&retrieved, &correct, 3), 0.0);
    }

    #[test]
    fn recall_any_empty_correct_is_one() {
        let retrieved = s(&["x", "y"]);
        let correct: Vec<String> = vec![];
        assert_eq!(recall_any_at_k(&retrieved, &correct, 5), 1.0);
    }

    #[test]
    fn recall_all_requires_every_correct_id() {
        let retrieved = s(&["a", "b", "c"]);
        let correct = s(&["a", "b"]);
        assert_eq!(recall_all_at_k(&retrieved, &correct, 5), 1.0);

        let correct_partial = s(&["a", "z"]);
        assert_eq!(recall_all_at_k(&retrieved, &correct_partial, 5), 0.0);
    }

    #[test]
    fn ndcg_perfect_ranking() {
        // One correct ID at rank 1 → dcg = 1 / log2(2) = 1.0; ideal = 1.0.
        let retrieved = s(&["a", "x", "y"]);
        let correct = s(&["a"]);
        let ndcg = ndcg_at_k(&retrieved, &correct, 5);
        assert!((ndcg - 1.0).abs() < 1e-6);
    }

    #[test]
    fn ndcg_decays_with_rank() {
        // Same single correct ID at rank 3 → dcg = 1 / log2(4) = 0.5;
        // ideal still places it at rank 1 → idcg = 1.0.
        let retrieved = s(&["x", "y", "a"]);
        let correct = s(&["a"]);
        let ndcg = ndcg_at_k(&retrieved, &correct, 5);
        assert!((ndcg - 0.5).abs() < 1e-6, "got {ndcg}");
    }

    #[test]
    fn ndcg_no_match_is_zero() {
        let retrieved = s(&["x", "y", "z"]);
        let correct = s(&["a"]);
        assert_eq!(ndcg_at_k(&retrieved, &correct, 5), 0.0);
    }

    #[test]
    fn ndcg_two_correct_at_ranks_1_and_3() {
        // dcg = 1/log2(2) + 0/log2(3) + 1/log2(4) = 1.0 + 0.0 + 0.5 = 1.5
        // idcg = 1/log2(2) + 1/log2(3) = 1.0 + 0.6309... = 1.6309...
        let retrieved = s(&["a", "x", "b"]);
        let correct = s(&["a", "b"]);
        let ndcg = ndcg_at_k(&retrieved, &correct, 5);
        let expected = 1.5 / (1.0 + 1.0 / 3.0_f64.log2());
        assert!(
            (ndcg - expected).abs() < 1e-6,
            "got {ndcg}, expected {expected}"
        );
    }

    // ── composite_score (lifted from forge_identity per 2A-5 T2.2) ─────

    #[test]
    fn composite_score_uniform_weights() {
        let scores = [0.5, 0.5, 0.5, 0.5];
        let weights = [0.25, 0.25, 0.25, 0.25];
        assert!((composite_score(&scores, &weights) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn composite_score_forge_identity_six_dim_weights() {
        // Replays the forge-identity DIM_WEIGHTS [0.15×5, 0.25] config with
        // perfect 1.0 scores. Result must be exactly 1.0.
        let scores = [1.0; 6];
        let weights = [0.15, 0.15, 0.15, 0.15, 0.15, 0.25];
        assert!((composite_score(&scores, &weights) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn composite_score_isolation_six_dim_weights() {
        // 2A-5 spec §3.3 weights: D1 0.25, D2 0.15, D3 0.10, D4 0.10,
        // D5 0.15, D6 0.25. With all 1.0 scores, composite must be 1.0.
        let scores = [1.0; 6];
        let weights = [0.25, 0.15, 0.10, 0.10, 0.15, 0.25];
        assert!((composite_score(&scores, &weights) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn composite_score_d1_below_min_with_others_perfect() {
        // 2A-5 calibration scenario: D1 = 0.84 (below 0.95 min) others
        // perfect. Composite would be 0.96 — but the per-dim min gate
        // catches this case. The math here proves composite alone could
        // mislead; per-dim min is load-bearing.
        let scores = [0.84, 1.0, 1.0, 1.0, 1.0, 1.0];
        let weights = [0.25, 0.15, 0.10, 0.10, 0.15, 0.25];
        let composite = composite_score(&scores, &weights);
        // 0.25 * 0.84 + 0.75 * 1.0 = 0.21 + 0.75 = 0.96
        assert!((composite - 0.96).abs() < 1e-9, "got {composite}");
    }

    #[test]
    #[should_panic(expected = "length mismatch")]
    fn composite_score_panics_on_length_mismatch() {
        let scores = [1.0; 6];
        let weights = [0.25; 5];
        let _ = composite_score(&scores, &weights);
    }

    #[test]
    #[should_panic(expected = "weights must sum to 1.0")]
    fn composite_score_panics_on_weight_sum_drift() {
        let scores = [1.0; 6];
        let weights = [0.10; 6]; // sums to 0.60, not 1.0
        let _ = composite_score(&scores, &weights);
    }
}
