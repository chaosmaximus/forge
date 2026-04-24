# Forge-Identity Observability — Design v4 (Phase 2A-4d.2, Observability API Tier)

**Status:** DRAFT v4 — 2026-04-24. Lean rewrite: v1-v3 drowned in line-number precision that drove three rounds of fabricated-recon blockers. v4 describes architecture and decisions and leaves call-site enumeration, line-range verification, and incidental refactors to implementation-time agent tools (grep / cargo check / compiler). Implementation agents re-verify against HEAD as they go.

**Phase position:** Second of three 2A-4d tiers.

| Tier | Status |
|------|--------|
| 2A-4d.1 Instrumentation (spans + metrics + kpi_events) | **COMPLETE** @ `dd4d9cb` |
| **2A-4d.2 Observability API (this spec)** | **DRAFT v4** |
| 2A-4d.3 Bench harness | blocked on 2A-4d.2 |

**Tier 1 promises this tier honors:**
- `forge-next observe` CLI — Tier 1 line 10.
- `forge_layer_freshness_seconds` gauge — Tier 1 line 10.
- `kpi_events` retention reaper — Tier 1 line 10.
- **`/inspect audit` shape — SUPERSEDED.** Tier 1 T13.1 added `_with_errors` variants for the 11 helpers; `PhaseOutcome.error_count` already carries swallowed errors. T1 of this tier writes a Tier 1 addendum making this explicit; `error_rate` shape covers the remaining surface.

---

## 1. Goal

Make the `kpi_events` rows and gauge family that Tier 1 writes **queryable** — from CLI, SSE, and HUD — without a separate operator stack.

Success looks like:
- "p95 latency of phase_23 over 1h" — one CLI invocation.
- "which phases errored in 24h" — one CLI invocation.
- Live tail of consolidation passes — `curl /api/subscribe?events=consolidate_pass_completed`.
- HUD shows `cons:23✓ 1.2s` for the latest pass.
- Prometheus gains `forge_layer_freshness_seconds{table}`.
- `kpi_events` retention bounded.

---

## 2. Architecture

### 2.1 Five `Inspect` shapes

New `Request::Inspect { shape, window, filter, group_by }` dispatched through existing `POST /api`. Variants land inside `ResponseData::Inspect` (the existing `Response::Ok { data }` envelope).

| Shape | Purpose | Data source |
|-------|---------|-------------|
| `row_count` | Per-layer row counts + freshness | `GaugeSnapshot` (see 2.3) |
| `latency` | p50/p95/p99/mean per group | `kpi_events.latency_ms` |
| `error_rate` | errored-passes / total per group | `kpi_events.metadata_json.$.error_count` |
| `throughput` | event counts per group over window | `kpi_events` |
| `phase_run_summary` | per-run_id rollup (duration, errors, trace_id) | `kpi_events` |

