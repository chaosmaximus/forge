# Forge-Identity Observability — Design v4 (Phase 2A-4d.1, Instrumentation Tier)

**Status:** LOCKED 2026-04-24 at HEAD `b2dfa20` after a targeted convergence review confirmed every R3 finding was closed in v4 and no new BLOCKER was introduced. Three rounds of adversarial review (Claude + Codex) caught: v1 recon errors (OTLP wiring, events table), v2 design errors (retention in held lock, heterogeneous return types), v3 precision errors (wrong paths, wrong table name, decay double-count, unversioned JSON contract). v4 addresses all. Planner re-verifies recon at implementation time per Task 1.

**Phase position:** First of three 2A-4d tiers.

| Tier | Description | Unblocks |
|------|-------------|----------|
| **2A-4d.1 (this spec)** | Per-phase `tracing::info_span!` + 3 new Prometheus metric families + write phase observations to `kpi_events` + convert worker `eprintln!`/`println!` to `tracing` | Everything downstream. |
| 2A-4d.2 | Observability API (`/inspect`, SSE, `forge-next observe`, HUD drift, `forge_layer_freshness_seconds`, `kpi_events` retention reaper) | Live user-facing observability. |
| 2A-4d.3 | Bench harness + fixtures + `bench_runs` table + CI-per-commit + leaderboard | Quality as time series. |

Each tier ships independently (two reviews + merge + dogfood) before the next starts.

---

## 1. Goal

Turn every consolidator phase into a first-class observable event.

**Before this work:**
- Workers emit ~184 `eprintln!`/`println!` calls across 11 files; not structured, not queryable, not exported.
- `forge_*` Prometheus metrics cover 7 top-level gauges; no per-phase or per-table granularity.
- `tracing-opentelemetry` + `opentelemetry-otlp 0.27` are already wired (`init_otlp_layer` at `main.rs:91-130`); spans land in Jaeger/Grafana when `FORGE_OTLP_ENABLED=true` — but nothing in the consolidator emits phase-level spans.
- `kpi_events` table (`schema.rs:255-266`) exists but has ZERO writers. Documented-but-unbuilt shared logging surface.

**After this work:**
- Every consolidator phase emits a `tracing::info_span!` rooted at `consolidate_pass`; spans carry `phase_name`, `output_count`, `duration_ms`, `error_count`, `correlation_id`. OTLP picks them up where enabled — zero new init code.
- `ForgeMetrics` gains 3 new families: per-phase duration histogram, per-phase output counter, per-table rows gauge.
- Each consolidator phase writes one row to `kpi_events` with `event_type='phase_completed'` and a versioned `metadata_json` payload. Tier 1 claims the `phase_completed` event_type namespace.
- Worker code has zero `eprintln!`/`println!` outside `#[cfg(test)]`.

**Success metric:** a Grafana dashboard can, without custom SQL, answer: duration per phase over time; skills inferred per hour; slowest phase on average; trace of one consolidation pass (span per phase).

---

## 2. Verified reconnaissance (2026-04-24, HEAD `7ed071e`)

