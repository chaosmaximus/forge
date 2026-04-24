//! Forge-Identity benchmark harness — SKELETON (Phase 2A-4d.3 T2).
//!
//! This is **structural scaffolding** for the 6-dimension forge-identity
//! observability benchmark. Per the master v6 design
//! (`docs/benchmarks/forge-identity-master-design.md`), a run must:
//!
//!   1. Spin up a fresh in-process `DaemonState::new(":memory:")` daemon
//!      so we test the real manas stack (no mocks).
//!   2. Seed synthetic inputs via ChaCha20 PRNG — identical seed → byte
//!      identical workload on every machine.
//!   3. Use SHA-256 hex tokens for content so semantic dedup cannot
//!      collapse fixtures.
//!   4. Run each dimension against the daemon and score on [0, 1].
//!   5. Compute a weighted composite score and a pass/fail flag.
//!   6. Run a suite of infrastructure assertions (schema, drift caps,
//!      token boundaries, etc.) and FAIL-FAST if any fires — per master
//!      v6 §6 the dimension scores are only meaningful once the
//!      infrastructure is sane.
//!
//! T2 (this file) ships:
//!   * All config / score / check structs and serde derives.
//!   * 6 `dim_N_*` stubs returning score 0.0 / pass false.
//!   * 14 `InfrastructureCheck` placeholders (all failing).
//!   * The `run_bench` orchestrator wiring + fail-fast on infra checks
//!     + `summary.json` artifact writer.
//!   * Unit tests that lock in the skeleton shape.
//!   * One integration test stub that exercises the fail-fast path.
//!
//! T3 / T4 / T5 / T6 then fill in individual `dim_N_*` and
//! `run_infrastructure_checks` bodies in parallel. Because each function
//! has a fixed signature with non-overlapping bodies, the parallel dispatch
//! is merge-safe.
//!
//! Ownership map for the follow-up tasks:
//!   * T3 — Dim 3 (preference time-ordering) + Dim 6 (preference staleness
//!     + mixed-corpus recall).
//!   * T4 — Dim 4 (valence flipping).
//!   * T5 — Dim 5 (behavioral skill inference).
//!   * T6 — Dim 1 (identity facet persistence) + Dim 2 (disposition drift)
//!     + the 14 infrastructure assertions in `run_infrastructure_checks`.

use std::path::PathBuf;

use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};

use crate::server::handler::DaemonState;

// ── Configuration ────────────────────────────────────────────────

/// Configuration for a single Forge-Identity bench run.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchConfig {
    /// Seed for the ChaCha20 PRNG driving dataset synthesis.
    pub seed: u64,
    /// Directory to write `summary.json` into.
    pub output_dir: PathBuf,
    /// Optional calibrated composite threshold. `None` means "no
    /// threshold yet — the run prints the observed composite without
    /// asserting equality."
    pub expected_composite: Option<f64>,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            output_dir: PathBuf::from("bench_results_forge_identity"),
            expected_composite: None,
        }
    }
}

// ── Dimension weights & minimums (master v6 §4) ──────────────────

/// Master v6 §4 — per-dimension weights summing to 1.0.
const DIM_WEIGHTS: [f64; 6] = [0.15, 0.15, 0.15, 0.15, 0.15, 0.25];

/// Master v6 §4 — per-dimension minimum scores for pass.
const DIM_MINIMUMS: [f64; 6] = [0.85, 0.85, 0.80, 0.85, 0.80, 0.80];

/// Master v6 §4 — overall composite threshold.
const COMPOSITE_THRESHOLD: f64 = 0.95;

// ── Scoring structs ──────────────────────────────────────────────

/// Score for a single dimension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionScore {
    pub name: String,
    pub score: f64,
    pub min: f64,
    pub pass: bool,
}

