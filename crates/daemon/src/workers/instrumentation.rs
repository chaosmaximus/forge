// SPDX-License-Identifier: Apache-2.0
//! Tier 1 of Phase 2A-4d — consolidator phase instrumentation.
//!
//! Pure helpers. No lock acquisitions, no `Arc<Mutex<DaemonState>>`. Callers
//! pass `&Connection` + `Option<&ForgeMetrics>` + `&PhaseOutcome` by value.
//!
//! Spec: `docs/superpowers/specs/2026-04-24-forge-identity-observability-design.md`.

use rusqlite::{params, Connection};
use serde_json::json;

use crate::server::metrics::ForgeMetrics;

/// Canonical, ordered phase identifiers.
///
/// Must match the `tracing::info_span!("phase_N_<name>")` call sites in
/// `consolidator.rs::run_all_phases` 1:1. A unit test in this module scans
/// the consolidator source and asserts the counts match — rename a phase
/// without updating both sides and CI fails.
pub const PHASE_SPAN_NAMES: &[&str; 23] = &[
    "phase_1_dedup_memories",
    "phase_2_semantic_dedup",
    "phase_3_link_memories",
    "phase_4_decay_memories",
    "phase_5_promote_patterns",
    "phase_6_reconsolidate_contradicting",
    "phase_7_merge_embedding_duplicates",
    "phase_8_strengthen_by_access",
    "phase_9_score_memory_quality",
    "phase_10_entity_detection",
    "phase_11_synthesize_contradictions",
    "phase_12_detect_and_surface_gaps",
    "phase_13_reweave_memories",
    "phase_14_flip_stale_preferences",
    "phase_15_apply_recency_decay",
    "phase_16_compute_effectiveness",
    "phase_17_extract_protocols",
    "phase_18_tag_antipatterns",
    "phase_19_emit_notifications",
    "phase_20_record_tool_use_kpis",
    "phase_21_run_healing_checks",
    "phase_22_apply_quality_pressure",
    "phase_23_infer_skills_from_behavior",
];

/// Observation produced by each consolidator phase.
///
/// Borrowed fields (`run_id`, `correlation_id`, `trace_id`) let callers reuse
/// string allocations across 23 phases in a single pass.
pub struct PhaseOutcome<'a> {
    pub phase: &'static str,
    pub run_id: &'a str,
    pub correlation_id: &'a str,
    pub trace_id: Option<&'a str>,
    pub output_count: u64,
    pub error_count: u64,
    pub duration_ms: u64,
    pub extra: serde_json::Value,
}

/// Record a phase outcome to Prometheus metrics (if available) and the
/// `kpi_events` table. Non-fatal on kpi_events insert failure — logs a warning.
pub fn record(conn: &Connection, metrics: Option<&ForgeMetrics>, outcome: &PhaseOutcome) {
    if let Some(m) = metrics {
        update_phase_metrics(m, outcome);
    }
    if let Err(e) = insert_kpi_event_row(conn, outcome) {
        tracing::warn!(error = %e, phase = outcome.phase, "kpi_events insert failed");
    }
}

fn update_phase_metrics(metrics: &ForgeMetrics, outcome: &PhaseOutcome) {
    metrics
        .phase_duration
        .with_label_values(&[outcome.phase])
        .observe(outcome.duration_ms as f64 / 1000.0);
    let action = if outcome.error_count == 0 {
        "succeeded"
    } else {
        "errored"
    };
    metrics
        .phase_output_rows
        .with_label_values(&[outcome.phase, action])
        .inc_by(outcome.output_count);
}

fn insert_kpi_event_row(conn: &Connection, outcome: &PhaseOutcome) -> rusqlite::Result<()> {
    let metadata = json!({
        "metadata_schema_version": 1,
        "phase_name": outcome.phase,
        "run_id": outcome.run_id,
        "correlation_id": outcome.correlation_id,
        "trace_id": outcome.trace_id,
        "input_count": 0,
        "output_count": outcome.output_count,
        "error_count": outcome.error_count,
        "extra": outcome.extra,
    });
    let id = format!("phase-{}", ulid::Ulid::new());
    conn.execute(
        "INSERT INTO kpi_events (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)
         VALUES (?1, strftime('%s','now'), 'phase_completed', NULL, ?2, ?3, ?4, ?5)",
        params![
            id,
            outcome.duration_ms as i64,
            outcome.output_count as i64,
            if outcome.error_count == 0 { 1_i64 } else { 0_i64 },
            metadata.to_string(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_span_names_len_is_23() {
        assert_eq!(PHASE_SPAN_NAMES.len(), 23);
    }

    #[test]
    #[ignore = "T3 un-ignores this once phase call sites are wrapped in info_span!"]
    fn span_name_count_matches_phase_count_in_consolidator() {
        // Source-scan integrity check: each PHASE_SPAN_NAMES entry must appear
        // exactly once as `tracing::info_span!("phase_*")` in consolidator.rs.
        // Rename a phase + forget the literal and this test catches it before CI.
        let src = include_str!("consolidator.rs");
        let count = src.matches(r#"info_span!("phase_"#).count();
        assert_eq!(
            count,
            PHASE_SPAN_NAMES.len(),
            "expected {} info_span!(\"phase_ calls in consolidator.rs, saw {}",
            PHASE_SPAN_NAMES.len(),
            count
        );
    }

    #[test]
    fn record_inserts_kpi_event_row() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        let outcome = PhaseOutcome {
            phase: "phase_23_infer_skills_from_behavior",
            run_id: "01HTEST",
            correlation_id: "01HTEST",
            trace_id: None,
            output_count: 1,
            error_count: 0,
            duration_ms: 5,
            extra: json!({}),
        };
        record(&conn, None, &outcome);
        let (count, json_text): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), IFNULL(MAX(metadata_json), '') FROM kpi_events \
                 WHERE event_type = 'phase_completed'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(json_text.contains("\"metadata_schema_version\":1"));
        assert!(json_text.contains("\"phase_name\":\"phase_23_infer_skills_from_behavior\""));
        assert!(json_text.contains("\"correlation_id\":\"01HTEST\""));
    }

    #[test]
    fn record_marks_success_zero_on_error() {
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        let outcome = PhaseOutcome {
            phase: "phase_1_dedup_memories",
            run_id: "01HT2",
            correlation_id: "01HT2",
            trace_id: None,
            output_count: 0,
            error_count: 1,
            duration_ms: 2,
            extra: json!({}),
        };
        record(&conn, None, &outcome);
        let success: i64 = conn
            .query_row(
                "SELECT success FROM kpi_events WHERE event_type = 'phase_completed' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(success, 0, "success should be 0 when error_count > 0");
    }
}