| # | Fact | Evidence |
|---|------|----------|
| 1 | 23 consolidator phases in `run_all_phases`. | `grep -cE "^\s*// Phase [0-9]+:" consolidator.rs` → 23 |
| 2 | OTLP exporter fully wired at `main.rs:91-130`; uses `opentelemetry-otlp 0.27` tonic/gRPC via `FORGE_OTLP_*` env vars. | `main.rs:91-178` |
| 3 | `OtlpConfig` struct at `config.rs:356-365` is UNUSED by code. `main.rs:140-141` comment explains the chicken-and-egg: config loading logs, logger isn't initialized yet. | `rg OtlpConfig --type rust` |
| 4 | `kpi_events` exists; **zero writers in codebase today**. `kpi_snapshots` + `kpi_benchmarks` also exist, also zero writers. | `grep -rn "INSERT INTO kpi_events"` returns empty. |
| 5 | 184 `eprintln!`/`println!` sites across 11 worker files. Largest: `consolidator.rs` (69), `indexer.rs` (33), `extractor.rs` (22). | `grep -c 'eprintln!\|println!' crates/daemon/src/workers/*.rs` |
| 6 | `PHASE_ORDER`, `Request::ProbePhase`, and its handler are all `#[cfg(any(test, feature = "bench"))]`. | `consolidator.rs:37`, `request.rs:142`, `handler.rs:1391` |
| 7 | `prometheus 0.13` + `ForgeMetrics` Registry exist with 7 families. | `server/metrics.rs` |
| 8 | `reaper.rs` is session-specific (heartbeat timeouts). Generic time-based sweeper absent. | `workers/reaper.rs` |
| 9 | `cargo test --workspace` baseline at HEAD: 1388+ pass, 0 failed, 1 ignored. | Last full run. |
| 10 | Phase fn signatures are heterogeneous. `ops::*` lives in `crates/daemon/src/db/ops.rs` (single file, ~2000 lines) and `detect_entities` is in `crates/daemon/src/db/manas.rs:1947`. **There is NO `workers/ops/` directory** (v3 recon error). | `ls crates/daemon/src/workers` + `grep "fn decay_memories" crates/daemon/src/db/ops.rs` |
| 11 | `run_consolidator` holds `state.lock().await` across all of `run_all_phases` (`consolidator.rs:1897-1905`). | Direct inspection. |
| 12 | `identity` table exists at `schema.rs:479` with only `created_at`, no `updated_at`. There is **no `identity_trait` table** (v3 recon error). | `grep "CREATE TABLE IF NOT EXISTS"` in schema.rs. |
| 13 | Memory subtypes (decision, lesson, pattern) share the `memory` table via a `memory_type` column — NOT distinct tables. | `schema.rs:388`+ |
| 14 | `decay_memories(conn, limit, half_life)` returns `(checked, faded)` where `faded ⊆ checked`. Summing them double-counts. | `db/ops.rs:925-955` |
| 15 | `forge_consolidation_harness.rs` pins `seed: 42`, uses in-memory state, deterministic corpus + embeddings. Run-to-run stable. | `crates/daemon/tests/forge_consolidation_harness.rs:6` |
| 16 | `docs/architecture/` does **not** exist. Existing docs subdirs: `benchmarks/`, `images/`, `operations/`, `superpowers/`. | `ls docs/` |

Planner re-verifies these at implementation time in case anything drifts.

---

## 3. Architecture

### 3.1 Span wrapping — heterogeneous return types

Phase fn signatures are NOT refactored. Timing + counter capture happens at the call site via the `PhaseOutcome` helper (new module `workers/instrumentation.rs`).

**`PhaseOutcome` struct:**
```rust
pub struct PhaseOutcome {
    pub phase: &'static str,            // span name, must appear in PHASE_SPAN_NAMES
    pub run_id: String,                  // ULID of consolidate_pass
    pub output_count: u64,
    pub error_count: u64,                // 1 for phases that return Err; see projection table
    pub duration_ms: u64,
    pub extra: serde_json::Value,        // phase-specific fields; contract in §3.4
}
```

**Four canonical wrappers** (one per return shape; §4 Task 3 enumerates per-phase choice):

**W1 — plain `usize` (Phase 3, 10, 23 — 3 phases):**
```rust
let span = tracing::info_span!("phase_23_infer_skills_from_behavior", run_id = %run_id);
let _enter = span.enter();
let t0 = std::time::Instant::now();
let output = infer_skills_from_behavior(conn, cfg.min_sessions, cfg.window_days);
let out = PhaseOutcome { phase: "phase_23_infer_skills_from_behavior", run_id: run_id.clone(),
    output_count: output as u64, error_count: 0, duration_ms: t0.elapsed().as_millis() as u64,
    extra: json!({}) };
record(conn, &metrics, &out);
```

