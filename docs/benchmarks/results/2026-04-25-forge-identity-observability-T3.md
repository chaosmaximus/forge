# Phase 2A-4d.3 Bench Harness — T3 dogfood results

**Date:** 2026-04-25
**HEAD:** `8dccf58` (T15.3 per-dim isolation + adversarial review fixes applied)
**Isolation:** `FORGE_DIR=/tmp/fi-dogfood`, all ambient state reset
**Test flow:** release `forge-bench forge-identity --seed 42`, fresh DB

## What was verified live

### `forge-bench forge-identity --seed 42`

```
[forge-identity] === results ===
[forge-identity] composite=0.8490
[forge-identity] pass=false
[forge-identity] identity_facet_persistence       = 1.0000 (min 0.85, pass=true)
[forge-identity] disposition_drift                = 0.0000 (min 0.85, pass=false)
[forge-identity] preference_time_ordering         = 1.0000 (min 0.80, pass=true)
[forge-identity] valence_flipping                 = 1.0000 (min 0.85, pass=true)
[forge-identity] behavioral_skill_inference       = 1.0000 (min 0.80, pass=true)
[forge-identity] preference_staleness             = 0.9960 (min 0.80, pass=true)
[forge-identity] wall_duration_ms=909
[forge-identity] FAIL
forge-bench: forge-identity FAIL: composite below threshold or per-dim minimum
exit 1
```

**Composite math check:**
`0.15 × 1.0 + 0.15 × 0.0 + 0.15 × 1.0 + 0.15 × 1.0 + 0.15 × 1.0 + 0.25 × 0.996 = 0.849` ✅

**Wall-clock cost** of per-dim DaemonState isolation (T15.3 B1+B2 fix):
- Pre-isolation (shared state): 146ms
- Post-isolation (6 fresh `:memory:` + master state): 909ms
- Within CI budget; T1 measured forge-consolidation at 416ms baseline.

### `kpi_events` row written end-to-end

```
$ sqlite3 /tmp/fi-dogfood/forge.db "SELECT event_type, success, result_count,
    json_extract(metadata_json, '$.bench_name'),
    json_extract(metadata_json, '$.composite'),
    json_extract(metadata_json, '$.event_schema_version')
  FROM kpi_events;"
bench_run_completed|0|6|forge-identity|0.849|1
```

✅ `event_type='bench_run_completed'`, `success=0` (pass=false), `result_count=6` (6 dims).
✅ Payload v1 with bench_name, composite, event_schema_version.

### Tier 2 amendments (D8 + D9) verified

- **D8 (per-shape window cap):** `parse_window_secs` rejects 200d for `BenchRunSummary` shape ✅; existing 5 shapes still reject 8d.
- **D9 (per-event-type retention):** kpi_reaper test `test_bench_run_completed_survives_31_days` passes with default 180d override; phase_completed reaped at 31d via global default.

### Master v6 §13 D7 isolation invariant (T15.3 B1+B2)

`run_bench` now opens 1 master DaemonState for infra checks, drops it,
then spawns 6 separate `DaemonState::new(":memory:")` instances — one
per dim. Phase 4 decay / Phase 23 / ForceConsolidate side effects
inside Dim 4 + Dim 5 cannot leak into Dim 6's pre-consolidator
fixture (master v6 §7 line 200 invariant).

## Tier 3 feature checklist