/// Single infrastructure assertion result (master v6 §6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InfrastructureCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Final score for a Forge-Identity run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityScore {
    pub seed: u64,
    pub composite: f64,
    pub dimensions: [DimensionScore; 6],
    pub infrastructure_checks: Vec<InfrastructureCheck>,
    pub pass: bool,
    pub wall_duration_ms: u64,
}

// ── Dimension stubs ──────────────────────────────────────────────
//
// Each stub returns score 0.0 / pass false so composite scoring stays
// deterministic until the body is filled in by its owning task. The
// signatures are LOCKED; T3/T4/T5/T6 only edit the bodies.

/// Dim 1: identity facet persistence across sessions.
/// T6 implements this; stub returns score 0.0 / pass false.
fn dim_1_identity_facet_persistence(
    _state: &DaemonState,
    _rng: &mut ChaCha20Rng,
) -> DimensionScore {
    DimensionScore {
        name: "identity_facet_persistence".to_string(),
        score: 0.0,
        min: DIM_MINIMUMS[0],
        pass: false,
    }
}

/// Dim 2: disposition drift within the master v6 bounded-delta envelope.
/// T6 implements this; stub returns score 0.0 / pass false.
fn dim_2_disposition_drift(_state: &DaemonState, _rng: &mut ChaCha20Rng) -> DimensionScore {
    DimensionScore {
        name: "disposition_drift".to_string(),
        score: 0.0,
        min: DIM_MINIMUMS[1],
        pass: false,
    }
}

/// Dim 3: preference time-ordering — newer preferences win ties.
/// T3 implements this; stub returns score 0.0 / pass false.
fn dim_3_preference_time_ordering(_state: &DaemonState, _rng: &mut ChaCha20Rng) -> DimensionScore {
    DimensionScore {
        name: "preference_time_ordering".to_string(),
        score: 0.0,
        min: DIM_MINIMUMS[2],
        pass: false,
    }
}

/// Dim 4: valence flipping — polarity reversals are detected and applied.
/// T4 implements this; stub returns score 0.0 / pass false.
fn dim_4_valence_flipping(_state: &DaemonState, _rng: &mut ChaCha20Rng) -> DimensionScore {
    DimensionScore {
        name: "valence_flipping".to_string(),
        score: 0.0,
        min: DIM_MINIMUMS[3],
        pass: false,
    }
}

/// Dim 5: behavioral skill inference from tool-use patterns.
/// T5 implements this; stub returns score 0.0 / pass false.
fn dim_5_behavioral_skill_inference(
    _state: &DaemonState,
    _rng: &mut ChaCha20Rng,
) -> DimensionScore {
    DimensionScore {
        name: "behavioral_skill_inference".to_string(),
        score: 0.0,
        min: DIM_MINIMUMS[4],
        pass: false,
    }
}

/// Dim 6: preference staleness + mixed-corpus recall.
/// T3 implements this; stub returns score 0.0 / pass false.
fn dim_6_preference_staleness(_state: &DaemonState, _rng: &mut ChaCha20Rng) -> DimensionScore {
    DimensionScore {
        name: "preference_staleness".to_string(),
        score: 0.0,
        min: DIM_MINIMUMS[5],
        pass: false,
    }
}

// ── Infrastructure checks (master v6 §6 — 14 assertions) ─────────

/// Placeholder implementation. T6 flips each entry to a real check.
/// Until then every assertion returns `passed: false` so `run_bench`
/// exercises its fail-fast path exactly as it will in production when
/// the infrastructure drifts.
fn run_infrastructure_checks(_state: &DaemonState) -> Vec<InfrastructureCheck> {
    // Master v6 §6 assertions 1–14. Names are the authoritative handles
    // downstream code (alerts, dashboards) keys off of — T6 must
    // preserve these exact strings when wiring real checks.
    const CHECK_NAMES: [&str; 14] = [
        "identity_table_schema",
        "disposition_table_schema",
        "preference_table_schema",
        "skill_table_schema",
        "disposition_max_delta_const",
        "preference_bounded_staleness_const",
        "valence_polarity_invariant",
        "identity_facet_uniqueness",
        "preference_monotonic_timestamps",
        "skill_inference_token_budget",
        "synthetic_embedding_determinism",
        "sha256_token_uniqueness",
        "consolidator_run_policy",
        "fail_closed_on_drift",
    ];

    CHECK_NAMES
        .iter()
        .map(|name| InfrastructureCheck {
            name: (*name).to_string(),
            passed: false,
            detail: "TODO: T6 implements".to_string(),
        })
        .collect()
}