**W2 — `Result<usize>` (Phases 1, 2, 5, 7, 9, 11 — 6 phases):**
```rust
let span = tracing::info_span!(…); let _enter = span.enter();
let t0 = std::time::Instant::now();
let (output, err) = match ops::dedup_memories(conn) {
    Ok(n) => (n as u64, 0),
    Err(e) => { tracing::error!(error = %e, "phase_1 failed"); (0, 1) }
};
record(conn, &metrics, &PhaseOutcome { …, output_count: output, error_count: err, … });
```

**W3 — tuple `(u, v)`:** per §3.1a Projection Table below.

**W4 — `Vec<Item>` + inner UPDATE loop (Phase 6):** output_count = `candidates.len()`. Swallowed per-update errors surface via `tracing::warn!` inside the loop; `error_count = 0` at span close because the call-site cannot observe loop errors cleanly without a signature change. (Acceptable: Tier 1 prioritizes per-phase attribution; fine-grained within-phase errors are Tier 2 / `/inspect audit` territory.)

#### 3.1a PhaseOutputProjection table (per-phase formula)

| Phase | Fn | Return | `output_count` | `error_count` tracked? | Notes |
|-------|----|----|----|-----|-----|
| 1 `dedup_memories` | ops::dedup_memories | `Result<usize>` | Ok value; 0 on Err | Yes (W2) | — |
| 2 `semantic_dedup` | ops::semantic_dedup | `Result<usize>` | Ok value; 0 on Err | Yes (W2) | — |
| 3 `link_memories` | ops::link_memories | `usize` | raw | No (W1) | Internal `.unwrap_or(0)` — swallowed SQL errs invisible. |
| 4 `decay_memories` | ops::decay_memories | `Result<(checked, faded)>` | **`faded`** (NOT `checked + faded` — faded ⊆ checked, sum double-counts). | Yes (W2 adapted) | `checked` recorded in `extra.checked_count`. |
| 5 `promote_patterns` | ops::promote_patterns | `Result<usize>` | Ok; 0 on Err | Yes (W2) | — |
| 6 `reconsolidate_contradicting` | ops::find_reconsolidation_candidates + inner loop | `Result<Vec<Memory>>` | `candidates.len()` | Partial (W4) | Inner UPDATE errors → `tracing::warn!`; call-site cannot count cleanly. |
| 7 `merge_embedding_duplicates` | ops::merge_embedding_duplicates | `Result<usize>` | Ok; 0 on Err | Yes (W2) | — |
| 8 `strengthen_by_access` | ops::strengthen_by_access | `Result<(usize, usize)>` | **sum** (boosted + normalized are disjoint) | Yes (W2 adapted) | Distinguishable via `extra.strengthened_boosted/normalized`. |
| 9 `score_memory_quality` | quality scorer | `Result<usize>` | Ok; 0 on Err | Yes (W2) | — |
| 10 `entity_detection` | db::manas::detect_entities | `Result<usize>` | Ok; 0 on Err | Yes (W2) | — |
| 11 `synthesize_contradictions` | ops::synthesize_contradictions | `usize` | raw | No (W1) | Internal error-swallow; see Phase 3 note. |
| 12 `detect_and_surface_gaps` | ops::detect_and_surface_gaps | `usize` | raw | No (W1) | — |
| 13 `reweave_memories` | ops::reweave_memories | `usize` | raw | No (W1) | — |
| 14 `flip_stale_preferences` | ops::flip_stale_preferences | `Result<usize>` | Ok; 0 on Err | Yes (W2) | 2A-4a shipped. |
| 15 `apply_recency_decay` | ops::apply_recency_decay | `usize` | raw | No (W1) | 2A-4b shipped. |
| 16 `compute_effectiveness` | db::effectiveness::* | `Result<usize>` | Ok; 0 on Err | Yes (W2) | — |
| 17 `extract_protocols` | ops::extract_protocols | `usize` | raw | No (W1) | — |
| 18 `tag_antipatterns` | ops::tag_antipatterns | `usize` | raw | No (W1) | — |
| 19 `emit_notifications` | notifications::enqueue_* | `usize` | raw | No (W1) | Throttled internally. |
| 20 `record_tool_use_kpis` | kpi:: | `usize` | raw | No (W1) | — |
| 21 `run_healing_checks` | healing::run | `HealingStats` | `stats.healed` | Partial | `extra.healing_full_stats = {...}`. |
| 22 `apply_quality_pressure` | ops::apply_quality_pressure | `usize` | raw | No (W1) | Internal `.unwrap_or(0)` ×2. |
| 23 `infer_skills_from_behavior` | skill_inference:: | `usize` | raw | No (W1) | 2A-4c2 shipped. |

