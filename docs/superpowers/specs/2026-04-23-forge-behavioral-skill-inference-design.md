# Forge-Behavioral-Skill-Inference — Phase 2A-4c2 Design

**Phase:** 2A-4c2 of Phase 2A-4 Forge-Identity master decomposition.
**Date:** 2026-04-23
**Parent master:** `docs/benchmarks/forge-identity-master-design.md` §5 2A-4c2, §6 Infrastructure assertions 8–12
**Prerequisite:** 2A-4c1 Forge-Tool-Use-Recording shipped 2026-04-23 (HEAD `cf74fb3`).
**Target milestone:** Dim 5 (Behavioral skill inference) — 80% of the composite weight for that dim, 0.15 of overall Forge-Identity composite.

## Summary

Add a new consolidator phase — **Phase 23: `infer_skills_from_behavior`** — that detects recurring clean tool-use patterns across sessions (substrate: 2A-4c1's `session_tool_call` table) and elevates them into the existing `skill` table with a canonical fingerprint + partial unique index for deduplication. Adds `Request::ProbePhase` (test/bench-gated) so infrastructure assertion 9 can verify Phase 23 runs after Phase 17. Updates the `<skills>` renderer in `CompileContext` so inferred skills surface to the agent (dual-gate: `success_count > 0 OR inferred_at IS NOT NULL`).

**Scope cap:** unit-test correctness + manual curl dogfood. Claude Code hook auto-wiring deferred to follow-up. `informed_by` edge between Phase 17 protocols and Phase 23 skills deferred. Skill retirement, success_count updates, fuzzy fingerprinting, cross-agent attribution, LLM-based skill naming — all explicitly out of scope per master.

## 1. Architecture

Phase 23 is a new stage in the existing consolidator pipeline. It runs once per `run_consolidation` call, immediately after Phase 17 (`extract_protocols`) and before Phase 22 (`quality_pressure`).

```
run_consolidation (orchestrator, crates/daemon/src/workers/consolidator.rs)
  ├─ Phase 1..16 (existing)
  ├─ Phase 17: extract_protocols            ← existing at :277 / :1149
  ├─ Phase 23: infer_skills_from_behavior   ← NEW
  ├─ Phase 22: quality_pressure              ← existing at :474
  └─ ...
```

- **Input:** `session_tool_call` rows from the last `skill_inference_window_days` (default 30).
- **Filter:** row-level — only rows with `success=1 AND user_correction_flag=0`.
- **Grouping:** by `(agent, session_id)` in Rust.
- **Fingerprint:** canonical JSON of `(sorted unique tool_names, sorted tool_arg_shapes)` → sha256 hex.
  - `tool_arg_shapes` = top-level keys of each call's `tool_args` object, each call's key-set sorted, then outer list sorted.
- **Elevation gate:** a fingerprint qualifies when it appears in ≥ `skill_inference_min_sessions` (default 3) distinct sessions under the same agent.
- **Output:** zero or more rows in the `skill` table, one per qualifying `(agent, fingerprint)`. Duplicates upsert via `ON CONFLICT(agent, fingerprint) DO UPDATE` merging `inferred_from`.
- **Rendering:** `<skills>` renderer at `recall.rs:1058-1100` updated per Q1 — dual-gate filter includes `inferred_at IS NOT NULL`, adds `inferred_sessions="N"` attribute for Phase 23 rows in place of `uses="N"`.
- **Observability:** `Request::ProbePhase { phase_name } → { executed_at_phase_index, executed_after }`, gated `#[cfg(any(test, feature = "bench"))]`. Reads a static `PHASE_ORDER` const.

**Boundary with Phase 17:** Phase 17 processes memories (protocol promotion from `memory` table); Phase 23 processes tool-call log events (from `session_tool_call` table). Disjoint input/output tables. Both phases can produce outputs from the same underlying user behavior (user *says* X + agent *does* X); both rows are kept, distinct attributions, no merge. (Cross-reference `informed_by` edge is out of scope for 2A-4c2.)

## 2. Components

### 2.1 Schema evolution

Add four columns + a partial unique index to the existing `skill` table, following the idempotent-ALTER pattern at `schema.rs:759, 767`:

```sql
ALTER TABLE skill ADD COLUMN agent TEXT NOT NULL DEFAULT 'claude-code';
ALTER TABLE skill ADD COLUMN fingerprint TEXT NOT NULL DEFAULT '';
ALTER TABLE skill ADD COLUMN inferred_from TEXT NOT NULL DEFAULT '[]';
ALTER TABLE skill ADD COLUMN inferred_at TEXT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_skill_agent_fingerprint
  ON skill(agent, fingerprint)
  WHERE fingerprint != '';
```

The partial index (`WHERE fingerprint != ''`) avoids colliding with pre-existing rows that have the default empty `fingerprint`.

### 2.2 Config additions

New fields on `ConsolidationConfig` (in `config.rs`):

```rust
pub struct ConsolidationConfig {
    // existing fields...
    pub skill_inference_min_sessions: usize,                 // default 3, range 1..=20
    pub skill_inference_window_days: u32,                    // default 30, range 1..=365
    pub skill_inference_tool_name_similarity_threshold: f64, // default 1.0 (strict, future-proof)
}
```

Loaded from `[consolidation]` section of `~/.forge/config.toml`. Validators reject out-of-range values at daemon start — no runtime check in Phase 23 itself.

### 2.3 Pure helpers (new module)

Three pure functions, unit-testable without DB, live in `crates/daemon/src/workers/consolidator.rs` or a sibling `consolidator/skill_inference.rs` module:

```rust
/// Canonical fingerprint per §1. Pure function.
pub fn canonical_fingerprint(calls: &[ToolCall]) -> String;

/// Rule-based domain tag per Q5 heuristic.
pub fn infer_domain(tool_names: &[String]) -> &'static str;

/// Display name per Q5 template: "Inferred: {sorted-tools} [{hash8}]".
pub fn format_skill_name(tool_names: &[String], fingerprint: &str) -> String;
```

Local `ToolCall` struct: `{ tool_name: String, arg_keys: Vec<String> }` — only what the fingerprint needs.

**Domain inference heuristic:**

```
if any(["Read","Write","Edit","Glob","Grep","MultiEdit","NotebookEdit"]) → "file-ops"
else if any(["Bash"])                                                    → "shell"
else if any(["WebFetch","WebSearch"])                                    → "web"
else if any(["TodoWrite","Task"])                                        → "workflow"
else if any(tool_name starts with "mcp__")                               → "integration"
else                                                                     → "general"
```

### 2.4 Phase 23 orchestrator function

```rust
/// Phase 23: Behavioral Skill Inference — elevate recurring clean tool-use
/// patterns from session_tool_call to skill table.
pub fn infer_skills_from_behavior(
    conn: &Connection,
    min_sessions: usize,
    window_days: u32,
) -> usize {
    // 1. SELECT clean rows within window
    // 2. group by (agent, session_id), build per-session fingerprint
    // 3. aggregate fingerprints → { (agent, fp) -> BTreeSet<session_id> }
    // 4. filter where set.len() >= min_sessions
    // 5. INSERT ... ON CONFLICT(agent, fingerprint) DO UPDATE
    //      SET inferred_from = json_merge(existing + new),
    //          inferred_at = excluded.inferred_at
    // Returns total rows affected.
}
```

Return type `usize` matches Phase 17's `extract_protocols`. Registered in the `run_consolidation` orchestrator between the Phase 17 and Phase 22 call sites.

### 2.5 `Request::ProbePhase` + `ResponseData::PhaseProbe`

```rust
// crates/core/src/protocol/request.rs
#[cfg(any(test, feature = "bench"))]
Request::ProbePhase { phase_name: String }

// crates/core/src/protocol/response.rs
#[cfg(any(test, feature = "bench"))]
ResponseData::PhaseProbe {
    executed_at_phase_index: usize,
    executed_after: Vec<String>,
}
```

Static registry in `consolidator.rs`, listing phases in **execution order** (first to last):

```rust
#[cfg(any(test, feature = "bench"))]
pub const PHASE_ORDER: &[(&str, usize)] = &[
    // fn_name (consolidator function name), phase_number (1-based doc numbering)
    ("extract_protocols", 17),
    ("infer_skills_from_behavior", 23),
    // (additional phases may be added later — 2A-4c2 only requires these
    // two to satisfy master assertion 9; extending the list is non-breaking)
];
```

The `phase_number` is the "Phase N" doc convention (independent of array position — e.g., Phase 23 comes *after* Phase 17 in execution but the numbers don't sort in execution order). `executed_after` is derived from array position (prior `fn_name` entries), which gives the genuine execution-order answer.

Handler: linear-scan for `phase_name`, return its `phase_number` as `executed_at_phase_index`, return all prior `fn_name` entries as `executed_after`. Unknown phase → `Response::Error { message: format!("unknown_phase: {phase_name}") }` (consistent with 2A-4c1's `unknown_session:` prefix pattern).

### 2.6 Renderer update

Two changes to `<skills>` rendering at `recall.rs:1058-1100`:

1. Dual-gate filter — from `success_count > 0` to `success_count > 0 OR inferred_at IS NOT NULL`.
2. XML emission branch on `s.inferred_at.is_some()`:
   - Some → `<skill domain="{d}" inferred_sessions="{N}">{name}</skill>` where `N = json_array_len(inferred_from)`.
   - None (pre-existing) → current `<skill domain="{d}" uses="{count}">{name}</skill>`.

Both attribute names coexist; downstream consumers should accept either.

## 3. Data flow

### 3.1 Happy path — 3 sessions, 1 fingerprint, elevation

**Seed (`session_tool_call` rows):**

| id | session_id | agent | tool_name | tool_args (top-level keys) | success | corr |
|----|------------|-------|-----------|---------------------------|---------|------|
| 01A | SA | claude-code | Read | `file_path` | 1 | 0 |
| 02A | SA | claude-code | Edit | `file_path, new_string, old_string` | 1 | 0 |
| 03A | SA | claude-code | Bash | `cmd` | 1 | 0 |
| 01B | SB | claude-code | Read | `file_path` | 1 | 0 |
| 02B | SB | claude-code | Edit | `file_path, new_string, old_string` | 1 | 0 |
| 03B | SB | claude-code | Bash | `cmd` | 1 | 0 |
| 01C | SC | claude-code | Read | `file_path` | 1 | 0 |
| 02C | SC | claude-code | Edit | `file_path, new_string, old_string` | 1 | 0 |
| 03C | SC | claude-code | Bash | `cmd` | 1 | 0 |

**Step 1 — SQL select:**

```sql
SELECT agent, session_id, tool_name, tool_args, created_at
FROM session_tool_call
WHERE success = 1 AND user_correction_flag = 0
  AND created_at > datetime('now', '-30 days')
ORDER BY agent, session_id, created_at
```

Returns 9 rows.

**Step 2 — Rust fingerprint per session:**

SA: unique tool_names sorted = `["Bash","Edit","Read"]`; per-call arg_keys sorted + outer sorted = `[["cmd"],["file_path"],["file_path","new_string","old_string"]]`; JSON canonical: `[["Bash","Edit","Read"],[["cmd"],["file_path"],["file_path","new_string","old_string"]]]`; sha256 → `ab12cd34...`.

SB, SC: same fingerprint (different *values* don't matter; only structural key-sets).

**Step 3 — aggregate:**

```
{("claude-code", "ab12cd34..."): {"SA","SB","SC"}}
```

**Step 4 — filter ≥ 3 sessions:**

Entry qualifies. Elevate.

**Step 5 — INSERT ON CONFLICT:**

Compute:
- Name: `"Inferred: Bash+Edit+Read [ab12cd34]"`
- Domain: `"file-ops"` (Edit/Read match file-ops rule)
- `inferred_from`: `'["SA","SB","SC"]'`
- `inferred_at`: `2026-04-23T14:22:15Z`
- `success_count`: 0

```sql
INSERT INTO skill (
    id, name, domain, description, steps, source,
    agent, fingerprint, inferred_from, inferred_at, success_count
)
VALUES (?, ?, ?, '', '[]', 'inferred', 'claude-code', ?, ?, ?, 0)
ON CONFLICT(agent, fingerprint) DO UPDATE SET
    inferred_from = (
        SELECT json_group_array(DISTINCT value) FROM (
            SELECT value FROM json_each(skill.inferred_from)
            UNION
            SELECT value FROM json_each(excluded.inferred_from)
        )
    ),
    inferred_at = excluded.inferred_at
```

Uses SQLite `JSON1` extension functions (`json_each`, `json_group_array`) — these are bundled in every SQLite version the daemon supports, no extension load needed.

**Step 6 — CompileContext rendering (next call):**

Dual-gate renderer now sees `inferred_at IS NOT NULL`. Emits:

```xml
<skills hint="use 'forge recall --layer skill &lt;keyword&gt;' for full steps">
  <skill domain="file-ops" inferred_sessions="3">Inferred: Bash+Edit+Read [ab12cd34]</skill>
</skills>
```

### 3.2 Reject: insufficient sessions

Only 2 matching sessions → `min_sessions=3` filter skips. No row. Next consolidation run, if a 3rd arrives, the INSERT fires then.

### 3.3 Reject: correction taint (row-level)

Session SA has `user_correction_flag=1` on its Edit row.
- Step 1 SQL filter drops that row.
- SA's remaining rows `[Read,Bash]` compute a different (2-tool) fingerprint.
- Step 3: the 3-tool fingerprint has only SB+SC = 2 sessions → below threshold.
- SA's 2-tool fingerprint has 1 session → below threshold.
- No elevation either way. Correct.

### 3.4 Idempotency (re-run with no new data)

Same fingerprint + same `(agent, fingerprint)` → `ON CONFLICT DO UPDATE`. `inferred_from` set unchanged after merge (existing sessions are already present). `inferred_at` refreshed to new timestamp. No duplicate row.

### 3.5 Re-run with new qualifying session (SD)

Rows from SA/SB/SC/SD.
- Step 3 set: `{"SA","SB","SC","SD"}` (4 sessions).
- Step 5: ON CONFLICT merge → `inferred_from = '["SA","SB","SC","SD"]'`, `inferred_at` updated.
- Renderer: `inferred_sessions="4"`.

## 4. Error handling

| Condition | Handling |
|---|---|
| `tool_args` not valid JSON | `tracing::warn!` with row id, skip that row. Session continues with other rows. |
| Empty / whitespace `tool_name` | Defensive skip with `warn`. Shouldn't happen post-T6 validation. |
| `session_id` NULL | Impossible per schema. No guard. |
| `canonical_fingerprint` failure | Catastrophic. `expect()` acceptable; panic = programming bug. Returns `String` (not `Result`). |
| SQLite `BUSY` on INSERT | Caught by existing consolidator write-lock layer. On persistent failure: `tracing::error!`, continue to next fingerprint's INSERT. |
| SQLite `CONSTRAINT` despite ON CONFLICT | Log + skip. Shouldn't happen; signals schema drift. |
| `io_error` / disk full | Propagate to consolidator orchestrator, it logs + moves to Phase 22. |
| Config out of range | Rejected at daemon start (validator in `config.rs`). No runtime guard in Phase 23. |
| Renderer column missing | Existing `.ok().unwrap_or_default()` recall.rs idiom returns empty vec. No crash. |
| `ProbePhase` unknown phase | `Response::Error { message: format!("unknown_phase: {phase_name}") }`. |
| Empty `session_tool_call` table | SELECT returns 0 rows → 0 elevations → no-op. Returns 0. |

Return signature of `infer_skills_from_behavior` is `usize` (count of elevations), matching Phase 17's `extract_protocols`. No `Result` type.

## 5. Testing

Six layers; ~29 new tests total.

### 5.1 L0 — Pure fingerprint unit tests

- `canonical_fingerprint_is_deterministic` — same calls reordered → same hash.
- `canonical_fingerprint_ignores_arg_values_only_keys` — `{"file_path":"/a"}` vs `{"file_path":"/b"}` → same hash.
- `canonical_fingerprint_distinguishes_different_arg_keys` — `{"cmd":"x"}` vs `{"cmd":"x","run_id":"y"}` → different hashes.
- `canonical_fingerprint_distinguishes_different_tool_sets` — `[Read,Edit]` vs `[Read,Edit,Bash]` → different.
- `infer_domain_file_ops_match` — `["Read","Edit"]` → `"file-ops"`.
- `infer_domain_shell_when_only_bash` — `["Bash"]` → `"shell"`.
- `infer_domain_mcp_prefix` — `["mcp__context7__query-docs"]` → `"integration"`.
- `infer_domain_general_fallback` — `["SomeUnknownTool"]` → `"general"`.
- `format_skill_name_contains_hash_prefix` — matches `"Inferred: X+Y+Z [<8 hex>]"`.

### 5.2 L1 — Phase 23 direct-call tests

- `infer_skills_from_behavior_elevates_at_three_sessions`
- `infer_skills_from_behavior_skips_at_two_sessions`
- `infer_skills_from_behavior_skips_corrected_rows`
- `infer_skills_from_behavior_skips_failed_rows`
- `infer_skills_from_behavior_skips_rows_outside_window`
- `infer_skills_from_behavior_merges_inferred_from_on_conflict`
- `infer_skills_from_behavior_idempotent_on_rerun`
- `infer_skills_from_behavior_separates_fingerprints`
- `infer_skills_from_behavior_separates_agents` — same fingerprint on `claude-code` vs `codex-cli` → 2 rows.

### 5.3 L1 — Renderer dual-gate tests

- `skills_renderer_includes_success_count_rows` — legacy `success_count=1, inferred_at=NULL` → `uses="1"`.
- `skills_renderer_includes_inferred_rows` — Phase-23 row → `inferred_sessions="3"`.
- `skills_renderer_excludes_zero_success_zero_inferred`.
- `skills_renderer_mixed_attributes_coexist`.

### 5.4 L1 — `ProbePhase` handler tests (gated)

- `probe_phase_returns_correct_index_for_registered_phase`
- `probe_phase_executed_after_contains_phase_17`
- `probe_phase_unknown_phase_errors`
- `probe_phase_phase_17_executed_at_index_17`

### 5.5 L3 — Integration (`tests/skill_inference_flow.rs`)

- `skill_inference_end_to_end_via_protocol` — 3× `register_session` → 9× `record_tool_use` matching fingerprint → `force_consolidate` → `compile_context` XML contains `<skill ... inferred_sessions="3">` with expected name.
- `skill_inference_does_not_emit_for_two_sessions` — same flow with 2 sessions, assert skill absent.

### 5.6 L4 — Schema rollback recipe

- `test_skill_phase23_columns_and_index_rollback_recipe_works_on_populated_db` — pre-assertion (per 2A-4c1 H1 precedent) + documented rollback + post-assertion columns+index gone.

### 5.7 L4 — Live dogfood (results doc phase)

- Rebuild daemon, restart.
- `curl` seeding script: 3 sessions × 3 tool calls each.
- `force_consolidate`.
- `compile_context` → capture `<skills>` XML.
- `probe_phase` (if bench feature compiled in) → confirm execution ordering.
- Results: `docs/benchmarks/results/2026-04-23-forge-behavioral-skill-inference.md`.

### 5.8 Test count estimate

| Layer | Count |
|---|---|
| L0 pure | 9 |
| L1 Phase 23 | 9 |
| L1 renderer | 4 |
| L1 ProbePhase | 4 |
| L3 integration | 2 |
| L4 schema | 1 |
| **Total new** | **29** |

Baseline 1352 → ~1381 lib tests + 4 integration tests post-ship.

## 6. Out of scope

- Skill retirement, `success_count` updates, `fail_count` tracking for Phase 23 rows.
- Fuzzy fingerprinting (tool-name similarity threshold < 1.0).
- Cross-agent skill attribution (e.g., a skill demonstrated across `claude-code` + `codex-cli` sessions).
- LLM-based skill naming or description generation.
- Claude Code `PostToolUse` hook auto-wiring (defers to follow-up phase).
- `informed_by` edge between overlapping Phase 17 protocols and Phase 23 skills.
- H3 from SP1 adversarial review (T5/T6 validation reorder) — separate follow-up.
- MCP 4th-slug namespace for SP1's per-tool counter — separate follow-up.

## 7. Dependencies

- **Upstream (must be present):** 2A-4c1 — `session_tool_call` table, `Request::RecordToolUse`, `Request::ListToolCalls`, HEAD `cf74fb3`.
- **Downstream (unblocked by this phase):** 2A-4d Forge-Identity Dim 5 scoring, skill-retirement follow-up, hook auto-wiring follow-up.

## 8. Master-design assertion satisfaction

| # | Assertion | Satisfied by |
|---|-----------|--------------|
| 6 | `skill_inference_min_sessions` ∈ 1..=20 | §2.2 config validator. |
| 8 | `skill` table has `agent, fingerprint, inferred_from, success_count, inferred_at`; unique index on `(agent, fingerprint)` | §2.1 migration + partial unique index. |
| 9 | Phase 23 registered + executes after Phase 17; verified via `Request::ProbePhase` | §2.5 static const + probe handler + L1 ProbePhase tests. |
| 12 | After seeding Phase 23 skill via `RecordToolUse` ≥ 3 sessions + `ForceConsolidate`, `<skills>` contains a `<skill>` with the seeded token | §2.6 renderer update + §5.5 L3 integration tests + §5.7 dogfood. |

## 9. Task budget

Master predicted 8-12 tasks; master also flagged the canonical-fingerprint complexity as the biggest unknown. Estimated task count below (will be refined in the implementation plan):

| Task | Scope |
|---|---|
| T1 | Schema migration — ALTER skill + partial unique index |
| T2 | Config additions + validator |
| T3 | Pure helpers (`canonical_fingerprint`, `infer_domain`, `format_skill_name`) + 9 L0 tests |
| T4 | `infer_skills_from_behavior` fn + SQL select + 9 L1 tests |
| T5 | Register Phase 23 in `run_consolidation` orchestrator |
| T6 | `Request::ProbePhase` + `ResponseData::PhaseProbe` + handler + `PHASE_ORDER` const + 4 L1 tests |
| T7 | Renderer dual-gate + `inferred_sessions=` attribute + 4 L1 tests |
| T8 | Integration test `tests/skill_inference_flow.rs` (2 tests) |
| T9 | Schema rollback recipe test (1 test) |
| T10 | Adversarial review |
| T11 | Live-daemon dogfood + results doc |

11 tasks — within master's 8-12 estimate.
