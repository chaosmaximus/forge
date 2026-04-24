# Forge-Identity Observability — Implementation Plan (Phase 2A-4d.1, Instrumentation Tier)

> **For agentic workers:** execute TDD — write the failing test first, watch it fail, implement, verify, commit. Each task commit passes `cargo fmt --all --check` + `cargo clippy --workspace -- -W clippy::all -D warnings` + `cargo test --workspace` + `scripts/check-harness-sync.sh`.

**Goal:** Land Tier 1 of 2A-4d per design spec `docs/superpowers/specs/2026-04-24-forge-identity-observability-design.md` (LOCKED at `b2dfa20`). Each consolidator phase emits a structured `info_span!`, writes a versioned row to `kpi_events`, and updates 3 new Prometheus metric families. All worker `eprintln!`/`println!` converge to `tracing`.

**Tech stack:** Rust workspace as-is. Existing deps: `tracing 0.1`, `tracing-subscriber 0.3`, `tracing-opentelemetry 0.28`, `opentelemetry-otlp 0.27` (tonic), `prometheus 0.13`. Zero new deps.

**Design doc:** `docs/superpowers/specs/2026-04-24-forge-identity-observability-design.md` v4 LOCKED.

---

## File structure

| File | Responsibility | Task |
|------|----------------|------|
| `crates/daemon/src/workers/instrumentation.rs` (CREATE) | `PhaseOutcome` struct, `PHASE_SPAN_NAMES` const, `record()` + `update_phase_metrics()` + `insert_kpi_event_row()` helpers, span-name integrity test | T2 |
| `crates/daemon/src/workers/mod.rs` (MODIFY) | Register `pub mod instrumentation;` | T2 |
| `crates/daemon/src/workers/consolidator.rs` (MODIFY) | Wrap 23 phase call sites in `info_span!` + `record()` per §3.1a; convert 69 `eprintln!` sites across 2 commits | T3, T6.1 |
| `crates/daemon/src/server/metrics.rs` (MODIFY) | Add 3 new metric families: `forge_phase_duration_seconds`, `forge_phase_output_rows_total`, `forge_table_rows`; extend `refresh_gauges` | T4 |
| `crates/daemon/src/server/http.rs` (MODIFY) | Thread `ForgeMetrics` into call-site wrapper — already stored in `AppState`, just expose via `state.metrics.as_ref()` inside `run_all_phases` entry (helper arg) | T4 |
| `crates/daemon/src/workers/{watcher,extractor,embedder,indexer,perception,disposition,diagnostics,reaper,mod,skill_inference}.rs` (MODIFY) | Convert `eprintln!`/`println!` → `tracing` per §3.5 mapping | T6.2–T6.8 |
| `.github/workflows/ci.yml` (MODIFY) | Add span-integrity + `tokio::spawn` prohibition guard in the `check` job | T7 |
| `docs/architecture/README.md` (CREATE) | Gateway + index for architecture docs | T5 |
| `docs/architecture/kpi_events-namespace.md` (CREATE) | Namespace register; claim `event_type='phase_completed'` for Tier 1 | T5 |
| `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1-baseline.md` (CREATE) | Pre-Tier-1 baseline: cold start timings + consolidator pass duration on the deterministic harness | T10 |
| `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1.md` (CREATE) | Post-Tier-1 results; acceptance artifact | T11 |

---

## Pre-implementation reconnaissance (Task 1, mandatory)

Before Task 2, re-run every recon command from spec §2 and confirm results match the spec's 16 recon facts. Drift since 2026-04-24 may invalidate assumptions.

**Commands:**

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge

# Fact 1: phase count
grep -cE "^\s*// Phase [0-9]+:" crates/daemon/src/workers/consolidator.rs
# Expected: 23

# Fact 2 + 3: OTLP wiring
grep -n "init_otlp_layer\|FORGE_OTLP" crates/daemon/src/main.rs
# Expected: init_otlp_layer at ~91, reads FORGE_OTLP_* env vars

# Fact 4: kpi_events writers
grep -rn "INSERT INTO kpi_events" crates/
# Expected: zero output (Tier 1 is the first writer)

