# Forge-Identity Observability — Design v2 (Phase 2A-4d.1, Instrumentation Tier)

**Status:** DESIGN v2 — revised after v1 adversarial review found 2 BLOCKERs (false reconnaissance about OTLP wiring and events-table existence) + 3 HIGHs (cfg asymmetry, phase count, refactor scope). Spec is LOCKED once this version passes two adversarial reviews.

**Phase position:** First of three 2A-4d tiers. Sequence:

| Tier | Phase | Description | Unblocks |
|------|-------|-------------|----------|
| **2A-4d.1 (this spec)** | Instrumentation | `info_span!` around every consolidator phase + 4 new Prometheus metric families + write to existing `kpi_events` table + replace worker `eprintln!`/`println!` with `tracing` events | Every later observability consumer. |
| 2A-4d.2 | Observability API | Generic `/inspect {layer, shape, window}` + SSE stream + `forge-next observe` CLI + HUD drift display | Live user-facing observability. |
| 2A-4d.3 | Bench Harness | `forge-bench identity` + fixtures + `bench_runs` table + ablation flags + CI-per-commit + leaderboard | Quality as a time series. |

Each tier ships independently (two reviews + merge + dogfood) before the next starts.

---

## 1. Goal

Turn every consolidator phase and every Manas layer write into a first-class observable event.

**Before this work:**
- Workers emit ~184 `eprintln!("[consolidator] …")` calls to stderr (counted 2026-04-24 across 11 files). Not structured. Not queryable. Not exported.
- `forge_*` Prometheus metrics exist for 7 top-level gauges but have no per-phase or per-layer granularity. You can't answer "how long did Phase 23 take last pass?" or "which phase errors most?".
- `tracing-opentelemetry` + `opentelemetry-otlp` 0.27 are **already wired** (`init_otlp_layer` at `crates/daemon/src/main.rs:91-130`) and active when `FORGE_OTLP_ENABLED=true` + `FORGE_OTLP_ENDPOINT` is set — but nothing in the consolidator emits phase-level spans, so the collector receives only top-level subscriber spans (limited value).
- `kpi_events` table exists (`schema.rs:255-266`) with columns `(id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)` + two indexes. Currently used by perception/recall telemetry; no consolidator writes.

**After this work:**
- Every consolidator phase is wrapped in `tracing::info_span!` rooted at a `consolidate_pass` span; spans carry `phase_name`, `input_count`, `output_count`, `duration_ms`, `error_count`. When OTLP is enabled, these land in Jaeger/Grafana as-is.
- `ForgeMetrics` gains 4 new families: per-phase duration histogram, per-phase output counter, per-layer rows gauge, per-layer freshness gauge.
- Each phase writes one row to `kpi_events` with `event_type="phase_completed"` and phase-specific fields in `metadata_json`. Tier 2's `/inspect` will read this.
- Worker code contains zero `eprintln!`/`println!` outside `#[cfg(test)]`. Every event is a structured `tracing::info!`/`warn!`/`error!`.

**Success metric:** A Grafana dashboard (or equivalent) can, without custom SQL, answer:
1. "Duration of each consolidation phase over the last hour."
2. "Rate of skills inferred per hour."
3. "Which worker is slowest on average."
4. "Trace view of a single consolidation pass (span per phase)."

---

## 2. Verified reconnaissance (2026-04-24 at HEAD `d30eaab`)

Each fact below was independently confirmed against current code; planner must re-check at implementation time in case anything drifts.

