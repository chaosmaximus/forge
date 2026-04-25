// SPDX-License-Identifier: Apache-2.0
//! Tier 1 of Phase 2A-4d — consolidator phase instrumentation.
//!
//! Pure helpers. No lock acquisitions, no `Arc<Mutex<DaemonState>>`. Callers
//! pass `&Connection` + `Option<&ForgeMetrics>` + `&PhaseOutcome` by value.
//!
//! Spec: `docs/superpowers/specs/2026-04-24-forge-identity-observability-design.md`.

use opentelemetry::trace::{TraceContextExt, TraceId};
use rusqlite::{params, Connection};
use serde_json::json;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::server::metrics::ForgeMetrics;

/// Extract the 32-char hex trace id from the currently-active `tracing::Span`
/// via its attached OpenTelemetry context.
///
/// Returns `None` when no OTLP provider is wired up (tracing-opentelemetry
/// then yields an all-zeros `TraceId`). This preserves the documented
/// contract that `trace_id` is `null` in `kpi_events.metadata_json` whenever
/// OTLP is disabled.
pub(crate) fn current_otlp_trace_id() -> Option<String> {
    let span = tracing::Span::current();
    let otel_context = span.context();
    let span_ref = otel_context.span();
    let trace_id = span_ref.span_context().trace_id();
    if trace_id == TraceId::INVALID {
        None
    } else {
        Some(trace_id.to_string())
    }
}

