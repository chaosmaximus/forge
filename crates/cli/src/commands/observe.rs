//! Phase 2A-4d.2 T8 — `forge-next observe` subcommand.
//!
//! Client-side mirror of `forge_core::protocol::Inspect*` types. `forge-core`
//! has no clap dep, so we define local clap-enabled enums here and convert.
//!
//! Responsibilities:
//! * Validate the `window` flag client-side (reject empty / zero / > 7 days).
//! * Build a `Request::Inspect` and dispatch it over the existing client.
//! * Render the response as a compact ASCII table (TTY default) or pretty JSON.
//!
//! Keep this module self-contained: the table renderer is an inline helper so
//! we don't drag in a `tabled` dep.
//!
//! Spec: `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier2-design.md`

use std::io::IsTerminal;

use forge_core::protocol::{
    InspectData, InspectFilter, InspectGroupBy, InspectShape, Request, Response, ResponseData,
};

use crate::client;

// ── Local clap-enabled mirrors of forge-core protocol enums ──────────────

/// Mirror of `forge_core::protocol::InspectShape` with a `clap::ValueEnum` impl.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ObserveShape {
    RowCount,
    Latency,
    ErrorRate,
    Throughput,
    PhaseRunSummary,
}

impl From<ObserveShape> for InspectShape {
    fn from(s: ObserveShape) -> Self {
        match s {
            ObserveShape::RowCount => InspectShape::RowCount,
            ObserveShape::Latency => InspectShape::Latency,
            ObserveShape::ErrorRate => InspectShape::ErrorRate,
            ObserveShape::Throughput => InspectShape::Throughput,
            ObserveShape::PhaseRunSummary => InspectShape::PhaseRunSummary,
        }
    }
}

/// Mirror of `forge_core::protocol::InspectGroupBy` with a `clap::ValueEnum` impl.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ObserveGroupBy {
    Phase,
    EventType,
    Project,
    RunId,
}

impl From<ObserveGroupBy> for InspectGroupBy {
    fn from(g: ObserveGroupBy) -> Self {
        match g {
            ObserveGroupBy::Phase => InspectGroupBy::Phase,
            ObserveGroupBy::EventType => InspectGroupBy::EventType,
            ObserveGroupBy::Project => InspectGroupBy::Project,
            ObserveGroupBy::RunId => InspectGroupBy::RunId,
        }
    }
}

/// Output format for the `observe` subcommand. If omitted, the CLI auto-picks
/// `Table` for TTYs and `Json` otherwise.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

// ── Client-side window validation ────────────────────────────────────────

/// Mirrors `crates/daemon/src/server/inspect.rs::parse_window_secs` to catch
/// bad input before a round-trip. The daemon re-validates.
const MAX_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;

/// Returns Ok(()) iff the window string parses to a duration in `(0, 7 days]`.
fn validate_window(window: &str) -> Result<(), String> {
    let trimmed = window.trim();
    if trimmed.is_empty() {
        return Err("window is empty".to_string());
    }
    let dur = humantime::parse_duration(trimmed)
        .map_err(|e| format!("invalid window '{window}': {e}"))?;
    let secs = dur.as_secs();
    if secs == 0 {
        return Err(format!("window '{window}' parses to zero duration"));
    }
    if secs > MAX_WINDOW_SECS {
        return Err(format!(
            "window '{window}' exceeds 7-day ceiling ({secs}s > {MAX_WINDOW_SECS}s)"
        ));
    }
    Ok(())
}

// ── Handler ──────────────────────────────────────────────────────────────

/// Entry point: construct the request, send it, render the response.
#[allow(clippy::too_many_arguments)]
pub async fn observe(
    shape: ObserveShape,
    window: String,
    layer: Option<String>,
    phase: Option<String>,
    event_type: Option<String>,
    project: Option<String>,
    group_by: Option<ObserveGroupBy>,
    format: Option<OutputFormat>,
) {
    if let Err(e) = validate_window(&window) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }

    let request = Request::Inspect {
        shape: shape.into(),
        window,
        filter: InspectFilter {
            layer,
            phase,
            event_type,
            project,
        },
        group_by: group_by.map(Into::into),
    };

    // Resolve format: explicit > TTY autodetect.
    let resolved_format = format.unwrap_or_else(|| {
        if std::io::stdout().is_terminal() {
            OutputFormat::Table
        } else {
            OutputFormat::Json
        }
    });

    match client::send(&request).await {
        Ok(response @ Response::Ok { .. }) => match resolved_format {
            OutputFormat::Json => print_json(&response),
            OutputFormat::Table => print_table(&response),
        },
        Ok(Response::Error { message }) => {
            eprintln!("error: {message}");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn print_json(response: &Response) {
    match serde_json::to_string_pretty(response) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("error: failed to serialize response: {e}");
            std::process::exit(1);
        }
    }
}

