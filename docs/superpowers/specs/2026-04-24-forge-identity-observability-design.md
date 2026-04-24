# Forge-Identity Observability — Design v3 (Phase 2A-4d.1, Instrumentation Tier)

**Status:** DESIGN v3 — revised after v2 adversarial review (Claude + Codex, 2026-04-24) found 2 BLOCKERs + 3 HIGHs + 3 MEDIUMs. v1 v2 histories preserved in git; this is the proposed LOCK-ready version.

**Phase position:** First of three 2A-4d tiers.

| Tier | Description | Unblocks |
|------|-------------|----------|
| **2A-4d.1 (this spec)** | Per-phase `tracing::info_span!` + 3 new Prometheus metric families + write phase observations to `kpi_events` + convert worker `eprintln!`/`println!` to `tracing` | Everything downstream. |
| 2A-4d.2 | Observability API (`/inspect`, SSE, `forge-next observe`, HUD drift) | Live user-facing observability. |
| 2A-4d.3 | Bench harness + fixtures + `bench_runs` table + CI per-commit + leaderboard | Quality as time series. |

Each tier ships independently (two reviews + merge + dogfood) before the next starts.

---

## 1. Goal

Turn every consolidator phase into a first-class observable event.

**Before this work:**
- Workers emit ~184 `eprintln!`/`println!` calls across 11 files; not structured, not queryable, not exported.
- `forge_*` Prometheus metrics cover 7 top-level gauges; no per-phase or per-table granularity.
- `tracing-opentelemetry` + `opentelemetry-otlp 0.27` are already wired (`init_otlp_layer` at `main.rs:91-130`); spans land in Jaeger/Grafana when `FORGE_OTLP_ENABLED=true` — but nothing in the consolidator emits phase-level spans, so the collector sees only top-level subscriber spans.
- `kpi_events` table (`schema.rs:255-266`) is SCHEMA-ONLY — no writers in the codebase today. It's a documented-but-unbuilt shared logging surface. v2 wrongly called it "currently used". v3 treats it as green-field and carves `event_type='phase_completed'` explicitly.

**After this work:**
- Every consolidator phase emits a `tracing::info_span!` rooted at `consolidate_pass`; spans carry `phase_name`, `output_count`, `duration_ms`, `error_count`. OTLP picks them up where enabled — zero new init code.
- `ForgeMetrics` gains 3 new families: per-phase duration histogram, per-phase output counter, per-table rows gauge.
- Each consolidator phase writes one row to `kpi_events` with `event_type='phase_completed'` and phase-specific fields in `metadata_json`. This is the FIRST writer of `kpi_events`; Tier 1 claims the `phase_completed` namespace explicitly.
- Worker code has zero `eprintln!`/`println!` outside `#[cfg(test)]`. Every event is a structured `tracing::info!`/`warn!`/`error!`.

**Success metric:** a Grafana dashboard can, without custom SQL, answer: duration per phase over time; skills inferred per hour; slowest phase on average; trace of one consolidation pass (span per phase).

---

## 2. Verified reconnaissance (2026-04-24, HEAD `65ebdf3`)

Each fact below confirmed independently against current code. Planner re-verifies at implementation time in case anything drifts.

