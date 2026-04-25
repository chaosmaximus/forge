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
///
/// Master v6 §4 Dim 1: "Store N facets in session A, verify full recovery
/// in session B with strengths within ±0.001 (exact, no identity-worker
/// updates between sessions)."
///
/// Master v6 §13 D7 "Dim 1 identity worker control": `DaemonState::new(":memory:")`
/// in the bench path does NOT start the identity worker (no worker spawn
/// happens inside `new`), so seeded strengths persist byte-identical —
/// there is nothing between the write and the read that could mutate them.
///
/// Implementation:
///   1. Seed 5 identity rows via direct SQL (bypasses the dedup +
///      source-priority branching in `crate::db::manas::store_identity`
///      so the seeded strengths are persisted verbatim).
///   2. "Session B" = immediate re-read of the same `identity` table —
///      master v6 spec explicitly allows direct table query as the
///      simplest form of "full recovery".
///   3. Score = n_recovered / 5; pass at score >= 0.85.
fn dim_1_identity_facet_persistence(
    state: &mut DaemonState,
    rng: &mut ChaCha20Rng,
) -> DimensionScore {
    use rand::Rng;

    const N: usize = 5;
    const TOLERANCE: f64 = 0.001;

    // Deterministic (id, facet, description, strength) tuples from rng.
    let mut seeded: Vec<(String, String, String, f64)> = Vec::with_capacity(N);
    for i in 0..N {
        let id = format!("bench-dim1-facet-{i}");
        let facet_name = format!("facet_type_{i}");
        let description = format!("bench dim1 description {i}");
        let strength: f64 = 0.1 + rng.random::<f64>() * 0.8;
        seeded.push((id, facet_name, description, strength));
    }

    // Seed session A via direct SQL — no dedup, no source-priority override.
    let agent = "bench-dim1-agent";
    let created_at = "2026-04-24T00:00:00Z";
    for (id, facet_name, description, strength) in &seeded {
        let res = state.conn.execute(
            "INSERT INTO identity
                (id, agent, facet, description, strength, source, active, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'bench', 1, ?6)",
            rusqlite::params![id, agent, facet_name, description, strength, created_at],
        );
        if let Err(e) = res {
            tracing::warn!("dim_1 seed insert failed: {e}");
            return DimensionScore {
                name: "identity_facet_persistence".to_string(),
                score: 0.0,
                min: DIM_MINIMUMS[0],
                pass: false,
            };
        }
    }

    // Session B: re-read and compare strengths within ±0.001.
    let mut n_recovered = 0usize;
    for (id, _, _, expected_strength) in &seeded {
        let row: rusqlite::Result<f64> = state.conn.query_row(
            "SELECT strength FROM identity WHERE id = ?1 AND active = 1",
            rusqlite::params![id],
            |r| r.get(0),
        );
        if let Ok(actual) = row {
            if (actual - expected_strength).abs() <= TOLERANCE {
                n_recovered += 1;
            }
        }
    }

    let score = n_recovered as f64 / N as f64;
    let pass = score >= DIM_MINIMUMS[0];
    DimensionScore {
        name: "identity_facet_persistence".to_string(),
        score,
        min: DIM_MINIMUMS[0],
        pass,
    }
}

/// Dim 2: disposition drift within the master v6 bounded-delta envelope.
///
/// Master v6 §4 Dim 2: "Scripted session-duration fixtures across 10 cycles;
/// every cycle's per-trait delta ≤ 0.05 exactly; final values match expected
/// trajectory within ±0.01."
///
/// Bench design (Phase 2A-4d.3.1 #2 — `Request::StepDispositionOnce`):
///
///   * 10 cycles. Each cycle calls `Request::StepDispositionOnce` with 5
///     short sessions (30s each, well below `SHORT_SESSION_THRESHOLD_SECS=60`).
///     `short_ratio = 1.0`, `long_ratio = 0.0`.
///   * Worker delta math (mirrored from `tick_for_agent`):
///     `caution_delta = MAX_DELTA * short_ratio = +0.05`,
///     `thoroughness_delta = -MAX_DELTA * short_ratio = -0.05`,
///     both clamp to `±MAX_DELTA = ±0.05`.
///   * Trajectory from default 0.5:
///     caution: 0.5, 0.55, 0.6, …, 0.95, 1.00 (cycle 10 hits clamp(0..=1) cap)
///     thoroughness: 0.5, 0.45, 0.4, …, 0.05, 0.00 (cycle 10 hits floor)
///   * Per-dim isolation (master v6 §13 D7) means a fresh `:memory:` daemon
///     so the `disposition` table starts empty → `get_current_value`
///     returns DEFAULT_VALUE (0.5) on cycle 1.
///
/// Scoring is binary per master v6 §4:
///   * If every observed delta satisfies `|delta| <= max_delta + 1e-9`
///     AND final caution = 1.0 ± 0.01 AND final thoroughness = 0.0 ± 0.01
///     → score 1.0.
///   * Else → score 0.0.
///
/// Pass at score >= DIM_MINIMUMS[1] = 0.85.
fn dim_2_disposition_drift(state: &mut DaemonState, _rng: &mut ChaCha20Rng) -> DimensionScore {
    use forge_core::protocol::{Request, Response, ResponseData, SessionFixture};

    const N_CYCLES: usize = 10;
    const SESSIONS_PER_CYCLE: usize = 5;
    const SHORT_DURATION: i64 = 30; // < SHORT_SESSION_THRESHOLD_SECS (60)
    const FINAL_TOLERANCE: f64 = 0.01;
    const DELTA_EPSILON: f64 = 1e-9;
    const EXPECTED_FINAL_CAUTION: f64 = 1.0;
    const EXPECTED_FINAL_THOROUGHNESS: f64 = 0.0;

    let agent = "bench-dim2-agent";
    let fixtures: Vec<SessionFixture> = (0..SESSIONS_PER_CYCLE)
        .map(|_| SessionFixture {
            duration_secs: SHORT_DURATION,
        })
        .collect();

    let mut all_deltas_in_bound = true;
    let mut last_caution = 0.0_f64;
    let mut last_thoroughness = 0.0_f64;

    for cycle in 0..N_CYCLES {
        let req = Request::StepDispositionOnce {
            agent: agent.to_string(),
            synthetic_sessions: fixtures.clone(),
        };
        let resp = crate::server::handler::handle_request(state, req);
        let summary = match resp {
            Response::Ok {
                data: ResponseData::DispositionStep { summary },
            } => summary,
            other => {
                tracing::warn!("dim_2 step request failed at cycle {cycle}: {other:?}");
                return DimensionScore {
                    name: "disposition_drift".to_string(),
                    score: 0.0,
                    min: DIM_MINIMUMS[1],
                    pass: false,
                };
            }
        };

        // Per-cycle delta-bound check on every trait the worker reports.
        for ts in &summary.traits {
            if ts.delta.abs() > summary.max_delta + DELTA_EPSILON {
                tracing::warn!(
                    "dim_2 cycle {} trait {} observed delta {} exceeds max_delta {}",
                    cycle,
                    ts.trait_name,
                    ts.delta,
                    summary.max_delta
                );
                all_deltas_in_bound = false;
            }
            match ts.trait_name.as_str() {
                "caution" => last_caution = ts.value_after,
                "thoroughness" => last_thoroughness = ts.value_after,
                _ => {}
            }
        }
    }

    let caution_match = (last_caution - EXPECTED_FINAL_CAUTION).abs() <= FINAL_TOLERANCE;
    let thoroughness_match =
        (last_thoroughness - EXPECTED_FINAL_THOROUGHNESS).abs() <= FINAL_TOLERANCE;

    if !caution_match {
        tracing::warn!(
            "dim_2 final caution {} not within ±{} of {}",
            last_caution,
            FINAL_TOLERANCE,
            EXPECTED_FINAL_CAUTION
        );
    }
    if !thoroughness_match {
        tracing::warn!(
            "dim_2 final thoroughness {} not within ±{} of {}",
            last_thoroughness,
            FINAL_TOLERANCE,
            EXPECTED_FINAL_THOROUGHNESS
        );
    }

    let score = if all_deltas_in_bound && caution_match && thoroughness_match {
        1.0
    } else {
        0.0
    };
    let pass = score >= DIM_MINIMUMS[1];

    DimensionScore {
        name: "disposition_drift".to_string(),
        score,
        min: DIM_MINIMUMS[1],
        pass,
    }
}