| # | Fact | Evidence |
|---|------|----------|
| 1 | **23 consolidator phases** run in `consolidator.rs::run_all_phases` (not 22). Phase 23 was inserted between phases 17 and 18 per 2A-4c2 T5. | `grep -cE "^\s*// Phase [0-9]+:" crates/daemon/src/workers/consolidator.rs` → 23 |
| 2 | **OTLP exporter is fully wired** — `init_otlp_layer` at `main.rs:91-130` builds a `TracerProvider` via `opentelemetry_otlp 0.27` (tonic/gRPC), sets it as the global provider, and composes a `tracing-opentelemetry::OpenTelemetryLayer` into the subscriber when `FORGE_OTLP_ENABLED=true` + `FORGE_OTLP_ENDPOINT` is set. | `main.rs:91-178`; `Cargo.toml:opentelemetry` block |
| 3 | **OTLP config dual-path.** `OtlpConfig` struct (`config.rs:356-365`) is NEVER read — wiring uses `FORGE_OTLP_*` env vars directly. Resolve this in Tier 1: either read the config struct or delete it. | `rg OtlpConfig --type rust` shows references only in `config.rs` + tests. |
| 4 | **`kpi_events` table exists** with columns `(id TEXT PK, timestamp INTEGER, event_type TEXT, project TEXT, latency_ms INTEGER, result_count INTEGER, success INTEGER, metadata_json TEXT)`. Indexed on `timestamp` and `event_type`. Currently written by perception + recall telemetry. No consolidator writes. | `schema.rs:255-266`; `rg "INSERT INTO kpi_events"` → 0 consolidator hits. |
| 5 | **`eprintln!`/`println!` site count in `crates/daemon/src/workers/`: 184 across 11 files**. Largest: `consolidator.rs` (69), `indexer.rs` (33), `extractor.rs` (22). | `grep -c 'eprintln!\|println!' crates/daemon/src/workers/*.rs \| sort -n` |
| 6 | **`PHASE_ORDER` const + `Request::ProbePhase` + handler are ALL `#[cfg(any(test, feature = "bench"))]`.** Symmetric today. | `consolidator.rs:37`, `request.rs:142`, `handler.rs:1391`. |
| 7 | **`prometheus 0.13` + `ForgeMetrics` Registry already exist** (`server/metrics.rs:20`) with 7 metric families, `refresh_gauges` per scrape, and a `/metrics` endpoint that returns 404 when disabled. 6 unit tests cover it. | `server/metrics.rs`; `Cargo.toml:58` |
| 8 | **`reaper` worker is session-specific** (`reaper.rs` — heartbeat timeouts). Not a generic time-based sweeper. Any new retention policy needs either an expansion of `reaper.rs` or a new worker, not "drop into reaper". | `crates/daemon/src/workers/reaper.rs` |
| 9 | **Daemon test baseline at HEAD `d30eaab`: 1386 daemon lib tests (+ ~35 integration), `cargo test --workspace` returns 1388+ pass**. Instrumentation must not regress this. | Output of last `cargo test --workspace` (pre-spec). |
| 10 | **`consolidator.rs` phase fns have non-uniform return types** (`Ok(usize)`, `Ok((usize, usize))` for recency decay, `Ok(Vec<Memory>)`, etc.). A `PhaseResult` refactor that unifies them would touch 23 call sites + ~20 `ops::*` functions across `workers/ops/` + tests. | `grep "match ops::" consolidator.rs`; multiple return shapes visible in existing code. |

---

## 3. Architecture

### 3.1 Span hierarchy (no phase-fn signature change)

v1 proposed refactoring all 23 phase fns to return a `PhaseResult` struct — recon fact #10 shows this is a ~20-file cross-cutting refactor that would balloon Tier 1 scope. **v2 keeps all phase fn signatures unchanged.** Timing + counter capture happens at the *call site* in `run_all_phases`.

Each call site is wrapped in a local pattern:

```rust
// Example for Phase 23.
let _span = tracing::info_span!(
    "phase_23_infer_skills_from_behavior",
    run_id = %run_id,
    min_sessions = config.skill_inference_min_sessions,
    window_days = config.skill_inference_window_days,
);
let t0 = std::time::Instant::now();
let output = infer_skills_from_behavior(
    conn,
    config.skill_inference_min_sessions,
    config.skill_inference_window_days,
);
let duration_ms = t0.elapsed().as_millis() as u64;
stats.skills_inferred = output;
tracing::info!(
    output_count = output,
    duration_ms,
    "phase_23 complete"
);
kpi_events::record_phase(conn, "phase_23_infer_skills_from_behavior", duration_ms, output, &run_id);
```

Input counts that aren't returned by the existing phase fn are read from a pre-phase `COUNT(*)` where meaningful; otherwise left as `0` with a doc comment. This avoids the PhaseResult refactor cost.

Spans:
- Root: `consolidate_pass { run_id, triggered_by: "scheduled"|"force_consolidate"|"startup_task", start_iso }`. Run ID is a ULID generated at pass start.
- Children: one `phase_N_<name>` span per phase. 23 children per pass.
- Grandchildren: not committed in Tier 1 (e.g., per-INSERT spans are too fine-grained; defer to Tier 2 if needed).

### 3.2 Prometheus surface — 4 new families

