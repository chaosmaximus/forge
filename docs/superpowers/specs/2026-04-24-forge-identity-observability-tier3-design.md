# Observability Tier 3 (2A-4d.3) — Bench Harness + Time-Series Quality — Design v2

**Status:** LOCKED v2 — 2026-04-24. User approved path A (bundled Tier 3) after Claude v1 review addressed (4 BLOCKERs + 6 HIGHs + 5 MEDIUMs). Codex review stalled in investigation loop (>10min, no terminal verdict) — T14 adversarial review cycle folds any net-new findings.
**Phase position:** Third (and final) of the three 2A-4d observability tiers.
**Predecessors:** 2A-4d.1 (instrumentation, shipped at `dd4d9cb`) + 2A-4d.2 (observability API, shipped at `250cb1a`).
**Sibling spec (LOCKED):** `docs/benchmarks/forge-identity-master-design.md` v6 — the 6-dimension forge-identity bench specification. This Tier 3 spec **defers** all bench-internal decisions (dimension weights, dataset generators, score formulas, infrastructure assertions) to that doc and focuses on:

1. Landing the forge-identity bench as the bench_runs-layer payload (per master v6).
2. Wiring **all** existing benches (forge-consolidation, forge-context, forge-persist, longmemeval, locomo, forge-identity) with a common `bench_run_completed` kpi_events emit.
3. Adding a CI-per-commit fast-bench job.
4. Adding a leaderboard surface on top of Tier 2's `/inspect` API, with a **per-shape window cap** amendment to Tier 2.

---

## 1. Goal

**Turn bench runs into time-series quality observability.** Before this work, every bench result is a one-shot JSON dumped to `bench_results/summary.json`. No history, no per-commit trend, no "is recall@5 getting better?" question answerable without a grep over a dozen result docs.

**After this work:**
- Every `forge-bench <name>` invocation emits one `kpi_events` row with `event_type='bench_run_completed'` and a versioned payload (`event_schema_version: 1`). Tier 2's `/inspect` handler dispatches by shape enum — the **event_type filter path inside the SQL aggregation** reuses Tier 2's generic machinery, but a new `InspectShape::BenchRunSummary` variant, `InspectData::BenchRunSummary` payload, handler function, grouping-validity row, row type, and CLI mirror must be added (the standard shape-extension pattern — see §3.3 for the enumerated list).
- A new `/inspect bench_run_summary` shape aggregates bench runs with per-bench / per-seed / per-commit grouping, over a per-shape window cap of **180 days** (see §4 D8).
- A CI job runs the **fast** benches (forge-consolidation in-process, forge-identity in-process) on every push to `master` (PR runs too, gated `continue-on-error: true` initially), records the result, fails CI if the composite dips below the bench's calibrated threshold.
- `forge-next observe --shape bench-run-summary --bench forge-identity --window 90d` returns the 90-day quality trend as a table or JSON.

**Success metric:** a reviewer can, without custom SQL, answer "did this commit regress forge-consolidation composite vs the prior 90 days?" from a single CLI invocation.

---

## 2. Verified reconnaissance (2026-04-24, HEAD `250cb1a`)