// ── Composite scoring helper ─────────────────────────────────────

/// Weighted sum of per-dimension scores (master v6 §4).
fn composite_score(dimensions: &[DimensionScore; 6]) -> f64 {
    let mut total = 0.0;
    for (i, d) in dimensions.iter().enumerate() {
        total += DIM_WEIGHTS[i] * d.score;
    }
    total
}

/// Evaluate per-dimension pass (score ≥ min).
fn mark_pass(d: DimensionScore) -> DimensionScore {
    let pass = d.score >= d.min;
    DimensionScore { pass, ..d }
}

// ── Orchestrator ─────────────────────────────────────────────────

/// Run a full Forge-Identity benchmark and return the composite score.
///
/// Flow (master v6 §7):
///   1. Spin up a fresh `:memory:` daemon state.
///   2. Run the 14 infrastructure assertions. ANY failure aborts early
///      with empty (zeroed) dimensions — dimension scores are only
///      meaningful when the infra invariants hold.
///   3. Seed a ChaCha20 PRNG from `config.seed`.
///   4. Evaluate each of the 6 dimensions.
///   5. Compute the weighted composite score.
///   6. Overall pass = composite ≥ 0.95 AND every dim passes AND every
///      infra check passes.
///   7. Persist `summary.json` to `config.output_dir`.
pub async fn run_bench(config: BenchConfig) -> Result<IdentityScore, String> {
    let start = std::time::Instant::now();

    // 1. Fresh in-memory daemon.
    let state = DaemonState::new(":memory:").map_err(|e| format!("state init: {e}"))?;

    // 2. Infrastructure checks — fail fast.
    let infrastructure_checks = run_infrastructure_checks(&state);
    let infra_all_pass = infrastructure_checks.iter().all(|c| c.passed);

    if !infra_all_pass {
        let zeroed = zeroed_dimensions();
        let score = IdentityScore {
            seed: config.seed,
            composite: 0.0,
            dimensions: zeroed,
            infrastructure_checks,
            pass: false,
            wall_duration_ms: start.elapsed().as_millis() as u64,
        };
        write_summary(&config.output_dir, &score)?;
        return Ok(score);
    }

    // 3. Seeded PRNG.
    let mut rng = super::common::seeded_rng(config.seed);

    // 4. Dimensions (T3/T4/T5/T6 own the bodies).
    let dimensions: [DimensionScore; 6] = [
        mark_pass(dim_1_identity_facet_persistence(&state, &mut rng)),
        mark_pass(dim_2_disposition_drift(&state, &mut rng)),
        mark_pass(dim_3_preference_time_ordering(&state, &mut rng)),
        mark_pass(dim_4_valence_flipping(&state, &mut rng)),
        mark_pass(dim_5_behavioral_skill_inference(&state, &mut rng)),
        mark_pass(dim_6_preference_staleness(&state, &mut rng)),
    ];

    // 5. Composite.
    let composite = composite_score(&dimensions);

    // 6. Overall pass.
    let all_dims_pass = dimensions.iter().all(|d| d.pass);
    let pass = composite >= COMPOSITE_THRESHOLD && all_dims_pass && infra_all_pass;

    let score = IdentityScore {
        seed: config.seed,
        composite,
        dimensions,
        infrastructure_checks,
        pass,
        wall_duration_ms: start.elapsed().as_millis() as u64,
    };

    // 7. Artifact.
    write_summary(&config.output_dir, &score)?;

    Ok(score)
}

