# Forge-Identity Observability — Design (Phase 2A-4d.1, Instrumentation Tier)

**Status:** DESIGN, pre-implementation. Two adversarial reviews required before landing code (Claude code-reviewer + Codex rescue, inverted prompts). Spec is LOCKED once reviews pass.

**Phase position:** First of three 2A-4d tiers. Sequence:

| Tier | Phase | Description | Unblocks |
|------|-------|-------------|----------|
| **2A-4d.1 (this spec)** | Instrumentation | Spans on every phase + per-layer Prometheus metrics + OTLP exporter + replace ad-hoc `eprintln!`/`println!` with structured events | Every later observability consumer. No bench without this. |
| 2A-4d.2 | Observability API | Generic `/inspect {layer, shape, window}` over Tier 1 data + SSE stream + `forge-next observe` CLI + HUD drift display | Live user-facing observability. |
| 2A-4d.3 | Bench Harness | `forge-bench identity` + fixtures v1 + `bench_runs` table + ablation flags + CI-per-commit + leaderboard | Quality as a time series. |

Each tier ships independently (two reviews + merge + dogfood) before the next starts.

---

## 1. Goal

Turn every consolidation phase and every Manas layer write into a first-class observable event. Before this work:

- Workers emit `eprintln!("[consolidator] ...")` to stderr — not queryable, not structured, not exported.
- Metrics exist for 7 top-level counts (memories, edges, embeddings, etc.) but **per-phase** and **per-layer** granularity is absent. You can't answer "how long did Phase 23 take this run?" or "how many skills were pruned last week?".
- OTLP exporter is a config struct with no wiring.
- The daemon is a black box in a Grafana/Jaeger environment.

After this work:

- Every consolidator phase produces a `tracing::info_span!` with `phase_name`, `input_count`, `output_count`, `duration_ms`, plus typed structured fields per phase.
- Prometheus gains 4 new metric families keyed by `phase` / `layer` labels.
- OTLP tracing exporter actually emits spans to the configured collector when `[otlp].enabled = true`.
- `/metrics` endpoint returns the expanded surface unchanged in shape (text/plain; version=0.0.4), backward-compatible with existing scrapers.
- No more `eprintln!` / `println!` in production code paths. Workers emit `tracing::info!` / `warn!` / `error!` events with structured fields.

**Success metric:** A Grafana dashboard wired to the daemon can answer, with no custom SQL:

1. "Duration of each consolidation phase over the last hour."
2. "Rate of skills inferred per hour."
3. "Which worker is slowest on average."
4. "Live trace of a single consolidation pass (span per phase)."

---

## 2. Pre-implementation reconnaissance

**Facts that must be re-verified at implementation time.** Each is drawn from current code as of `d9dc8e6`.