| # | Fact | Evidence |
|---|------|----------|
| 1 | `forge-bench` binary exists with 5 subcommands: `longmemeval`, `locomo`, `forge-context`, `forge-consolidation`, `forge-persist`. Adding `forge-identity` makes 6. | `crates/daemon/src/bin/forge-bench.rs` |
| 2 | Bench harnesses follow an **in-process** pattern (`forge-consolidation`, `forge-context`, `forge-identity`-to-be) or a **subprocess** pattern (`forge-persist`). All use ChaCha20-seeded determinism. Output = `summary.json` + per-question JSONL. | `crates/daemon/src/bench/mod.rs` + `common.rs:seeded_rng` |
| 3 | `forge-consolidation` hit 1.0 composite on all 5 seeds. **Wall-clock runtime not recorded in the 2026-04-17 results doc** — T1 must measure fresh on `ubuntu-latest` CI before locking the "bench-fast" naming and the §3.4 CI plan (addresses Claude H3). | `docs/benchmarks/results/forge-consolidation-2026-04-17.md` (no wall-clock row) |
| 4 | `forge-identity-master-design.md` v6 LOCKED. Specifies 6 dimensions with per-dim minimums, 14 infrastructure assertions, pass gate at composite ≥ 0.95 AND every dim ≥ min. Implementation lives at `crates/daemon/src/bench/forge_identity.rs` (does not exist yet). | doc headers + absence of file |
| 5 | Prerequisite features all shipped: 2A-4a (valence flipping via `Request::FlipPreference` + `Request::ListFlipped` at `request.rs`), 2A-4b (`Request::ReaffirmPreference` + bench-gated `Request::ComputeRecencyFactor` at `request.rs:135`), 2A-4c1 (tool-use schema + `Request::RecordToolUse` + `Request::ListToolCalls`), 2A-4c2 (Phase 23 `phase_23_infer_skills_from_behavior` + bench-gated `Request::ProbePhase` at `request.rs:142`). | `grep -rn "FlipPreference\|ReaffirmPreference\|ComputeRecencyFactor\|RecordToolUse\|ProbePhase" crates/core/src/protocol/` |
| 6 | **`bench` Cargo feature already declared in both crates** (not a Tier 3 deliverable). `crates/core/Cargo.toml`: `[features]\nbench = []`. `crates/daemon/Cargo.toml`: `[features]\nbench = ["forge-core/bench"]`. Request variants `ComputeRecencyFactor` (line 135) and `ProbePhase` (line 142) already gate behind it. | `grep -A1 "^\[features\]" crates/{core,daemon}/Cargo.toml` |
| 7 | `kpi_events` schema: `{id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json}`; indexes on `timestamp`, `event_type`, and expression `json_extract(metadata_json, '$.phase_name')`. | `crates/daemon/src/db/schema.rs` (kpi_events block) |
| 8 | Tier 2's `/inspect` handler dispatches by `InspectShape` enum (5 shapes); a new shape requires extending the enum + adding a handler. The `event_type` filter path **inside** shape handlers is generic — placeholder `bench_run_completed` row already present in unit tests at `inspect.rs:973, 998`. Namespace is free. | `crates/daemon/src/server/inspect.rs`, `crates/core/src/protocol/inspect.rs` |
| 9 | **Tier 2 global window cap is 7 days** — hardcoded in `inspect.rs:26` `const MAX_WINDOW_SECS: u64 = 7 * 86_400` and mirrored in `crates/cli/src/commands/observe.rs:80`. Unit tests at `inspect.rs:612-614` assert `8d`, `2w`, `365d` all return errors. Tier 3 requires 180-day windows for leaderboard queries → **Tier 3 amends Tier 2 with a per-shape cap** (see §4 D8). | `grep -n "MAX_WINDOW_SECS\|parse_window_secs" crates/daemon/src/server/inspect.rs` |
| 10 | `docs/architecture/events-namespace.md` registers 5 event kinds; only `consolidate_pass_completed` is v1-versioned. Tier 3 adds `bench_run_completed` (v1). | `cat docs/architecture/events-namespace.md` |
| 11 | `kpi_events` retention reaper shipped in Tier 2 T7 at a default 30 days (`config.rs:kpi_events_retention_days = 30`). Tier 3 **either** extends this to 180 days globally OR introduces per-event-type retention (see §4 D9). | `crates/daemon/src/config.rs` + `crates/daemon/src/workers/kpi_reaper.rs` |
| 12 | CI has 3 jobs today: `check` (fmt + clippy + span integrity), `test` (ubuntu + macos matrix), `plugin-surface`. No bench job. No `actions/upload-artifact` outside `release.yml`. Default artifact retention is GitHub's 90d. | `.github/workflows/ci.yml` + `release.yml` |
| 13 | `ConsolidationStats` ships `Serialize`+`Deserialize` (Tier 2 T5 made it so). Bench score structs follow the same precedent. | `crates/daemon/src/workers/consolidator.rs` header |
| 14 | The CLI extension pattern for `/inspect` shapes: (a) add `InspectShape` variant in `forge-core`, (b) add `InspectData` variant (tagged "kind"), (c) add row type, (d) implement shape handler in `daemon/src/server/inspect.rs`, (e) extend `resolve_group_by` validity matrix, (f) mirror as `ObserveShape` in `forge-cli` with `From` impl + CLI-local `validate_window` if shape-specific cap, (g) add contract tests in `crates/core/src/protocol/contract_tests.rs`. Precedent: Tier 2 shipped 5 shapes via this pattern. | `crates/core/src/protocol/inspect.rs` + `crates/cli/src/commands/observe.rs` |

Planner re-verifies these at implementation time.