| # | Fact | Evidence |
|---|------|----------|
| 1 | 23 consolidator phases in `run_all_phases`. | `grep -cE "^\s*// Phase [0-9]+:" consolidator.rs` → 23 |
| 2 | OTLP exporter fully wired at `main.rs:91-130`; uses `opentelemetry-otlp 0.27` tonic/gRPC via `FORGE_OTLP_*` env vars. | `main.rs:91-178`; `Cargo.toml` |
| 3 | `OtlpConfig` struct at `config.rs:356-365` is UNUSED by code — only referenced in tests. `main.rs:140-141` comment: *"We read env vars directly (not ForgeConfig) to avoid a chicken-and-egg problem — config loading logs, but the logger isn't initialized yet."* | `rg OtlpConfig --type rust` returns only `config.rs` + tests. |
| 4 | `kpi_events` table exists (schema.rs:255-266). **Zero writers in codebase today.** | `grep -rn "INSERT INTO kpi_events" crates/` returns nothing. |
| 5 | 184 `eprintln!`/`println!` sites across 11 worker files. Largest: `consolidator.rs` (69), `indexer.rs` (33), `extractor.rs` (22). | `grep -c 'eprintln!\|println!' crates/daemon/src/workers/*.rs` |
| 6 | `PHASE_ORDER`, `Request::ProbePhase`, and its handler are all `#[cfg(any(test, feature = "bench"))]` — symmetric today. | `consolidator.rs:37`, `request.rs:142`, `handler.rs:1391` |
| 7 | `prometheus 0.13` + `ForgeMetrics` Registry already exist with 7 families. | `server/metrics.rs` |
| 8 | `reaper.rs` is session-specific (heartbeat timeouts). No generic time-based sweeper exists. | `workers/reaper.rs` |
| 9 | `cargo test --workspace` baseline at HEAD: 1388+ pass, 0 failed, 1 ignored. | Last full run pre-spec. |
| 10 | Phase fn return types are heterogeneous: `Ok(usize)`, `Ok((usize, usize))`, `Ok(Vec<Memory>)`, `Result<()>`, bare `usize`. Unifying them via a `PhaseResult` refactor would touch ~20 files including benches + tests. | See `match ops::` blocks in consolidator.rs. |
| 11 | `run_consolidator` holds `state.lock().await` across the entire `run_all_phases` call (`consolidator.rs:1897-1905`). Lock hold spans the full 23-phase pass. | Direct inspection of `run_consolidator`. |
| 12 | `identity` table (`schema.rs:479`) has no `updated_at` column (only `created_at`). `kpi_snapshots` + `kpi_benchmarks` tables exist alongside `kpi_events` but also have zero writers. | Schema + grep. |
| 13 | Memory subtypes (decision, lesson, pattern) share the `memory` table — not distinct tables. | `schema.rs:388`+ uses a `memory_type` column. |

---

## 3. Architecture

### 3.1 Span wrapping — preserve heterogeneous return types

Phase fn signatures are NOT refactored. Timing + counter capture happens at the call site with a small helper that handles each return shape. Three concrete templates:

**Template A — plain `usize` return (most common, e.g. Phase 23):**
```rust
let _span = tracing::info_span!(
    "phase_23_infer_skills_from_behavior",
    run_id = %run_id,
);
let t0 = std::time::Instant::now();
let output = infer_skills_from_behavior(conn, config.min_sessions, config.window_days);
let duration_ms = t0.elapsed().as_millis() as u64;
let outcome = PhaseOutcome {
    phase: "phase_23_infer_skills_from_behavior",
    output_count: output as u64,
    error_count: 0,
    duration_ms,
};
record_phase_outcome(conn, &run_id, &outcome);
update_phase_metrics(&metrics, &outcome);
stats.skills_inferred = output;
```

**Template B — `Result<T>` return with error arm (e.g. Phase 1 dedup):**
```rust
let _span = tracing::info_span!("phase_1_dedup_memories", run_id = %run_id);
let t0 = std::time::Instant::now();
let (output_count, error_count) = match ops::dedup_memories(conn) {
    Ok(n) => (n as u64, 0),
    Err(e) => {
        tracing::error!(error = %e, "phase_1 failed");
        (0, 1)
    }
};
let outcome = PhaseOutcome {
    phase: "phase_1_dedup_memories",
    output_count, error_count,
    duration_ms: t0.elapsed().as_millis() as u64,
};
record_phase_outcome(conn, &run_id, &outcome);
update_phase_metrics(&metrics, &outcome);
stats.exact_dedup = output_count as usize;
```