/// Canonical, ordered phase identifiers.
///
/// Must match the `tracing::info_span!("phase_N_<name>")` call sites in
/// `consolidator.rs::run_all_phases` 1:1. A unit test in this module scans
/// the consolidator source and asserts the counts match — rename a phase
/// without updating both sides and CI fails.
// Execution order in `run_all_phases` as of 2026-04-24. Numeric suffix is the
// phase's historical id (Phase 23 was inserted between 17 and 18 in 2A-4c2);
// list order here matches run-time firing order, not numeric id.
pub const PHASE_SPAN_NAMES: &[&str; 23] = &[
    "phase_1_dedup_memories",
    "phase_2_semantic_dedup",
    "phase_3_link_memories",
    "phase_4_decay_memories",
    "phase_5_promote_patterns",
    "phase_6_reconsolidate_contradicting",
    "phase_7_merge_embedding_duplicates",
    "phase_8_strengthen_by_access",
    "phase_9_detect_contradictions",
    "phase_10_decay_activation",
    "phase_11_entity_detection",
    "phase_12_synthesize_contradictions",
    "phase_13_detect_gaps",
    "phase_14_reweave_memories",
    "phase_15_quality_scoring",
    "phase_16_portability_classification",
    "phase_17_extract_protocols",
    "phase_23_infer_skills_from_behavior",
    "phase_18_tag_antipatterns",
    "phase_19_emit_notifications",
    "phase_20_auto_supersede",
    "phase_21_session_staleness_fade",
    "phase_22_apply_quality_pressure",
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
/// `kpi_events` table. `kpi_events` persistence failures are NOT counted
/// against the phase's own error budget — they go to a dedicated
/// `forge_phase_persistence_errors_total` counter so operators can tell a
/// phase-level failure (Phase 9 query blew up) apart from an
/// instrumentation-layer failure (SQLite INSERT OR IGNORE absorbed the row,
/// or the statement returned an error). Prior to this split, a phase that
/// hit BOTH kinds of error in one call was double-counted under
/// `phase_output_rows{action="errored"}`.
pub fn record(conn: &Connection, metrics: Option<&ForgeMetrics>, outcome: &PhaseOutcome) {
    if let Some(m) = metrics {
        update_phase_metrics(m, outcome);
    }
    // Honor an explicit override from the caller (tests, replayers), but
    // when `outcome.trace_id` is None auto-populate from the currently-
    // active tracing span's attached OTLP context. All-zeros trace ids
    // (no provider installed) collapse to None so the stored JSON stays
    // `"trace_id":null`.
    let computed_trace_id = if outcome.trace_id.is_some() {
        None
    } else {
        current_otlp_trace_id()
    };
    match insert_kpi_event_row(conn, outcome, computed_trace_id.as_deref()) {
        Ok(rows_written) => {
            if rows_written == 0 {
                // ULID PK collision (2^-80 per monotonic-ms window).
                tracing::warn!(
                    phase = outcome.phase,
                    "kpi_events insert hit ULID PK collision — row dropped"
                );
                if let Some(m) = metrics {
                    m.phase_persistence_errors
                        .with_label_values(&[outcome.phase, "ulid_collision"])
                        .inc();
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, phase = outcome.phase, "kpi_events insert failed");
            if let Some(m) = metrics {
                m.phase_persistence_errors
                    .with_label_values(&[outcome.phase, "insert_error"])
                    .inc();
            }
        }
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

fn insert_kpi_event_row(
    conn: &Connection,
    outcome: &PhaseOutcome,
    computed_trace_id: Option<&str>,
) -> rusqlite::Result<usize> {
    // Caller-supplied `outcome.trace_id` wins; otherwise fall back to the
    // trace id we pulled off the active OTLP-backed span. Either way, an
    // absent value stays as JSON null rather than the all-zeros sentinel
    // so dashboards / schema consumers see a clean null when OTLP is off.
    let trace_id_for_json = outcome.trace_id.or(computed_trace_id);
    let metadata = json!({
        "metadata_schema_version": 1,
        "phase_name": outcome.phase,
        "run_id": outcome.run_id,
        "correlation_id": outcome.correlation_id,
        "trace_id": trace_id_for_json,
        "output_count": outcome.output_count,
        "error_count": outcome.error_count,
        "extra": outcome.extra,
    });
    let id = format!("phase-{}", ulid::Ulid::new());
    // INSERT OR IGNORE: on the astronomically rare ULID PK collision we'd
    // rather drop the row than error — collisions at the 80-bit random
    // component per-ms are ~2^-80 per monotonic tick. Return rows_affected
    // so `record()` can tell "inserted" apart from "ignored" and emit a
    // metric for the latter. Avoids silent divergence between Prometheus
    // and kpi_events.
    // Phase 2A-4d.2.1 #4 (W7): write run_id to its own indexed column
    // so the HUD 24h rollup can dedupe via index walk rather than
    // full-table JSON scan. Same value also stays in metadata_json
    // for v1-schema compatibility.
    conn.execute(
        "INSERT OR IGNORE INTO kpi_events (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json, run_id)
         VALUES (?1, strftime('%s','now'), 'phase_completed', NULL, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            outcome.duration_ms as i64,
            outcome.output_count as i64,
            if outcome.error_count == 0 { 1_i64 } else { 0_i64 },
            metadata.to_string(),
            outcome.run_id,
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_span_names_len_is_23() {
        assert_eq!(PHASE_SPAN_NAMES.len(), 23);
    }

    #[test]
    fn span_name_count_matches_phase_count_in_consolidator() {
        // Source-scan integrity check: each PHASE_SPAN_NAMES entry must appear
        // exactly once as `tracing::info_span!("phase_*")` in consolidator.rs.
        // Rename a phase + forget the literal and this test catches it before CI.
        //
        // Earlier version counted the total number of `info_span!("phase_`
        // occurrences, which silently accepted duplicates. This per-name loop
        // rejects both missing names *and* names with >1 literal occurrence.
        //
        // Phase 2A-4d.1.1 #4: prior version did a raw `src.matches(needle)`
        // on the unfiltered include_str! buffer, so a rustdoc/line/block
        // comment anywhere in consolidator.rs containing the literal
        // `info_span!("phase_X")` would push the per-name count to >=2 and
        // fail the guard for a non-bug reason. Strip line + block comments
        // before counting (string literals are intentionally kept — the
        // search needle IS a string literal, so blanking strings would
        // defeat the count). A `syn`-based AST pass would catch the
        // remaining string-literal false-positive class but pulls a
        // proc-macro dep into the test compile graph for one test —
        // tracked as still-deferred under the 2A-4d.1.1 follow-up backlog.
        let raw_src = include_str!("consolidator.rs");
        let src = strip_comments_for_test(raw_src);
        let total = src.matches(r#"info_span!("phase_"#).count();
        assert_eq!(
            total,
            PHASE_SPAN_NAMES.len(),
            "expected {} total info_span!(\"phase_ calls in consolidator.rs (after stripping comments + string literals), saw {}",
            PHASE_SPAN_NAMES.len(),
            total
        );
        for name in PHASE_SPAN_NAMES {
            let needle = format!(r#"info_span!("{name}""#);
            let count = src.matches(needle.as_str()).count();
            assert_eq!(
                count, 1,
                "phase span `{name}` should appear exactly once in consolidator.rs after stripping comments + string literals (found {count})"
            );
        }
    }

    /// Strip Rust line + block comments from a source buffer, replacing
    /// each character with a space so line numbers and offsets stay
    /// stable. **Keeps string literals intact** — the integrity test
    /// matches `info_span!("phase_X")` literally, so blanking out the
    /// `"phase_X"` body would defeat the count.
    ///
    /// This neutralises the documented false-positive class flagged by
    /// the T12 review — a rustdoc comment containing
    /// `info_span!("phase_X")` no longer counts. Test-string false
    /// positives (e.g. a string literal embedding the macro form) are
    /// left as a known limitation; in practice no consolidator.rs
    /// string contains that fragment, and a syn-based AST pass would
    /// be the proper long-term fix (tracked in the 2A-4d.1.1 backlog).
    fn strip_comments_for_test(src: &str) -> String {
        enum State {
            Code,
            LineComment,
            BlockComment,
        }
        let mut out = String::with_capacity(src.len());
        let mut state = State::Code;
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match state {
                State::LineComment => {
                    if bytes[i] == b'\n' {
                        out.push('\n');
                        state = State::Code;
                    } else {
                        out.push(' ');
                    }
                    i += 1;
                }
                State::BlockComment => {
                    if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        out.push_str("  ");
                        state = State::Code;
                        i += 2;
                    } else {
                        out.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
                        i += 1;
                    }
                }
                State::Code => {
                    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                        out.push_str("  ");
                        state = State::LineComment;
                        i += 2;
                    } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        out.push_str("  ");
                        state = State::BlockComment;
                        i += 2;
                    } else {
                        out.push(bytes[i] as char);
                        i += 1;
                    }
                }
            }
        }
        out
    }

    #[test]
    fn strip_comments_for_test_neutralises_doc_and_block_comments() {
        // The reviewer's named false-positive class: doc comments or
        // line/block comments containing `info_span!("phase_X")` must
        // not be counted as real macro invocations.
        let src = "/// info_span!(\"phase_doc\")\n\
                   // info_span!(\"phase_line\")\n\
                   /* info_span!(\"phase_block\") */\n\
                   let _x = info_span!(\"phase_real\");\n";
        let stripped = strip_comments_for_test(src);
        assert!(
            !stripped.contains("info_span!(\"phase_doc\""),
            "doc comment must be neutralised; got: {stripped}"
        );
        assert!(
            !stripped.contains("info_span!(\"phase_line\""),
            "line comment must be neutralised; got: {stripped}"
        );
        assert!(
            !stripped.contains("info_span!(\"phase_block\""),
            "block comment must be neutralised; got: {stripped}"
        );
        assert!(
            stripped.contains("info_span!(\"phase_real\""),
            "real call must survive; got: {stripped}"
        );
    }

    #[test]
    fn metadata_json_contains_all_v1_contract_fields() {
        // Regression test for HIGH-3: if the metadata_json v1 contract ever
        // drops or renames a field, the /inspect (Tier 2) API breaks. Lock
        // in every documented key.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        let outcome = PhaseOutcome {
            phase: "phase_1_dedup_memories",
            run_id: "01HFIELD",
            correlation_id: "01HFIELD",
            trace_id: Some("abc123"),
            output_count: 42,
            error_count: 1,
            duration_ms: 7,
            extra: json!({"some_key": "some_value"}),
        };
        record(&conn, None, &outcome);
        let json_text: String = conn
            .query_row(
                "SELECT metadata_json FROM kpi_events WHERE event_type = 'phase_completed' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for key in [
            "metadata_schema_version",
            "phase_name",
            "run_id",
            "correlation_id",
            "trace_id",
            "output_count",
            "error_count",
            "extra",
        ] {
            let needle = format!("\"{key}\"");
            assert!(
                json_text.contains(&needle),
                "v1 metadata_json must include field `{key}`; got {json_text}"
            );
        }
        // Schema version pinned to 1 so downstream consumers can reject
        // unknown versions deterministically.
        assert!(json_text.contains("\"metadata_schema_version\":1"));
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

    #[test]
    fn kpi_insert_error_routes_to_persistence_counter_not_phase_output() {
        // Regression test for T12 review finding HIGH-1: when kpi_events
        // INSERT fails, the persistence error must not be double-counted
        // as a phase-level error. Drop the kpi_events table to force the
        // INSERT to fail, then confirm:
        //   - phase_output_rows{errored} is NOT bumped by record()'s
        //     failure path (it only reflects outcome.error_count),
        //   - phase_persistence_errors{kind="insert_error"} IS bumped.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        conn.execute("DROP TABLE kpi_events", []).unwrap();

        let metrics = crate::server::metrics::ForgeMetrics::new();
        let outcome = PhaseOutcome {
            phase: "phase_1_dedup_memories",
            run_id: "01HINS",
            correlation_id: "01HINS",
            trace_id: None,
            output_count: 5,
            error_count: 0,
            duration_ms: 1,
            extra: json!({}),
        };
        record(&conn, Some(&metrics), &outcome);

        let errored = metrics
            .phase_output_rows
            .with_label_values(&[outcome.phase, "errored"])
            .get();
        assert_eq!(
            errored, 0,
            "phase_output_rows{{errored}} must not be bumped by an insert_error"
        );
        let succeeded = metrics
            .phase_output_rows
            .with_label_values(&[outcome.phase, "succeeded"])
            .get();
        assert_eq!(
            succeeded, 5,
            "phase_output_rows{{succeeded}} should reflect output_count since the phase itself succeeded"
        );
        let persistence_err = metrics
            .phase_persistence_errors
            .with_label_values(&[outcome.phase, "insert_error"])
            .get();
        assert_eq!(
            persistence_err, 1,
            "phase_persistence_errors{{insert_error}} must be bumped by the failed kpi insert"
        );
    }

    #[test]
    fn record_trace_id_is_null_without_otlp_provider() {
        // Negative regression for the all-zeros → null collapse. When no
        // OpenTelemetry provider is active the default tracer returns the
        // INVALID `TraceId` (32 zeros). We must store `"trace_id":null`
        // in metadata_json rather than leaking the zeros sentinel.
        //
        // Paired with `record_populates_trace_id_from_active_otlp_span`
        // below which exercises the positive path with a real tracer.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        let _enter = tracing::info_span!("unit_test_no_otlp").entered();
        let outcome = PhaseOutcome {
            phase: "phase_1_dedup_memories",
            run_id: "01HNOP",
            correlation_id: "01HNOP",
            trace_id: None,
            output_count: 0,
            error_count: 0,
            duration_ms: 1,
            extra: json!({}),
        };
        record(&conn, None, &outcome);
        let json_text: String = conn
            .query_row(
                "SELECT metadata_json FROM kpi_events WHERE event_type = 'phase_completed' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            json_text.contains("\"trace_id\":null"),
            "without an OTLP provider, trace_id must serialize to null; got {json_text}"
        );
        assert!(
            !json_text.contains("00000000000000000000000000000000"),
            "all-zeros TraceId sentinel must never leak into metadata_json; got {json_text}"
        );
    }

    #[test]
    fn record_populates_trace_id_from_active_otlp_span() {
        // Positive regression: wire a real in-process OpenTelemetry tracer
        // provider (no exporter — default builder keeps spans in memory
        // and discards on drop), open a `tracing::info_span!` whose OTLP
        // parent context points at a real span on that tracer, and
        // confirm that `record()` picks up the 32-char hex trace id.
        //
        // `OpenTelemetrySpanExt::set_parent` only survives `span.context()`
        // lookups when a subscriber with the `tracing_opentelemetry::layer`
        // is active. We install one *locally* via
        // `tracing_subscriber::with_default` so no global state leaks into
        // other tests in the same process.
        use opentelemetry::trace::{TraceContextExt, Tracer, TracerProvider as _};
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        use tracing_subscriber::layer::SubscriberExt;

        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();

        let provider = opentelemetry_sdk::trace::TracerProvider::builder().build();
        let tracer = provider.tracer("forge-test");
        let otlp_layer = tracing_opentelemetry::layer().with_tracer(tracer.clone());
        let subscriber = tracing_subscriber::registry().with(otlp_layer);

        let otel_span = tracer.start("parent-trace");
        let cx = opentelemetry::Context::current_with_span(otel_span);
        let expected_trace_id = cx.span().span_context().trace_id().to_string();
        assert_eq!(
            expected_trace_id.len(),
            32,
            "OTLP TraceId must render as 32 hex chars"
        );
        assert_ne!(
            expected_trace_id, "00000000000000000000000000000000",
            "real provider must mint a non-zero trace id"
        );

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!("phase_under_test");
            span.set_parent(cx);
            let _enter = span.enter();

            let outcome = PhaseOutcome {
                phase: "phase_1_dedup_memories",
                run_id: "01HOTL",
                correlation_id: "01HOTL",
                trace_id: None,
                output_count: 0,
                error_count: 0,
                duration_ms: 1,
                extra: json!({}),
            };
            record(&conn, None, &outcome);
        });

        let json_text: String = conn
            .query_row(
                "SELECT metadata_json FROM kpi_events WHERE event_type = 'phase_completed' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let needle = format!("\"trace_id\":\"{expected_trace_id}\"");
        assert!(
            json_text.contains(&needle),
            "metadata_json should carry the active OTLP span's trace id ({expected_trace_id}); got {json_text}"
        );
    }

    #[test]
    fn record_caller_override_beats_span_trace_id() {
        // Contract: explicit `outcome.trace_id = Some(...)` must win over
        // any span-derived value. Replayers and backfill jobs rely on
        // this to stamp historical trace ids onto newly written rows.
        crate::db::vec::init_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        crate::db::schema::create_schema(&conn).unwrap();
        let outcome = PhaseOutcome {
            phase: "phase_1_dedup_memories",
            run_id: "01HOVR",
            correlation_id: "01HOVR",
            trace_id: Some("deadbeefdeadbeefdeadbeefdeadbeef"),
            output_count: 0,
            error_count: 0,
            duration_ms: 1,
            extra: json!({}),
        };
        record(&conn, None, &outcome);
        let json_text: String = conn
            .query_row(
                "SELECT metadata_json FROM kpi_events WHERE event_type = 'phase_completed' LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            json_text.contains("\"trace_id\":\"deadbeefdeadbeefdeadbeefdeadbeef\""),
            "caller-supplied trace_id must be preserved verbatim; got {json_text}"
        );
    }
}
