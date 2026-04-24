//! Types for the `Inspect` RPC (Phase 2A-4d.2 Observability API).
//!
//! Clients query `kpi_events` and the per-layer gauge snapshot through one
//! shape-parameterized RPC. See the Tier 2 design spec under
//! `docs/superpowers/specs/` for architectural context.

use serde::{Deserialize, Serialize};

/// One of five shapes a caller can request. Each shape has its own validity
/// matrix for `group_by` (documented in the Tier 2 spec §2.1).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectShape {
    /// Per-layer row counts + freshness. Reads the gauge snapshot.
    RowCount,
    /// Latency percentiles (p50/p95/p99/mean) per group.
    Latency,
    /// `errored / total` pass ratio per group.
    ErrorRate,
    /// Event counts + first/last timestamp per group.
    Throughput,
    /// Per-run_id summary: duration, phase count, error count, trace id.
    PhaseRunSummary,
}

/// Grouping dimension for shapes that support it. "No grouping" is modeled
/// as `Option::None` in the Request — no variant here represents it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectGroupBy {
    Phase,
    EventType,
    Project,
    RunId,
}

/// Filters applied to the underlying query. `layer` gates `shape=row_count`;
/// other shapes honor `phase`/`event_type`/`project`. The handler nulls out
/// dropped fields and returns them in `effective_filter`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", default)]
pub struct InspectFilter {
    pub layer: Option<String>,
    pub phase: Option<String>,
    pub event_type: Option<String>,
    pub project: Option<String>,
}

/// Response payload, tagged by shape.
///
/// `RowCount` data is served from the atomic `GaugeSnapshot` (see Tier 2 §2.3);
/// all other shapes aggregate `kpi_events` rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InspectData {
    RowCount { rows: Vec<LayerRow> },
    Latency { rows: Vec<LatencyRow> },
    ErrorRate { rows: Vec<ErrorRateRow> },
    Throughput { rows: Vec<ThroughputRow> },
    PhaseRunSummary { rows: Vec<PhaseRunRow> },
}

/// One Manas table's row count + staleness, sampled from the gauge snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerRow {
    pub layer: String,
    pub count: i64,
    /// Seconds since the gauge snapshot was last refreshed.
    pub snapshot_age_secs: u64,
    /// Seconds since the most recent write to THIS table. `None` when empty.
    pub freshness_secs: Option<i64>,
}

/// Per-group latency percentiles computed in Rust from `kpi_events.latency_ms`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatencyRow {
    pub group_key: String,
    pub count: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub mean_ms: f64,
    /// Samples dropped from this group's percentile calc because the
    /// per-group cap was hit. `0` when not truncated.
    pub truncated_samples: u64,
}

/// Per-group error rate from `kpi_events.metadata_json.$.error_count`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorRateRow {
    pub group_key: String,
    pub total: u64,
    pub errored: u64,
    pub rate: f64,
}

/// Per-group throughput across a window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThroughputRow {
    pub group_key: String,
    pub count: u64,
    pub first_ts_secs: i64,
    pub last_ts_secs: i64,
}

/// One consolidation pass rollup, identified by `run_id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhaseRunRow {
    pub run_id: String,
    pub start_ts_secs: i64,
    /// Sum of per-phase `latency_ms` — equals wall-clock pass time only when
    /// phases run sequentially (they currently do).
    pub phases_duration_ms_sum: u64,
    pub phase_count: u64,
    pub error_count: u64,
    pub trace_id: Option<String>,
    pub correlation_id: Option<String>,
}

/// Default window when a caller omits `window`.
pub fn default_inspect_window() -> String {
    "1h".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn inspect_shape_round_trips_as_snake_case() {
        for (shape, expected) in [
            (InspectShape::RowCount, "row_count"),
            (InspectShape::Latency, "latency"),
            (InspectShape::ErrorRate, "error_rate"),
            (InspectShape::Throughput, "throughput"),
            (InspectShape::PhaseRunSummary, "phase_run_summary"),
        ] {
            let json = serde_json::to_value(shape).unwrap();
            assert_eq!(json, json!(expected));
            let back: InspectShape = serde_json::from_value(json).unwrap();
            assert_eq!(back, shape);
        }
    }

    #[test]
    fn inspect_group_by_round_trips_as_snake_case() {
        for (group, expected) in [
            (InspectGroupBy::Phase, "phase"),
            (InspectGroupBy::EventType, "event_type"),
            (InspectGroupBy::Project, "project"),
            (InspectGroupBy::RunId, "run_id"),
        ] {
            let json = serde_json::to_value(group).unwrap();
            assert_eq!(json, json!(expected));
            let back: InspectGroupBy = serde_json::from_value(json).unwrap();
            assert_eq!(back, group);
        }
    }

    #[test]
    fn inspect_filter_defaults_to_all_none() {
        let filter: InspectFilter = serde_json::from_value(json!({})).unwrap();
        assert!(filter.layer.is_none());
        assert!(filter.phase.is_none());
        assert!(filter.event_type.is_none());
        assert!(filter.project.is_none());
    }

    #[test]
    fn inspect_filter_round_trips() {
        let original = InspectFilter {
            layer: Some("memory".into()),
            phase: Some("phase_1_exact_dedup".into()),
            event_type: Some("phase_completed".into()),
            project: None,
        };
        let json = serde_json::to_value(&original).unwrap();
        let back: InspectFilter = serde_json::from_value(json).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn inspect_data_row_count_serializes_with_kind_tag() {
        let data = InspectData::RowCount {
            rows: vec![LayerRow {
                layer: "memory".into(),
                count: 42,
                snapshot_age_secs: 5,
                freshness_secs: Some(120),
            }],
        };
        let json = serde_json::to_value(&data).unwrap();
        assert_eq!(json["kind"], "row_count");
        assert_eq!(json["rows"][0]["layer"], "memory");
        assert_eq!(json["rows"][0]["count"], 42);
        let back: InspectData = serde_json::from_value(json).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn layer_row_empty_table_serializes_freshness_as_null() {
        let row = LayerRow {
            layer: "entity".into(),
            count: 0,
            snapshot_age_secs: 1,
            freshness_secs: None,
        };
        let json = serde_json::to_value(&row).unwrap();
        assert!(json["freshness_secs"].is_null());
    }

    #[test]
    fn latency_row_with_truncation_round_trips() {
        let row = LatencyRow {
            group_key: "phase_23_infer_skills_from_behavior".into(),
            count: 20000,
            p50_ms: 12.5,
            p95_ms: 48.0,
            p99_ms: 102.3,
            mean_ms: 17.9,
            truncated_samples: 500,
        };
        let json = serde_json::to_value(&row).unwrap();
        let back: LatencyRow = serde_json::from_value(json).unwrap();
        assert_eq!(back, row);
    }

    #[test]
    fn phase_run_row_handles_null_trace_id() {
        let row = PhaseRunRow {
            run_id: "01HXYZPHASETEST".into(),
            start_ts_secs: 1_745_500_000,
            phases_duration_ms_sum: 1234,
            phase_count: 23,
            error_count: 0,
            trace_id: None,
            correlation_id: Some("01HXYZPHASETEST".into()),
        };
        let json = serde_json::to_value(&row).unwrap();
        assert!(json["trace_id"].is_null());
        let back: PhaseRunRow = serde_json::from_value(json).unwrap();
        assert_eq!(back, row);
    }

    #[test]
    fn default_window_is_one_hour() {
        assert_eq!(default_inspect_window(), "1h");
    }
}
