---
name: forge-observe
description: "Use when debugging slow phases, investigating drift, or doing dashboard-free live checks on daemon internals. Example triggers: 'why is consolidation slow', 'show latency for phase X', 'is layer 5 stale', 'inspect kpi events', 'what ran in the last hour'. Runs forge-next observe to query kpi_events or the per-layer gauge snapshot."
---

# Forge Observe — Live Introspection Without a Dashboard

`forge-next observe` queries the daemon's Tier 2 observability surface: per-layer gauge snapshots and `kpi_events`. One subcommand, five shapes, no Prometheus required.

Spec: `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier2-design.md`

## When to Use

- **Slow phases** — `latency` shape tells you p50/p95/p99 per phase across a window.
- **Drift / staleness** — `row_count` reads the `GaugeSnapshot` and reports per-layer freshness.
- **Error hunting** — `error_rate` surfaces which phases / event types are failing.
- **Traffic shape** — `throughput` shows event counts and the first/last-seen timestamps per group.
- **Post-mortem a run** — `phase_run_summary` rolls up a single consolidation pass by `run_id`.

Reach for `observe` instead of tailing the daemon log when you need aggregates. Reach for the log when you need the actual error strings.

## Shapes

| Shape                 | What it returns                                              | Source          |
|-----------------------|--------------------------------------------------------------|-----------------|
| `row_count`           | Per-layer row count + snapshot + per-table freshness seconds | `GaugeSnapshot` |
| `latency`             | p50 / p95 / p99 / mean latency_ms per group                  | `kpi_events`    |
| `error_rate`          | `errored / total` ratio per group                            | `kpi_events`    |
| `throughput`          | Event count + first/last timestamp per group                 | `kpi_events`    |
| `phase_run_summary`   | Per-`run_id` duration, phase count, error count, trace id    | `kpi_events`    |

All shapes honor `--window <humantime>` (default `1h`, ceiling 7 days) and `--group-by {phase|event_type|project|run_id}` where the shape supports grouping.

## Examples

```bash
# How many rows per Manas layer right now, and are any tables stale?
forge-next observe --shape row-count

# p50/p95/p99 per phase over the last 4 hours
forge-next observe --shape latency --window 4h --group-by phase

# Error rate for phase_completed events across the last day
forge-next observe --shape error-rate --window 1d --event-type phase_completed --group-by phase

# Which phases ran in the last 15 minutes, and how often?
forge-next observe --shape throughput --window 15m --group-by phase

# Roll up each consolidation pass in the last hour (use with `--format json` for agents)
forge-next observe --shape phase-run-summary --window 1h --format json
```

## Format Rules

- `--format table` — compact ASCII, aligned columns, one header row.
- `--format json` — pretty-printed `Response::Inspect` (full metadata: window_secs, effective_filter, stale, truncated).
- Omit `--format` — auto-picks Table on a TTY, JSON otherwise. Safe to pipe.

## Anti-Patterns

- Windows > 7 days — rejected client-side before the round-trip.
- Asking for `row_count` with a `group_by` — the daemon will return the shape without grouping; use the other four shapes if you need per-group breakdowns.
- Polling `observe` in a tight loop — the HUD consolidation segment (T6) is the right surface for that. Use `observe` for investigations, not monitoring.