---

## 3. Architecture

Three layers, each independently testable:

### 3.1 Bench payload layer — `forge-identity` bench (per master v6)

Defer entirely to `docs/benchmarks/forge-identity-master-design.md` v6. No dimension/generator/score decisions duplicated here. Key deliverables (per master §5 "2A-4d"):

- `crates/daemon/src/bench/forge_identity.rs` — in-process harness, 6 dataset generators, 6 audit functions, composite scorer.
- `crates/daemon/tests/forge_identity_harness.rs` — integration test.
- `forge-bench forge-identity --seed N --output DIR [--expected-composite 0.95]` — CLI subcommand in `src/bin/forge-bench.rs`. Fits the existing `ForgeConsolidation`/`ForgeContext` clap pattern cleanly (same field layout).
- 14 infrastructure assertions per master v6 §6.
- Calibration to 1.0 composite on 5 seeds.
- Results doc at `docs/benchmarks/results/forge-identity-YYYY-MM-DD.md`.

**Dim 6b query-embedding resolution (LOCKED in v2):** Per master v6 §13 "Resolve in 2A-4d detailed design," Tier 3 picks **option (a)** — extend `Request::Recall` with optional `query_embedding: Option<Vec<f32>>` gated on `#[cfg(any(test, feature = "bench"))]`. Rationale: orthogonal to production path (no behavior change for live callers); aligned with existing `ComputeRecencyFactor`/`ProbePhase` bench-gated variants; bench can pin the embedding deterministically. T3 (below) implements this; the master v6 §4 Dim 6b row's semantics hold unchanged.

**Parity tests (master v6 §9 deliverable 5):** each new bench-only helper introduced by Tier 3 (`query_embedding` extension at minimum) ships a `#[test]` that calls both the bench-gated path and the production path with matching inputs and asserts output equivalence. Tracked in T3.

### 3.2 Telemetry layer — `bench_run_completed` event

Every bench CLI run emits **one** `kpi_events` row at the tail of execution:

```
event_type        = 'bench_run_completed'
project           = runtime project if FORGE_DIR present, else None
latency_ms        = wall_duration_ms (total bench runtime)
result_count      = bench dimension count (6 for identity, 5 for consolidation, 0 for benches that don't score via dimensions)
success           = 1 if composite >= bench's pass threshold, else 0
timestamp         = unix seconds at bench completion
metadata_json     = {
  "event_schema_version": 1,
  "bench_name": "forge-identity" | "forge-consolidation" | "forge-context" | "forge-persist" | "longmemeval" | "locomo",
  "seed": u64,
  "composite": f64,
  "pass": bool,
  "dimensions": [{ "name": "...", "score": f64, "min": f64, "pass": bool }],
  "dimension_scores": { "dim_name_1": f64, ... },   // flat map alongside array for stable queryability (addresses Claude M2)
  "commit_sha": Option<String>,
  "commit_dirty": bool,                              // from `git status --porcelain`
  "commit_timestamp_secs": Option<i64>,
  "hardware_profile": "ubuntu-latest-ci" | "macos-latest-ci" | "local",  // canonical set (addresses Claude M6)
  "run_id": String,                                  // ULID — same scheme as consolidate_pass_completed
  "bench_specific_stats": {}                         // opaque — each bench defines its own shape
}
```

**Per-bench `dimensions[].name` registry:** pinned in `docs/architecture/events-namespace.md` for each of the 6 benches (addresses Claude M2). Future benches add a row; renames bump `event_schema_version` to 2.

**Commit SHA detection order:**
1. `$GITHUB_SHA` (GitHub Actions)
2. `git rev-parse HEAD 2>/dev/null` (local git)
3. `None` (no git repo, shallow clone without `fetch-depth: 0`)

Plus `commit_dirty` = true iff `git status --porcelain` non-empty.

**Why reuse `kpi_events` instead of a new `bench_runs` table:**
- Tier 2's `/inspect` generic SQL path already filters by `event_type` — no schema duplication.
- Tier 2 retention reaper already sweeps `kpi_events`. Tier 3 bumps retention for `bench_run_completed` to 180d per D9.
- `bench_runs` as a first-class table would duplicate the event-stream semantics for no clear win. If future needs (specialized indexes, multi-year retention) surface, a materialized view or denormalized summary table layers over `kpi_events` without breaking anything.