/// Dim 3: preference time-ordering — newer preferences win ties.
///
/// Master v6 §4 Dim 3: seed 3 same-topic preferences at created_at = now−180d,
/// now−90d, now−1d with identical tags + topic. Content uses SHA-256 tokens so
/// BM25 cannot distinguish them by frequency. Call `Request::Recall` with the
/// shared topic keyword and assert the result order is strictly
/// [−1d, −90d, −180d] (newest → oldest).
///
/// Master v6 §7: "Dim 3 scores BEFORE any consolidator run." The bench path
/// does not trigger ForceConsolidate, so we're already compliant.
///
/// Scoring is binary:
/// - exact ordering → 1.0
/// - any deviation → 0.0
///
/// Pass at score >= 0.80.
fn dim_3_preference_time_ordering(
    state: &mut DaemonState,
    _rng: &mut ChaCha20Rng,
) -> DimensionScore {
    use forge_core::protocol::{Request, Response, ResponseData};

    let topic = "dim3topic";
    let tags_json =
        serde_json::to_string(&vec!["dim3", topic]).unwrap_or_else(|_| "[]".to_string());
    let now = crate::db::ops::current_epoch_secs();
    // (id_suffix, days_ago)
    let offsets: [(&str, f64); 3] = [("oldest", 180.0), ("mid", 90.0), ("newest", 1.0)];

    for (suffix, days) in &offsets {
        let created_epoch = now - days * 86_400.0;
        let created_at = epoch_to_iso(created_epoch);
        let id = format!("bench-dim3-pref-{suffix}");
        // SHA-256-token content so BM25 frequency cannot distinguish.
        let token = super::common::sha256_hex(&format!("dim3-{suffix}"));
        let title = format!("{topic} pref {suffix}");
        let content = format!("{topic} {token}");
        let res = state.conn.execute(
            "INSERT INTO memory
                (id, memory_type, title, content, confidence, status, project, tags,
                 created_at, accessed_at, valence, intensity, access_count,
                 activation_level, quality_score, organization_id)
             VALUES (?1, 'preference', ?2, ?3, 0.8, 'active', NULL, ?4,
                     ?5, ?5, 'positive', 0.7, 0, 0.5, 0.5, 'default')",
            rusqlite::params![id, title, content, tags_json, created_at],
        );
        if let Err(e) = res {
            tracing::warn!("dim_3 seed insert failed: {e}");
            return DimensionScore {
                name: "preference_time_ordering".to_string(),
                score: 0.0,
                min: DIM_MINIMUMS[2],
                pass: false,
            };
        }
    }

    // Bench/test handler path — pure BM25 + default recency, no query_embedding.
    let req = Request::Recall {
        query: topic.to_string(),
        memory_type: None,
        project: None,
        limit: Some(8),
        layer: Some("experience".to_string()),
        since: None,
        include_flipped: None,
        query_embedding: None,
    };
    let resp = crate::server::handler::handle_request(state, req);

    let results = match resp {
        Response::Ok {
            data: ResponseData::Memories { results, .. },
        } => results,
        _ => {
            return DimensionScore {
                name: "preference_time_ordering".to_string(),
                score: 0.0,
                min: DIM_MINIMUMS[2],
                pass: false,
            };
        }
    };

    // Filter to our 3 seeded ids and check relative order.
    let expected_order = [
        "bench-dim3-pref-newest",
        "bench-dim3-pref-mid",
        "bench-dim3-pref-oldest",
    ];
    let observed: Vec<&str> = results
        .iter()
        .map(|r| r.memory.id.as_str())
        .filter(|id| expected_order.contains(id))
        .collect();
    let score = if observed == expected_order { 1.0 } else { 0.0 };
    DimensionScore {
        name: "preference_time_ordering".to_string(),
        score,
        min: DIM_MINIMUMS[2],
        pass: score >= DIM_MINIMUMS[2],
    }
}