**Types** (in `forge-core`, no clap):

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectShape { RowCount, Latency, ErrorRate, Throughput, PhaseRunSummary }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InspectGroupBy { Phase, EventType, Project, RunId }   // no Success variant — dead flag

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", default)]
pub struct InspectFilter {
    pub layer: Option<String>,
    pub phase: Option<String>,
    pub event_type: Option<String>,   // defaults to "phase_completed" at handler
    pub project: Option<String>,
}
```

Row types for each shape carry explicit, typed fields (no free-form JSON). `freshness_secs: Option<i64>` for empty-table honesty; `trace_id: Option<String>` for null-collapse.

**Shape × group_by validity:**

| Shape | Allowed group_by (None → default) |
|-------|------------------------------------|
| row_count | None only |
| latency | Phase (default), RunId, None |
| error_rate | Phase (default), EventType, None |
| throughput | EventType (default), Phase, Project, None |
| phase_run_summary | None (group is internally RunId) |

Response echoes `effective_group_by` + `effective_filter` so clients see what the handler actually applied.

**Window grammar:** `humantime::parse_duration`. Reject `0`, negatives, bare integers, strings >7d. One daemon-side dep (`humantime = "2"`). CLI validates client-side with the same parser to fail fast.

**Row cap, per group:** `latency`, `error_rate`, `throughput` each cap at `MAX_ROWS_PER_GROUP = 20_000` samples per group_key; truncation flagged per-row (`truncated_samples` count) so percentile bias is visible to clients. Global query-level absolute ceiling `200_000` rows as a hard stop to avoid unbounded memory on pathological inputs.

### 2.2 `consolidate_pass_completed` SSE event

New `ForgeEvent { event: "consolidate_pass_completed", data, timestamp }` emitted at the tail of `run_all_phases`, inside the `_pass_span` scope (so `current_otlp_trace_id()` returns the pass-level id, not a phase child's).

Payload:
```json
{
  "event_schema_version": 1,
  "run_id": "01HXYZ…",
  "correlation_id": "01HXYZ…",
  "trace_id": null,
  "pass_wall_duration_ms": 1234,
  "phase_count": 23,
  "error_count": 0,
  "stats": { …ConsolidationStats… }
}
```

**Implementation points:**
- `run_all_phases` gains `Option<&EventSender>` as a 4th parameter. Borrow is safe — `broadcast::Sender::send` is non-blocking, no `.await`, no lock re-entry.
- `pass_wall_duration_ms` = `Instant::now()` captured at function entry, measured right before the emit.
- `phase_count` = `PHASE_SPAN_NAMES.len()` (not a literal 23 — Tier 1 already gates len via integrity test).
- `error_count` = derived from kpi_events for this run_id, filtered by `timestamp >= pass_start_secs` so the query uses the timestamp index (no full-table scan):
  ```sql
  SELECT COALESCE(SUM(COALESCE(json_extract(metadata_json, '$.error_count'), 0)), 0)
  FROM kpi_events
  WHERE timestamp >= :pass_start_secs
    AND event_type = 'phase_completed'
    AND json_extract(metadata_json, '$.run_id') = :run_id
  ```
- `stats` = `ConsolidationStats` serialized. Requires adding `Serialize` derive.
- `current_otlp_trace_id()` must be `pub(crate)` (is currently private).

**Call-site updates:** every caller of `run_all_phases` (production, bench, unit tests, integration tests) updates to the new 4-arg signature. Production callers pass `Some(&events)` using **clone-before-lock** when they hold a state lock: clone the `Sender` handle outside the lock, then call `run_all_phases` with a borrow of the clone. Non-locking production callers (e.g., `ForceConsolidate` handler which owns its own reader) pass the event sender directly. Bench/test callers pass `None`. The agent implementing T5 finds the sites and classifies each.

**Event name:** `consolidate_pass_completed`. Distinct from existing `consolidation` under the SSE exact-match filter — no collision.

**Panic path:** if a phase panics, `_pass_span` drops via unwind; emit never runs; that pass misses its SSE event. Acceptable for v4 — clients treat SSE as best-effort and poll `phase_run_summary` for the authoritative ledger. Panic-safe emit via `scopeguard::defer` is a future option.

### 2.3 `GaugeSnapshot` + `forge_layer_freshness_seconds`

**Problem:** `refresh_gauges` today updates 11 `forge_table_rows{table}` gauges plus 4 scalars serially after a single SELECT. Between those `.set()` calls, `/metrics` scrapers and `/inspect row_count` readers can observe torn values.

**Fix (scoped to `/inspect`):** introduce a `GaugeSnapshot` struct holding all 11 tables + 4 scalars + `refreshed_at_secs`. `refresh_gauges` builds the new snapshot from the SELECT result, writes the Prometheus gauges (existing behavior), then swaps the snapshot atomically under a write-lock. Readers clone the snapshot under a read-lock — a single point-in-time view.

```rust
pub struct ForgeMetrics {
    // … existing collectors …
    pub snapshot: Arc<parking_lot::RwLock<GaugeSnapshot>>,
}

pub struct GaugeSnapshot {
    pub refreshed_at_secs: u64,      // 0 = never refreshed
    pub tables: TableGauges,          // named struct; one field per Manas table
    pub memories_total: i64,
    pub edges_total: i64,
    pub embeddings_total: i64,
    pub active_sessions: i64,
}

pub struct TableGauges {
    pub memory: RowAndFreshness,
    pub skill: RowAndFreshness,
    // …11 total
}

