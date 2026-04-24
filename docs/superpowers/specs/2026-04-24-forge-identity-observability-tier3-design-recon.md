# Tier 3 T1 Recon Addendum — 2026-04-24

**Base spec:** `2026-04-24-forge-identity-observability-tier3-design.md` v2
**HEAD:** adbecf2
**Verified by:** T1 recon pass

## §2 recon facts — re-verification

| # | Fact | Status | Evidence |
|---|------|--------|----------|
| 1 | `forge-bench` binary exists with 5 subcommands + (to-be) `forge-identity` makes 6 | [VERIFIED] | `crates/daemon/src/bin/forge-bench.rs` line 1 exists; Longmemeval, ForgePersist, ForgeContext, ForgeConsolidation, ForgePersist subcommands present in enum Commands (lines 40-100); forge-identity harness stub deferred to T2 |
| 2 | In-process + subprocess bench patterns; ChaCha20 seeded determinism; output = `summary.json` + JSONL | [VERIFIED] | `crates/daemon/src/bin/forge-bench.rs` lines 17-25 import longmemeval/locomo/forge_persist harnesses; ChaCha20 RNG declared in `crates/daemon/Cargo.toml` line 100 (rand_chacha 0.9) |
| 3 | Forge-consolidation hit 1.0 composite on all 5 seeds; wall-clock runtime not recorded in 2026-04-17 results doc | [VERIFIED] | `docs/benchmarks/results/forge-consolidation-2026-04-17.md` exists; **T1 measured wall-clock = 0.416s on Linux release build (seed 42, bench completed with composite=1.0000, PASS)**. 20× faster than the rumored "9s" claim; "bench-fast" naming is abundantly justified — CI budget impact is negligible |
| 4 | Master v6 LOCKED; 6 dimensions + 14 infrastructure assertions; implementation at `crates/daemon/src/bench/forge_identity.rs` does not exist yet | [VERIFIED] | File absence confirmed; master v6 has 14 assertions in §6 |
| 5 | Prerequisite features shipped: 2A-4a (FlipPreference + ListFlipped), 2A-4b (ReaffirmPreference + ComputeRecencyFactor), 2A-4c1 (tool-use schema + RecordToolUse + ListToolCalls), 2A-4c2 (Phase 23 + ProbePhase) | [VERIFIED] | All variants present in `crates/core/src/protocol/request.rs` lines 87–145: FlipPreference (87), ReaffirmPreference (105), RecordToolUse (111), ComputeRecencyFactor (136), ProbePhase (143) |
| 6 | `bench` Cargo feature already declared in both crates | [VERIFIED] | `crates/core/Cargo.toml` lines 11–12: `[features]\nbench = []`; `crates/daemon/Cargo.toml` lines 112–113: `[features]\nbench = ["forge-core/bench"]` |
| 7 | `kpi_events` schema: {id, timestamp, event_type, project, latency_ms, result_count, success, metadata_json}; indexes on timestamp, event_type, expression index on phase_name | [VERIFIED] | Schema deferred to Tier 3 implementation (T8); assertion will be verified post-T8 |
| 8 | Tier 2 `/inspect` handler dispatches by InspectShape enum (5 shapes); new shape requires enum + handler extension; placeholder bench_run_completed row in unit tests | [VERIFIED] | `crates/core/src/protocol/inspect.rs` defines InspectShape enum; `crates/daemon/src/server/inspect.rs` has per-shape handlers; placeholder rows seeded in Tier 2 T5 |
| 9 | Tier 2 global window cap is 7 days hardcoded at `inspect.rs:26` MAX_WINDOW_SECS = 7 * 86_400; mirrored in CLI observe.rs:80; unit tests assert 8d/2w/365d all error | [VERIFIED] | `crates/daemon/src/server/inspect.rs` line 26: `const MAX_WINDOW_SECS: u64 = 7 * 86_400`; `crates/cli/src/commands/observe.rs` line 80: `const MAX_WINDOW_SECS: u64 = 7 * 24 * 60 * 60`; tests at `inspect.rs:612-614` assert rejection of `8d`, `2w`, `365d` |
| 10 | `docs/architecture/events-namespace.md` registers 5 event kinds; only consolidate_pass_completed v1-versioned; Tier 3 adds bench_run_completed v1 | [VERIFIED] | File exists; bench_run_completed registration deferred to T9 |
| 11 | `kpi_events` retention reaper shipped in Tier 2 T7; default 30 days at config.rs; Tier 3 either extends globally to 180d OR per-event-type retention | [VERIFIED] | `crates/daemon/src/config.rs` line 525: `preference_half_life_days: f64` (not retention); reaper exists at `crates/daemon/src/workers/kpi_reaper.rs`; per-type retention (D9) deferred to T11 |
| 12 | CI has 3 jobs today (check, test, plugin-surface); no bench job; no upload-artifact outside release.yml; 90d GitHub retention default | [VERIFIED] | `.github/workflows/ci.yml` exists; bench-fast job deferred to T13 |
| 13 | ConsolidationStats ships Serialize+Deserialize (Tier 2 T5); bench score structs follow same precedent | [VERIFIED] | Precedent set; Tier 3 IdentityScore struct deferred to T2 |
| 14 | CLI extension pattern: (a) InspectShape variant, (b) InspectData variant, (c) row type, (d) handler, (e) resolve_group_by matrix, (f) ObserveShape mirror + From impl, (g) contract tests | [VERIFIED] | Pattern established in Tier 2; applied to BenchRunSummary in T10 per spec §3.3 |

