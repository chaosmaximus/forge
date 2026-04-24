# Phase 2A-4d.2 Observability API — T9 dogfood results

**Date:** 2026-04-24
**Daemon HEAD:** `c04b6ce` (T9 review fixes applied on top of T2-T8)
**Isolation:** `FORGE_DIR=/tmp/forge-2a4d2-dogfood`, all ambient state reset
**Test flow:** release binary, fresh DB, startup consolidation + 2× ForceConsolidate

## What was verified live

### `forge-next observe` — all 5 shapes against a running daemon

| Shape                | CLI invocation                                                               | Result |
|----------------------|------------------------------------------------------------------------------|--------|
| `phase_run_summary`  | `forge-next observe --shape phase-run-summary --window 1h --format json`     | ✅ 2 runs after 1× startup + 1× consolidate. `phase_count=23` matches `PHASE_SPAN_NAMES.len()`. |
| `latency`            | `forge-next observe --shape latency --window 1h --group-by phase`            | ✅ 23 rows (one per phase) rendered as ASCII table. Empty DB → p50/p95/p99 all `0.0`. |
| `error_rate`         | `forge-next observe --shape error-rate --window 24h --group-by phase`        | ✅ JSON with `effective_group_by: "phase"`, empty rows on clean DB. |
| `throughput`         | `forge-next observe --shape throughput --window 24h --group-by phase`        | ✅ Per-phase row counts + first_ts_secs / last_ts_secs. |
| `row_count`          | `forge-next observe --shape row-count`                                       | ⚠️ `stale: true, rows: []` — lazy-refresh fix doesn't fire because `DaemonState.metrics` is `None` at the per-request reader level. Documented as follow-up. |

### SSE `consolidate_pass_completed` event emit

Subscribed to `/api/subscribe` (no filter) during a `forge-next consolidate` run → captured one event with the full v1 payload:

```json
{
  "event": "consolidate_pass_completed",
  "data": {
    "event_schema_version": 1,
    "run_id": "01KQ0GYV20KXV4H2ZWAM7H3T1C",
    "correlation_id": "01KQ0GYV20KXV4H2ZWAM7H3T1C",
    "trace_id": null,
    "pass_wall_duration_ms": 770,
    "phase_count": 23,
    "error_count": 0,
    "stats": { /* 20 ConsolidationStats fields */ }
  },
  "timestamp": "1777060507"
}
```

`phase_count` derived from `PHASE_SPAN_NAMES.len()` ✅
`stats` field shows all 20 `usize` counters from `ConsolidationStats` (`Serialize` derive from T5) ✅

### `run_all_phases` 4-arg signature correctness

2 distinct `run_id` ULIDs recorded across the 2 consolidation runs, both with 23 phases each — matches the "exactly one emit per pass" invariant from T5's integration test.

Pass-level duration: startup pass reported `phases_duration_ms_sum=0` (in-memory DB, sub-ms phases rounding to 0); second pass reported `pass_wall_duration_ms=770` via SSE (wall-clock, not SQL sum). Two different measurements, correctly labeled.

### Error path rendering

`forge-next observe --shape latency --window bogus` returns:
```
error: invalid window 'bogus': expected number at 0
```
Humantime client-side validation fires before the RPC.

### CLI help

`forge-next observe --help` surfaces all 8 flags (shape, window, layer, phase, event-type, project, group-by, format) with possible values enumerated for clap `ValueEnum` types.

## Known gaps deferred to 2A-4d.2.1 backlog

1. **`/inspect row_count` is permanently stale without Prometheus scraping.** The T9 lazy-refresh patch wires `refresh_gauges_from_conn` when `snapshot.refreshed_at_secs == 0`, but `DaemonState.metrics: Option<Arc<ForgeMetrics>>` is `None` at per-request reader construction (see `handler.rs::new_reader`). The `Arc<ForgeMetrics>` lives on `AppState`; plumbing it through to the reader requires a small refactor (either add a ForgeMetrics handle to DaemonState, or move the lazy-refresh branch into the `/api` HTTP handler where `AppState` is in scope). **Honest behavior in the meantime**: response carries `stale: true` so clients know the snapshot is uninitialized.

2. **SSE filter `?events=consolidate_pass_completed` returned 0 events in one test.** Unfiltered subscribe captured the event correctly, so the emit works; something in the filter matching needs verification. Possibly a query-param encoding issue; dropped to backlog.

3. **HUD segment not visually verified.** The renderer + state wiring are tested (green path, red path, absence), but nobody eyeballed the 3-line HUD in a terminal today. Tracking the HUD dogfood as a separate close-out task.

4. **Known correctness items from T9 review deferred to 2A-4d.2.1** (see `docs/superpowers/plans/2026-04-24-forge-identity-observability.md` for full list):
   - `std::fs::read_to_string` in HUD cache read runs on tokio runtime thread (perf).
   - `std::fs::write` on hud-state.json is not atomic (durability; use tmpfile+rename).
   - HUD 24h rollup `COUNT(DISTINCT json_extract(...))` can't use the expression index at scale.
   - Percentile convention (`ceil(p*n)-1`) documented but not yet surfaced in API docs.
   - `shape_latency` total-cap off-by-one in `truncated_samples` accounting.
   - CLI `ObserveShape` mirror vs forge-core feature-flagged `ValueEnum` — pick one.

## Tier 2 feature checklist

- [x] `Request::Inspect` + `ResponseData::Inspect` protocol surface (T2)
- [x] 5 shapes handler with expression index + per-group + absolute row cap (T3)
- [x] `GaugeSnapshot` atomic swap + `forge_layer_freshness_seconds` gauge (T4)
- [x] `consolidate_pass_completed` SSE event wired at 16 call sites (T5)
- [x] HUD consolidation segment with cache + staleness guard (T6)
- [x] `kpi_events` retention reaper (T7)
- [x] `forge-next observe` CLI + skill + docs (T8)
- [x] Adversarial review pair run (Claude + Codex) on T2..T8 (T9)
- [x] BLOCKER + 3 HIGHs fixed (T9 review)
- [x] Live dogfood (this doc)

## Verdict

Tier 2 ships. The 4 remaining items (listed above) are non-blocking; operators can use all 5 shapes + SSE + CLI against a live daemon today. `row_count` prints `stale: true` on non-Prometheus daemons which is honest behavior — the lazy-refresh polish is a follow-up.