pub struct RowAndFreshness {
    pub count: i64,
    pub freshness_secs: Option<i64>,  // None when table is empty
}
```

`parking_lot::RwLock` (already a compatible choice; daemon uses `tokio::sync::RwLock` in async paths but `parking_lot` is standard for tight synchronous sections like snapshot swap — the lock is never held across `.await`).

**Out of scope:** `/metrics` scrape still reads Prometheus collectors directly and can observe torn values. Acceptable because Prometheus aggregation tolerates sub-scrape drift. Atomic `/metrics` is a future option.

**Freshness — TEXT timestamp columns.** The Manas tables store `created_at`/`detected_at`/`ingested_at`/`discovered_at` as **TEXT** ISO datetime strings (verified at spec time; implementation re-verifies). The expanded SELECT must pass the TEXT through `strftime('%s', …)` before subtracting:

```sql
SELECT
    (SELECT COUNT(*) FROM memory)                                                AS mem_count,
    (SELECT CASE WHEN COUNT(*) = 0 THEN NULL
                 ELSE CAST(strftime('%s','now') AS INTEGER)
                    - CAST(strftime('%s', MAX(created_at)) AS INTEGER)
                END
       FROM memory)                                                               AS mem_fresh_secs,
    -- … same pattern per table. Column name per table is picked from schema at impl time
    --   (most are `created_at`; `perception.detected_at`, `domain_dna.discovered_at`, etc.).
```

Implementation agent reads `db/schema.rs` to pick the right column per table.

**Prometheus gauge** `forge_layer_freshness_seconds{table}` uses `-1` as the empty-table sentinel (Prometheus can't emit NULL). Internal Rust / JSON keeps `Option<i64>`. Documented in the metric's help text.

### 2.4 HUD consolidation segment, cached

`build_hud_state` runs on every broadcast event today. Naively adding consolidation queries multiplies reader load on extraction bursts.

**Design:** populate the new `consolidation` field in `hud-state.json` **only** when `event.event == "consolidate_pass_completed"`. On other events, carry over from the last-written state file. Absent when no pass has ever fired (first-boot / no kpi_events rows).

- On `consolidate_pass_completed`: build from event payload (no phase-lookup SQL), plus one 24h rollup query (pass count, error-pass count).
- On other events: read last `consolidation` subtree from `hud-state.json`. Missing file / parse failure / missing subtree → `None`, segment omitted.
- **Staleness guard:** if `latest_run_ts_secs` from cache is older than `2 × consolidation_interval_secs`, treat as None (prevents serving a day-old pass after a daemon restart).

HUD state struct gains `consolidation: Option<Consolidation>` with `#[serde(default)]`. Older HUD binaries ignore the new key safely (existing serde tolerance).

Renderer adds a segment (`cons:23✓ 1.2s` green, `cons:19/23 err 3.4s` red). Lowest priority in the width budget; truncates first.

### 2.5 `forge-next observe` CLI

Tier 1 promise — note CLI name (`observe`) diverges from RPC name (`Inspect`); intentional, documented.

CLI mirrors `InspectShape`/`InspectGroupBy` as local clap `ValueEnum` types with `From` impls to the core enums. Core stays free of clap (forge-core has only serde/serde_json/ulid).

`OutputFormat` default: `Table` if `std::io::stdout().is_terminal()`, else `Json`. Rust 1.70+ required; workspace uses edition 2021 with no pinned MSRV but actual toolchain is well above. No new dep for formatting — inline tabular helper.

Client-side humantime validation on `--window` before RPC roundtrip.

### 2.6 `kpi_events` retention reaper

New worker. Rowid-subquery batched DELETE (bundled rusqlite lacks `SQLITE_ENABLE_UPDATE_DELETE_LIMIT`):

```sql
DELETE FROM kpi_events
 WHERE rowid IN (
     SELECT rowid FROM kpi_events
      WHERE timestamp < :cutoff_secs
      LIMIT :batch
 )
```

Batch = 10,000 rows. **`tokio::time::sleep(50ms).await`** between batches (NOT `std::thread::sleep` — matches the existing tokio-async pattern of every other daemon worker).