**Emit site & connection model (addresses Claude H6):** a new helper `crates/daemon/src/bench/telemetry.rs` exposes `emit_bench_run_completed(db_path: &Path, payload)` that opens a short-lived `rusqlite::Connection` in WAL mode (`PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000`), does one `INSERT INTO kpi_events ...`, and closes. This connection is **separate from the bench's scoring DB** (which is always `:memory:` for in-process benches, or subprocess-owned for forge-persist). The emit is a no-op when `FORGE_DIR` is unset; a single stderr line documents the skip so CI misconfigurations are visible. When the live daemon is concurrently writing, WAL + busy_timeout handles contention; the emit is a single INSERT (< 10ms typical) so contention is bounded.

**Registration:** `docs/architecture/events-namespace.md` gets a new row for `bench_run_completed` v1 with the payload contract above + the per-bench `dimensions[].name` registry.

### 3.3 Leaderboard surface — `bench_run_summary` `/inspect` shape

A 6th `/inspect` shape, adding the following via Tier 2's extension pattern (addresses Claude B3):

**Core protocol (forge-core):**
- New `InspectShape::BenchRunSummary` variant in `crates/core/src/protocol/inspect.rs`.
- New `InspectData::BenchRunSummary { rows: Vec<BenchRunRow> }` variant (tagged `"kind": "bench_run_summary"`).
- New row type:
  ```
  BenchRunRow {
    bench_name: String,
    group_key: String,         // commit_sha / seed / bench_name per group_by
    runs: u64,
    pass_rate: f64,            // fraction of rows with pass=true
    composite_mean: f64,
    composite_p50: f64,
    composite_p95: f64,
    first_ts_secs: i64,
    last_ts_secs: i64,
  }
  ```
- Contract tests added for the new variant.

**Daemon (server/inspect.rs):**
- New `shape_bench_run_summary` handler function.
- New `resolve_group_by` row — accepts `BenchName` (default), `CommitSha`, `Seed`; rejects `Phase`, `EventType`, `Project`, `RunId`.
- New `effective_filter` block — accepts optional `bench_name`, `commit_sha`.
- Shape-specific window cap via the D8 mechanism (180d for this shape).

**CLI (forge-cli):**
- New `ObserveShape::BenchRunSummary` variant in `crates/cli/src/commands/observe.rs` with `From<ObserveShape> for InspectShape` entry.
- New `ObserveGroupBy::BenchName`, `ObserveGroupBy::CommitSha`, `ObserveGroupBy::Seed` variants.
- CLI-local `validate_window` uses the same shape-aware cap function as the daemon (180d for this shape).
- Table formatter renders `BenchRunRow` columns.