# Fact 5: eprintln sites
grep -c 'eprintln!\|println!' crates/daemon/src/workers/*.rs

# Fact 10: no workers/ops/ directory
ls crates/daemon/src/workers/ops/
# Expected: ENOENT

# Fact 12: identity table shape
grep -A5 "CREATE TABLE IF NOT EXISTS identity" crates/daemon/src/db/schema.rs
# Expected: no updated_at column

# Fact 14: decay return
grep -B1 -A3 "fn decay_memories" crates/daemon/src/db/ops.rs
# Expected: Result<(usize, usize)> with doc "(checked_count, faded_count)"

# Fact 16: architecture dir absence
ls docs/architecture/ 2>&1 | head -3
# Expected: ENOENT
```

If ANY diverges from spec §2, stop and update the spec before proceeding.

---

## Task 2: Introduce `workers/instrumentation.rs` helper

**Files:**
- Create: `crates/daemon/src/workers/instrumentation.rs`
- Modify: `crates/daemon/src/workers/mod.rs` (add `pub mod instrumentation;`)

**Goal:** Pure helper module — no lock acquisitions, no new external deps.

- [ ] **2.1. Write the failing span-integrity unit test** (in `instrumentation.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_span_names_len_is_23() {
        assert_eq!(PHASE_SPAN_NAMES.len(), 23);
    }

    #[test]
    fn span_name_count_matches_phase_count_in_consolidator() {
        let src = include_str!("consolidator.rs");
        let count = src.matches("info_span!(\"phase_").count();
        assert_eq!(
            count, PHASE_SPAN_NAMES.len(),
            "expected {} info_span!(\"phase_ calls in consolidator.rs, saw {}",
            PHASE_SPAN_NAMES.len(), count
        );
    }
}
```

- [ ] **2.2. Run test** — must fail (consolidator.rs has 0 `info_span!("phase_` calls pre-T3).

- [ ] **2.3. Implement `PhaseOutcome` + `PHASE_SPAN_NAMES` + helpers:**

```rust
//! Tier 1 of Phase 2A-4d — consolidator phase instrumentation.
//!
//! Pure helpers. No lock acquisitions, no Arc<Mutex<DaemonState>>.
//! Callers pass &Connection + &ForgeMetrics + &PhaseOutcome by value.

use rusqlite::{params, Connection};
use serde_json::json;

use crate::server::metrics::ForgeMetrics;

pub const PHASE_SPAN_NAMES: &[&str; 23] = &[
    "phase_1_dedup_memories",
    "phase_2_semantic_dedup",
    "phase_3_link_memories",
    "phase_4_decay_memories",
    "phase_5_promote_patterns",
    "phase_6_reconsolidate_contradicting",
    "phase_7_merge_embedding_duplicates",
    "phase_8_strengthen_by_access",
    "phase_9_score_memory_quality",
    "phase_10_entity_detection",
    "phase_11_synthesize_contradictions",
    "phase_12_detect_and_surface_gaps",
    "phase_13_reweave_memories",
    "phase_14_flip_stale_preferences",
    "phase_15_apply_recency_decay",
    "phase_16_compute_effectiveness",
    "phase_17_extract_protocols",
    "phase_18_tag_antipatterns",
    "phase_19_emit_notifications",
    "phase_20_record_tool_use_kpis",
    "phase_21_run_healing_checks",
    "phase_22_apply_quality_pressure",
    "phase_23_infer_skills_from_behavior",
];

pub struct PhaseOutcome<'a> {
    pub phase: &'static str,
    pub run_id: &'a str,
    pub correlation_id: &'a str,
    pub trace_id: Option<&'a str>,
    pub output_count: u64,
    pub error_count: u64,
    pub duration_ms: u64,
    pub extra: serde_json::Value,
}

// record(), update_phase_metrics(), and insert_kpi_event_row() are implemented
// in crates/daemon/src/workers/instrumentation.rs — see that file for the
// authoritative source. The shipped version differs from the original plan
// pseudocode in these ways (applied during T9.2, T12, and T13.2):
//
//   * Metadata JSON no longer carries the reserved `input_count` field. Every
//     caller was setting it to 0; the field was dropped rather than kept as
//     dead scaffolding.
//   * `insert_kpi_event_row` uses `INSERT OR IGNORE` and returns
//     `rusqlite::Result<usize>` (rows written) so the caller can distinguish a
//     successful insert from a ULID PK collision that OR-IGNORE absorbed.
//   * `record()` auto-populates `trace_id` from the currently-active
//     `tracing::Span`'s OpenTelemetry context when the caller passes `None`.
//     All-zeros `TraceId::INVALID` collapses to JSON null so the "OTLP off →
//     trace_id null" contract holds.
//   * Persistence-layer failures (SQL insert error, ULID collision)
//     increment a dedicated `forge_phase_persistence_errors_total{phase, kind}`
//     counter instead of stomping `phase_output_rows{action="errored"}`.
```

- [ ] **2.4. Wire module into `mod.rs`:** `pub mod instrumentation;`

- [ ] **2.5. Run tests** — only `phase_span_names_len_is_23` passes; the source-scan test still fails (expected until T3 lands spans).

- [ ] **2.6. Commit:**

```
feat(2A-4d.1 T2): workers::instrumentation helper + PhaseOutcome

Pure helper module — PhaseOutcome struct, PHASE_SPAN_NAMES const of 23
canonical phase identifiers, record()/update_phase_metrics()/
insert_kpi_event_row() helpers. No lock acquisitions; accepts
&Connection + Option<&ForgeMetrics> + &PhaseOutcome.

kpi_events metadata_json is versioned (metadata_schema_version: 1) per
spec §3.4. correlation_id (ULID) and trace_id (OTLP hex) are separate
fields to resolve the encoding-mismatch.

Span-integrity test scans consolidator.rs source for info_span!("phase_
occurrences; currently fails (0 spans). T3 will land the spans and
flip the test green.
```

---

## Task 3: Wrap 23 phase call sites in `run_all_phases`

**Files:**
- Modify: `crates/daemon/src/workers/consolidator.rs`

Per spec §3.1 + projection table §3.1a. Use templates W1/W2/W3/W4 per phase's return type.

Preamble added at the top of `run_all_phases`:
```rust
let run_id = ulid::Ulid::new().to_string();
let _pass_span = tracing::info_span!("consolidate_pass", run_id = %run_id, triggered_by = "scheduled").entered();
```

Each phase call site becomes (example Phase 23, W1):
```rust
{
    let _span = tracing::info_span!("phase_23_infer_skills_from_behavior").entered();
    let t0 = std::time::Instant::now();
    let output = infer_skills_from_behavior(conn, config.skill_inference_min_sessions, config.skill_inference_window_days);
    let outcome = PhaseOutcome {
        phase: "phase_23_infer_skills_from_behavior",
        run_id: &run_id,
        correlation_id: &run_id,
        trace_id: None, // populated by tracing-opentelemetry automatically when OTLP is on
        output_count: output as u64,
        error_count: 0,
        duration_ms: t0.elapsed().as_millis() as u64,
        extra: serde_json::json!({}),
    };
    crate::workers::instrumentation::record(conn, metrics, &outcome);
    stats.skills_inferred = output;
}
```

**`metrics: Option<&ForgeMetrics>`** is threaded as a new argument to `run_all_phases`. Callers currently: `run_consolidator` loop body + 4 test sites. Tests pass `None`.

- [ ] **3.1. Add `metrics: Option<&ForgeMetrics>` param to `run_all_phases`** + update all callers (test + prod). Verify compile.

- [ ] **3.2. For EACH of the 23 phases** — in phase-order — apply the right template per §3.1a. Commit NOT per phase (one commit with all 23 wraps) so the span-integrity test flips green atomically.

- [ ] **3.3. Verify span-integrity test passes** (`cargo test -p forge-daemon workers::instrumentation::tests`).

- [ ] **3.4. Verify one per-phase smoke test** — seed 1 memory, call `run_all_phases(&conn, &config, None)`, assert `SELECT COUNT(*) FROM kpi_events WHERE event_type='phase_completed'` == 23.

- [ ] **3.5. Run full clippy + fmt + tests.** Expected: 1388 + 1 (per-phase smoke) + 2 (span-integrity) = 1391+.

- [ ] **3.6. Commit:**

```
feat(2A-4d.1 T3): instrument 23 consolidator phases with info_span! + PhaseOutcome

Every phase call site in run_all_phases is now wrapped in
tracing::info_span!("phase_N_<name>") and emits a PhaseOutcome via
workers::instrumentation::record(). Output counts follow the
§3.1a projection table; phases that swallow errors internally
(Phase 3, 11, 12, 13, 15, 17, 18, 19, 20, 22, 23) have error_count=0
at the call site — documented as a known gap (R8).

Span-integrity test (T2) now passes: 23 info_span! calls matching
PHASE_SPAN_NAMES.

After one force_consolidate, kpi_events has 23 rows with
event_type='phase_completed' and metadata_schema_version: 1.
```

---

## Task 4: 3 new Prometheus metric families

**Files:**
- Modify: `crates/daemon/src/server/metrics.rs`

- [ ] **4.1. Write failing test** — `test_new_metric_families_registered` asserting registry contains `forge_phase_duration_seconds`, `forge_phase_output_rows_total`, `forge_table_rows`.

- [ ] **4.2. Extend `ForgeMetrics`:**

```rust
pub struct ForgeMetrics {
    // existing fields…
    pub phase_duration: HistogramVec,      // label: phase
    pub phase_output_rows: IntCounterVec,   // labels: phase, action
    pub table_rows: IntGaugeVec,            // label: table
}
```

Registration in `new()`; `buckets` per spec §3.2. Cardinality check: 80 new series.

- [ ] **4.3. Extend `refresh_gauges`** to set `table_rows` from per-table `SELECT COUNT(*)`:

```rust
for (table, label) in &[
    ("memory", "memory"), ("skill", "skill"), ("edge", "edge"),
    ("identity", "identity"), ("disposition", "disposition"),
    ("platform", "platform"), ("tool", "tool"),
    ("perception", "perception"), ("declared", "declared"),
    ("domain_dna", "domain_dna"), ("entity", "entity"),
] {
    if let Ok(count) = reader.conn.query_row(
        &format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_, i64>(0)
    ) {
        metrics.table_rows.with_label_values(&[label]).set(count);
    }
}
```

- [ ] **4.4. `/metrics` handler is unchanged** — `refresh_gauges` + `registry.gather()` already handles it.

- [ ] **4.5. Verify tests** — new test passes; existing 6 pass.

- [ ] **4.6. Commit:**

```
feat(2A-4d.1 T4): add 3 Prometheus metric families

New families:
  - forge_phase_duration_seconds (histogram, per phase)
  - forge_phase_output_rows_total (counter, per phase × action)
  - forge_table_rows (gauge, per sql table — no _total suffix)

11 tables labeled: memory, skill, edge, identity, disposition,
platform, tool, perception, declared, domain_dna, entity. Matches
schema.rs.

Cardinality: 23 + 46 + 11 = 80 new time series.

refresh_gauges() queries each table's count on /metrics scrape.
Histogram + counter are updated at the call-site wrapper (T3).
```

---

## Task 5: `docs/architecture/` + kpi_events namespace register

**Files:**
- Create: `docs/architecture/README.md`
- Create: `docs/architecture/kpi_events-namespace.md`

- [ ] **5.1. Write the gateway:** ≤20 lines — "this directory documents cross-cutting architectural surfaces shared by multiple tiers".

- [ ] **5.2. Write the namespace register:** claim `phase_completed` for Tier 1. Document the versioned `metadata_json` v1 contract. Note that Tier 2 (`/inspect`) will consume these rows, so schema evolution requires a version bump.

- [ ] **5.3. Commit:**

```
docs(2A-4d.1 T5): architecture/ gateway + kpi_events namespace register

Tier 1 claims event_type='phase_completed' with
metadata_schema_version=1. Tier 2+ writers register here before writing.
```

---

## Task 6: `eprintln!`/`println!` convergence (8 commits)

Each sub-task is one commit. Max ≤35 sites/commit. Run clippy + fmt + tests after each.

- [ ] **6.1.** `consolidator.rs` part A (lines 1–~1000, ≤35 sites).
- [ ] **6.2.** `consolidator.rs` part B (lines ~1000–end, remaining sites).
- [ ] **6.3.** `indexer.rs` (33 sites).
- [ ] **6.4.** `extractor.rs` (22 sites).
- [ ] **6.5.** `diagnostics.rs` (13 sites).
- [ ] **6.6.** `disposition.rs` (12 sites).
- [ ] **6.7.** `watcher.rs` + `perception.rs` + `embedder.rs` (≤26 bundled).
- [ ] **6.8.** `reaper.rs` + `workers/mod.rs` + `skill_inference.rs` (~9 bundled).

Conversion rules (stable across all commits):
- `eprintln!("[W] X")` → `tracing::info!(target: "forge::W", "X")`
- `eprintln!("[W] error: {e}")` → `tracing::error!(target: "forge::W", error = %e, "…")`
- `eprintln!("[W] WARN X")` → `tracing::warn!(target: "forge::W", "X")`

Commit template per sub-task:

```
chore(2A-4d.1 T6.N): convert <file> eprintln! → tracing (sites: M)

Mechanical conversion per spec §3.5 mapping. No behavior change.
Acceptance: grep 'eprintln!\|println!' <file> (excl. #[cfg(test)])
returns 0.
```

After T6.8: final acceptance — `grep -rn 'eprintln!\|println!' crates/daemon/src/workers/*.rs | grep -v '#\[cfg(test' | wc -l` → 0.

---

## Task 7: CI span-integrity + tokio::spawn guard

**Files:**
- Modify: `.github/workflows/ci.yml` (the `check` job — NOT `plugin-surface`).

Append step after the existing clippy step in the `check` job:

```yaml
      - name: Span integrity guard
        run: |
          set -euo pipefail
          count=$(grep -c 'info_span!("phase_' crates/daemon/src/workers/consolidator.rs)
          [ "$count" = "23" ] || { echo "span count $count != 23"; exit 1; }
          ! grep -n 'tokio::spawn' crates/daemon/src/workers/consolidator.rs
          ! grep -n 'tokio::spawn' crates/daemon/src/db/ops.rs
```

- [ ] **7.1.** Add step.
- [ ] **7.2.** Verify by temporarily adding a `tokio::spawn` to a comment (not code) and confirming CI fails; revert.
- [ ] **7.3. Commit:**

```
ci(2A-4d.1 T7): span-integrity + tokio::spawn guard on check job

Fails if consolidator.rs has != 23 info_span!("phase_ calls or if any
tokio::spawn sneaks into the phase execution surface. Keeps Tier 1's
span attribution honest under future refactors.
```

---

## Task 8: Adversarial reviews on T1–T7 diff

Dispatch two subagents in parallel:
- Claude `general-purpose` reviewer — full open-ended adversarial review of the diff.
- Codex `codex-rescue` reviewer — second opinion, inverted prompt.

Probe angles (seed):
1. Does every phase wrapper handle its return type correctly per §3.1a?
2. Does the span-integrity test actually catch a renamed phase?
3. Are the 184 converted `tracing` calls leveled correctly (info/warn/error)?
4. Does the metrics refresh query hit a non-existent table?
5. Do any converted events leak secret-bearing context (file paths, commands) at info level?

Record raw review outputs in the commit history. Address BLOCKER + HIGH in T9.

---

## Task 9: Address review findings

One commit per finding, `fix(2A-4d.1 T9): address <severity>-<n>-<slug>`.

---

## Task 10: Latency baseline

- [ ] **10.1.** Ensure baseline is captured BEFORE T2 lands. If T10 is being done late, revert T2–T7 temporarily on a branch to capture baseline, then replay. (Or: baseline was captured at T1/recon time per spec §3.7.)

- [ ] **10.2.** Run `cargo test -p forge-daemon --test forge_consolidation_harness --release -- --test-threads=1` N=5 times. Capture consolidator pass duration per phase.

- [ ] **10.3.** Compute median-of-medians (MoM) per phase.

- [ ] **10.4.** Write baseline file: `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1-baseline.md`.

- [ ] **10.5.** Repeat on HEAD (post-T3+T4+T6 merge). Compute deltas.

- [ ] **10.6.** Verify budget (§3.7):
  - Cold start (OTLP off) regression ≤ 20 ms.
  - Cold start (OTLP on + local Jaeger) regression ≤ 100 ms.
  - Steady-state CPU regression ≤ 2%.
  - `force_consolidate` on seeded 100-memory DB regression ≤ 10 ms total.

- [ ] **10.7.** Commit results doc.

---

## Task 11: Live-daemon dogfood + results doc

- [ ] **11.1.** Rebuild release daemon at HEAD.
- [ ] **11.2.** Spin up local Jaeger-all-in-one container: `docker run -d -p 16686:16686 -p 4317:4317 jaegertracing/all-in-one:latest`.
- [ ] **11.3.** Launch daemon: `FORGE_OTLP_ENABLED=true FORGE_OTLP_ENDPOINT=http://localhost:4317 FORGE_DIR=/tmp/forge-t1-dogfood /path/to/forge-daemon &`.
- [ ] **11.4.** Seed 100 memories + 3 sessions + tool-use pattern (reuse T11 dogfood script from 2A-4c2; adapt for this tier).
- [ ] **11.5.** Call `force_consolidate`.
- [ ] **11.6.** Assertions:
  - `curl $API -d '{"method":"force_consolidate"}'` returns `skills_inferred` and all other counters.
  - `curl http://127.0.0.1:8420/metrics` contains `forge_phase_duration_seconds`, `forge_phase_output_rows_total`, `forge_table_rows`.
  - `sqlite3 /tmp/forge-t1-dogfood/forge.db "SELECT COUNT(*) FROM kpi_events WHERE event_type='phase_completed'"` == 23 per pass.
  - Every kpi_events row has `metadata_schema_version: 1` in metadata_json.
  - Jaeger UI at `http://localhost:16686` shows a trace with 23 child spans under `consolidate_pass` root.
- [ ] **11.7.** Kill daemon + cleanup tempdir + Jaeger container.
- [ ] **11.8.** Write results doc at `docs/benchmarks/results/2026-04-XX-forge-identity-observability-T1.md` — screenshots, counts, migration note for operators who grep stderr.
- [ ] **11.9.** Update `HANDOFF.md` §Lifted constraints with Tier 1 entry.
- [ ] **11.10.** Commit results + HANDOFF.

---

## Acceptance (repeated from spec §8 for operator convenience)

- [ ] `/metrics` ≥ 10 families.
- [ ] `SELECT COUNT(*) FROM kpi_events WHERE event_type='phase_completed'` == 23 per pass.
- [ ] `metadata_schema_version == 1` in every row.
- [ ] Jaeger shows 23 child spans under `consolidate_pass`.
- [ ] `grep -rn 'eprintln!\|println!' crates/daemon/src/workers/*.rs | grep -v '#\[cfg(test'` returns 0.
- [ ] `PHASE_ORDER`, `Request::ProbePhase`, handler remain cfg-gated.
- [ ] `cargo test --workspace` ≥ 1388 baseline + ≥ 23 new + auxiliary tests.
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` clean.
- [ ] `scripts/check-harness-sync.sh` clean.
- [ ] CI span-integrity + `tokio::spawn` guard installed.
- [ ] Latency budget within limits.
- [ ] Two adversarial reviews complete on the diff.
- [ ] `docs/architecture/kpi_events-namespace.md` committed.

---

## Unblocks

- Tier 2 (2A-4d.2) — `/inspect {layer, shape, window}` reads from `kpi_events` + per-table gauges.
- Tier 3 (2A-4d.3) — bench scoring consumes Tier 2's queries.
- Any future operator Grafana dashboard.
- 2A-4c2 Codex-LOW "no structured tracing for `skills_inferred`" — closed here.

---

## 2A-4d.1.1 Follow-Up Backlog

Single source of truth for every finding surfaced by adversarial reviews on the T1–T12 diffs.
The T13 wave (commits `a0429ea` + `e8c9116`) closed four of these. The remaining four are
documented below with their fix plan and why-deferred rationale — none block Tier 2
design work.

### Closed in T13

| # | Finding | Closed by |
| --- | ------- | --------- |
| 1 | Claude BLOCKER-1 / BLOCKER-4 — `error_count` honesty (11 swallowing helpers, Phase 9 9a/9b split) | T13.1 `a0429ea` |
| 2 | Claude HIGH-1 — `correlation_id` / `trace_id` wiring (pull OTLP trace id from current span inside `record()`) | T13.2 `e8c9116` |
| 3 | Claude HIGH-2 — `refresh_gauges` holds WAL read lock across 15 SELECTs (collapsed to single scalar-subquery SELECT) | T13.3 `e8c9116` |
| 4 | Claude HIGH-3 + HIGH-4 — T10 harness `N=5` and shared-state accumulation (N=20, fresh `ForgeMetrics` per iter, 1.15× relative ratio) | T13.4 `e8c9116` |

### Still open — 2A-4d.1.1 follow-up

These stay deferred because each is either structural (warrants its own design review) or
cosmetic (zero user-visible impact at Tier 1). Reopen when Tier 2 design surfaces a
concrete consumer that depends on them.

#### 1. Codex MEDIUM — consolidator holds state `Mutex` across all 23 phases

**Finding:** `run_all_phases` holds `Arc<Mutex<DaemonState>>` for its full duration
(~2–30 s on warm DBs). Any handler that also locks the state mutex waits on the full
pass. `/metrics` already avoids the problem via `new_reader`; HTTP handlers sharing
`state.conn` still wait.

**Fix plan:** move the consolidator to its own SQLite connection (mirror `WriterActor`),
or acquire-release the state lock per phase.

**Why deferred:** structural refactor affecting many assumptions. Acceptable for Tier 1
dogfood on a local daemon. Reopen when Tier 2 introduces concurrent readers that can't
tolerate multi-second blocking.

#### 2. Claude HIGH-4 from T8 — `record()` inside span scope

**Finding:** `record(...)` runs *inside* each phase's `info_span!(...)` scope, so its own
`tracing::warn!` drops get nested under the phase span in log aggregators.
Instrumentation-layer errors misattribute to the phase being instrumented. Cosmetic —
no correctness impact.

**Fix plan:** capture `PhaseOutcome` from a block expression and call `record()` after
the scope drops. Phase 19 already does this (shipped in T9.2). Apply to the remaining
22 phases.

**Why deferred:** 22 sites, zero user-visible benefit until Tier 2 surfaces phase spans
in a UI. Log-aggregator nesting noise is invisible at operator-SLO level.

#### 3. Claude HIGH-5 + HIGH-6 from T12 — CI guard scrubber brittleness

**Finding:** the awk scrubber in `scripts/ci/check_spans.sh` doesn't recognise `r#"…"#`
raw string literals (production string containing `{` / `}` would corrupt
`#[cfg(test)] mod X { … }` brace-balance scope detection). Similarly,
`#[cfg(all(test, feature = "foo"))]` doesn't match the `cfg\(test\)` anchor, so a
feature-gated test module would have its `tokio::spawn` falsely flagged.

**Fix plan:** extend the awk to recognise raw strings (count `#` on entry/exit), broaden
the `#[cfg(...)]` regex to match any form containing `test`. Proper long-term fix:
rewrite as `scripts/ci/check_spans.rs` using `syn` for actual AST parsing.

**Why deferred:** neither form is used in the current codebase. `syn`-based rewrite pairs
naturally with §4 below (integrity test AST-ification).

#### 4. Claude MEDIUM-10 from T12 — integrity test uses substring match on `include_str!`

**Finding:** a rustdoc comment or test string containing `info_span!("phase_1_…")` would
make the per-name count hit 2 and fail the guard for a non-bug reason.

**Fix plan:** parse consolidator.rs via `syn` in the test; count AST-level macro
invocations, not text substrings.

**Why deferred:** no such duplicate exists today. `syn` dependency pull would bloat the
compile graph for a single test; batch with §3 above.

#### 5. Claude MEDIUM-9 from T12 — T10 doesn't exercise OTLP path

**Finding:** Variant B in the T10 harness exercises only the Prometheus and kpi_events
write paths. It never constructs an OTLP tonic exporter, so the real production hot-path
latency (spans serialised + shipped over gRPC) is unmeasured.

**Fix plan:** add a Variant C that constructs a real `BatchSpanProcessor` backed by a
no-op span sink; measure the tracing_opentelemetry layer overhead.

**Why deferred:** separate latency story with its own numbers and reproduction steps.
T13.4 set up the harness to accept this extension without test-infra churn.

---

## 2A-4d.2.1 Follow-Up Backlog

Tier 2 landed at HEAD `c04b6ce` via T1-T9. Adversarial review pair (Claude + Codex)
returned `lockable-with-fixes`; four real correctness issues (BLOCKER-1 vacuous
test, HUD red-path math, snapshot lazy-refresh, staleness guard clamp) closed in
T9 fixes. Seven items deferred here — none block 2A-4d.3.

### Closed in T9

| # | Finding | Closed by |
|---|---------|-----------|
| 1 | `matches!` not wrapped in `assert!` → vacuous error tests | T9 `c04b6ce` |
| 2 | Codex Q9: HUD rendered `ok/total err Ns` from total-error-count → nonsensical | T9 `c04b6ce` (now `cons:23 ⚠Ne Ns`) |
| 3 | `/inspect row_count` permanently stale without `/metrics` scraper | T9 `c04b6ce` — lazy refresh at dispatch (partial; see open #1 below) |
| 4 | HUD cache staleness = 2× interval → 48h max at long intervals | T9 `c04b6ce` — clamped to `[300, 3600]` secs |

### Open — non-blocking for 2A-4d.3

#### 1. `/inspect row_count` lazy-refresh needs Arc wired through DaemonState

**Finding:** T9 wired a branch that calls `refresh_gauges_from_conn` when
`snapshot.refreshed_at_secs == 0 && DaemonState.metrics.is_some()`. But
`DaemonState::new_reader` sets `metrics: None` at every per-request reader,
so the branch never fires in the HTTP path. Dogfood confirmed: `row_count`
returns `stale: true, rows: []` on a running daemon.

**Fix plan:** either (a) thread `Arc<ForgeMetrics>` from `AppState` into the
per-request `DaemonState` at request construction time, or (b) move the
lazy-refresh branch up into the HTTP handler where `AppState` is in scope.
(b) is cleaner.

**Why deferred:** non-destructive — `stale: true` is honest behavior.
Plumb requires its own commit + test.

#### 2. SSE filter `?events=consolidate_pass_completed` returned 0 events in one test

**Finding:** Unfiltered `/api/subscribe` received the event; with a filter one
test capture was silent. The filter code does `types.contains(&event.event)`
on a comma-split query — straightforward on paper.

**Fix plan:** reproduce in a unit test on `subscribe_handler`'s filter logic.

**Why deferred:** emit works; client-side filtering is an escape hatch.

#### 3. HUD I/O on tokio runtime — refactor to spawn_blocking + atomic write

**Finding:** (Codex Q2 + Claude HIGH-5/6) `build_hud_state` runs synchronous
`std::fs::read_to_string` (cache read) + `rusqlite::Connection::open`
(per-event reader) + `std::fs::write` (non-atomic) on the tokio runtime
thread. On bursty extraction events the thread is blocked for I/O and
hud-state.json can be observed mid-write.

**Fix plan:** (a) cache a long-lived read-only `Connection` in the HUD writer
task; (b) wrap cache read/write in `tokio::task::spawn_blocking`; (c) use
tmpfile + atomic rename for the write.

**Why deferred:** not user-visible in low-event-rate workloads. Batch as one
"HUD I/O refactor" issue.

#### 4. HUD 24h rollup not index-backed

**Finding:** (Claude HIGH-2) `COUNT(DISTINCT json_extract(metadata_json,
'$.run_id'))` can't use `idx_kpi_events_phase` (indexes `phase_name`, not
`run_id`). On month-old DBs the query scans all 24h rows per HUD update.

**Fix plan:** either add a `run_id TEXT` column to `kpi_events` with its own
index, or maintain the rollup in-memory (daemon-resident counter).

**Why deferred:** <100ms until `kpi_events` accumulates months of rows.

#### 5. Percentile convention surfaced in API docs

**Finding:** (Claude MEDIUM-1) `shape_latency` uses ceiling-rank percentiles
(`sorted[ceil(p*n)-1]`). For `n=2, p=0.5` this returns the minimum — tests
lock the behavior but Inspect API docs don't tell consumers.

**Fix plan:** one paragraph in `docs/api-reference.md` §Inspect.

**Why deferred:** doc polish; not a correctness bug.

#### 6. `shape_latency` truncation counter off-by-one

**Finding:** (Claude MEDIUM-2) When the global `MAX_TOTAL_ROWS` cap fires,
the row that triggered the break increments `total_seen` but is never added
to any bucket, so `truncated_samples` per group undercounts by 1 for the
affected group. `truncated: true` on the response is correct.

**Fix plan:** move the `total_seen` increment below the bucket insert, OR
credit the skipped row to the offending group.

**Why deferred:** cosmetic; the boolean flag is the signal callers act on.

#### 7. CLI `ObserveShape` mirror vs `forge-core` `ValueEnum` feature flag

**Finding:** (Claude MEDIUM-5) Forge-cli re-declares `InspectShape` as
`ObserveShape` with a `From` impl because forge-core has no clap dep. Any
Tier 3 shape requires a two-file update.

**Fix plan:** Option A — `[features] cli = ["clap"]` in forge-core, derive
`ValueEnum` conditionally. Option B — replace mirrors with `FromStr` table.

**Why deferred:** current shim works. Reopen when Tier 3 adds a new shape.