1. **`prometheus` crate is already a dep** (`crates/daemon/Cargo.toml:58`). No new major deps for metrics.
2. **`ForgeMetrics` struct exists** (`crates/daemon/src/server/metrics.rs:20`) with 7 families, built + registered in `AppState`, exposed at `/metrics`. Tests exist (6 in same file).
3. **`MetricsConfig.enabled: bool`** defaults true; CI tests cover disable path.
4. **`OtlpConfig { enabled: bool, endpoint: String, service_name: String }`** exists at `config.rs:356` but is NEVER read from anywhere that exports traces. Reconnaissance required: grep `OtlpConfig` in the codebase — if results are only in `config.rs` + tests, the exporter is un-wired and this spec must ship it. If any `init_otlp()` fn exists, this spec extends rather than creates.
5. **Worker stderr noise.** `grep -rn "eprintln!" crates/daemon/src/workers/ | wc -l` currently returns ~70+. Each gets reviewed and replaced with either `tracing::info!`/`warn!`/`error!` with structured fields, OR deleted if it's purely debug.
6. **Consolidator phase list.** `crates/daemon/src/workers/consolidator.rs:29-45` has a `PHASE_ORDER` const with 2 entries (cfg-gated to test/bench). Tier 1 must decide: keep that const test-gated (probe API stays as-is) OR promote to production and derive instrumentation wrapper from it. Recommend: promote, because span names should match phase identifiers exactly.
7. **Existing `tracing` use.** Daemon already uses `tracing::info!/warn!/error!` in ~40 sites (handler.rs, config.rs, workers/*). Good. Replacing `eprintln!` is a **convergence** task, not a migration.
8. **Events table.** `perception` already writes to an events table (pattern TBD). Reconnaissance required: check if it's a `perception_event` table or a generic `event` table. If generic, extend it for phase events. If perception-specific, add a new `phase_event` table in Tier 1.
9. **Subscribe endpoint.** `Request::Subscribe` exists (line ~245 of request.rs). Recon whether it streams domain events today — if so, Tier 2's SSE stream extends rather than creates.
10. **OTLP bundled transports.** `opentelemetry-otlp` in the Rust ecosystem supports `grpc-tonic` and `http-json`. gRPC is the SOA default (Jaeger, Datadog, Grafana Cloud). This spec picks gRPC and does not add HTTP-JSON.

**Go/no-go checkpoint:** If reconnaissance reveals `init_otlp()` already exists and exports spans, this spec's Task 4 becomes a fix-up rather than a new feature. Planner adjusts then.

---

## 3. Architecture

### 3.1 Span hierarchy

Every consolidator pass is rooted at a `consolidate_pass` span. Each phase is a child span. Each significant DB write is a grandchild.

```
consolidate_pass { run_id, triggered_by, start_iso }
├── phase_17_extract_protocols { input_rows, output_rows, duration_ms }
│   └── skill_insert { skill_id, domain }
├── phase_23_infer_skills_from_behavior { input_rows, output_rows, duration_ms, min_sessions, window_days }
│   └── skill_upsert { skill_id, agent, fingerprint, project, action=insert|merge }
├── phase_18_tag_antipatterns { ... }
└── ... (20 more phases)
```

Every span carries:

- `phase_name` — stable string matching `PHASE_ORDER`.
- `input_count: u64` — rows considered.
- `output_count: u64` — rows changed.
- `duration_ms: u64` — milliseconds (synthesized from span close - open).
- `error_count: u64` — failed sub-ops (swallowed-error count — surfaces silent failures).

Per-phase fields additionally carry phase-specific context (e.g., Phase 23 carries `min_sessions`, `window_days`, `patterns_seen`).

### 3.2 Prometheus metric surface

Four new metric families, all under `forge_phase_*` and `forge_layer_*` namespaces.

```
# Phase duration histogram, per phase.
forge_phase_duration_seconds{phase="phase_23_infer_skills_from_behavior"}
  buckets: [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 30.0]

# Phase output counter, per phase (cumulative writes/updates).
forge_phase_output_rows_total{phase=..., action=inserted|updated|skipped|errored}

# Per-layer row gauges (supplements existing per-table gauges).
forge_layer_rows_total{layer="skill|decision|lesson|pattern|platform|tool|identity|disposition|perception|declared|entity|edge|memory"}

# Layer freshness gauge — seconds since the last write in that layer.
forge_layer_freshness_seconds{layer=...}
```

Refreshed on `/metrics` scrape via the existing `refresh_gauges()` pattern.

### 3.3 OTLP tracing exporter

When `[otlp].enabled = true`, daemon initializes an OTLP gRPC tracer at startup and routes every `tracing` span through it. Every `tracing::info!/warn!/error!` becomes a span event on the current span, or a span if the attributes include `span_name`.

Implementation shape (Rust stack, SOA-conventional):
- `tracing-opentelemetry` bridges `tracing` → `opentelemetry` `Span` API.
- `opentelemetry_otlp::new_exporter().tonic()` builds the gRPC exporter.
- `opentelemetry_sdk::trace::TracerProvider` owned by the daemon for lifecycle management (shutdown on SIGTERM).

Fails loudly at startup if `[otlp].enabled = true` but `endpoint` is unreachable. Does NOT fail if `enabled = false` (the default).

### 3.4 Phase event log (ground truth for later tiers)

New SQLite table `phase_event`:

```sql
CREATE TABLE IF NOT EXISTS phase_event (
    id              INTEGER PRIMARY KEY,
    run_id          TEXT NOT NULL,           -- ULID of the consolidate_pass
    phase_name      TEXT NOT NULL,
    started_at      TEXT NOT NULL,           -- ISO 8601 UTC
    finished_at     TEXT NOT NULL,
    duration_ms     INTEGER NOT NULL,
    input_count     INTEGER NOT NULL,
    output_count    INTEGER NOT NULL,
    error_count     INTEGER NOT NULL DEFAULT 0,
    trace_id        TEXT,                    -- hex trace id when OTLP on; NULL otherwise
    details_json    TEXT NOT NULL DEFAULT '{}'  -- phase-specific structured fields
);

CREATE INDEX IF NOT EXISTS idx_phase_event_started
    ON phase_event(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_phase_event_name
    ON phase_event(phase_name, started_at DESC);
```

Every consolidator phase inserts one row at close. Tier 2's `/inspect {shape: recent}` and `/inspect {shape: drift}` read from this table. Bound by a configurable retention window (default 30 days, daily reaper removes older rows).

### 3.5 `eprintln!` / `println!` convergence

Production paths convert:

- `eprintln!("[consolidator] inferred N skills")` → `tracing::info!(skills_inferred = N, phase = "phase_23", "phase complete")`.
- `eprintln!("[extractor] error processing {path}: {e}")` → `tracing::error!(error = %e, path = %path, "extractor: processing failed")`.
- `println!(...)` remains only in `forge-cli` user-facing output (explicit, intentional).

Tests that assert on `eprintln!` output migrate to `tracing_test` or inspection of `Subscribe` event stream.

---

## 4. Task list (for the plan file, not implemented here)

1. **Re-verify reconnaissance.** Run the commands in §2, update any drift.
2. **Promote `PHASE_ORDER`.** Remove `#[cfg(any(test, feature = "bench"))]` from `consolidator.rs:29`. Add a compile-time test that every run function is represented. (Addresses Claude-H3 carry-forward from 2A-4c2 T10.)
3. **Instrument consolidator with `info_span!`.** Wrap each phase fn call in `consolidator.rs:run_all_phases` with a span. Add `input_count` / `output_count` / `duration_ms` / `error_count` fields by refactoring phase fns to return a small `PhaseResult` struct.
4. **Add `phase_event` table + recorder.** Schema migration + insert on phase-span close.
5. **Expand Prometheus surface.** Add the 4 new metric families (§3.2) to `ForgeMetrics`. Wire `refresh_gauges()` to include new gauges.
6. **Wire OTLP exporter.** `init_otlp()` fn called at daemon startup iff `[otlp].enabled`. Graceful shutdown.
7. **Convert `eprintln!`/`println!` → `tracing`.** One commit per worker directory. Include structured fields for every non-trivial statement.
8. **Adversarial reviews.** Claude code-reviewer + Codex rescue, inverted prompts, on the T1-T7 diff.
9. **Address BLOCKER/HIGH findings.**
10. **Live-daemon dogfood + results doc.** Fresh release daemon, seeded consolidation, prove: `/metrics` shows new families with non-zero values; OTLP spans land in a local Jaeger-all-in-one; `phase_event` table has one row per phase per pass.
11. **Schema rollback recipe test** for the `phase_event` table.

---

## 5. Non-goals (explicit, won't slip in)

- **No Observability API** (`/inspect`). Tier 2.
- **No bench scoring / `bench_runs` table.** Tier 3.
- **No HUD UI changes.** Tier 2.
- **No Grafana dashboard files.** Users can build their own once `/metrics` + OTLP are live; we don't commit dashboard JSON (it's stack-specific and drifts).
- **No SSE / streaming endpoint.** Tier 2.
- **No ground-truth fixture curation.** Tier 3.
- **No migration of historical `eprintln!` output to `phase_event`.** Starts fresh; backfill not worth the complexity.

---

## 6. Backward compatibility

- `/metrics` continues to expose all 7 pre-existing families. New families are additive.
- `ResponseData::Doctor` unchanged. Users still call it the same way.
- `ConsolidationComplete` unchanged (2P-1b §15 already exposed `skills_inferred`).
- `phase_event` table is new; no existing migration depends on it.
- OTLP disabled by default — existing users who haven't configured it see zero change.

**Breaking behavior:** workers stop printing to stderr. Operators who grep daemon logs for `[consolidator]` lines need to update their queries. Since all new events carry stable `phase = "..."` field, grep becomes `jq '. | select(.phase == "phase_23")'` on the JSON log. Migration note goes in results doc.

---

## 7. Open questions (must be answered before plan is written)

- **Q1.** Do we commit to `tracing-opentelemetry` as the bridge, or roll our own span → OTLP converter? (Recommendation: use `tracing-opentelemetry`. Industry standard, maintained by the tracing team, handles the Rust lifetime + context propagation correctly. Adds ~2 crates to the dep tree.)
- **Q2.** Retention policy for `phase_event` — 30 days default, configurable via `[metrics].phase_event_retention_days`? (Recommendation: yes, 30 days, configurable. Daily reaper worker removes older rows.)
- **Q3.** Backward-compat for existing Prometheus scrapers that pinned to the 7-family surface? (Recommendation: add-only is safe. Any scraper that breaks on new families is already broken.)
- **Q4.** Should `PhaseResult` include a `details: serde_json::Value` for phase-specific fields, or each phase declares its own strongly-typed struct? (Recommendation: `details: serde_json::Value` is pragmatic; strongly-typed adds ceremony for no gain today. Revisit if a downstream consumer starts depending on specific fields.)

---

## 8. Acceptance criteria

- [ ] `/metrics` returns ≥ 11 metric families (7 existing + 4 new).
- [ ] `phase_event` table populated with at least one row per consolidation phase after a single `force_consolidate` on a seeded DB.
- [ ] With `[otlp].enabled = true` and a local Jaeger-all-in-one container running, a consolidation pass produces one trace with 22 spans (one per phase) in Jaeger's UI.
- [ ] `grep -rn 'eprintln!\|println!' crates/daemon/src/workers/` returns zero hits (excluding test modules).
- [ ] `PHASE_ORDER` is NOT cfg-gated.
- [ ] `cargo test --workspace` passes (target: 1400+ pass, 0 failed).
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` clean.
- [ ] `scripts/check-harness-sync.sh` clean.
- [ ] Two adversarial reviews complete; BLOCKERs and HIGHs either fixed or explicitly deferred.
- [ ] Results doc at `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1.md` with: families listed, sample metric output, sample OTLP trace screenshot link, `eprintln!` count before/after, commit SHAs per task.

---

## 9. Risks

- **R1 — OTLP dep churn.** `opentelemetry-*` crates have had frequent breaking API changes. Lock to a specific version; document re-verification in reconnaissance.
- **R2 — Span overhead.** Every phase being wrapped in a span adds ~1-5μs overhead. Consolidator runs every 30 min (900s default); overhead is noise. Verify under load with a baseline bench (Tier 3 catches if we regress).
- **R3 — `phase_event` table growth.** 22 phases × 48 passes/day × 365 days = ~380k rows/yr. Not a scale concern. Index hits well-behaved.
- **R4 — Spam from tracing events.** If every `tracing::info!` becomes an OTLP span event, Jaeger gets noisy. Mitigation: use `debug!` for low-signal events, reserve `info!` for phase-level events. Review pass in Task 7.
- **R5 — Test-only phases leak `eprintln!`.** Keep allowed in `#[cfg(test)]` blocks; the convergence only targets production code paths. Test coverage asserts this.

---

## 10. References

- `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md` — Phase 23 (2A-4c2).
- `docs/benchmarks/results/2026-04-24-forge-behavioral-skill-inference.md` — 2A-4c2 dogfood; identifies Codex-LOW "no structured tracing" as a carry-forward, which this spec addresses.
- `HANDOFF.md` §Phase 2P-1b backlog, item §15 (skills_inferred response field) — shipped; this spec follows the pattern.
- `crates/daemon/src/server/metrics.rs` — existing Prometheus surface.
- `crates/daemon/src/config.rs:336-375` — `MetricsConfig` / `OtlpConfig` scaffolding.
- Upstream: `tracing-opentelemetry` README (pin to latest stable at implementation time).