**Template C — tuple `(u, v)` return (e.g. Phase 4 decay):**
Define which component is "output" at the spec level — pick the most operator-meaningful. For recency decay: `(normal_decayed, accel_decayed)`; output_count = `normal_decayed + accel_decayed`. Spec carries a `PhaseOutputProjection` table mapping every phase to its output-count function:

| Phase | Fn return | output_count formula |
|-------|-----------|----------------------|
| 1 | `Result<usize>` | `Ok(n)` → `n`; `Err` → `0` + error_count++ |
| 2 | `Result<usize>` | same |
| 3 | `usize` | raw |
| 4 | `(usize, usize)` | sum |
| 5 | `Result<usize>` | same as #1 |
| 6 | `Vec<Memory>` + inner UPDATE | `updated_count` from inner loop |
| 7 | `Result<usize>` | same |
| 8 | `Result<(usize, usize)>` | sum-of-tuple; errors bump error_count |
| 9 | `Result<usize>` | same |
| 10 | `usize` | raw |
| 11 | `Result<usize>` | entity detection count |
| 12-22 | `Result<usize>` or `usize` | raw (see plan) |
| 23 | `usize` | raw |

The plan file enumerates each phase with its signature + chosen projection. No signature changes.

**`PhaseOutcome` struct** lives in `workers/mod.rs` (or a new `workers/instrumentation.rs`). Read-only data — no public API surface exposed beyond Tier 1 internal use.

### 3.2 Prometheus surface — 3 new families (not 4)