fn print_table(response: &Response) {
    let Response::Ok {
        data:
            ResponseData::Inspect {
                shape,
                window,
                window_secs,
                effective_filter,
                effective_group_by,
                stale,
                truncated,
                data,
                ..
            },
    } = response
    else {
        // Defensive: observe() only dispatches Ok-with-Inspect to this path,
        // but we don't panic if something upstream changes.
        eprintln!("error: unexpected response shape for observe");
        std::process::exit(1);
    };

    // Header line — one-liner context.
    let shape_name = match shape {
        InspectShape::RowCount => "row_count",
        InspectShape::Latency => "latency",
        InspectShape::ErrorRate => "error_rate",
        InspectShape::Throughput => "throughput",
        InspectShape::PhaseRunSummary => "phase_run_summary",
    };
    println!("shape={shape_name}  window={window} ({window_secs}s)");

    // Effective filter / group_by echo (only non-default fields).
    let mut meta: Vec<String> = Vec::new();
    if let Some(v) = &effective_filter.layer {
        meta.push(format!("layer={v}"));
    }
    if let Some(v) = &effective_filter.phase {
        meta.push(format!("phase={v}"));
    }
    if let Some(v) = &effective_filter.event_type {
        meta.push(format!("event_type={v}"));
    }
    if let Some(v) = &effective_filter.project {
        meta.push(format!("project={v}"));
    }
    if let Some(g) = effective_group_by {
        let g = match g {
            InspectGroupBy::Phase => "phase",
            InspectGroupBy::EventType => "event_type",
            InspectGroupBy::Project => "project",
            InspectGroupBy::RunId => "run_id",
        };
        meta.push(format!("group_by={g}"));
    }
    if *stale {
        meta.push("stale".to_string());
    }
    if *truncated {
        meta.push("truncated".to_string());
    }
    if !meta.is_empty() {
        println!("{}", meta.join("  "));
    }
    println!();

    // Render rows.
    match data {
        InspectData::RowCount { rows } => {
            let header = ["layer", "count", "snapshot_age_secs", "freshness_secs"];
            let body: Vec<[String; 4]> = rows
                .iter()
                .map(|r| {
                    [
                        r.layer.clone(),
                        r.count.to_string(),
                        r.snapshot_age_secs.to_string(),
                        r.freshness_secs
                            .map_or_else(|| "-".to_string(), |v| v.to_string()),
                    ]
                })
                .collect();
            write_table(&header, &body);
        }
        InspectData::Latency { rows } => {
            let header = [
                "group", "count", "p50_ms", "p95_ms", "p99_ms", "mean_ms", "trunc",
            ];
            let body: Vec<[String; 7]> = rows
                .iter()
                .map(|r| {
                    [
                        r.group_key.clone(),
                        r.count.to_string(),
                        format!("{:.1}", r.p50_ms),
                        format!("{:.1}", r.p95_ms),
                        format!("{:.1}", r.p99_ms),
                        format!("{:.1}", r.mean_ms),
                        r.truncated_samples.to_string(),
                    ]
                })
                .collect();
            write_table(&header, &body);
        }
        InspectData::ErrorRate { rows } => {
            let header = ["group", "total", "errored", "rate"];
            let body: Vec<[String; 4]> = rows
                .iter()
                .map(|r| {
                    [
                        r.group_key.clone(),
                        r.total.to_string(),
                        r.errored.to_string(),
                        format!("{:.4}", r.rate),
                    ]
                })
                .collect();
            write_table(&header, &body);
        }
        InspectData::Throughput { rows } => {
            let header = ["group", "count", "first_ts_secs", "last_ts_secs"];
            let body: Vec<[String; 4]> = rows
                .iter()
                .map(|r| {
                    [
                        r.group_key.clone(),
                        r.count.to_string(),
                        r.first_ts_secs.to_string(),
                        r.last_ts_secs.to_string(),
                    ]
                })
                .collect();
            write_table(&header, &body);
        }
        InspectData::PhaseRunSummary { rows } => {
            let header = [
                "run_id",
                "start_ts_secs",
                "phases_ms",
                "phase_count",
                "errors",
                "trace_id",
            ];
            let body: Vec<[String; 6]> = rows
                .iter()
                .map(|r| {
                    [
                        r.run_id.clone(),
                        r.start_ts_secs.to_string(),
                        r.phases_duration_ms_sum.to_string(),
                        r.phase_count.to_string(),
                        r.error_count.to_string(),
                        r.trace_id.clone().unwrap_or_else(|| "-".to_string()),
                    ]
                })
                .collect();
            write_table(&header, &body);
        }
    }
}