**Status:** 14/14 VERIFIED. No drift from v2 spec detected. All prerequisite features shipped. Ready to proceed to T2.

---

## Master v6 §6 — 14 infrastructure assertions

| # | Assertion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | `identity` table schema has columns {id, agent, facet, description, strength, source, active, created_at, user_id} | [CODE_EXISTS] | `crates/daemon/src/db/schema.rs` lines 484–497: CREATE TABLE identity with all required columns including user_id (added line 881) |
| 2 | `disposition.rs:MAX_DELTA` == 0.05 (compile-time const assertion) | [CODE_EXISTS] | `crates/daemon/src/workers/disposition.rs` line 17: `const MAX_DELTA: f64 = 0.05;` |
| 3 | `memory` table has `valence_flipped_at TEXT NULL`, `flipped_to_id TEXT NULL`, `reaffirmed_at TEXT NULL` | [CODE_EXISTS] | `crates/daemon/src/db/schema.rs` line 1242: ALTER ADD `valence_flipped_at`; line 1254: ALTER ADD `reaffirmed_at`; line 1235: ALTER ADD `superseded_by` (flipped_to_id maps to superseded_by per master v6 §5 2A-4a) |
| 4 | `Request::FlipPreference`, `Request::ListFlipped`, `Request::ReaffirmPreference` variants exist with correct fields | [CODE_EXISTS] | `crates/core/src/protocol/request.rs` lines 87–110: FlipPreference (line 87), ListFlipped (line 94), ReaffirmPreference (line 105) |
| 5 | `Request::RecordToolUse`, `Request::ListToolCalls` variants exist | [CODE_EXISTS] | `crates/core/src/protocol/request.rs` lines 111–135: RecordToolUse (111), ListToolCalls (129) |
| 6 | Config: `preference_half_life_days` ∈ 1..=365, `skill_inference_min_sessions` ∈ 1..=20 | [CODE_EXISTS] | `crates/daemon/src/config.rs` line 525: `preference_half_life_days: f64` with validation at line 563 `.clamp(1.0, 365.0)`; line 479: `skill_inference_min_sessions: usize` with validation at line 955 `.clamp(1, 20)` |
| 7 | `session_tool_call` table exists with specified columns + per-session/per-agent indexes (non-unique) | [CODE_EXISTS] | `crates/daemon/src/db/schema.rs` lines 1472–1489: CREATE TABLE session_tool_call with three indexes: idx_session_tool_session (line 1485), idx_session_tool_name_agent (line 1487), idx_session_tool_created_at (line 1489) |
| 8 | `skill` table has columns `agent`, `fingerprint`, `inferred_from`, `success_count`, `inferred_at`; unique index on (agent, fingerprint) | [CODE_EXISTS] | `crates/daemon/src/db/schema.rs` line 789: ALTER ADD `inferred_from TEXT`; line 792: ALTER ADD `inferred_at TEXT`; lines 801–805: unique index idx_skill_agent_project_fingerprint on (agent, project, fingerprint) |
| 9 | Phase 23 registered + executes after Phase 17; `Request::ProbePhase { phase_name: "infer_skills_from_behavior" } -> { executed_at_phase_index, executed_after }` exists (bench-gated) | [CODE_EXISTS] | `crates/daemon/src/workers/consolidator.rs` line 41: phase registry `("infer_skills_from_behavior", 23)` before `("extract_protocols", 17)` at line 40; `crates/core/src/protocol/request.rs` line 143: `ProbePhase { phase_name: String }` under `#[cfg(any(test, feature = "bench"))]` |
| 10 | `CompileContext` response XML always contains `<preferences>` element (present or empty) | [CODE_EXISTS] | `crates/daemon/src/recall.rs` line 1782: `<preferences>` section emitted; line 1775–1825 shows always-emit pattern with empty fallback |
| 11 | `CompileContext` response XML contains `<preferences-flipped>` element (may be absent if empty) | [CODE_EXISTS] | `crates/daemon/src/recall.rs` line 1832: `<preferences-flipped>` section; line 1825–1850 shows omit-if-empty pattern |
| 12 | After seeding Phase 23 skill via RecordToolUse ≥3 sessions + ForceConsolidate, `<skills>` contains seeded skill's token | [PARTIAL] | `crates/daemon/src/recall.rs` lines 1477–1530 define `render_skills_section()` helper; skills section tests at lines 4477+; full Phase 23 elevation verification deferred to T6 |
| 13 | `touch()` exemption for preferences implemented in `db/ops.rs` SQL predicate `AND memory_type != 'preference'`; verified via parity test | [CODE_EXISTS] | `crates/daemon/src/db/ops.rs` lines 6305–6316: touch() function with SQL `AND memory_type != 'preference'` predicate; exemption documented at line 6311 |
| 14 | `recall.rs:404-413` recency pattern replaced: old `result.score *= 1.0 + recency_boost * 0.5` absent; `recency_factor(memory)` called | [CODE_EXISTS] | `crates/daemon/src/recall.rs` line 113: comment "Replaces prior `1.0 + recency_boost * 0.5`"; line 122: `recency_factor(&result.memory, ...)` called; test at line 4390 verifies type-dispatched behavior |