```
# Phase duration histogram, per phase.
forge_phase_duration_seconds{phase="phase_23_infer_skills_from_behavior"}
  buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0]

# Phase output counter, per phase × action.
forge_phase_output_rows_total{phase="...", action="inserted|updated|skipped|errored"}

# Per-layer row gauge (supplements the 7 existing top-level gauges).
forge_layer_rows_total{layer="skill|decision|lesson|pattern|platform|tool|identity|disposition|perception|declared|entity"}

# Per-layer freshness — seconds since last write in that layer.
forge_layer_freshness_seconds{layer=...}
```

**Cardinality math:** 23 phases × 1 histogram = 23 series; 23 phases × 4 actions = 92 series; 11 layers × 1 gauge + 11 × 1 freshness = 22 series. **Total new series: ≤ 137.** Prometheus budgets are typically 100k+ per instance — this is well under noise. In-memory registry impact on the daemon: negligible.

Gauges are refreshed in `refresh_gauges()` on each `/metrics` scrape. Counters are `.inc()`ed at span close in the call-site wrapper. Histograms `.observe(duration)` at span close.

### 3.3 OTLP integration — no new wiring, just span emission

Recon fact #2 confirms OTLP is already wired end-to-end. The sole Tier 1 OTLP task is: **when phases emit spans via `info_span!`, `tracing-opentelemetry` picks them up automatically and exports to the configured OTLP endpoint**. Zero new init code.