/// Inline width-aligned ASCII table writer. N columns, headers + rows, two
/// spaces between columns. No external dep.
fn write_table<const N: usize>(header: &[&str; N], rows: &[[String; N]]) {
    let mut widths: [usize; N] = [0; N];
    for (i, h) in header.iter().enumerate() {
        widths[i] = h.len();
    }
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }
    let render = |row: &[&str; N]| {
        let mut parts: Vec<String> = Vec::with_capacity(N);
        for (i, cell) in row.iter().enumerate() {
            parts.push(format!("{:<width$}", cell, width = widths[i]));
        }
        parts.join("  ")
    };
    let header_refs: [&str; N] = std::array::from_fn(|i| header[i]);
    println!("{}", render(&header_refs));
    let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    println!("{}", sep.join("  "));
    if rows.is_empty() {
        println!("(no rows)");
    } else {
        for row in rows {
            let refs: [&str; N] = std::array::from_fn(|i| row[i].as_str());
            println!("{}", render(&refs));
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::protocol::{InspectData, LayerRow};

    #[test]
    fn from_observe_shape_to_core_round_trips() {
        let pairs = [
            (ObserveShape::RowCount, InspectShape::RowCount),
            (ObserveShape::Latency, InspectShape::Latency),
            (ObserveShape::ErrorRate, InspectShape::ErrorRate),
            (ObserveShape::Throughput, InspectShape::Throughput),
            (ObserveShape::PhaseRunSummary, InspectShape::PhaseRunSummary),
        ];
        for (observe, core) in pairs {
            let converted: InspectShape = observe.into();
            assert_eq!(converted, core);
        }
    }

    #[test]
    fn from_observe_group_by_to_core_round_trips() {
        let pairs = [
            (ObserveGroupBy::Phase, InspectGroupBy::Phase),
            (ObserveGroupBy::EventType, InspectGroupBy::EventType),
            (ObserveGroupBy::Project, InspectGroupBy::Project),
            (ObserveGroupBy::RunId, InspectGroupBy::RunId),
        ];
        for (observe, core) in pairs {
            let converted: InspectGroupBy = observe.into();
            assert_eq!(converted, core);
        }
    }

    #[test]
    fn client_side_window_validation_rejects_bad_input() {
        assert!(validate_window("").is_err());
        assert!(validate_window("   ").is_err());
        assert!(validate_window("0s").is_err());
        assert!(validate_window("0m").is_err());
        assert!(validate_window("8d").is_err());
        assert!(validate_window("999d").is_err());
        assert!(validate_window("notatime").is_err());
    }

    #[test]
    fn client_side_window_validation_accepts_good_input() {
        assert!(validate_window("1h").is_ok());
        assert!(validate_window("30m").is_ok());
        assert!(validate_window("7d").is_ok());
        assert!(validate_window("1h30m").is_ok());
        assert!(validate_window("5m").is_ok());
        assert!(validate_window("24h").is_ok());
    }

    #[test]
    fn table_format_renders_row_count_without_panicking() {
        // This test exercises the table renderer paths for `row_count`. It
        // builds a fake Inspect response and calls print_table. Output goes
        // to stdout — we only care that it doesn't panic and that the code
        // paths for header + rows + null handling all execute.
        let response = Response::Ok {
            data: ResponseData::Inspect {
                shape: InspectShape::RowCount,
                window: "1h".to_string(),
                window_secs: 3600,
                generated_at_secs: 1_745_500_000,
                effective_filter: InspectFilter::default(),
                effective_group_by: None,
                stale: false,
                truncated: false,
                data: InspectData::RowCount {
                    rows: vec![
                        LayerRow {
                            layer: "memory".to_string(),
                            count: 42,
                            snapshot_age_secs: 5,
                            freshness_secs: Some(120),
                        },
                        LayerRow {
                            layer: "entity".to_string(),
                            count: 0,
                            snapshot_age_secs: 5,
                            freshness_secs: None,
                        },
                    ],
                },
            },
        };
        // Smoke test: must not panic.
        print_table(&response);
    }

    #[test]
    fn write_table_pads_columns_and_handles_empty_rows() {
        // Direct exercise of the table writer so empty-row + width-padding
        // paths are covered without needing a full Response.
        let header = ["a", "bb", "ccc"];
        let rows: Vec<[String; 3]> = vec![];
        write_table(&header, &rows);

        let rows = vec![
            ["1".to_string(), "22".to_string(), "333".to_string()],
            ["long".to_string(), "x".to_string(), "yy".to_string()],
        ];
        write_table(&header, &rows);
    }
}