**Acknowledged limitation:** Phases 3, 11, 12, 13, 15, 17, 18, 19, 20, 22, 23 (11 phases) internally `.unwrap_or(0)` their SQL errors, so `error_count` cannot be set from the call site. `/inspect audit` in Tier 2 can surface these via structured tracing::error! events that Tier 1 DOES emit for internal failures. Documented as a Risk (R8).

### 3.2 Prometheus surface — 3 new families

```
forge_phase_duration_seconds{phase="phase_23_infer_skills_from_behavior"}   # histogram
  buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0]

forge_phase_output_rows_total{phase=..., action="succeeded|errored"}         # counter

forge_table_rows{table=...}                                                  # gauge (NO _total)
```

`forge_table_rows` labels MUST match actual SQLite table names. Verified against `schema.rs`:
- `memory`, `skill`, `edge`, `identity`, `disposition`, `platform`, `tool`, `perception`, `declared`, `domain_dna`, `entity`

(That's 11 tables. `identity_trait` DOES NOT exist — v3 error corrected. Memory subtypes share the `memory` table; they are NOT separate labels.)

**Cardinality:** 23 × 1 + 23 × 2 + 11 × 1 = **80 new series.** Well under Prometheus budget.

Gauges refresh via extended `refresh_gauges()`. Counter `.inc()`, histogram `.observe()` in the call-site wrapper.

### 3.3 OTLP integration — zero new wiring

OTLP is fully wired. Tier 1 only emits spans. When `FORGE_OTLP_ENABLED=true`, `tracing-opentelemetry` picks them up via the existing layer and exports through the existing gRPC batch exporter.

**`OtlpConfig` vs env-var dual-path:** EXPLICITLY DEFERRED (Tier 3). `main.rs:140-141`'s chicken-and-egg is real; resolving needs two-phase init or a subscriber-reinstall dance. Unused config struct stays as reserved scaffolding.

**Collector-unreachable behavior:** `opentelemetry_sdk::trace::BatchSpanProcessor` (0.27) **does not retry** failed exports. On queue-full it drops spans + emits via `otel::handle_error`. Daemon never exits on collector problems. v3 called this "standard retry"; v4 correctly states: **drop-on-failure, no automatic retry, process continues.** If operators want retry, it's a Tier 3 upstream task.

### 3.4 `kpi_events` — first writer, versioned contract

v3 had `metadata_json` unversioned as a "stable" surface. v4 versions it from day 1. Tier 1 writes:

```sql
INSERT INTO kpi_events (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)
VALUES (?, strftime('%s','now'), 'phase_completed', NULL, ?, ?, ?, ?);
```

`id` format: **`phase-<ULID>`** (26-char ULID prefixed with `phase-` so Tier 2 can grep event provenance without table joins). ULID generated via existing `ulid::Ulid::new().to_string()` pattern (same as `crates/daemon/src/db/metrics.rs:56`).

**Versioned metadata_json v1 contract:**
```json
{
  "metadata_schema_version": 1,
  "phase_name": "phase_23_infer_skills_from_behavior",
  "run_id": "01HXYZ...",
  "correlation_id": "01HXYZ...",
  "trace_id": "abc123..." | null,
  "output_count": 1,
  "error_count": 0,
  "extra": {}
}
```

Field semantics:
- `metadata_schema_version`: integer, bumped on breaking change. Tier 2 readers validate.
- `correlation_id`: ULID (26 chars). Populated from the `consolidate_pass` run ULID. Always present — this is the primary join key across phases in the same pass.
- `trace_id`: 32-char hex OTLP trace id. Populated only when OTLP is enabled + active. Null when OTLP is off. Keeping these as **separate fields** resolves the v3 ULID/trace_id encoding-mismatch issue (Claude M4 + Codex P11).
- `run_id`: alias for `correlation_id` kept for human-readable logging; may merge in a future version.
- `extra`: phase-specific JSON object, see §3.1a for projections that use it.

**Namespace register:** `docs/architecture/kpi_events-namespace.md` is created as part of Task 6. Claims `phase_completed` for Tier 1. Other writers (Tier 2, Tier 3, future phases) register here first.

`docs/architecture/` is a new directory (recon #16). Task 6 creates it with a minimal `README.md` gateway file pointing at the namespace register.

**Retention:** EXPLICITLY DEFERRED to Tier 2 as a dedicated reaper worker. Tier 1 ships unbounded table growth — at 23 × 48 × 365 ≈ 400k rows/year (~40 MB) this is acceptable for a year of pre-retention use while Tier 2 lands.

### 3.5 `eprintln!`/`println!` convergence

184 sites × 11 files. Commits sized by sites-per-commit (not lines, per Codex P8):

| File | Sites | Commits | Per-commit sites |
|------|-------|---------|------------------|
| `consolidator.rs` | 69 | 2 | ≤ 35 sites each |
| `indexer.rs` | 33 | 1 | 33 |
| `extractor.rs` | 22 | 1 | 22 |
| `diagnostics.rs` | 13 | 1 | 13 |
| `disposition.rs` | 12 | 1 | 12 |
| `watcher.rs` + `perception.rs` + `embedder.rs` | 26 | 1 bundled | ≤ 26 |
| `reaper.rs` + `mod.rs` | 9 | 1 bundled | 9 |
| `skill_inference.rs` | 0 | — | — |

**8 commits** (v3 said 7 — consolidator split remains 2). Consolidator 69-site split: by **contiguous source-line regions** (commit A = line range 1-1000, commit B = 1000+). Codex P9's "init/error/progress" heuristic doesn't map; regions are the honest split.

Conversion patterns:
- `eprintln!("[W] X")` → `tracing::info!(target: "forge::W", "X")`.
- `eprintln!("[W] error: {e}")` → `tracing::error!(target: "forge::W", error = %e, "…")`.
- `println!(...)` stays only in `forge-cli`.

### 3.6 Span-name integrity

v3 left span names as raw string literals at call sites (silent drift risk if a phase is renamed). v4 introduces:

```rust
// workers/instrumentation.rs
pub const PHASE_SPAN_NAMES: &[&str; 23] = &[
    "phase_1_dedup_memories",
    "phase_2_semantic_dedup",
    // … in the ORDER phases fire in run_all_phases
    "phase_23_infer_skills_from_behavior",
];
```

Each call site uses the matching string literal. A unit test asserts:
```rust
#[test]
fn span_name_count_matches_phase_count() {
    let src = include_str!("consolidator.rs");
    let count = src.matches("info_span!(\"phase_").count();
    assert_eq!(count, PHASE_SPAN_NAMES.len());
}
```

This catches silent drift — rename a phase, forget the literal, test fails.

### 3.7 Latency budget — deterministic baseline

`forge_consolidation_harness.rs` is deterministic (recon #15). Baseline + post-Tier-1 measurements both use:

```
cargo test -p forge-daemon --test forge_consolidation_harness \
  --release -- --test-threads=1 --nocapture
```

N = 5 runs. Median-of-medians (MoM) per-phase. Baseline file lives at `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1-baseline.md`, committed as the first Tier-1 commit.

Budget (MoM deltas):
- Cold start, OTLP disabled: ≤ 20 ms regression.
- Cold start, OTLP enabled + local collector: ≤ 100 ms.
- Steady-state CPU (5 min idle): ≤ 2%.
- `force_consolidate` on seeded 100-memory DB: ≤ 10 ms total.

Regression > budget = deferral. Measurement is Task 11.

### 3.8 Lock-scope non-regression

All helpers (`record`, `update_phase_metrics`, `record_phase_outcome_insert`) accept `&Connection` + `&ForgeMetrics` only. **No helper takes `Arc<Mutex<DaemonState>>` or calls `.lock().await`.** The existing `run_consolidator` already holds the state lock across `run_all_phases` (recon #11); Tier 1 introduces zero new lock acquisitions and cannot deadlock.

Compile-time enforced by signature review at Task 2.

### 3.9 Future-proof CI guard

Grep check added to the `check` CI job (Rust-source policy — NOT `plugin-surface`, which is JSON/shellcheck land):

```yaml
- name: Span-integrity guard
  run: |
    set -euo pipefail
    # Ensure PHASE_SPAN_NAMES stays in sync with actual span count.
    count=$(grep -c 'info_span!("phase_' crates/daemon/src/workers/consolidator.rs)
    [ "$count" = "23" ] || { echo "span count $count != 23"; exit 1; }
    # Ensure no new tokio::spawn sneaks into consolidator without .instrument(span).
    ! grep -n 'tokio::spawn' crates/daemon/src/workers/consolidator.rs
    ! grep -n 'tokio::spawn' crates/daemon/src/db/ops.rs      # phase ops shouldn't spawn either
```

---

## 4. Task list (plan file elaborates)

1. **Re-verify recon.** Re-run §2 commands.
2. **Create `workers/instrumentation.rs`** with `PhaseOutcome`, `PHASE_SPAN_NAMES`, `record(...)`, `update_phase_metrics(...)`, `insert_kpi_event_row(...)`. Pure helpers; no lock acquisitions.
3. **Wrap 23 phase call sites** in `run_all_phases` per §3.1 templates + §3.1a projection table. One commit for the wrapping (so span-integrity test flips green at once).
4. **Extend `ForgeMetrics`** with the 3 new families per §3.2. Wire `refresh_gauges` for `forge_table_rows`.
5. **Write `docs/architecture/kpi_events-namespace.md`** (+ `docs/architecture/README.md` gateway).
6. **Convert `eprintln!`/`println!` → `tracing`** per §3.5. 8 commits.
7. **CI guards** per §3.9 (both span-integrity + tokio::spawn prohibition).
8. **Adversarial reviews** on T1-T7 diff.
9. **Address BLOCKER + HIGH findings.**
10. **Latency-budget measurement** per §3.7 — baseline + post snapshots committed.
11. **Live-daemon dogfood + results doc** at `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1.md`. Seeded DB, `force_consolidate`, prove: `/metrics` has 3 new families with non-zero values + `kpi_events` has 23 `phase_completed` rows per pass + OTLP spans visible in a local Jaeger container.

---

## 5. Non-goals (explicit)

- **No `/inspect` endpoint.** Tier 2.
- **No `bench_runs` / scoring.** Tier 3.
- **No HUD UI changes.** Tier 2.
- **No Grafana dashboard JSON committed.**
- **No SSE / streaming.** Tier 2.
- **No `OtlpConfig` → env-var resolution.** Tier 3.
- **No `PhaseResult` / `ops::*` signature refactor** (§3.1 keeps signatures).
- **No `kpi_events` retention.** Tier 2.
- **No `forge_layer_freshness_seconds`.** Tier 2.
- **No promotion of `PHASE_ORDER` / `Request::ProbePhase`.** §3.6.
- **No error_count for ~11 phases that internally `.unwrap_or(0)`.** Tier 2's `/inspect audit` covers them.

---

## 6. Backward compatibility

- `/metrics`: 7 existing families unchanged + 3 additive.
- `ResponseData::Doctor` / `ConsolidationComplete`: unchanged.
- `kpi_events` schema unchanged; Tier 1 is the first writer.
- OTLP env vars continue to work.

**Breaking:** worker stderr stops carrying `[consolidator] …` unstructured lines. Operators update log filters to JSON event queries. Migration note in results doc.

---

## 7. Open questions (resolved)

All open questions from v3 closed:
- **Q1 (retention policy):** Tier 2 defines; Tier 1 emits freely.
- **Q2 (trace_id fallback):** Separate `correlation_id` (ULID) + `trace_id` (OTLP hex).
- **Q3 (span naming):** underscore-only, literal in code, matched by `PHASE_SPAN_NAMES` const.

---

## 8. Acceptance criteria

- [ ] `/metrics` returns ≥ 10 metric families (7 existing + 3 new) with non-zero values after a seeded consolidation pass.
- [ ] After one `force_consolidate` on a seeded DB, `SELECT COUNT(*) FROM kpi_events WHERE event_type='phase_completed'` == 23 (every phase emits one row, including no-op passes).
- [ ] `metadata_schema_version == 1` in every row.
- [ ] With `FORGE_OTLP_ENABLED=true` + `FORGE_OTLP_ENDPOINT=http://localhost:4317` + a local Jaeger-all-in-one, one pass produces one trace with 23 child spans.
- [ ] `grep -rn 'eprintln!\|println!' crates/daemon/src/workers/*.rs` (excluding `#[cfg(test)]` blocks) returns 0.
- [ ] `PHASE_ORDER`, `Request::ProbePhase`, and its handler remain cfg-gated.
- [ ] `cargo test --workspace` passes ≥ baseline (1388) + ≥ 23 new instrumentation tests + any auxiliary (helper / metrics / contract) tests; explicit upper bound NOT enforced.
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` clean.
- [ ] `scripts/check-harness-sync.sh` clean.
- [ ] CI span-integrity guard + tokio::spawn prohibition installed in the `check` job (Rust-source policy lives alongside Rust validation, not in `plugin-surface`).
- [ ] Latency budget measured (§3.7) and within limits.
- [ ] Two adversarial reviews complete on T1-T7 diff.
- [ ] `docs/architecture/kpi_events-namespace.md` committed with `phase_completed` entry.

---

## 9. Risks (residual)

- **R1** — `opentelemetry` dep API churn. Locked to 0.27.
- **R2** — Convergence fatigue over 8 commits. Mitigated by sites-per-commit cap.
- **R3** — `kpi_events` write amplification. 1.1k inserts/day; negligible.
- **R4** — Span context through `tokio::spawn`. CI guard (§3.9).
- **R5** — OTLP collector unreachable. Drop-on-failure + log warning.
- **R6** — Unbounded `kpi_events` growth. ~40 MB/year; acceptable until Tier 2.
- **R7** — `kpi_snapshots`/`kpi_benchmarks` siblings — don't confuse namespaces; Tier 1 only writes `kpi_events` with `event_type='phase_completed'`.
- **R8** — **`error_count` cannot be set for 11 phases that swallow errors internally** (§3.1a notes). Tier 2's `/inspect audit` shape recovers them via tracing::error! events.
- **R9** — Baseline run-to-run noise. Mitigated via N=5 MoM + deterministic seed (§3.7).
- **R10** — `metadata_json` v1 contract evolution. Version field bumps on any breaking change; Tier 2 readers must validate.

---

## 10. References

- `crates/daemon/src/main.rs:91-178` — OTLP wiring.
- `crates/daemon/src/server/metrics.rs` — Prometheus surface.
- `crates/daemon/src/db/schema.rs:255-266` — `kpi_events`.
- `crates/daemon/src/db/schema.rs:479` — `identity` table.
- `crates/daemon/src/db/ops.rs` — phase ops (single file, no `workers/ops/` dir).
- `crates/daemon/src/db/manas.rs:1947` — `detect_entities` (Phase 11).
- `crates/daemon/src/workers/consolidator.rs:82-…` — `run_all_phases`.
- `crates/daemon/tests/forge_consolidation_harness.rs` — deterministic baseline harness.
- v1, v2, v3 preserved in git history (`d30eaab`, `65ebdf3`, `7ed071e`).