**Tier 1 resolves the config dual-path** (recon fact #3): `init_otlp_layer` today reads `FORGE_OTLP_ENABLED` + `FORGE_OTLP_ENDPOINT` + `FORGE_OTLP_SERVICE_NAME` env vars. We change it to read `OtlpConfig` from `ForgeConfig`, with env var overrides (same precedence pattern as `HttpConfig`). Any existing operator relying on env vars continues to work.

Fails-loud policy: if `otlp.enabled = true` but the collector is unreachable at daemon startup, the daemon **logs a warning + starts without OTLP export** (does NOT exit). OTLP is an observability concern; unavailable collector must never take the daemon down. This is the industry-standard "fire-and-forget observability" pattern.

### 3.4 `kpi_events` — reuse, don't reinvent

v1 proposed a new `phase_event` table. Recon fact #4 shows `kpi_events` already exists with a suitable shape. **v2 reuses it.**

Schema stays identical — no migration needed. Consolidator writes:

```sql
INSERT INTO kpi_events (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)
VALUES (?, strftime('%s','now'), 'phase_completed', NULL, ?, ?, ?, ?);
```

with `metadata_json` carrying the phase-specific structured fields:

```json
{
  "phase_name": "phase_23_infer_skills_from_behavior",
  "run_id": "01HXYZ...",
  "input_count": 0,
  "output_count": 1,
  "error_count": 0,
  "trace_id": "abc123..." | null,
  "details": { "min_sessions": 3, "window_days": 30, "patterns_seen": 5 }
}
```

`result_count` column stores `output_count` (for fast roll-up queries without JSON parsing). `latency_ms` stores `duration_ms`. `success` is `1` if `error_count == 0`.

**Retention:** `kpi_events` has no existing reaper. Tier 1 adds a minimal retention task that runs inside the existing consolidator worker (same thread, same interval) — `DELETE FROM kpi_events WHERE timestamp < strftime('%s','now') - (86400 * retention_days)`. Default 30 days; override via new `metrics.kpi_events_retention_days` config field. Runs once per consolidator pass.

### 3.5 `eprintln!` / `println!` convergence

Recon fact #5: 184 sites across 11 files. v1's "one commit per worker directory" was unreviewable. **v2 ships one commit per worker file**, rolled up roughly as:

| Group | Files | Sites | Commit estimate |
|-------|-------|-------|-----------------|
| Consolidator | `consolidator.rs` | 69 | 1 commit, small diff per grep cluster |
| Indexer | `indexer.rs` | 33 | 1 commit |
| Extractor | `extractor.rs` | 22 | 1 commit |
| Diagnostics + Disposition | `diagnostics.rs`, `disposition.rs` | 13+12 | 1 commit each |
| Watcher / Embedder / Reaper / Perception / mod.rs | small files | 5-11 each | 1 bundled commit (≤ 40 lines) |

Conversion pattern:
- `eprintln!("[consolidator] inferred {n} skills")` → `tracing::info!(skills_inferred = n, phase = "phase_23", "phase complete")`.
- `eprintln!("error X: {e}")` → `tracing::error!(error = %e, "error X")`.
- `println!(...)` remains ONLY in `forge-cli` (user-facing output, explicit intent).

### 3.6 `PHASE_ORDER` cfg symmetry — explicitly kept test/bench-only

v1 Task 2 proposed promoting `PHASE_ORDER` to prod. Recon fact #6 shows the const + `Request::ProbePhase` + handler are all cfg-symmetric today — the review flagged that breaking that symmetry silently (promoting only the const) introduces latent drift.

**v2 resolves by NOT promoting any of them.** Instrumentation does not need `PHASE_ORDER` — span names are string literals at call sites (`"phase_23_infer_skills_from_behavior"`), and those literals are maintained alongside the phase call, not in a cross-file const. If a future phase reorders or renames, the span literal stays next to the call.

`Request::ProbePhase` stays a test-assertion escape hatch for master-design §9 verification, as 2A-4c2 T6 intended.

If a later tier needs a prod-visible list of phase names (e.g., Tier 2's `/inspect shape=audit`), it can be added as a separate `pub const PHASE_NAMES: &[&str]` next to `PHASE_ORDER`, decoupled from the probe API.

### 3.7 Span overhead & latency budget

Explicit Tier 1 budget:
- **Cold startup latency** with OTLP enabled + collector available: must regress ≤ 50 ms vs. OTLP-disabled baseline on the same machine.
- **Steady-state CPU** for a daemon with OTLP enabled and consolidator idle: must regress ≤ 2% over a 5-minute window.
- **Consolidator pass duration** for a seeded 100-memory DB: must regress ≤ 5 ms total (across 23 phases) vs. pre-Tier-1 baseline.

Measured in the Tier 1 dogfood results doc with a cold-start timer + `pidstat` sampling. Regression > budget = deferral, not ship.

---

## 4. Task list (for the plan file, not implemented here)

1. **Re-verify recon.** Re-run the commands in §2 at implementation time; update any drift.
2. **Wrap phase call sites in `info_span!` + timers** inside `run_all_phases`. 23 call sites. No signature change to phase fns.
3. **Expand Prometheus surface** — 4 new families per §3.2, wired into `refresh_gauges()` where gauges; `.observe()`/`.inc()` at call-site wrappers where histograms/counters.
4. **Write to `kpi_events` from each phase** via a `kpi_events::record_phase(conn, name, latency_ms, output, run_id)` helper.
5. **Resolve OTLP config dual-path** — read `OtlpConfig` struct; keep env var overrides.
6. **`eprintln!`/`println!` convergence** — one commit per worker file per §3.5 table.
7. **Retention task for `kpi_events`** — inline DELETE in consolidator's per-pass loop; add `metrics.kpi_events_retention_days` config (default 30).
8. **Adversarial reviews** (Claude + Codex) on T1-T7 diff.
9. **Address BLOCKER/HIGH findings.**
10. **Live-daemon dogfood + results doc.** Rebuild release daemon; seed 100 memories; run `force_consolidate`; verify: `/metrics` has ≥ 11 families with live values; `kpi_events` has 23 rows `event_type='phase_completed'` after one pass; OTLP spans land in a local Jaeger-all-in-one container.
11. **Latency budget measurement** per §3.7.

---

## 5. Non-goals (explicit, won't slip in)

- **No Observability API** (`/inspect`). Tier 2.
- **No bench scoring / `bench_runs` table.** Tier 3.
- **No HUD UI changes.** Tier 2.
- **No Grafana dashboard files** committed. Users build their own against the metric names; we don't pin a dashboard JSON.
- **No SSE / streaming endpoint.** Tier 2.
- **No ground-truth fixture curation.** Tier 3.
- **No promotion of `PHASE_ORDER` / `Request::ProbePhase` to prod** (see §3.6).
- **No `PhaseResult` refactor of `ops::*` fns** (see §3.1).

---

## 6. Backward compatibility

- `/metrics` exposes all 7 existing families unchanged + 4 additive families. Operators with pinned scrapers see additive data only.
- `ResponseData::Doctor` unchanged.
- `ConsolidationComplete` unchanged (2P-1b §15 already exposed `skills_inferred`).
- `kpi_events` schema unchanged (reused, not altered). No migration required.
- OTLP env vars `FORGE_OTLP_*` continue to work as override; existing operators see no change.

**Breaking behavior:** worker stderr stops carrying `[consolidator] …` style lines. Operators grepping daemon logs update their filters: JSON tracing events carry stable `phase`, `worker` fields that are more expressive. Migration note in the results doc covers this.

---

## 7. Open questions

- **Q1.** Should `trace_id` in `kpi_events.metadata_json` be always-populated (ULID fallback when OTLP disabled) or NULL when OTLP disabled? (Recommendation: always-populated — tier 2 can correlate even without a collector. Use the consolidate_pass ULID when no OTLP.)
- **Q2.** Retention default — 30 days feel right, or 7? (Recommendation: 30 days. 23 phases × 48 passes/day × 30 = ~33k rows. Negligible.)
- **Q3.** `OtlpConfig.enabled` default — stays `false`? (Recommendation: yes. Observability is opt-in.)

---

## 8. Acceptance criteria

- [ ] `/metrics` returns ≥ 11 metric families (7 existing + 4 new) with non-zero values after a seeded consolidation pass.
- [ ] After one `force_consolidate`, `SELECT COUNT(*) FROM kpi_events WHERE event_type='phase_completed'` == 23.
- [ ] With `FORGE_OTLP_ENABLED=true` + a local Jaeger-all-in-one container, a consolidation pass produces one trace with 23 child spans visible in Jaeger UI.
- [ ] `grep -rn 'eprintln!\|println!' crates/daemon/src/workers/*.rs` (excluding `#[cfg(test)]` blocks) returns 0 matches.
- [ ] `PHASE_ORDER`, `Request::ProbePhase`, and its handler remain `#[cfg(any(test, feature = "bench"))]` (§3.6 intentional).
- [ ] `cargo test --workspace` passes ≥ baseline (1388) + 23 new instrumentation tests.
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` clean.
- [ ] `scripts/check-harness-sync.sh` clean.
- [ ] Two adversarial reviews complete; BLOCKERs + HIGHs fixed or explicitly deferred with rationale.
- [ ] Latency budget (§3.7) measured + within limits.
- [ ] Results doc at `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1.md` — OTLP trace screenshot link, `/metrics` sample output, `eprintln!` count before/after, commit SHAs per task.

---

## 9. Risks

- **R1 — OTLP dep churn.** `opentelemetry-*` had breaking changes pre-0.24. Locked to 0.27 per recon fact #2; monitor upstream before touching exporter init.
- **R2 — Prometheus cardinality.** Bounded at 137 new series (§3.2); safe.
- **R3 — `kpi_events` table write amplification.** 23 inserts/pass × 48 passes/day = ~1.1k inserts/day. WAL + single-writer ensures this is trivial. Indexes on `(timestamp)` + `(event_type)` are well-behaved.
- **R4 — `tracing-opentelemetry` span context propagation through `tokio::spawn`.** Consolidator runs on a single worker task (not spawned per phase), so span context is preserved via thread-local `Current`. Verified: no `tokio::spawn` inside `run_all_phases`. If future phases spawn, use `.instrument(span)`.
- **R5 — `eprintln!` convergence fatigue.** 184 sites × 11 commits. Reviewers may rubber-stamp mid-series. Mitigation: each commit self-contained, ≤ 70 lines, ≤ 1 file touched.
- **R6 — OTLP collector unreachable at startup.** Fails-loud policy (§3.3) is to warn + continue; does NOT crash. Verified in dogfood.
- **R7 — Worker-level span explosion.** 184 `eprintln!` sites become `tracing::info!` calls — some are hot-loop. When OTLP is enabled, every call becomes an exported event. Mitigation: reserve `info!` for state-change events; use `debug!` for fine-grained. Task 6 (convergence) reviews each site and picks the right level.

---

## 10. References

- `crates/daemon/src/main.rs:91-178` — existing OTLP wiring.
- `crates/daemon/src/server/metrics.rs` — existing Prometheus surface.
- `crates/daemon/src/db/schema.rs:255-266` — `kpi_events` table.
- `crates/daemon/src/workers/consolidator.rs:37-41` — `PHASE_ORDER` + cfg gate.
- `crates/core/src/protocol/request.rs:142-145` — `Request::ProbePhase` + cfg gate.
- `crates/daemon/src/config.rs:340-375` — `MetricsConfig` / `OtlpConfig`.
- `docs/benchmarks/results/2026-04-24-forge-behavioral-skill-inference.md` — 2A-4c2 dogfood; Codex-LOW carry-forward (no structured tracing) addressed here.
- `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md` — Phase 23 (2A-4c2).
- `HANDOFF.md` §Lifted constraints — history of phase shipping cadence.