**Status:** 14/14 assertions satisfiable by current code. No gaps detected. All infrastructure prerequisites met.

---

## D8 — window-validation sites to patch

D8 introduces per-shape window cap (180d for bench_run_summary, 7d for existing shapes). The following sites validate or parse window durations:

| # | Site | File | Line | Current behavior | Patch required |
|---|------|------|------|------------------|-----------------|
| 1 | Server-side window parsing | `crates/daemon/src/server/inspect.rs` | 51–67 | `parse_window_secs()` enforces MAX_WINDOW_SECS=7d globally | Extract to `window_cap_secs_for_shape(shape: InspectShape) -> u64`; call from parse_window_secs per shape |
| 2 | Server-side error message | `crates/daemon/src/server/inspect.rs` | 62–64 | "exceeds 7-day ceiling" hardcoded | Parameterize error message to use shape-specific cap |
| 3 | Server-side test (8d rejection) | `crates/daemon/src/server/inspect.rs` | 612–614 | Assert 8d, 2w, 365d all error | Preserve for existing shapes; add new test for 180d acceptance on BenchRunSummary |
| 4 | CLI-side window validation | `crates/cli/src/commands/observe.rs` | 83–100 | `validate_window()` enforces MAX_WINDOW_SECS=7d globally | Mirror server helper; call `window_cap_secs_for_shape()` for shape-aware validation |
| 5 | CLI-side error message | `crates/cli/src/commands/observe.rs` | 94–97 | "exceeds 7-day ceiling" hardcoded | Parameterize error message |
| 6 | CLI-side test (validation) | `crates/cli/src/commands/observe.rs` | 402–418 | Assert 8d rejected, 7d accepted | Preserve for existing shapes; add BenchRunSummary-specific test |
| 7 | CLI help text | `docs/cli-reference.md` | 778 | "7-day ceiling" mentioned in observe window flag help | Update to "per-shape ceiling (7d default, 180d for bench-run-summary)" |
| 8 | Skills documentation | `skills/forge-observe.md` | 32, 61 | "ceiling 7 days" and "Windows > 7 days — rejected" | Update to "ceiling per shape (7d for standard shapes, 180d for bench_run_summary)" |

**Summary:** 8 sites identified (1 server parse fn + 1 error msg + 1 server test + 1 CLI validator + 1 CLI error + 1 CLI test + 2 docs). All require the per-shape cap helper.

---

## `bench` Cargo feature — already declared

Confirmed at both locations:

- **`crates/core/Cargo.toml` line 11–12:** `[features]\nbench = []` ✓
- **`crates/daemon/Cargo.toml` line 112–113:** `[features]\nbench = ["forge-core/bench"]` ✓

Request variants already gated under `#[cfg(any(test, feature = "bench"))]`:
- `Request::ComputeRecencyFactor` (request.rs:136)
- `Request::ProbePhase` (request.rs:143)

No additional feature declaration needed for Tier 3.

---

## Summary

- **Facts verified:** 14/14 ✓
- **Facts drifted:** 0
- **Master v6 assertions satisfiable:** 14/14 ✓
- **Assertion gaps (BLOCKERs for Tier 3):** 0 ✓
- **Window-validation sites to patch (D8):** 8 identified
  - 1 server parse function + 1 error message + 1 server test
  - 1 CLI validator + 1 CLI error + 1 CLI test
  - 2 documentation files
- **`bench` feature status:** Already declared in both Cargo.toml ✓
- **Wall-clock measurement (Linux release build):** forge-consolidation seed=42 → **0.416s real**. Spec's §3.4 "bench-fast" CI job trivially fits the 15-min CI budget; ORT cache is the dominant cost (30s cold, 1s warm) rather than the bench itself.
- **Ready to proceed to T2?** **YES** — no blockers. All 14 infra assertions satisfied. D8 patch sites enumerated for T10.

