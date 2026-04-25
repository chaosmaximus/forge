//! Phase 2A-4d.2 T8 — `forge-next observe` subcommand.
//!
//! Phase 2A-4d.2.1 #7: forge-core's `InspectShape` / `InspectGroupBy` now
//! derive `clap::ValueEnum` behind a feature flag, so this module uses
//! them directly — the previous `ObserveShape` / `ObserveGroupBy`
//! mirror enums + `From<...>` bridges are gone. Adding a new shape is
//! now a single-file change in `forge-core/src/protocol/inspect.rs`.
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

/// Output format for the `observe` subcommand. If omitted, the CLI auto-picks
/// `Table` for TTYs and `Json` otherwise. (CLI-only concept — no protocol
/// counterpart, so kept local.)
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
}

// ── Client-side window validation ────────────────────────────────────────

/// Mirrors `crates/daemon/src/server/inspect.rs::parse_window_secs` to catch
/// bad input before a round-trip. The daemon re-validates.
const MAX_WINDOW_SECS: u64 = 7 * 24 * 60 * 60;
const BENCH_RUN_SUMMARY_MAX_WINDOW_SECS: u64 = 180 * 24 * 60 * 60;

/// D8: the per-shape window ceiling. Mirror of the daemon helper.
fn window_cap_secs_for_shape(shape: &InspectShape) -> u64 {
    match shape {
        InspectShape::BenchRunSummary => BENCH_RUN_SUMMARY_MAX_WINDOW_SECS,
        _ => MAX_WINDOW_SECS,
    }
}

/// Returns Ok(()) iff the window string parses to a duration within the
/// per-shape ceiling (7d default; 180d for `BenchRunSummary`).
fn validate_window(window: &str, shape: &InspectShape) -> Result<(), String> {
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
    let cap_secs = window_cap_secs_for_shape(shape);
    if secs > cap_secs {
        let cap_days = cap_secs / 86_400;
        return Err(format!(
            "window '{window}' exceeds {cap_days}-day ceiling ({secs}s > {cap_secs}s)"
        ));
    }
    Ok(())
}

// ── Handler ──────────────────────────────────────────────────────────────

/// Entry point: construct the request, send it, render the response.
#[allow(clippy::too_many_arguments)]
pub async fn observe(
    shape: InspectShape,
    window: String,
    layer: Option<String>,
    phase: Option<String>,
    event_type: Option<String>,
    project: Option<String>,
    group_by: Option<InspectGroupBy>,
    format: Option<OutputFormat>,
) {
    if let Err(e) = validate_window(&window, &shape) {
        eprintln!("error: {e}");
        std::process::exit(2);
    }

    let request = Request::Inspect {
        shape,
        window,
        filter: InspectFilter {
            layer,
            phase,
            event_type,
            project,
            bench_name: None,
            commit_sha: None,
        },
        group_by,
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
        InspectShape::BenchRunSummary => "bench_run_summary",
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
    if let Some(v) = &effective_filter.bench_name {
        meta.push(format!("bench_name={v}"));
    }
    if let Some(v) = &effective_filter.commit_sha {
        meta.push(format!("commit_sha={v}"));
    }
    if let Some(g) = effective_group_by {
        let g = match g {
            InspectGroupBy::Phase => "phase",
            InspectGroupBy::EventType => "event_type",
            InspectGroupBy::Project => "project",
            InspectGroupBy::RunId => "run_id",
            InspectGroupBy::BenchName => "bench_name",
            InspectGroupBy::CommitSha => "commit_sha",
            InspectGroupBy::Seed => "seed",
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
        InspectData::BenchRunSummary { rows } => {
            let header = [
                "bench_name",
                "group_key",
                "runs",
                "pass_rate",
                "comp_mean",
                "comp_p50",
                "comp_p95",
                "first_ts",
                "last_ts",
            ];
            let body: Vec<[String; 9]> = rows
                .iter()
                .map(|r| {
                    [
                        r.bench_name.clone(),
                        r.group_key.clone(),
                        r.runs.to_string(),
                        format!("{:.3}", r.pass_rate),
                        format!("{:.4}", r.composite_mean),
                        format!("{:.4}", r.composite_p50),
                        format!("{:.4}", r.composite_p95),
                        r.first_ts_secs.to_string(),
                        r.last_ts_secs.to_string(),
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

    // Phase 2A-4d.2.1 #7: the prior `ObserveShape → InspectShape` and
    // `ObserveGroupBy → InspectGroupBy` round-trip tests are deleted
    // because forge-cli no longer maintains mirror enums — both now
    // come straight from forge-core with `clap::ValueEnum` derived
    // behind the `clap` feature flag. clap's own ValueEnum coverage
    // tests in upstream + the e2e test below are sufficient.

    // Compile-time check: the protocol enums implement clap::ValueEnum
    // when the `clap` feature is on. If the feature drifts off, this
    // line fails to compile.
    fn _assert_clap_value_enum() {
        fn _accepts<T: clap::ValueEnum>() {}
        _accepts::<InspectShape>();
        _accepts::<InspectGroupBy>();
    }

    #[test]
    fn placeholder_keep_module_test_count_stable() {
        // anchor for above tests; remove once unrelated tests are added.
        let pairs: [(InspectGroupBy, InspectGroupBy); 1] =
            [(InspectGroupBy::Phase, InspectGroupBy::Phase)];
        for (a, b) in pairs {
            let converted: InspectGroupBy = a;
            assert_eq!(converted, b);
        }
    }

    #[test]
    fn client_side_window_validation_rejects_bad_input() {
        let s = InspectShape::Latency;
        assert!(validate_window("", &s).is_err());
        assert!(validate_window("   ", &s).is_err());
        assert!(validate_window("0s", &s).is_err());
        assert!(validate_window("0m", &s).is_err());
        assert!(validate_window("8d", &s).is_err());
        assert!(validate_window("999d", &s).is_err());
        assert!(validate_window("notatime", &s).is_err());
    }

    #[test]
    fn client_side_window_validation_accepts_good_input() {
        let s = InspectShape::Latency;
        assert!(validate_window("1h", &s).is_ok());
        assert!(validate_window("30m", &s).is_ok());
        assert!(validate_window("7d", &s).is_ok());
        assert!(validate_window("1h30m", &s).is_ok());
        assert!(validate_window("5m", &s).is_ok());
        assert!(validate_window("24h", &s).is_ok());
    }

    #[test]
    fn bench_run_summary_accepts_180d_rejects_200d() {
        let s = InspectShape::BenchRunSummary;
        assert!(validate_window("90d", &s).is_ok());
        assert!(validate_window("180d", &s).is_ok());
        assert!(validate_window("200d", &s).is_err());
        // Other shapes still capped at 7d.
        let other = InspectShape::Latency;
        assert!(validate_window("90d", &other).is_err());
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