Config: `ForgeConfig.observability.kpi_events_retention_days = 30`. Schedule: `tokio::spawn` loop, every 6h plus on startup. 30d × default cadence (30min) ≈ 33k rows ≈ 13MB logical, trivially cheap.

### 2.7 Events namespace doc

New `docs/architecture/events-namespace.md`. Registers actual emitted event names in the codebase (T7 agent enumerates emit sites at impl time — v1-v3 spec drafts guessed wrong and would have embarrassed the registration). Documents `consolidate_pass_completed` with `event_schema_version: 1` payload contract + evolution rules mirroring `kpi_events-namespace.md`.

---

## 3. Tasks

9 commits. Each task is scoped; implementation agent drives recon + execution.

**T1 — Re-verify recon + Tier 1 audit supersedence addendum**
- Walk §2 of this spec; grep / read to confirm architectural assumptions still hold at HEAD.
- Write Tier 1 addendum noting `/inspect audit` was superseded by Tier 1 T13.1.
- Drive-by: update any stale comments discovered (e.g., the "22 phases" mention in consolidator header).
- Also update `kpi_events-namespace.md` if it carries a stale "11 helpers `.unwrap_or(0)`" note — Tier 1 T13.1 fixed it.

**T2 — Core protocol types + serde**
- `InspectShape`, `InspectFilter`, `InspectGroupBy`, `InspectData` + 5 row types.
- `Request::Inspect` + `ResponseData::Inspect` (inside the `Ok { data }` envelope).
- Round-trip tests.

**T3 — Inspect handler + 5 shapes + expression index**
- Add `humantime = "2"` to `forge-daemon`.
- New module `server/inspect.rs` with `parse_window_secs` + per-shape dispatch.
- Migration: `CREATE INDEX idx_kpi_events_phase ON kpi_events(json_extract(metadata_json, '$.phase_name'))`.
- Per-group row cap + global absolute ceiling.
- Tests: each shape × each default group_by on seeded DB; invalid combinations → error; malformed window → error; SQL-injection probe in each filter.

**T4 — `GaugeSnapshot` atomic swap + `forge_layer_freshness_seconds`**
- Named struct `GaugeSnapshot { tables: TableGauges, … }`.
- `Arc<parking_lot::RwLock<GaugeSnapshot>>` on `ForgeMetrics`.
- Extend `refresh_gauges` SELECT with 11 freshness subqueries using `strftime('%s', MAX(col))`. Pick per-table column from `db/schema.rs`.
- Register Prometheus `forge_layer_freshness_seconds{table}` family (-1 sentinel for empty).
- `/inspect row_count` reads the snapshot; sets `stale: true` if `refreshed_at_secs == 0 || now - refreshed > 60s`.
- Tests: torn-read stress test; freshness accuracy within 1s; empty-table → None.

**T5 — `run_all_phases` event emit + call-site sweep**
- 4th parameter `Option<&EventSender>` on `run_all_phases`.
- Derive `Serialize` on `ConsolidationStats`.
- `pub(crate)` on `current_otlp_trace_id`.
- Emit block at the end of `run_all_phases`, inside `_pass_span` scope, before the `stats` return. Uses the indexed-timestamp SUM query from §2.2.
- Update every `run_all_phases(...)` call site — agent greps, classifies prod/bench/test, applies clone-before-lock where needed.
- Tests: subscribe + run on seeded corpus + assert one `consolidate_pass_completed` event with correct fields; OTLP-scoped trace_id test (same pattern as Tier 1 T13.2); ConsolidationStats serde round-trip.

**T6 — HUD consolidation segment**
- `HudState.consolidation: Option<Consolidation>` (serde default).
- `build_hud_state` branches on event name: build from payload vs. carry-over from state file.
- Staleness guard: cache older than 2× consolidation_interval → None.
- `render/line3.rs` segment, lowest priority in width budget.
- Tests: pass event → populated; extraction event → preserved from cache; missing/malformed state file → None; segment render green/red/absent.