**SQL shape** (preserves Tier 2's `LIMIT 50001` per-group cap + `MAX_TOTAL_ROWS = 200_000` absolute ceiling):
```sql
SELECT
  json_extract(metadata_json, '$.bench_name') AS bench_name,
  <group_key_expr> AS group_key,
  COUNT(*) AS runs,
  AVG(success) AS pass_rate,
  AVG(json_extract(metadata_json, '$.composite')) AS composite_mean,
  ...     -- p50, p95 computed client-side from row samples (see below)
  MIN(timestamp) AS first_ts_secs,
  MAX(timestamp) AS last_ts_secs
FROM kpi_events
WHERE event_type = 'bench_run_completed'
  AND timestamp >= ?window_start
  [AND json_extract(metadata_json, '$.bench_name') = ?bench_filter]
GROUP BY bench_name, group_key
LIMIT 50001;
```

Percentiles (p50 / p95) use Tier 2's ceiling-rank convention, computed by a **second pass** that pulls raw composite values per group (with the per-group row cap). Alternative: pure SQL via `NTILE(100)` window — deferred unless benchmarking shows client-side percentile is a cost (on 30-180d windows with reaped data, per-group cardinality is small — probably <200 rows per commit_sha).

**Expression index consideration:** if a `bench_name` index becomes a bottleneck (after T1 measurement shows the generic event_type index isn't sufficient), add `CREATE INDEX idx_kpi_events_bench_name ON kpi_events(json_extract(metadata_json, '$.bench_name'))`. Deferred to T10 follow-up — single expression index is cheap, decision is whether the workload demands it yet.

### 3.4 CI layer — per-commit fast-bench job

New job `bench-fast` in `.github/workflows/ci.yml`:

- **Runner:** `ubuntu-latest` only (macos adds cost without quality signal; forge-identity is fully deterministic).
- **Matrix:** `bench: [forge-consolidation, forge-identity]` — the two in-process benches; T1 confirms each runs in <X seconds on ubuntu-latest (target: < 60s; if measured > 120s, demote to a nightly job instead of per-commit).
- **Setup:**
  - Cache `.tools/onnxruntime-*` via `actions/cache@v4` keyed on `scripts/setup-dev-env.sh` hash.
  - `scripts/setup-dev-env.sh` (cold cache: ~30s; warm cache: ~1s).
- **Steps:**
  1. `cargo build --release -p forge-daemon --features bench --bin forge-bench`
  2. `FORGE_DIR=$RUNNER_TEMP/forge-ci ./target/release/forge-bench ${{ matrix.bench }} --seed 42 --output $RUNNER_TEMP/bench-results/${{ matrix.bench }}`
  3. Generate bench events summary: `FORGE_HTTP_PORT=$((RANDOM + 10000)) nohup ./target/release/forge-daemon &` → wait for health → `forge-next observe --shape bench-run-summary --window 1h --format json > $RUNNER_TEMP/bench-trend.json` → daemon teardown via `kill $!` (selective — per user CLAUDE.md directive to never `killall`).
  4. `actions/upload-artifact@v4` with `retention-days: 14`, `compression-level: 9`, path = `$RUNNER_TEMP/bench-results/ $RUNNER_TEMP/bench-trend.json`. **Excludes `$FORGE_DIR/forge.db`** (addresses Claude H4 — the DB can be tens of MB; only summary + trend JSON are uploaded).
  5. **Gate:** job fails if `summary.json.pass != true`. Each bench owns its own threshold inside the CLI (via `--expected-composite` flag or built-in default from master v6).
- **Rollout policy:** `continue-on-error: true` on first 14 days (promotes with explicit T16 task to required; see §7). Protects against flaky runs corrupting CI signal while the telemetry layer calibrates.

**Trend-comment (optional):** a follow-up `bench-trend-comment` job reads `$RUNNER_TEMP/bench-trend.json` artifact from the matrix job, posts a markdown summary to the PR. **Deferred to post-v1** — optional, not blocking.

---

## 4. Architecture decisions

- **D1 — Bench run storage.** `kpi_events` with `event_type='bench_run_completed'`. No new `bench_runs` table. Rationale: Tier 2 infrastructure already covers it generically; retention reaper already sweeps it. Reopen only when retention needs diverge or when a materialized view is clearly cheaper than repeat aggregation.
- **D2 — Event schema versioning.** `event_schema_version: 1`. Bumps happen only on semantic-breaking changes (removed field or type change). Additive fields (new dimensions, new bench names) keep v1. Version-bump protocol inherited from `consolidate_pass_completed` precedent.
- **D3 — CI runner matrix.** ubuntu-latest only, two benches (consolidation + identity). Rationale: macos is 3-4× the cost for CI with no quality-signal gain; subprocess benches (forge-persist) exceed the 15-min CI wall-clock budget.
- **D4 — Gating policy.** First 14 days `continue-on-error: true`; promote to required via T16 after 14 consecutive green master runs (not "1 week" — cycle time is forge-identity calibration time, not operator wall-clock time). Explicit task, not a verbal commitment.
- **D5 — Emit site.** Bench binary's tail, not the daemon's internal consolidator. Rationale: bench is a CLI, not a runtime worker; the daemon isn't always running when a bench executes.
- **D6 — Leaderboard is CLI-first, Grafana-later.** A `/inspect` shape + `forge-next observe` is enough for the "quality as time series" mandate. Grafana panels are a consumer, not a requirement.
- **D7 — Forge-identity scope bundled into Tier 3.** Rationale: master v6 is LOCKED; all prereq features shipped (2A-4a/b/c1/c2); without forge-identity the telemetry layer has only one live producer (consolidation), which is too thin. Bundling keeps the tier a single reviewable unit.
- **D8 — Per-shape window cap (Tier 2 amendment).** Tier 2 shipped a hardcoded global 7d cap. Tier 3 introduces a `window_cap_secs_for_shape(shape: InspectShape) -> u64` helper called from both `parse_window_secs` (server) and `validate_window` (client), returning 604_800 (7d) for the 5 existing shapes and 15_552_000 (180d) for `BenchRunSummary`. Tier 2 unit tests for existing shapes are preserved; new tests cover the 180d path. T1 enumerates all 7+ window-validation sites before the patch lands.
- **D9 — Per-event-type retention.** Tier 2's reaper uses a single `kpi_events_retention_days = 30` knob. Tier 3 amends `WorkerConfig` to accept `kpi_events_retention_days_by_type: HashMap<String, u32>` with default `{"bench_run_completed": 180}` and global-fallback 30. Reaper reads the map; benchmark events survive 180 days to serve the leaderboard window cap.
- **D10 — Dim 6b embedding integration.** Locked to option (a) from master v6 §13: extend `Request::Recall` with bench-gated `query_embedding: Option<Vec<f32>>`. Rationale: orthogonal to production; aligned with existing bench-gated variants; deterministic in-bench.

---

## 5. Out of scope

- **New `bench_runs` SQL table.** Replaced by `kpi_events`-based storage per D1.
- **Grafana dashboard JSON.** A consumer layer; deferred to a future operator-polish pass.
- **Multi-hardware leaderboard normalization.** v1 records `hardware_profile` in payload but does not normalize across profiles. Canonical set enforced at insert; normalization deferred.
- **Subprocess benches in CI.** forge-persist, longmemeval, locomo — too slow or too network-dependent for per-commit runs.
- **Retroactive backfill** of existing `docs/benchmarks/results/*.md` into `kpi_events`. New runs only.
- **Auto-open-PR-on-regression.** Deferred; v1 fails the CI job with `pass=false`.
- **Bench-comparison across branches.** v1 groups by `commit_sha` but doesn't compute branch-relative deltas.
- **Scheduled benches (cron).** CI-per-commit only; weekly/nightly separate work.
- **`forge.db` upload in CI artifacts.** Summary + trend JSON only; full DB excluded (addresses Claude H4).

---

## 6. Dependencies / blockers

- **LOCKED:** `docs/benchmarks/forge-identity-master-design.md` v6. Any drift forces an addendum here.
- **LOCKED:** Tier 2 shipped. `bench_run_summary` shape extends existing `/inspect`; per-shape cap (D8) and per-type retention (D9) are Tier 2 amendments, non-breaking for existing shapes.
- **Shipped prereqs:** 2A-4a (valence flipping), 2A-4b (recency decay + `ReaffirmPreference` + `ComputeRecencyFactor`), 2A-4c1 (tool-use schema + `RecordToolUse`), 2A-4c2 (Phase 23 + `ProbePhase`). Implementation-time recon re-verifies via infra assertions 1-14 from master v6.
- **`bench` Cargo feature** already declared in `crates/core/Cargo.toml:11-12` and `crates/daemon/Cargo.toml:112-113` (shipped in 2A-4b / 2A-4c2). Tier 3 consumes only.

---

## 7. Task breakdown

One logical change per task. Each task commit passes `cargo fmt --all --check` + `cargo clippy --workspace -- -W clippy::all -D warnings` + `cargo test --workspace` + `scripts/check-harness-sync.sh`.

**Critical change from v1:** T2 lands the `forge_identity.rs` skeleton + all 6 dimension **stubs** + composite scorer + shared RNG / fixture helpers. T3-T6 fill in individual dimension bodies (non-overlapping function edits, safe to parallelize). Addresses Claude H1.

| Task | Description | Agent-friendly? |
|------|-------------|-----------------|
| **T1** | Re-verify 14 recon facts at implementation time; re-verify 14 infra assertions from master v6 against current code; **measure forge-consolidation wall-clock on ubuntu-latest** (Claude H3 — replace unverified "9s" claim with a real number); enumerate all window-validation sites (daemon + CLI + skills + docs) touched by D8; confirm `bench` feature already declared. Output: recon addendum at `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier3-design-recon.md`. | Yes — recon pass |
| **T2** | `forge_identity.rs` skeleton + `IdentityScore` + `BenchConfig` + 6 dimension stubs returning `DimensionScore { name: "...", score: 0.0, min: 0.8, pass: false }` + composite scorer + shared RNG helpers (reuse `bench/common.rs` and `bench/scoring.rs`) + 14 infrastructure-assertion registry (stubs). Integration test stub that runs the scorer on empty fixtures. | Yes — generator agent |
| **T3** | Implement Dim 3 (preference time-ordering, pure recall) + Dim 6a (ComputeRecencyFactor formula probe) + Dim 6b (full-recall mixed corpus, uses option-(a) `query_embedding` extension per D10). Parity test for `query_embedding` bench-gated path. | Yes — generator agent |
| **T4** | Implement Dim 4 (valence flipping correctness) using `FlipPreference` + `ListFlipped` + `include_flipped` recall param. | Yes — generator agent |
| **T5** | Implement Dim 5 (behavioral skill inference) using `RecordToolUse` + Phase 23 + `ProbePhase`. | Yes — generator agent |
| **T6** | Implement Dim 1 (identity facet persistence) + Dim 2 (disposition drift). 14 infrastructure assertions filled in + fail-fast before dimensions run. | Yes — generator agent |
| **T7** | `forge-bench forge-identity` CLI subcommand in `src/bin/forge-bench.rs` + argument plumbing (seed, output, expected-composite). | Yes |
| **T8** | `crates/daemon/src/bench/telemetry.rs` — `emit_bench_run_completed` helper + payload struct + opt-in FORGE_DIR detection + WAL-mode connection + canonical `hardware_profile` set + `commit_dirty` capture. Wire into all 6 bench runners. | Yes — cross-cutting |
| **T9** | Register `bench_run_completed` v1 in `docs/architecture/events-namespace.md` + per-bench `dimensions[].name` registry. | Yes — docs |
| **T10** | Add `InspectShape::BenchRunSummary` in forge-core (enum + data variant + row type + contract tests) + daemon handler (+ `resolve_group_by` matrix row + `effective_filter` row) + CLI mirror (+ CLI shape-specific validator). Introduce `window_cap_secs_for_shape` helper (D8) with tests covering 7d for existing shapes + 180d for new shape. | Yes — generator agent |
| **T11** | Amend Tier 2 reaper for per-event-type retention (D9). Add `kpi_events_retention_days_by_type: HashMap<String, u32>` to `WorkerConfig.validated()`; reaper consults map per row, fallback to global default. Tests: 31d-old `bench_run_completed` row survives; 31d-old `phase_completed` row gets reaped; 181d-old `bench_run_completed` row gets reaped. | Yes |
| **T12** | Calibration: run forge-identity on 5 seeds, iterate bench-side / daemon-side until 1.0 composite on all 5 (forge-consolidation precedent: 3 cycles, 2 real bugs caught). Produce `docs/benchmarks/results/forge-identity-YYYY-MM-DD.md`. | Partially — calibration loop is interactive |
| **T13** | `.github/workflows/ci.yml` — new `bench-fast` job with matrix [forge-consolidation, forge-identity], `continue-on-error: true`, ORT cache, artifact upload (summary + trend JSON only, no forge.db, retention-days: 14), selective daemon kill (no `killall`). | Yes |
| **T14** | Adversarial review pair on T1-T13 diff (Claude + Codex). | Yes — review agents |
| **T15** | Address review findings (BLOCKER + HIGH must all close; MEDIUM / LOW triaged to backlog). | Yes |
| **T16** | Live dogfood: run `forge-bench forge-identity --seed 42`, verify `kpi_events` row via `/inspect bench_run_summary`, verify CI job on a throwaway PR, verify 180d window works end-to-end. Produce dogfood doc. | Partially — interactive |
| **T17** | Promote CI `bench-fast` job to required after 14 consecutive green master runs (tracked separately; not blocking phase closure). | No — temporal gate |
| **T18** | Close phase: update HANDOFF, append 2A-4d.3.1 backlog section to plan file, archive spec + plan. | Yes |

**Agent orchestration:** T3, T4, T5, T6 edit non-overlapping function bodies after T2 lands the skeleton — **safe to dispatch in parallel** via forge-generator subagents. T10 is independent and can run alongside T3-T6. T14 runs Claude + Codex in parallel. T1, T2, T7-T9, T11-T13, T15-T18 have inter-task dependencies and run serial.

**Estimated span:** 18 tasks total; 14 on the critical path (T1 → T2 → {T3..T6 parallel; T10 parallel} → T7 → T8 → T9 → T11 → T12 → T13 → T14 → T15 → T16 → T18). T17 is a temporal promotion gate that fires after merge. Calibration (T12) is the unknown — forge-consolidation took 3 cycles to reach 1.0; plan for 2-3 cycles.

---

## 8. Open questions (v2 → v3 triggers)

All v1 opens either resolved or explicitly deferred to v3 after Codex review returns:

1. ~~Dim 6b query-embedding integration~~ — **locked to option (a)** per D10 and master v6 §13.
2. **Artifact persistence across commits.** v1 is upload-only (GitHub 14d retention per D8 artifact policy). Leaderboard beyond 14d relies on the 180d retention in `kpi_events` (D9) — but that requires the daemon to be reading a persistent DB, not a CI-ephemeral one. Deferred question: does Tier 3 v1 need a persistent cross-commit DB (e.g., Litestream-replicated to GCS) from day one, or can we ship v1 with "each CI run populates its own DB, leaderboard is local-only" and add durable storage in 2A-4d.3.1?
3. ~~Per-bench event subtype vs single event_type~~ — **single `bench_run_completed` with `bench_name` discriminator + per-bench `dimensions[].name` registry** in `events-namespace.md`. Future bench renames bump `event_schema_version` to 2.
4. ~~Reaper retention for bench_run_completed~~ — **locked at 180d** per D9. Not a blocker.
5. **Codex review findings.** v2 was written before Codex completed. Any net-new BLOCKER/HIGH from Codex lands in v3.

---

## 9. Changes from v1 → v2

- **B1 → D8.** Per-shape window cap amendment to Tier 2. Dropped v1's "365d, higher than 7d ceiling" framing; locked at 180d for `bench_run_summary`, 7d for existing shapes.
- **B2 → Fact 6.** `bench` Cargo feature already declared (recon error in v1 stated "if not yet declared, Tier 3 adds it").
- **B3 → §1 + §3.3 rewrite.** Enumerated the full shape-extension surface (InspectShape, InspectData, row type, handler, group_by matrix, effective_filter, CLI mirror, contract tests). Dropped "zero changes to handler dispatch" phrasing.
- **B4 → T1 scope.** Recon task now enumerates all window-validation sites (7+) before D8 patch lands.
- **H1 → T2 restructured.** Skeleton + 6 stubs in T2; T3-T6 fill non-overlapping bodies; parallel dispatch safe.
- **H3 → T1 scope + Fact 3 honesty.** Dropped unverified "9s" claim; T1 measures forge-consolidation wall-clock on ubuntu-latest before §3.4 CI plan locks.
- **H4 → §3.4 + D8.** Artifact upload excludes `forge.db`; `retention-days: 14`; `compression-level: 9`.
- **H5 → D4 + T17.** Gate-promotion policy concrete: 14 green runs, tracked as T17.
- **H6 → §3.2 connection model.** WAL mode + busy_timeout; explicit that bench's scoring DB is separate from telemetry INSERT DB.
- **M2 → §3.2 + T9.** Added flat `dimension_scores: {name: score}` map + per-bench `dimensions[].name` registry in events-namespace.md.
- **M3 → D10.** Dim 6b locked to option (a).
- **M4 → D9.** Per-event-type retention; `bench_run_completed` = 180d.
- **M5 → §3.4.** `retention-days: 14` on artifact upload.
- **M6 → §3.2.** Canonical `hardware_profile` set; reject others at insert.

---

## 10. References

- `docs/benchmarks/forge-identity-master-design.md` v6 — LOCKED source of truth for the forge-identity bench internals.
- `docs/benchmarks/forge-consolidation-design.md` — precedent for in-process harness architecture.
- `docs/benchmarks/results/forge-consolidation-2026-04-17.md` — precedent for calibration + results-doc format.
- `docs/superpowers/specs/2026-04-24-forge-identity-observability-design.md` — Tier 1 LOCKED spec; establishes kpi_events versioning protocol.
- `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier2-design.md` — Tier 2 LOCKED spec; establishes `/inspect` shape extension pattern and window-cap machinery.
- `docs/architecture/events-namespace.md` — event registry Tier 3 extends.

---

## Changelog

- **v1 (2026-04-24):** Initial draft. Defers bench internals to master v6; scoped Tier 3 to forge-identity implementation + telemetry wrapper + leaderboard surface + CI-per-commit. Author's open-question list flagged known trade-offs for adversarial review.
- **v2 (2026-04-24):** Address Claude adversarial review (4 BLOCKERs + 6 HIGHs + 3 MEDIUMs). Key structural changes: per-shape window cap (D8), per-event-type retention (D9), Dim 6b query-embedding lock (D10), T2 restructured for parallel-safe T3-T6 dispatch, §1/§3.3 enumerate full shape-extension surface, §3.4 artifact upload excludes forge.db. Codex review pending — v3 folds in any net-new findings.