- [x] `forge-identity` bench harness in `crates/daemon/src/bench/forge_identity.rs` (1818 LoC)
- [x] 6 dimension scorers (5 implemented, Dim 2 stub blocked on backlog #2)
- [x] 14 infrastructure assertions per master v6 §6 (run BEFORE dimensions, fail-fast)
- [x] Per-dim DaemonState isolation (T15.3)
- [x] `bench_run_completed` v1 kpi_events event (T8)
- [x] `crates/daemon/src/bench/telemetry.rs` emit helper + 6 wiring sites
- [x] `InspectShape::BenchRunSummary` + `BenchRunRow` + `shape_bench_run_summary` (T10)
- [x] `window_cap_secs_for_shape` D8 — 7d for existing shapes, 180d for BenchRunSummary
- [x] `kpi_events_retention_days_by_type` D9 — 180d default for `bench_run_completed`
- [x] `forge-bench forge-identity` CLI subcommand (T7)
- [x] `forge-next observe --shape bench-run-summary` (T10 CLI mirror)
- [x] `.github/workflows/ci.yml` `bench-fast` job — matrix [forge-consolidation, forge-identity], continue-on-error: true
- [x] Adversarial review pair (Claude + Codex) on T1-T13 — 3 BLOCKERs + 5 HIGHs + 4 MEDIUMs + 3 LOWs surfaced
- [x] T15 close-out: 3 BLOCKERs + 5 HIGHs all addressed (B1+B2 isolation refactor, B3 docs, H1 tests, H3 token search, H4 schema-version field, H5 retention 0 sentinel)
- [x] Live dogfood (this doc)

## Known gaps deferred to 2A-4d.3.1

1. **Dim 2 disposition drift stub** — `Request::StepDispositionOnce` variant
   doesn't exist; composite caps at 0.849 until landed. Tracked as
   backlog #2 in `docs/superpowers/plans/2026-04-24-forge-identity-observability.md`.
2. **T12 calibration loop** — deferred until Dim 2 lands. Master v6 mandates
   1.0 composite on 5 seeds; without Dim 2 the bench returns 0.849
   honestly but cannot reach the gate.
3. **shape_bench_run_summary percentile cap pulls all rows into Rust**
   (T14 H2). Backlog #5.
4. **Forge harness context-injection volume** + **sub-agent commit-discipline
   failures** (backlog #3 + #4) — operator ergonomics, raised
   2026-04-24, no Tier 3 implementation pressure.
5. **4 cosmetic items** (MEDIUMs M1-M4) + 3 LOWs from T14 review —
   batched into 2A-4d.3.1 backlog #6 for the next operator-polish PR.

## Verdict

Tier 3 ships honestly:
- **Infrastructure is fully reviewed + tested + dogfooded.**
- **Composite ceiling is 0.849, capped by Dim 2 stub.**
- **5 of 6 dimensions calibrated to ≥ 0.996.**
- **CI bench-fast job lands continue-on-error per D4 rollout.**

The 0.95 composite threshold from master v6 is a planned-future state
once Dim 2 + StepDispositionOnce ship. No correctness regressions.
Operators get bench_run_completed telemetry + leaderboard surface +
180-day retention immediately; calibration story closes when 3.1
lands StepDispositionOnce.

## Tier 3 commits (master)

```
8dccf58 fix(2A-4d.3 T15.3): per-dim DaemonState isolation (B1 + B2)
a436781 fix(2A-4d.3 T15.2): T14 H1 + H3 + H5 — small fixes
fea74fd docs(2A-4d.3 T15.1): register bench_run_completed in kpi_events-namespace.md (B3 + H4)
f429694 feat(2A-4d.3 T13): CI bench-fast job
0c18fa7 feat(2A-4d.3 T11): per-event-type kpi_events retention (D9)
7df0727 docs(2A-4d.3 T9): register bench_run_completed v1 in events-namespace.md
e93d84b feat(2A-4d.3 T8): bench telemetry — emit_bench_run_completed + 6 wiring sites
5ede10b fix(2A-4d.3 T7): forge-identity exits non-zero on FAIL
eba3169 feat(2A-4d.3 T7): forge-bench forge-identity subcommand
d6399ab fix(2A-4d.3.1 #1): always-emit <preferences> in CompileContext XML
20864c8 docs(2A-4d.3.1): add backlog — infra #10, Dim 2 blocker, harness ergonomics
5d9e28c feat(2A-4d.3 T5): Dim 5 behavioral skill inference
a565fa9 feat(2A-4d.3 T4): Dim 4 valence flipping correctness
40d5214 feat(2A-4d.3 T10): InspectShape::BenchRunSummary + per-shape window cap (D8)
09a37a7 feat(2A-4d.3 T3+T6): Request::Recall query_embedding + Dim 1/3/6 + infra
7ed232e feat(2A-4d.3 T2): forge_identity.rs skeleton + 6 dim stubs + composite scorer
53a88ed docs(2A-4d.3 T1): recon addendum + wall-clock measurement
adbecf2 docs(2A-4d.3 T1): Tier 3 bench harness v2 design spec locked
```

19 commits this session, 250cb1a → 8dccf58.

## Test counts (1470 daemon-lib tests, all green)

- bench::forge_identity: 11/11
- bench::telemetry: 5/5
- workers::kpi_reaper: 8/8 (includes 3 D9-specific tests)
- server::inspect: 36/36 (includes 9 BenchRunSummary tests + 2 H1 corner-case tests)
- Plus all prior test suites unchanged.