/// Build a 6-element array of zeroed dimension scores. Used for the
/// fail-fast path when infrastructure checks abort the run.
fn zeroed_dimensions() -> [DimensionScore; 6] {
    [
        DimensionScore {
            name: "identity_facet_persistence".to_string(),
            score: 0.0,
            min: DIM_MINIMUMS[0],
            pass: false,
        },
        DimensionScore {
            name: "disposition_drift".to_string(),
            score: 0.0,
            min: DIM_MINIMUMS[1],
            pass: false,
        },
        DimensionScore {
            name: "preference_time_ordering".to_string(),
            score: 0.0,
            min: DIM_MINIMUMS[2],
            pass: false,
        },
        DimensionScore {
            name: "valence_flipping".to_string(),
            score: 0.0,
            min: DIM_MINIMUMS[3],
            pass: false,
        },
        DimensionScore {
            name: "behavioral_skill_inference".to_string(),
            score: 0.0,
            min: DIM_MINIMUMS[4],
            pass: false,
        },
        DimensionScore {
            name: "preference_staleness".to_string(),
            score: 0.0,
            min: DIM_MINIMUMS[5],
            pass: false,
        },
    ]
}

fn write_summary(output_dir: &std::path::Path, score: &IdentityScore) -> Result<(), String> {
    std::fs::create_dir_all(output_dir).map_err(|e| format!("create output dir: {e}"))?;
    let path = output_dir.join("summary.json");
    let body =
        serde_json::to_string_pretty(score).map_err(|e| format!("serialize summary.json: {e}"))?;
    std::fs::write(&path, body).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bench_config_defaults() {
        let cfg = BenchConfig {
            seed: 42,
            output_dir: PathBuf::from("tmp"),
            expected_composite: None,
        };
        assert_eq!(cfg.seed, 42);
        assert!(cfg.expected_composite.is_none());
    }

    #[test]
    fn test_dimension_score_pass_false_by_default() {
        let d = DimensionScore {
            name: "x".to_string(),
            score: 0.5,
            min: 0.8,
            pass: false,
        };
        assert!(!d.pass, "score < min must not pass");
        let marked = mark_pass(d);
        assert!(!marked.pass);
    }

    #[test]
    fn test_composite_weighted_sum() {
        // Zero scores → zero composite.
        let zeroed = zeroed_dimensions();
        assert!((composite_score(&zeroed) - 0.0).abs() < 1e-12);

        // Score 1.0 everywhere → weighted sum = sum of weights = 1.0.
        let mut ones = zeroed;
        for d in ones.iter_mut() {
            d.score = 1.0;
        }
        let total: f64 = DIM_WEIGHTS.iter().sum();
        assert!(
            (composite_score(&ones) - total).abs() < 1e-12,
            "weights should sum to 1.0; got {total}"
        );
        assert!((total - 1.0).abs() < 1e-12, "weights must sum to 1.0");
    }

    #[tokio::test]
    async fn test_run_bench_fails_fast_on_infra_check_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BenchConfig {
            seed: 42,
            output_dir: tmp.path().to_path_buf(),
            expected_composite: None,
        };
        let score = run_bench(cfg).await.expect("run_bench returns Ok");

        assert!(!score.pass, "T2 stub must not pass overall");
        assert_eq!(
            score.infrastructure_checks.len(),
            14,
            "master v6 §6 mandates 14 infra assertions"
        );
        assert!(
            score.infrastructure_checks.iter().all(|c| !c.passed),
            "T2 stub: every infra check is TODO and must fail"
        );
        // Fail-fast: dimensions zeroed out when infra fails.
        assert!(score.dimensions.iter().all(|d| d.score == 0.0));
        assert!(tmp.path().join("summary.json").exists());
    }
}