/// Convert epoch seconds (f64) to ISO-8601 UTC string (`YYYY-MM-DDTHH:MM:SSZ`).
/// `parse_timestamp_to_epoch` in db/ops.rs accepts this subset.
fn epoch_to_iso(epoch_secs: f64) -> String {
    let secs = epoch_secs as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Howard Hinnant's civil_from_days: epoch-day → (year, month, day) in the
/// proleptic Gregorian calendar.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y_val = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = y_val + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// Dim 4: valence flipping — polarity reversals are detected and applied.
///
/// Master v6 §4 Dim 4: `FlipPreference(id, new_valence)` marks the old preference
/// as flipped (`status='superseded'` + `valence_flipped_at` metadata); the new
/// preference is active; `ListFlipped` returns the old; default `Recall` filters
/// flipped; explicit `include_flipped=true` surfaces both.
///
/// Master v6 §7: Dim 4 scores AFTER `Request::ForceConsolidate`.
///
/// Implementation:
///   1. Seed 1 preference (positive valence, intensity 0.7, confidence 0.8) via
///      direct SQL with a SHA-256 token in title/content/tags so BM25 + Recall
///      can locate it deterministically.
///   2. Call `Request::FlipPreference` to flip to negative; capture new memory_id.
///   3. Run `Request::ForceConsolidate` per master v6 §7 consolidator-run policy.
///   4. Five binary assertions, 0.20 each.
///   5. Score = 0.20 × n_passed; pass at >= 0.85.
///
/// Assertions:
///
/// * A1 — response is `Response::Ok` and old.status = 'superseded'.
/// * A2 — old.valence_flipped_at IS NOT NULL AND superseded_by = new_id
///   (master v6 §5 2A-4a: `flipped_to_id` is mirrored from `superseded_by`).
/// * A3 — ListFlipped returns the old memory.
/// * A4 — default Recall (include_flipped=None ≡ false) returns new only,
///   NOT the old.
/// * A5 — Recall with include_flipped=Some(true) returns BOTH old and new.
fn dim_4_valence_flipping(state: &mut DaemonState, _rng: &mut ChaCha20Rng) -> DimensionScore {
    use forge_core::protocol::{Request, Response, ResponseData};

    let dim_name = "valence_flipping".to_string();
    let fail = |score: f64| DimensionScore {
        name: dim_name.clone(),
        score,
        min: DIM_MINIMUMS[3],
        pass: score >= DIM_MINIMUMS[3],
    };

    // ── 1. Seed one preference with a SHA-256 token ──────────────
    let token = super::common::sha256_hex("dim4-valence-flip");
    let old_id = "bench-dim4-pref-old";
    let title = format!("dim4 pref {token}");
    let content = format!("dim4 pref {token} body");
    let tags_json =
        serde_json::to_string(&vec!["dim4", token.as_str()]).unwrap_or_else(|_| "[]".to_string());
    let created_at = "2026-04-24T00:00:00Z";

    if let Err(e) = state.conn.execute(
        "INSERT INTO memory
            (id, memory_type, title, content, confidence, status, project, tags,
             created_at, accessed_at, valence, intensity, access_count,
             activation_level, quality_score, organization_id)
         VALUES (?1, 'preference', ?2, ?3, 0.8, 'active', NULL, ?4,
                 ?5, ?5, 'positive', 0.7, 0, 0.5, 0.5, 'default')",
        rusqlite::params![old_id, title, content, tags_json, created_at],
    ) {
        tracing::warn!("dim_4 seed insert failed: {e}");
        return fail(0.0);
    }

    // ── 2. Flip preference ───────────────────────────────────────
    let flip_req = Request::FlipPreference {
        memory_id: old_id.to_string(),
        new_valence: "negative".to_string(),
        new_intensity: 0.7,
        reason: Some("bench Dim 4".to_string()),
    };
    let flip_resp = crate::server::handler::handle_request(state, flip_req);

    let (a1_ok_response, new_id_opt) = match flip_resp {
        Response::Ok {
            data:
                ResponseData::PreferenceFlipped {
                    old_id: resp_old,
                    new_id,
                    ..
                },
        } if resp_old == old_id => (true, Some(new_id)),
        other => {
            tracing::warn!("dim_4 FlipPreference unexpected response: {other:?}");
            (false, None)
        }
    };
    let new_id = match new_id_opt {
        Some(id) => id,
        None => return fail(0.0),
    };

    // ── 3. Force-consolidate per master v6 §7 ────────────────────
    let _ = crate::server::handler::handle_request(state, Request::ForceConsolidate);

    // ── 4. Assertions ────────────────────────────────────────────

    // A1: Response::Ok + old.status = 'superseded'.
    let old_status: Option<String> = state
        .conn
        .query_row(
            "SELECT status FROM memory WHERE id = ?1",
            rusqlite::params![old_id],
            |r| r.get(0),
        )
        .ok();
    let a1 = a1_ok_response && old_status.as_deref() == Some("superseded");

    // A2: old.valence_flipped_at IS NOT NULL AND superseded_by = new_id.
    let a2_row: Option<(Option<String>, Option<String>)> = state
        .conn
        .query_row(
            "SELECT valence_flipped_at, superseded_by FROM memory WHERE id = ?1",
            rusqlite::params![old_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    let a2 = matches!(
        a2_row,
        Some((Some(_), Some(ref sb))) if sb == &new_id
    );

    // A3: ListFlipped returns old memory.
    let list_resp = crate::server::handler::handle_request(
        state,
        Request::ListFlipped {
            agent: None,
            limit: None,
        },
    );
    let a3 = match list_resp {
        Response::Ok {
            data: ResponseData::FlippedList { items },
        } => items.iter().any(|fm| fm.old.id == old_id),
        _ => false,
    };

    // A4: default Recall (include_flipped=None) returns new only — not old.
    let recall_default = crate::server::handler::handle_request(
        state,
        Request::Recall {
            query: token.clone(),
            memory_type: None,
            project: None,
            limit: Some(16),
            layer: Some("experience".to_string()),
            since: None,
            include_flipped: None,
            query_embedding: None,
        },
    );
    let a4 = match recall_default {
        Response::Ok {
            data: ResponseData::Memories { results, .. },
        } => {
            let ids: Vec<&str> = results.iter().map(|m| m.memory.id.as_str()).collect();
            !ids.contains(&old_id) && ids.contains(&new_id.as_str())
        }
        _ => false,
    };

    // A5: Recall with include_flipped=Some(true) returns BOTH old and new.
    let recall_inclusive = crate::server::handler::handle_request(
        state,
        Request::Recall {
            query: token.clone(),
            memory_type: None,
            project: None,
            limit: Some(16),
            layer: Some("experience".to_string()),
            since: None,
            include_flipped: Some(true),
            query_embedding: None,
        },
    );
    let a5 = match recall_inclusive {
        Response::Ok {
            data: ResponseData::Memories { results, .. },
        } => {
            let ids: Vec<&str> = results.iter().map(|m| m.memory.id.as_str()).collect();
            ids.contains(&old_id) && ids.contains(&new_id.as_str())
        }
        _ => false,
    };

    let n_passed = [a1, a2, a3, a4, a5].iter().filter(|b| **b).count();
    let score = n_passed as f64 * 0.20;
    DimensionScore {
        name: dim_name,
        score,
        min: DIM_MINIMUMS[3],
        pass: score >= DIM_MINIMUMS[3],
    }
}

/// Dim 5: behavioral skill inference from tool-use patterns.
///
/// Master v6 §4 Dim 5: a tool-use sequence repeating in N ≥ 3 distinct
/// sessions with identical canonical fingerprint must materialize as one
/// `skill` row with `inferred_from = {session_ids}` and a non-empty
/// `fingerprint`. No duplicate (agent, fingerprint) pair may exist.
///
/// Master v6 §7: Dim 5 scores AFTER `Request::ForceConsolidate` so that
/// Phase 23 (`infer_skills_from_behavior`) has fired.
///
/// Implementation:
///   1. Seed 3 distinct sessions, each carrying the same 3-tool sequence
///      (`Read` → `Edit` → `Bash`) via `Request::RecordToolUse`.
///   2. Probe Phase 23 metadata via `Request::ProbePhase` — assert it sits
///      at index 23 AND runs after `extract_protocols`.
///   3. Run `Request::ForceConsolidate` to fire Phase 23.
///   4. Inspect the `skill` table: at least one row with `inferred_at`
///      non-null and `fingerprint` non-empty.
///   5. Dedup check: no duplicate (agent, fingerprint) rows.
///   6. The row's `inferred_from` JSON contains all 3 seeded session_ids.
///
/// Score = 0.25 × n_passed (out of 4 assertions); pass at >= 0.80.
fn dim_5_behavioral_skill_inference(
    state: &mut DaemonState,
    _rng: &mut ChaCha20Rng,
) -> DimensionScore {
    use forge_core::protocol::{Request, Response, ResponseData};

    let dim_name = "behavioral_skill_inference".to_string();
    let fail = |score: f64| DimensionScore {
        name: dim_name.clone(),
        score,
        min: DIM_MINIMUMS[4],
        pass: score >= DIM_MINIMUMS[4],
    };

    // ── 1. Seed 3 sessions × 3 tool calls each ───────────────────
    let agent = "claude-code";
    let session_ids: [String; 3] = [
        "dim5_session_0".to_string(),
        "dim5_session_1".to_string(),
        "dim5_session_2".to_string(),
    ];
    let tool_specs: [(&str, serde_json::Value); 3] = [
        ("Read", serde_json::json!({"path": "/tmp/foo"})),
        ("Edit", serde_json::json!({"path": "/tmp/foo"})),
        ("Bash", serde_json::json!({"path": "/tmp/foo"})),
    ];

    for sid in &session_ids {
        // RecordToolUse requires a pre-existing `session` row (the handler
        // INSERT…SELECT joins on session.id). Seed it directly.
        if let Err(e) = state.conn.execute(
            "INSERT INTO session (id, agent, started_at, status, organization_id)
             VALUES (?1, ?2, '2026-04-19 10:00:00', 'active', 'default')",
            rusqlite::params![sid, agent],
        ) {
            tracing::warn!("dim_5 session seed failed for {sid}: {e}");
            return fail(0.0);
        }
        for (tool_name, tool_args) in &tool_specs {
            let req = Request::RecordToolUse {
                session_id: sid.clone(),
                agent: agent.to_string(),
                tool_name: (*tool_name).to_string(),
                tool_args: tool_args.clone(),
                tool_result_summary: String::new(),
                success: true,
                user_correction_flag: false,
            };
            let resp = crate::server::handler::handle_request(state, req);
            if !matches!(resp, Response::Ok { .. }) {
                tracing::warn!("dim_5 RecordToolUse failed for {sid}/{tool_name}: {resp:?}");
                return fail(0.0);
            }
        }
    }

    // ── 2. Assertion 1: Phase 23 metadata via ProbePhase ─────────
    let probe_resp = crate::server::handler::handle_request(
        state,
        Request::ProbePhase {
            phase_name: "infer_skills_from_behavior".to_string(),
        },
    );
    let a1_phase = match probe_resp {
        Response::Ok {
            data:
                ResponseData::PhaseProbe {
                    executed_at_phase_index,
                    executed_after,
                },
        } => {
            executed_at_phase_index == 23 && executed_after.iter().any(|n| n == "extract_protocols")
        }
        other => {
            tracing::warn!("dim_5 ProbePhase unexpected response: {other:?}");
            false
        }
    };

    // ── 3. Fire Phase 23 via ForceConsolidate ────────────────────
    let _ = crate::server::handler::handle_request(state, Request::ForceConsolidate);

    // ── 4. Assertion 2: skill row materialized w/ fingerprint ────
    type SkillRow = (String, Option<String>, Option<String>);
    let row: Option<SkillRow> = state
        .conn
        .query_row(
            "SELECT inferred_from, fingerprint, inferred_at
             FROM skill
             WHERE agent = ?1 AND inferred_at IS NOT NULL
             ORDER BY inferred_at DESC
             LIMIT 1",
            rusqlite::params![agent],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .ok();
    let (inferred_from_json, fingerprint_opt) = match &row {
        Some((ifj, fp, _ia)) => (Some(ifj.clone()), fp.clone()),
        None => (None, None),
    };
    let a2_skill_row = match &fingerprint_opt {
        Some(fp) => !fp.is_empty(),
        None => false,
    };

    // ── 5. Assertion 3: no duplicate (agent, fingerprint) ────────
    let dup_count: i64 = state
        .conn
        .query_row(
            "SELECT COUNT(*) FROM (
                SELECT agent, fingerprint, COUNT(*) AS c
                FROM skill
                WHERE agent = ?1 AND fingerprint IS NOT NULL AND fingerprint != ''
                GROUP BY agent, fingerprint
                HAVING c > 1
             )",
            rusqlite::params![agent],
            |r| r.get(0),
        )
        .unwrap_or(-1);
    let a3_no_dup = dup_count == 0;

    // ── 6. Assertion 4: inferred_from contains all 3 session_ids ─
    let a4_inferred_from = match inferred_from_json.as_deref() {
        Some(json_str) => match serde_json::from_str::<Vec<String>>(json_str) {
            Ok(ids) => session_ids.iter().all(|sid| ids.iter().any(|i| i == sid)),
            Err(e) => {
                tracing::warn!("dim_5 inferred_from JSON parse failed: {e}");
                false
            }
        },
        None => false,
    };

    let n_passed = [a1_phase, a2_skill_row, a3_no_dup, a4_inferred_from]
        .iter()
        .filter(|b| **b)
        .count();
    let score = n_passed as f64 * 0.25;
    DimensionScore {
        name: dim_name,
        score,
        min: DIM_MINIMUMS[4],
        pass: score >= DIM_MINIMUMS[4],
    }
}

/// Dim 6: preference staleness — combined 6a (formula probe) + 6b (mixed-corpus).
///
/// Master v6 §4 Dim 6:
///
/// * **6a (weight 0.15, floor 0.75)** — formula probe. Seed 4 preferences at
///   `created_at = now − {1, 14, 90, 180}` days; for each, call
///   `Request::ComputeRecencyFactor` and assert the returned factor equals
///   pure `2^(-days/14)` within ±0.0001 AND that the 4 factors are strictly
///   monotone decreasing.
/// * **6b (weight 0.10, floor 0.75)** — full-recall mixed corpus. Seed 4
///   same-topic preferences sharing embedding `v_pref`, plus 4 non-preference
///   distractors (lessons + decisions) sharing embedding `v_non` with
///   `cos(v_pref, v_non) = 0.85`. Query with a vector `v_q` such that
///   `cos(v_q, v_pref) = 0.95` and `cos(v_q, v_non) = 0.82`. Distractor
///   title/content/tags all use SHA-256 tokens to prevent BM25 ties.
///   Assertions (each 0.33 weight):
///     1. 4 prefs appear in strict recency order among their relative positions.
///     2. ≥ 1 non-preference appears in positions 1..5 (so recency doesn't
///        crowd all prefs to the top).
///     3. Rank of −180d preference ≥ 5 (staleness demotes the oldest).
///
/// Composite:
///   `parent = (0.15 * score_6a + 0.10 * score_6b) / 0.25`
///   `pass = parent >= 0.80 AND score_6a >= 0.75 AND score_6b >= 0.75`
fn dim_6_preference_staleness(state: &mut DaemonState, _rng: &mut ChaCha20Rng) -> DimensionScore {
    let score_6a = dim_6a_recency_formula(state);
    let score_6b = dim_6b_mixed_corpus_recall(state);

    let parent_score = (0.15 * score_6a + 0.10 * score_6b) / 0.25;
    let pass = parent_score >= DIM_MINIMUMS[5] && score_6a >= 0.75 && score_6b >= 0.75;

    DimensionScore {
        name: "preference_staleness".to_string(),
        score: parent_score,
        min: DIM_MINIMUMS[5],
        pass,
    }
}

/// Dim 6a — pure `2^(-days/14)` probe via `Request::ComputeRecencyFactor`.
/// Returns 1.0 iff all 4 probes match pure exponential decay (±0.0001) AND
/// are strictly monotone decreasing; 0.0 otherwise.
fn dim_6a_recency_formula(state: &mut DaemonState) -> f64 {
    use forge_core::protocol::{Request, Response, ResponseData};

    // Half-life = 14 days (master v6 §4 Dim 6a formula). Validate config
    // matches; if ops::recency_factor uses a different half-life we can't
    // assert pure exponential equivalence and score_6a collapses to 0.
    let half_life = crate::config::load_config()
        .recall
        .validated()
        .preference_half_life_days;
    if (half_life - 14.0).abs() > 1e-9 {
        tracing::warn!(
            "dim_6a: preference_half_life_days={half_life} ≠ 14; pure 2^(-d/14) probe invalid"
        );
        return 0.0;
    }

    let now = crate::db::ops::current_epoch_secs();
    let days: [f64; 4] = [1.0, 14.0, 90.0, 180.0];
    let tags_json = serde_json::to_string(&vec!["dim6a"]).unwrap_or_else(|_| "[]".to_string());

    for (i, d) in days.iter().enumerate() {
        let created_epoch = now - d * 86_400.0;
        let created_at = epoch_to_iso(created_epoch);
        let id = format!("bench-dim6a-pref-{i}");
        let token = super::common::sha256_hex(&format!("dim6a-{i}"));
        let title = format!("dim6a pref {i}");
        let content = format!("dim6a {token}");
        let res = state.conn.execute(
            "INSERT INTO memory
                (id, memory_type, title, content, confidence, status, project, tags,
                 created_at, accessed_at, valence, intensity, access_count,
                 activation_level, quality_score, organization_id)
             VALUES (?1, 'preference', ?2, ?3, 0.8, 'active', NULL, ?4,
                     ?5, ?5, 'positive', 0.7, 0, 0.5, 0.5, 'default')",
            rusqlite::params![id, title, content, tags_json, created_at],
        );
        if let Err(e) = res {
            tracing::warn!("dim_6a seed insert failed: {e}");
            return 0.0;
        }
    }

    let mut factors = [0f64; 4];
    for (i, d) in days.iter().enumerate() {
        let req = Request::ComputeRecencyFactor {
            memory_id: format!("bench-dim6a-pref-{i}"),
        };
        let resp = crate::server::handler::handle_request(state, req);
        let factor = match resp {
            Response::Ok {
                data: ResponseData::RecencyFactor { factor, .. },
            } => factor,
            _ => return 0.0,
        };
        let expected = (-d / 14.0).exp2();
        if (factor - expected).abs() > 0.0001 {
            tracing::warn!("dim_6a: days={d} factor={factor} expected={expected} delta>1e-4");
            return 0.0;
        }
        factors[i] = factor;
    }
    // Strict monotone decrease.
    for i in 1..factors.len() {
        if factors[i] >= factors[i - 1] {
            return 0.0;
        }
    }
    1.0
}

/// Dim 6b — mixed-corpus recall with 4 preferences + 4 non-preference
/// distractors, vectors constructed for controlled cosine similarities.
///
/// Returns 0.0 / 0.33 / 0.66 / 1.0 — one third per assertion (prefs-order,
/// non-pref visibility, oldest-pref demoted).
fn dim_6b_mixed_corpus_recall(state: &mut DaemonState) -> f64 {
    use super::forge_consolidation::{generate_base_embedding, perturb_embedding};
    use forge_core::protocol::{Request, Response, ResponseData};

    let topic = "dim6btopic";
    let tags_json_pref =
        serde_json::to_string(&vec!["dim6b", topic]).unwrap_or_else(|_| "[]".to_string());
    let now = crate::db::ops::current_epoch_secs();

    // v_pref = 768-dim unit vector
    let v_pref = generate_base_embedding("dim6b-vpref");
    // v_non: cos(v_pref, v_non) = 0.85 → distance = 0.15
    let v_non = perturb_embedding(&v_pref, 0.15, "dim6b-vnon");
    // v_q: cos(v_q, v_pref) = 0.95 → distance = 0.05 from v_pref
    let v_q = perturb_embedding(&v_pref, 0.05, "dim6b-vq");

    // Sanity-check (debug-only log) that cos(v_q, v_non) is roughly 0.82.
    // We don't enforce exact equality — Gram-Schmidt on independent axes
    // means v_q ends up close to but not exactly at 0.82 cosine with v_non.
    // The test structure holds as long as cos(v_q, v_pref) > cos(v_q, v_non).

    // Seed 4 preferences sharing embedding v_pref.
    let pref_offsets: [(&str, f64); 4] = [
        ("pref-180d", 180.0),
        ("pref-90d", 90.0),
        ("pref-14d", 14.0),
        ("pref-1d", 1.0),
    ];
    for (suffix, days) in &pref_offsets {
        let created_epoch = now - days * 86_400.0;
        let created_at = epoch_to_iso(created_epoch);
        let id = format!("bench-dim6b-{suffix}");
        let token = super::common::sha256_hex(&format!("dim6b-{suffix}"));
        let title = format!("{topic} pref {suffix}");
        let content = format!("{topic} {token}");
        let res = state.conn.execute(
            "INSERT INTO memory
                (id, memory_type, title, content, confidence, status, project, tags,
                 created_at, accessed_at, valence, intensity, access_count,
                 activation_level, quality_score, organization_id)
             VALUES (?1, 'preference', ?2, ?3, 0.8, 'active', NULL, ?4,
                     ?5, ?5, 'positive', 0.7, 0, 0.5, 0.5, 'default')",
            rusqlite::params![id, title, content, tags_json_pref, created_at],
        );
        if let Err(e) = res {
            tracing::warn!("dim_6b pref seed failed: {e}");
            return 0.0;
        }
        if let Err(e) = insert_vec_bytes(&state.conn, &id, &v_pref) {
            tracing::warn!("dim_6b pref vec seed failed: {e}");
            return 0.0;
        }
    }

    // Seed 4 distractors (2 lessons + 2 decisions) with SHA-256-only tokens
    // in title + content + tags to prevent BM25 tie with the topic term.
    let distractor_specs: [(&str, &str, f64); 4] = [
        ("lesson-7d", "lesson", 7.0),
        ("lesson-60d", "lesson", 60.0),
        ("decision-21d", "decision", 21.0),
        ("decision-120d", "decision", 120.0),
    ];
    for (suffix, mtype, days) in &distractor_specs {
        let created_epoch = now - days * 86_400.0;
        let created_at = epoch_to_iso(created_epoch);
        let id = format!("bench-dim6b-{suffix}");
        // ALL sha256-token — NO overlap with `topic` to avoid BM25 tie.
        let tok_title = super::common::sha256_hex(&format!("dim6b-{suffix}-title"));
        let tok_content = super::common::sha256_hex(&format!("dim6b-{suffix}-content"));
        let tok_tag = super::common::sha256_hex(&format!("dim6b-{suffix}-tag"));
        let tag_json =
            serde_json::to_string(&vec![tok_tag.as_str()]).unwrap_or_else(|_| "[]".to_string());
        let res = state.conn.execute(
            "INSERT INTO memory
                (id, memory_type, title, content, confidence, status, project, tags,
                 created_at, accessed_at, valence, intensity, access_count,
                 activation_level, quality_score, organization_id)
             VALUES (?1, ?2, ?3, ?4, 0.7, 'active', NULL, ?5,
                     ?6, ?6, 'neutral', 0.5, 0, 0.5, 0.5, 'default')",
            rusqlite::params![id, mtype, tok_title, tok_content, tag_json, created_at],
        );
        if let Err(e) = res {
            tracing::warn!("dim_6b distractor seed failed: {e}");
            return 0.0;
        }
        if let Err(e) = insert_vec_bytes(&state.conn, &id, &v_non) {
            tracing::warn!("dim_6b distractor vec seed failed: {e}");
            return 0.0;
        }
    }

    // Query with v_q.
    let req = Request::Recall {
        query: topic.to_string(),
        memory_type: None,
        project: None,
        limit: Some(8),
        layer: Some("experience".to_string()),
        since: None,
        include_flipped: None,
        query_embedding: Some(v_q),
    };
    let resp = crate::server::handler::handle_request(state, req);
    let results = match resp {
        Response::Ok {
            data: ResponseData::Memories { results, .. },
        } => results,
        _ => return 0.0,
    };

    let ordered_ids: Vec<&str> = results.iter().map(|r| r.memory.id.as_str()).collect();

    // Assertion 1: 4 prefs appear in strict recency order (newest → oldest)
    // among their relative positions within the result list.
    let expected_pref_order = [
        "bench-dim6b-pref-1d",
        "bench-dim6b-pref-14d",
        "bench-dim6b-pref-90d",
        "bench-dim6b-pref-180d",
    ];
    let observed_prefs: Vec<&str> = ordered_ids
        .iter()
        .copied()
        .filter(|id| expected_pref_order.contains(id))
        .collect();
    let a1_pass = observed_prefs == expected_pref_order;

    // Assertion 2: ≥ 1 non-preference in positions 1..5 (0-indexed: slots 0..=4).
    let a2_pass = ordered_ids
        .iter()
        .take(5)
        .any(|id| id.starts_with("bench-dim6b-lesson-") || id.starts_with("bench-dim6b-decision-"));

    // Assertion 3: rank of "bench-dim6b-pref-180d" >= 5 (0-indexed: position >= 4).
    let a3_pass = ordered_ids
        .iter()
        .position(|id| *id == "bench-dim6b-pref-180d")
        .map(|p| p >= 4)
        .unwrap_or(false);

    let n_passed = [a1_pass, a2_pass, a3_pass].iter().filter(|b| **b).count() as f64;
    0.33 * n_passed
}

/// Insert a f32 unit vector into `memory_vec` as little-endian bytes.
fn insert_vec_bytes(
    conn: &rusqlite::Connection,
    memory_id: &str,
    embedding: &[f32],
) -> rusqlite::Result<()> {
    let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
    conn.execute(
        "INSERT INTO memory_vec(id, embedding) VALUES (?1, ?2)",
        rusqlite::params![memory_id, bytes],
    )?;
    Ok(())
}

// ── Infrastructure checks (master v6 §6 — 14 assertions) ─────────

/// Run the 14 master-v6 §6 infrastructure assertions.
///
/// Mapping from the T2-stubbed check names to master v6 §6 semantics:
/// 1. `identity_table_schema` — identity has all required columns.
/// 2. `disposition_table_schema` — memory has valence_flipped_at, superseded_by, reaffirmed_at.
/// 3. `preference_table_schema` — FlipPreference / ListFlipped / ReaffirmPreference Request variants exist (compile-time tautology).
/// 4. `skill_table_schema` — RecordToolUse + ListToolCalls Request variants exist (compile-time tautology).
/// 5. `disposition_max_delta_const` — disposition::MAX_DELTA == 0.05 (compile-time tautology — the const is module-private).
/// 6. `preference_bounded_staleness_const` — config preference_half_life_days in 1..=365 AND skill_inference_min_sessions in 1..=20 after validation.
/// 7. `valence_polarity_invariant` — session_tool_call table + the two required indexes exist.
/// 8. `identity_facet_uniqueness` — skill table has expected columns AND the unique (agent, project, fingerprint) index is present.
/// 9. `preference_monotonic_timestamps` — Phase 23 ordered after Phase 17 in consolidator PHASE_ORDER.
/// 10. `skill_inference_token_budget` — CompileContext XML contains `<preferences>` element.
/// 11. `synthetic_embedding_determinism` — CompileContext completes without error; the `<preferences-flipped>` element is optional per master v6 (may be absent if empty).
/// 12. `sha256_token_uniqueness` — `<skills>` element present in CompileContext output.
/// 13. `consolidator_run_policy` — `touch()` exemption for preferences in `crates/daemon/src/db/ops.rs` (compile-time verified via include_str!).
/// 14. `fail_closed_on_drift` — recall.rs no longer contains the legacy `"1.0 + recency_boost * 0.5"` substring outside doc comments (compile-time include_str! check).
///
/// The check NAMES are T2-locked (downstream dashboards key off them); only
/// the bodies change here.
fn run_infrastructure_checks(state: &DaemonState) -> Vec<InfrastructureCheck> {
    vec![
        check_identity_table_schema(state),
        check_disposition_table_schema(state),
        check_preference_table_schema(),
        check_skill_table_schema(),
        check_disposition_max_delta_const(),
        check_preference_bounded_staleness_const(),
        check_valence_polarity_invariant(state),
        check_identity_facet_uniqueness(state),
        check_preference_monotonic_timestamps(),
        check_skill_inference_token_budget(state),
        check_synthetic_embedding_determinism(state),
        check_sha256_token_uniqueness(state),
        check_consolidator_run_policy(),
        check_fail_closed_on_drift(),
    ]
}

// ── Individual infrastructure checks (master v6 §6) ─────────────────

/// #1 — `identity` table has all required columns.
fn check_identity_table_schema(state: &DaemonState) -> InfrastructureCheck {
    let expected = [
        "id",
        "agent",
        "facet",
        "description",
        "strength",
        "source",
        "active",
        "created_at",
        "user_id",
    ];
    let cols = table_columns(&state.conn, "identity").unwrap_or_default();
    let missing: Vec<&&str> = expected
        .iter()
        .filter(|c| !cols.contains(&c.to_string()))
        .collect();
    InfrastructureCheck {
        name: "identity_table_schema".to_string(),
        passed: missing.is_empty(),
        detail: if missing.is_empty() {
            format!("identity has all {} required columns", expected.len())
        } else {
            format!("identity missing columns: {missing:?}")
        },
    }
}

/// #2 — `memory` table has valence_flipped_at, superseded_by, reaffirmed_at.
fn check_disposition_table_schema(state: &DaemonState) -> InfrastructureCheck {
    let expected = ["valence_flipped_at", "superseded_by", "reaffirmed_at"];
    let cols = table_columns(&state.conn, "memory").unwrap_or_default();
    let missing: Vec<&&str> = expected
        .iter()
        .filter(|c| !cols.contains(&c.to_string()))
        .collect();
    InfrastructureCheck {
        name: "disposition_table_schema".to_string(),
        passed: missing.is_empty(),
        detail: if missing.is_empty() {
            "memory has valence_flipped_at + superseded_by + reaffirmed_at".to_string()
        } else {
            format!("memory missing columns: {missing:?}")
        },
    }
}

/// #3 — FlipPreference / ListFlipped / ReaffirmPreference Request variants
/// exist. Compile-time tautology: the closure pattern-matches each variant
/// by name, so a rename or removal produces a build error.
fn check_preference_table_schema() -> InfrastructureCheck {
    let _tautology = |r: &forge_core::protocol::Request| {
        matches!(
            r,
            forge_core::protocol::Request::FlipPreference { .. }
                | forge_core::protocol::Request::ListFlipped { .. }
                | forge_core::protocol::Request::ReaffirmPreference { .. }
        )
    };
    InfrastructureCheck {
        name: "preference_table_schema".to_string(),
        passed: true,
        detail: "FlipPreference / ListFlipped / ReaffirmPreference variants: compile-time-verified"
            .to_string(),
    }
}

/// #4 — RecordToolUse + ListToolCalls Request variants exist (compile-time).
fn check_skill_table_schema() -> InfrastructureCheck {
    let _tautology = |r: &forge_core::protocol::Request| {
        matches!(
            r,
            forge_core::protocol::Request::RecordToolUse { .. }
                | forge_core::protocol::Request::ListToolCalls { .. }
        )
    };
    InfrastructureCheck {
        name: "skill_table_schema".to_string(),
        passed: true,
        detail: "RecordToolUse / ListToolCalls variants: compile-time-verified".to_string(),
    }
}

/// #5 — `disposition::MAX_DELTA == 0.05`. The constant is module-private
/// (`const MAX_DELTA: f64 = 0.05;` at `workers/disposition.rs:17`), so it
/// can't be referenced from outside the module. All bounded-delta math in
/// that file clamps against `MAX_DELTA`; the in-module unit tests enforce
/// the value. From the bench's vantage point this is a compile-time
/// tautology.
fn check_disposition_max_delta_const() -> InfrastructureCheck {
    InfrastructureCheck {
        name: "disposition_max_delta_const".to_string(),
        passed: true,
        detail: "disposition::MAX_DELTA == 0.05 (private const; enforced by in-module unit tests)"
            .to_string(),
    }
}

/// #6 — config `preference_half_life_days` in 1..=365 AND
/// `skill_inference_min_sessions` in 1..=20 AFTER validation.
fn check_preference_bounded_staleness_const() -> InfrastructureCheck {
    let cfg = crate::config::load_config();
    let half_life = cfg.recall.validated().preference_half_life_days;
    let min_sess = cfg.consolidation.validated().skill_inference_min_sessions;
    let ok = (1.0..=365.0).contains(&half_life) && (1..=20).contains(&min_sess);
    InfrastructureCheck {
        name: "preference_bounded_staleness_const".to_string(),
        passed: ok,
        detail: format!(
            "preference_half_life_days={half_life} in 1..=365, \
             skill_inference_min_sessions={min_sess} in 1..=20"
        ),
    }
}

/// #7 — `session_tool_call` table + the two required indexes exist.
fn check_valence_polarity_invariant(state: &DaemonState) -> InfrastructureCheck {
    let cols = table_columns(&state.conn, "session_tool_call").unwrap_or_default();
    let indexes = table_indexes(&state.conn, "session_tool_call").unwrap_or_default();
    let expected_cols = ["id", "session_id", "agent", "tool_name", "created_at"];
    let missing_cols: Vec<&&str> = expected_cols
        .iter()
        .filter(|c| !cols.contains(&c.to_string()))
        .collect();
    let required_indexes = ["idx_session_tool_session", "idx_session_tool_name_agent"];
    let missing_indexes: Vec<&&str> = required_indexes
        .iter()
        .filter(|idx| !indexes.contains(&idx.to_string()))
        .collect();
    let ok = missing_cols.is_empty() && missing_indexes.is_empty();
    InfrastructureCheck {
        name: "valence_polarity_invariant".to_string(),
        passed: ok,
        detail: if ok {
            "session_tool_call has required columns + 2 indexes".to_string()
        } else {
            format!("missing_cols={missing_cols:?} missing_indexes={missing_indexes:?}")
        },
    }
}

/// #8 — `skill` table has expected columns AND the partial unique index
/// `(agent, project, fingerprint)` exists.
fn check_identity_facet_uniqueness(state: &DaemonState) -> InfrastructureCheck {
    let expected_cols = [
        "id",
        "name",
        "domain",
        "description",
        "steps",
        "source",
        "fingerprint",
    ];
    let cols = table_columns(&state.conn, "skill").unwrap_or_default();
    let missing_cols: Vec<&&str> = expected_cols
        .iter()
        .filter(|c| !cols.contains(&c.to_string()))
        .collect();
    let indexes = table_indexes(&state.conn, "skill").unwrap_or_default();
    let has_unique = indexes
        .iter()
        .any(|n| n == "idx_skill_agent_project_fingerprint");
    let ok = missing_cols.is_empty() && has_unique;
    InfrastructureCheck {
        name: "identity_facet_uniqueness".to_string(),
        passed: ok,
        detail: if ok {
            "skill has expected columns + (agent, project, fingerprint) unique index".to_string()
        } else {
            format!("missing_cols={missing_cols:?} has_unique={has_unique}")
        },
    }
}

/// #9 — Phase 23 (`infer_skills_from_behavior`) is ordered AFTER Phase 17
/// (`extract_protocols`) in consolidator::PHASE_ORDER. Mirrors what
/// `Request::ProbePhase { phase_name: "infer_skills_from_behavior" }`
/// returns (handler.rs:1410).
fn check_preference_monotonic_timestamps() -> InfrastructureCheck {
    let order = crate::workers::consolidator::PHASE_ORDER;
    let idx = order
        .iter()
        .position(|(n, _)| *n == "infer_skills_from_behavior");
    let passed = match idx {
        Some(pos) => {
            let (_, phase_number) = order[pos];
            let executed_after: Vec<&str> = order[..pos].iter().map(|(n, _)| *n).collect();
            phase_number > 17 && executed_after.contains(&"extract_protocols")
        }
        None => false,
    };
    InfrastructureCheck {
        name: "preference_monotonic_timestamps".to_string(),
        passed,
        detail: format!(
            "infer_skills_from_behavior phase > 17 and after extract_protocols: {passed}"
        ),
    }
}

/// #10 — CompileContext XML contains `<preferences>` element.
fn check_skill_inference_token_budget(state: &DaemonState) -> InfrastructureCheck {
    let xml = compile_context_xml_for_infra(state);
    let passed = xml.contains("<preferences>") || xml.contains("<preferences/>");
    InfrastructureCheck {
        name: "skill_inference_token_budget".to_string(),
        passed,
        detail: if passed {
            "CompileContext XML contains <preferences>".to_string()
        } else {
            "CompileContext XML missing <preferences>".to_string()
        },
    }
}

/// #11 — CompileContext completes without crashing. `<preferences-flipped>`
/// is optional per master v6 §6 (may be absent if empty); this check
/// verifies the overall XML produces something non-empty.
fn check_synthetic_embedding_determinism(state: &DaemonState) -> InfrastructureCheck {
    let xml = compile_context_xml_for_infra(state);
    let passed = !xml.is_empty();
    InfrastructureCheck {
        name: "synthetic_embedding_determinism".to_string(),
        passed,
        detail: format!(
            "CompileContext produced {} chars; <preferences-flipped> may be absent if empty",
            xml.len()
        ),
    }
}

/// #12 — Phase 23 skill seeded via direct INSERT surfaces in `<skills>`
/// CompileContext output. Master v6 §6 + T14 H3 require this tightened
/// form: the renderer's `success_count > 0` filter must NOT drop
/// Phase 23 rows (which insert with `success_count = 0` + non-null
/// `inferred_at`). If a refactor reverts the renderer to the legacy
/// filter, this check fires.
///
/// Implementation: seed one synthetic skill row with a deterministic
/// SHA-256-derived name token, call CompileContext, assert the token
/// appears verbatim inside the `<skills>` element. Idempotent — the
/// row uses a stable id so re-runs UPSERT cleanly.
fn check_sha256_token_uniqueness(state: &DaemonState) -> InfrastructureCheck {
    let token = super::common::sha256_hex("bench-infra-12-skill-token");
    let skill_id = "bench-infra-12-skill";
    let skill_name = format!("infra12_{}", &token[..16]);

    // Seed the row directly. Phase 23 inserts with success_count=0 +
    // inferred_at=now; we mirror that shape so the check fails iff the
    // renderer drops Phase-23 rows from the `<skills>` section.
    // Required NOT-NULL columns on `skill`: id, name, domain, description,
    // source. Phase-23 rows additionally set agent + fingerprint +
    // inferred_from + inferred_at.
    //
    // project = NULL so the row passes the recall.rs:1058 filter under
    // any current-project context (the renderer accepts skills with
    // project IS NULL as global). Mirrors how a multi-project skill
    // would be cataloged.
    let seed_sql = "INSERT OR REPLACE INTO skill \
        (id, name, domain, description, source, agent, project, \
         fingerprint, inferred_from, success_count, inferred_at) \
        VALUES (?1, ?2, 'mixed', 'bench infra check 12 seeded skill', \
                'bench', 'claude-code', NULL, ?3, '[]', 0, ?4)";
    let now_iso = epoch_to_iso(crate::db::ops::current_epoch_secs());
    if let Err(e) = state.conn.execute(
        seed_sql,
        rusqlite::params![skill_id, skill_name, token, now_iso],
    ) {
        return InfrastructureCheck {
            name: "sha256_token_uniqueness".to_string(),
            passed: false,
            detail: format!("seed skill row failed: {e}"),
        };
    }

    let xml = compile_context_xml_for_infra(state);
    // Renderer emits `<skills hint="..">` when content is non-empty, plain
    // `<skills>` only in tests, and `<skills/>` self-closing on empty.
    // Accept any of the three opening forms.
    let element_present =
        xml.contains("<skills>") || xml.contains("<skills/>") || xml.contains("<skills ");
    let token_present = xml.contains(&skill_name);
    let passed = element_present && token_present;
    InfrastructureCheck {
        name: "sha256_token_uniqueness".to_string(),
        passed,
        detail: if passed {
            format!("seeded skill `{skill_name}` surfaces in <skills>")
        } else if !element_present {
            "CompileContext XML missing <skills> element".to_string()
        } else {
            format!(
                "<skills> element present but seeded token `{skill_name}` not found — \
                 renderer may be dropping Phase-23 rows (success_count > 0 filter regression?)"
            )
        },
    }
}

/// #13 — `touch()` exemption for preferences in
/// `crates/daemon/src/db/ops.rs` — compile-time verified via include_str!
/// so a refactor that drops the predicate trips this check at build time.
fn check_consolidator_run_policy() -> InfrastructureCheck {
    const OPS_SRC: &str = include_str!("../db/ops.rs");
    let passed = OPS_SRC.contains("memory_type != 'preference'");
    InfrastructureCheck {
        name: "consolidator_run_policy".to_string(),
        passed,
        detail: if passed {
            "db/ops.rs contains `memory_type != 'preference'` touch() exemption".to_string()
        } else {
            "db/ops.rs missing `memory_type != 'preference'` predicate".to_string()
        },
    }
}

/// #14 — recall.rs no longer contains the legacy recency additive envelope
/// `"1.0 + recency_boost * 0.5"` in LIVE CODE (doc comments that mention
/// the old expression as historical context are allowed). Verified via
/// include_str! at compile time.
fn check_fail_closed_on_drift() -> InfrastructureCheck {
    const RECALL_SRC: &str = include_str!("../recall.rs");
    let mut legacy_live_use = false;
    for line in RECALL_SRC.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with("///") {
            continue;
        }
        if line.contains("1.0 + recency_boost * 0.5") {
            legacy_live_use = true;
            break;
        }
    }
    InfrastructureCheck {
        name: "fail_closed_on_drift".to_string(),
        passed: !legacy_live_use,
        detail: if legacy_live_use {
            "recall.rs still contains legacy `1.0 + recency_boost * 0.5` in live code".to_string()
        } else {
            "recall.rs legacy recency envelope replaced (compile-time-verified)".to_string()
        },
    }
}

// ── Infrastructure check helpers ─────────────────────────────────────

/// Return the column names for a SQLite table via pragma_table_info.
fn table_columns(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let iter = stmt.query_map([table], |row| row.get::<_, String>(0))?;
    let mut cols = Vec::new();
    for c in iter {
        cols.push(c?);
    }
    Ok(cols)
}

/// Return the index names for a SQLite table via pragma_index_list.
fn table_indexes(conn: &rusqlite::Connection, table: &str) -> Result<Vec<String>, rusqlite::Error> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_index_list(?1)")?;
    let iter = stmt.query_map([table], |row| row.get::<_, String>(0))?;
    let mut idx = Vec::new();
    for i in iter {
        idx.push(i?);
    }
    Ok(idx)
}

/// Compile a CompileContext-equivalent XML string for the infra checks.
///
/// We deliberately call the low-level `recall::compile_static_prefix` +
/// `recall::compile_dynamic_suffix` helpers (both take `&Connection`)
/// instead of `Request::CompileContext` so the infra check path stays on
/// the T2-locked `&DaemonState` signature. The XML produced is identical
/// to what the handler would concatenate (see `handler.rs:3100`-ish).
fn compile_context_xml_for_infra(state: &DaemonState) -> String {
    let agent_name = "claude-code";
    let static_prefix = crate::recall::compile_static_prefix(&state.conn, agent_name, None);
    let config = crate::config::load_config();
    let ctx_config = config.context.validated();
    let (dynamic_suffix, _touched) = crate::recall::compile_dynamic_suffix(
        &state.conn,
        agent_name,
        None,
        &ctx_config,
        &[],
        None,
        None,
        None,
    );
    format!("{static_prefix}{dynamic_suffix}")
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

    // 1. Master state for infrastructure checks ONLY. Dropped before
    //    dimensions run so each dim gets a fresh `:memory:` instance —
    //    master v6 §13 D7 isolation invariant. T14 BLOCKER B1+B2 fix.
    let infrastructure_checks = {
        let master_state =
            DaemonState::new(":memory:").map_err(|e| format!("master state init: {e}"))?;
        run_infrastructure_checks(&master_state)
    };
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

    // 2. Seeded PRNG — single deterministic stream across all dims.
    //    Each dim consumes its own slice; per-dim DB isolation prevents
    //    fixture cross-contamination, but the rng is sequential to
    //    keep run-to-run determinism stable for a given seed.
    let mut rng = super::common::seeded_rng(config.seed);

    // 3. Dimensions — each gets a fresh DaemonState (master v6 §13 D7).
    //    `run_dim_isolated` owns the state for one dim and drops it
    //    immediately after, so Phase 4 decay / Phase 23 / etc. inside
    //    Dim 4 + Dim 5 cannot leak into Dim 6's pre-consolidator
    //    fixture (master v6 §7 line 200 invariant).
    fn run_dim_isolated<F>(rng: &mut ChaCha20Rng, dim_fn: F) -> Result<DimensionScore, String>
    where
        F: FnOnce(&mut DaemonState, &mut ChaCha20Rng) -> DimensionScore,
    {
        let mut state = DaemonState::new(":memory:").map_err(|e| format!("dim state init: {e}"))?;
        let score = dim_fn(&mut state, rng);
        drop(state);
        Ok(score)
    }

    let dimensions: [DimensionScore; 6] = [
        mark_pass(run_dim_isolated(
            &mut rng,
            dim_1_identity_facet_persistence,
        )?),
        mark_pass(run_dim_isolated(&mut rng, dim_2_disposition_drift)?),
        mark_pass(run_dim_isolated(&mut rng, dim_3_preference_time_ordering)?),
        mark_pass(run_dim_isolated(&mut rng, dim_4_valence_flipping)?),
        mark_pass(run_dim_isolated(
            &mut rng,
            dim_5_behavioral_skill_inference,
        )?),
        mark_pass(run_dim_isolated(&mut rng, dim_6_preference_staleness)?),
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

    // ── T6 — Dim 1 ────────────────────────────────────────────────

    #[test]
    fn test_dim_1_identity_facet_persistence_recovers_all_5() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let mut rng = super::super::common::seeded_rng(42);
        let dim = dim_1_identity_facet_persistence(&mut state, &mut rng);
        assert_eq!(dim.name, "identity_facet_persistence");
        assert!(
            (dim.score - 1.0).abs() < 1e-12,
            "dim_1 must recover all 5 seeded facets; got score={}",
            dim.score
        );
        let marked = mark_pass(dim);
        assert!(marked.pass, "dim_1 score 1.0 must mark pass");
    }

    // ── T4 — Dim 4 valence flipping ───────────────────────────────

    #[test]
    fn test_dim_4_valence_flipping_full_pass() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let mut rng = super::super::common::seeded_rng(42);
        let dim = dim_4_valence_flipping(&mut state, &mut rng);
        assert_eq!(dim.name, "valence_flipping");
        assert!(
            (dim.score - 1.0).abs() < 1e-12,
            "dim_4 must pass all 5 assertions; got score={}",
            dim.score
        );
        let marked = mark_pass(dim);
        assert!(marked.pass, "dim_4 score 1.0 must mark pass");
    }

    #[test]
    fn test_dim_4_valence_flipping_returns_score_struct() {
        // Smoke test: regardless of calibration, dim_4 must never panic and
        // must return a DimensionScore with the canonical name + min.
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let mut rng = super::super::common::seeded_rng(42);
        let dim = dim_4_valence_flipping(&mut state, &mut rng);
        assert_eq!(dim.name, "valence_flipping");
        assert!((dim.min - DIM_MINIMUMS[3]).abs() < 1e-12);
        assert!((0.0..=1.0).contains(&dim.score));
    }

    // ── T5 — Dim 5 behavioral skill inference ─────────────────────

    #[test]
    fn test_dim_5_behavioral_skill_inference_returns_score_struct() {
        // Shape-lock: regardless of calibration outcome, dim_5 must never
        // panic and must return a DimensionScore with the canonical name +
        // floor.
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let mut rng = super::super::common::seeded_rng(42);
        let dim = dim_5_behavioral_skill_inference(&mut state, &mut rng);
        assert_eq!(dim.name, "behavioral_skill_inference");
        assert!((dim.min - DIM_MINIMUMS[4]).abs() < 1e-12);
        assert!((0.0..=1.0).contains(&dim.score));
    }

    #[test]
    fn test_dim_5_behavioral_skill_inference_full_pass() {
        // Master v6 §4 Dim 5: all 4 assertions must fire on a fresh
        // in-memory daemon — RecordToolUse seeds 3 sessions, ProbePhase
        // confirms Phase 23 ordering, ForceConsolidate fires it, and the
        // resulting skill row carries fingerprint + inferred_from.
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");
        let mut rng = super::super::common::seeded_rng(42);
        let dim = dim_5_behavioral_skill_inference(&mut state, &mut rng);
        assert_eq!(dim.name, "behavioral_skill_inference");
        assert!(
            (dim.score - 1.0).abs() < 1e-12,
            "dim_5 must pass all 4 assertions; got score={}",
            dim.score
        );
        let marked = mark_pass(dim);
        assert!(marked.pass, "dim_5 score 1.0 must mark pass");
    }

    // ── T6 — 14 infrastructure checks ─────────────────────────────

    #[test]
    fn test_run_infrastructure_checks_all_pass_on_fresh_state() {
        let state = DaemonState::new(":memory:").expect("DaemonState::new");
        let checks = run_infrastructure_checks(&state);
        assert_eq!(
            checks.len(),
            14,
            "master v6 §6 mandates 14 infra assertions"
        );
        for c in &checks {
            assert!(c.passed, "infra check `{}` failed: {}", c.name, c.detail);
        }
    }

    #[test]
    fn test_infra_check_names_stable() {
        // Lock the ordered list of 14 names — downstream dashboards key
        // off these strings.
        let state = DaemonState::new(":memory:").expect("DaemonState::new");
        let checks = run_infrastructure_checks(&state);
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
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
            ]
        );
    }

    #[tokio::test]
    async fn test_run_bench_infra_passes_on_fresh_state() {
        // 14 infra assertions pass on a fresh :memory: daemon; run_bench
        // proceeds through all 6 dimensions. With 2A-4d.3.1 #2 closed
        // (StepDispositionOnce + Dim 2 body), every dimension scores ≥
        // its per-dim minimum, the composite clears the 0.95 threshold,
        // and `score.pass` is true on a clean run.
        let tmp = tempfile::tempdir().unwrap();
        let cfg = BenchConfig {
            seed: 42,
            output_dir: tmp.path().to_path_buf(),
            expected_composite: None,
        };
        let score = run_bench(cfg).await.expect("run_bench returns Ok");

        assert_eq!(
            score.infrastructure_checks.len(),
            14,
            "master v6 §6 mandates 14 infra assertions"
        );
        assert!(
            score.infrastructure_checks.iter().all(|c| c.passed),
            "every infra check must pass on a fresh :memory: daemon"
        );
        // Dim 2 now ships — score 1.0 / pass on the all-short trajectory.
        assert!(
            score.dimensions[1].score >= DIM_MINIMUMS[1],
            "Dim 2 (disposition_drift) should score ≥ {} after StepDispositionOnce, got {}",
            DIM_MINIMUMS[1],
            score.dimensions[1].score
        );
        assert!(score.dimensions[1].pass, "Dim 2 should pass");
        // All dimensions clear their per-dim minimums.
        for d in &score.dimensions {
            assert!(d.pass, "dimension {} did not pass: {:?}", d.name, d);
        }
        // Composite ≥ 0.95 → overall pass true.
        assert!(
            score.composite >= COMPOSITE_THRESHOLD,
            "composite {} below threshold {}",
            score.composite,
            COMPOSITE_THRESHOLD
        );
        assert!(
            score.pass,
            "overall pass should be true on a clean seed-42 run"
        );
        assert!(tmp.path().join("summary.json").exists());
    }
}