**T7 — Retention reaper + events namespace doc**
- New `workers/kpi_reaper.rs` using rowid-subquery batched DELETE + `tokio::time::sleep` between batches.
- Config: `ForgeConfig.observability.kpi_events_retention_days = 30`; wire into config struct.
- Spawn loop in `main.rs` (6h + on startup).
- New `docs/architecture/events-namespace.md`. Agent enumerates actual emit sites via grep (do **not** trust stale 4-event claim in code comments) and registers each.
- Tests: seed + reap; interleave INSERT mid-reap; cutoff accuracy.

**T8 — `forge-next observe` CLI + skill + docs**
- `commands/observe.rs` with local `ObserveShape`/`ObserveGroupBy` clap enums + `From` to core.
- Humantime client-side validation.
- TTY-detect table format default; inline tabular helper.
- `skills/forge-observe.md` (harness propagation).
- Update `docs/cli-reference.md`, `docs/api-reference.md`.
- Integration test: spawn daemon → CLI → parse stdout.

**T9 — Adversarial review pair + fixes + dogfood**
- Claude + Codex on the T2-T8 diff.
- Address BLOCKER/HIGH, defer LOW with rationale.
- Live dogfood: daemon + consolidation + curl each shape + SSE tail + CLI + HUD segment. Record in `docs/benchmarks/results/`.
- Deferred backlog appended to plan file.

---

## 4. Testing strategy

- Unit per module (handler, window parser, snapshot, reaper, HUD cache).
- Integration: `t20_inspect_roundtrip.rs`, `t21_consolidate_pass_event.rs`, `t22_observe_cli.rs`, `t23_inspect_latency_perf.rs` (<100ms p95 on 10k rows), `t24_hud_consolidation_segment.rs`. Flat files, no subdir — matches existing `t10_instrumentation_latency.rs` convention.
- Regression: `cargo clippy --workspace -- -W clippy::all -D warnings` = 0; full workspace tests green; `check_spans.sh` OK.

---

## 5. Risks (kept)

1. **Panic inside a phase** → SSE event missed for that pass. Polling `phase_run_summary` is the ledger. Panic-safe emit is a future option.
2. **`/metrics` torn-read** remains by design; Prometheus tolerates.
3. **Expression index use** — simple `GROUP BY json_extract(...)` patterns hit the index. Verify with `EXPLAIN QUERY PLAN` at T3.
4. **Reaper vs. writes** — reap is benign (rows inserted with old timestamps after subquery execute get reaped next cycle).
5. **HUD cache after restart** — staleness guard at 2× consolidation_interval prevents serving day-old data.
6. **`ConsolidationStats: Serialize`** is additive; no existing consumer affected.
7. **humantime accepts compound forms** (`1h30m`). Accepted; documented in CLI help.

---

## 6. Out of scope

- `/inspect audit` — superseded by Tier 1 T13.1.
- `/metrics` atomic snapshot — future.
- Panic-safe emit — future.
- Historical row_count drift (needs `table_rows_snapshot` event) — Tier 3.
- `PRAGMA query_only=1` readers hardening — Tier 3.
- `arcswap` over RwLock — profile first.

---

## 7. Harness impact

| Layer | Impact |
|-------|--------|
| Plugin / hooks / agents | None |
| Skills | New `skills/forge-observe.md` (T8) |
| Docs | cli-reference, api-reference, new events-namespace (T7), Tier 1 addendum (T1) |
| CLAUDE.md | None |

---

## 8. Success checklist

- Clippy 0 warnings, workspace tests green.
- `t23_inspect_latency_perf` < 100ms p95 on 10k rows.
- Adversarial review pair (T9) BLOCKER/HIGH closed.
- Tier 1 audit-supersedence addendum committed.
- `events-namespace.md` registers actual (not imagined) event names.
- Deferred backlog appended to plan file.

---

## 9. References

- Tier 1 spec (locked v4): `docs/superpowers/specs/2026-04-24-forge-identity-observability-design.md`
- Tier 1 plan: `docs/superpowers/plans/2026-04-24-forge-identity-observability.md`
- kpi_events register: `docs/architecture/kpi_events-namespace.md`
- Tier 1 dogfood: `docs/benchmarks/results/2026-04-24-forge-identity-observability-T1.md`