v2 proposed 4 families; v3 drops `forge_layer_freshness_seconds` (Claude H2: no uniform source column across "layers" which aren't even distinct tables for memory subtypes). Freshness becomes Tier 2 scope where layer semantics get defined properly.

```
# Phase duration histogram, per phase.
forge_phase_duration_seconds{phase="phase_23_infer_skills_from_behavior"}
  buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0]

# Phase output counter, per phase × action.
forge_phase_output_rows_total{phase=..., action="succeeded|errored"}

# Per-table rows gauge (named by SQL TABLE, not conceptual "layer").
forge_table_rows{table="skill|memory|identity_trait|disposition|platform|tool|
                         perception|declared|domain_dna|entity|edge"}
```

Note: `forge_table_rows` (gauge) has no `_total` suffix per Claude M2 (Prometheus naming convention reserves `_total` for counters).

**Cardinality:** 23 phases × 1 histogram = 23 series; 23 phases × 2 actions = 46 series; 11 tables × 1 gauge = 11 series. **Total new series: 80.** Well under Prometheus budget.

Gauges refresh on each `/metrics` scrape via extended `refresh_gauges()`. Counters `.inc_by(n)` + histograms `.observe(d)` in the call-site wrapper.

### 3.3 OTLP integration — zero new wiring

OTLP is fully wired (recon #2). Tier 1's sole OTLP delivery: phase spans are picked up by the existing `tracing-opentelemetry` layer and exported through the existing gRPC batch exporter when `FORGE_OTLP_ENABLED=true`.

**The `OtlpConfig` vs env-var dual-path is EXPLICITLY DEFERRED.** Claude H3 correctly identified main.rs's own comment about the chicken-and-egg issue (logger not yet initialized when config is parsed). Resolving this in Tier 1 would either (a) reintroduce that bug, or (b) require a subscriber-reinstall dance that's a known OTLP footgun. v3 leaves env vars as the sole entry point. The unused `OtlpConfig` struct stays in `config.rs` as reserved scaffolding; a follow-up task (Tier 3 candidate) will either wire it via two-phase init or delete it.

**Fails-loud policy:** if `FORGE_OTLP_ENABLED=true` but the collector is unreachable, daemon logs a warning + starts without OTLP export. The daemon never exits on collector problems. (`opentelemetry-otlp` with `tonic` uses `with_batch_exporter` which does not validate connectivity at init; "fail loud" means log at init, retry on export, drop on repeated failure — standard batch-exporter behavior. Spec does not ask the daemon to verify the collector at init time.)

### 3.4 `kpi_events` — FIRST writer, namespace carved explicitly

v2 claimed `kpi_events` was already used. Recon fact #4 proved otherwise: green-field. v3 treats Tier 1 as the table's first writer and reserves `event_type='phase_completed'` for consolidator observations.

No schema change. Writes:

```sql
INSERT INTO kpi_events (id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json)
VALUES (?, strftime('%s','now'), 'phase_completed', NULL, ?, ?, ?, ?);
```

`id` is a ULID (reuses daemon's existing ULID generator). `latency_ms` = `duration_ms`. `result_count` = `output_count`. `success` = `1` if `error_count == 0`.

`metadata_json` contract (STABLE surface Tier 2 will consume):
```json
{
  "phase_name": "phase_23_infer_skills_from_behavior",
  "run_id": "01HXYZ...",
  "input_count": 0,
  "output_count": 1,
  "error_count": 0,
  "trace_id": "abc123..." | null
}
```

No `details` object inside metadata_json in Tier 1 — keep the contract small. Phase-specific fields can be added later as explicit extensions (versioned `metadata_schema_version` if we ever need it).

**`event_type` namespace table (claims register):** Tier 1 claims `phase_completed`. Future additions go in a namespace table inside `HANDOFF.md` or a dedicated `docs/architecture/kpi_events-namespace.md` so Tier 2, Tier 3, and any other writer coordinate upfront.

**Retention — EXPLICITLY DEFERRED to Tier 2.** v2 tried to inline retention in the held consolidator lock (Claude B2 concurrency issue). v3 drops retention from Tier 1 entirely. At 23 inserts × 48 passes/day × 30 days = 33k rows, table stays under 1MB for a month. For the Tier-1 ship window this is a non-concern. Tier 2 (`/inspect` needs fast reads) owns retention: either a dedicated reaper worker (new file) or an extension of the existing reaper with a second timer. Documented in §4 Task 10.

### 3.5 `eprintln!`/`println!` convergence

184 sites × 11 files. Commits per file, size-capped at ~70 lines per commit; larger files split by concern (init/error/progress).

| File | Sites | Commits |
|------|-------|---------|
| `consolidator.rs` | 69 | 2 commits (eprintln: ≤35 lines each) |
| `indexer.rs` | 33 | 1 commit |
| `extractor.rs` | 22 | 1 commit |
| `diagnostics.rs` | 13 | 1 commit |
| `disposition.rs` | 12 | 1 commit |
| `watcher.rs` + `perception.rs` + `embedder.rs` | 11 + 8 + 7 | 1 bundled commit |
| `reaper.rs` + `mod.rs` | 6 + 3 | 1 bundled commit |
| `skill_inference.rs` | 0 | n/a |

**7 commits total.** Each has a consistent conversion map:
- `eprintln!("[W] X")` → `tracing::info!(target: "forge::W", "X")`.
- `eprintln!("[W] error: {e}")` → `tracing::error!(target: "forge::W", error = %e, "...")`.
- `println!(...)` stays ONLY in `forge-cli` (user-facing output).

### 3.6 `PHASE_ORDER` cfg symmetry — no change

Keep all three cfg-gated. Instrumentation uses literal span names at call sites. Recon fact #6 and v2 design apply.

### 3.7 Span overhead & latency budget

**Baseline-and-compare:** measure cold-start time + steady-state CPU + consolidator-pass duration PRE-Tier-1 (baseline snapshot stored in `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1-baseline.md`) vs POST-Tier-1. Use existing `forge_consolidation_harness.rs` as the harness.

Budget:
- Cold start with OTLP disabled: regression ≤ 20 ms.
- Cold start with OTLP enabled + local collector: regression ≤ 100 ms (includes collector handshake).
- Steady-state CPU over 5 min (idle consolidator): regression ≤ 2%.
- `force_consolidate` on seeded 100-memory DB: regression ≤ 10 ms total.

Regression > budget = deferral. Measured at Task 11 (see §4).

### 3.8 Future-proofing for spawn context loss

Tier 1 adds a CI grep check:
```bash
! grep -n "tokio::spawn" crates/daemon/src/workers/consolidator.rs
```
Fails CI if any `tokio::spawn` is introduced without explicit `.instrument(span)` — documented as a convention in CONTRIBUTING.md.

---

## 4. Task list

1. **Re-verify recon.** Re-run §2 commands.
2. **Introduce `PhaseOutcome` struct + helpers.** `record_phase_outcome(conn, run_id, outcome)` inserts to `kpi_events`; `update_phase_metrics(metrics, outcome)` bumps histogram + counter. Lands in `workers/instrumentation.rs` (new file, ≤ 80 lines).
3. **Wrap each of 23 phase call sites in `run_all_phases`** per §3.1 templates. Write unit tests at phase-by-phase granularity: "after Phase N, kpi_events has one new row with phase_name = …".
4. **Extend `ForgeMetrics`** with the 3 new families per §3.2. Wire `refresh_gauges` for the new gauge; `.observe` / `.inc_by` in the helper.
5. **Convert `eprintln!`/`println!` → `tracing`** per §3.5. 7 commits.
6. **Acceptance check + docs update.** `scripts/check-harness-sync.sh` stays green; update HANDOFF §Lifted constraints with Tier 1 entry.
7. **Adversarial reviews on T1-T6 diff.** Claude + Codex, inverted prompts.
8. **Address BLOCKER + HIGH findings.**
9. **Live-daemon dogfood** + results doc at `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1.md`. Steps: seeded DB, `force_consolidate`, prove `/metrics` has 3 new families with non-zero values + `kpi_events` has 23 `phase_completed` rows per pass + OTLP spans visible in a local Jaeger container.
10. **Carry-forwards recorded in HANDOFF 2P-1b backlog** (tentatively 2A-4d.2 scope): retention reaper for `kpi_events`; `forge_layer_freshness_seconds`; `OtlpConfig` wiring via two-phase init.
11. **Latency-budget measurement** per §3.7 — baseline snapshot + post-Tier-1 snapshot, both committed.

---

## 5. Non-goals (explicit)

- **No `/inspect` endpoint.** Tier 2.
- **No `bench_runs` table, no scoring.** Tier 3.
- **No HUD UI changes.** Tier 2.
- **No Grafana dashboard JSON committed.**
- **No SSE / streaming.** Tier 2.
- **No `OtlpConfig` wiring** (§3.3 defers).
- **No `PhaseResult` refactor of `ops::*`** (§3.1 keeps signatures).
- **No `kpi_events` retention worker** (§3.4 defers to Tier 2).
- **No `forge_layer_freshness_seconds`** (§3.2 defers to Tier 2).
- **No promotion of `PHASE_ORDER` / `Request::ProbePhase`** (§3.6).
- **No freshness/drift queries** — Tier 2 owns those.

---

## 6. Backward compatibility

- `/metrics` exposes 7 existing families unchanged + 3 additive. Scrapers pinned to the old surface see additive data only.
- `ResponseData::Doctor` unchanged.
- `ConsolidationComplete` unchanged (2P-1b §15 already added `skills_inferred`).
- `kpi_events` schema unchanged; Tier 1 is the first writer.
- OTLP env vars continue to work as-is.

**Breaking:** worker stderr stops carrying `[consolidator] …` unstructured lines. Operators update log filters to JSON event queries (`jq 'select(.phase == "phase_23")'`). Migration note in results doc.

---

## 7. Open questions

- **Q1.** Retention for `kpi_events` — does Tier 2 inherit a pre-defined policy or design its own? (v3 recommendation: Tier 2 defines; Tier 1 emits freely.)
- **Q2.** Does `trace_id` fall back to the consolidate_pass ULID when OTLP is off? (v3 recommendation: yes — Tier 2's correlation query treats trace_id as "correlation id" whether or not OTLP is running.)
- **Q3.** Span naming — `phase_23_infer_skills_from_behavior` vs `phase_23.infer_skills_from_behavior` vs `forge.consolidator.phase_23.infer_skills_from_behavior`? (v3 recommendation: underscore-only, tier is implicit via the `consolidate_pass` parent span.)

---

## 8. Acceptance criteria

- [ ] `/metrics` returns ≥ 10 metric families (7 existing + 3 new) with non-zero values after a seeded consolidation pass.
- [ ] After one `force_consolidate` on a seeded DB, `SELECT COUNT(*) FROM kpi_events WHERE event_type='phase_completed'` == 23. Every phase emits a row including zero-work passes (no-op phases write `output_count=0, success=1`).
- [ ] With `FORGE_OTLP_ENABLED=true` + `FORGE_OTLP_ENDPOINT=http://localhost:4317` + a local Jaeger-all-in-one container, one pass produces one trace with 23 child spans in Jaeger.
- [ ] `grep -rn 'eprintln!\|println!' crates/daemon/src/workers/*.rs` (excluding `#[cfg(test)]` blocks) returns 0.
- [ ] `PHASE_ORDER`, `Request::ProbePhase`, and its handler remain `#[cfg(any(test, feature = "bench"))]`.
- [ ] `cargo test --workspace` passes ≥ baseline (1388) + ≥ 23 new per-phase instrumentation tests.
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` clean.
- [ ] `scripts/check-harness-sync.sh` clean.
- [ ] CI grep guard (§3.8) installed in `plugin-surface` job.
- [ ] Latency budget (§3.7) measured in baseline-and-compare doc, within budget.
- [ ] Two adversarial reviews complete; BLOCKERs + HIGHs addressed or explicitly deferred with rationale.
- [ ] `kpi_events` namespace register committed to docs/architecture/.

---

## 9. Risks (residual)

- **R1** — `opentelemetry` dep API churn. Locked to 0.27; monitor upstream.
- **R2** — `eprintln!` convergence fatigue across 7 commits. Mitigation: each commit ≤ 70 lines, one-file-one-concern.
- **R3** — `kpi_events` write amplification. 23 × 48 = ~1.1k inserts/day; negligible.
- **R4** — Span context through `tokio::spawn`. Tier 1 CI grep guard (§3.8) prevents regression. Existing code has zero spawns inside `run_all_phases` (recon #11 implicit).
- **R5** — OTLP collector unreachable at startup. Log + continue; never exits. Covered in §3.3.
- **R6** — No retention in Tier 1 → unbounded growth. Bounded in practice by deferral to Tier 2; at worst 23 × 48 × 365 = ~400k rows/year (~40 MB). SQLite handles this fine.
- **R7** — `kpi_snapshots` and `kpi_benchmarks` tables exist alongside `kpi_events` (also zero writers). Don't confuse namespaces: Tier 1 only writes to `kpi_events`. Namespace register makes this explicit.

---

## 10. References

- `crates/daemon/src/main.rs:91-178` — OTLP wiring.
- `crates/daemon/src/server/metrics.rs` — Prometheus surface.
- `crates/daemon/src/db/schema.rs:255-266` — `kpi_events`.
- `crates/daemon/src/workers/consolidator.rs:82-…` — `run_all_phases`.
- `docs/benchmarks/results/2026-04-24-forge-behavioral-skill-inference.md` — Codex-LOW carry-forward "no structured tracing" that this tier addresses.
- `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md` — 2A-4c2 precedent.
- v1 and v2 of this spec preserved in git history (commits `d30eaab`, `65ebdf3`).
