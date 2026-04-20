# SP1 — Dark-Loop Closure: Writer-Channel + Populator Fixes

**Date**: 2026-04-20
**Author**: session-1776623798 (Claude Code, Opus 4.7 1M)
**Sub-project**: 1 of 5 in the daemon bug-fix decomposition (see DAEMON-STRATEGY-V3-2026-04-20.md)
**Covers bugs**: #45, #53, #54, #55 from `forge-app-private/product/engineering/daemon-team/SESSION-GAPS.md`
**Methodology**: superpowers:brainstorming → writing-plans → executing-plans with TDD + `simplify` skill + adversarial Codex review at each commit

## 0. Summary

Four feedback loops in the daemon are dark: context-injection counter never increments, extraction telemetry always reads 0, per-tool counters stay at 0 across all 42 tools, and the skill registry table sits empty. Live reconnaissance (2026-04-20) confirms all four are still dark despite an 8-active-session daemon with 249 messages in-flight. This sub-project wires each loop and adds doctor probes so regressions fail red instead of silently.

The four bugs cluster as **three counter-wiring gaps + one populator-invocation gap**:

| Bug | Type | Fix shape |
|---|---|---|
| #45 | Counter-wiring (writer-channel) | Add `try_send(RecordInjection)` to 3 proactive handlers in `handler.rs` |
| #53 | Counter-wiring (new command) | Add `WriteCommand::RecordExtraction`, wire extractor success + error paths |
| #54 | Counter-wiring (direct write) | Extract tool names from transcript chunks, call existing `record_tool_use()` |
| #55 | Populator-invocation | Call existing `refresh_skills()` at daemon init |

Delivered as one PR on a feature branch `sp1/dark-loops` (not master, to avoid conflicts with active parallel work — see §11), four fix-commits plus one trailing commit (integration test). Doctor probes are deferred to a follow-up PR per §11.2.

## 1. Context (why)

### 1.1 — State of affairs (2026-04-20 telemetry)

- `forge-next context-stats` → `Injections: 0, Effectiveness: 0.0%` on a dogfood daemon with 8 active sessions + 249 messages.
- `forge-next stats` → `Extractions: 0 (0 errors)` over the same 24h window.
- `forge-next tools` → 42 tools, every one `used: 0x`.
- `forge-next skills-list` → `count: 0` while 15 skills exist in `forge-app-private/skills/`.

All four counters sit on top of code that exists and is reachable — the wiring to record events is missing in four distinct places.

### 1.2 — Strategic weight

DAEMON-STRATEGY-V3-2026-04-20 names #45 as **the one bet**: "fix `context_injections = 0 ever` in Week 4 — the whole strategy depends on it." Without this loop lit, every benchmark claim about session-start memory injection is unverifiable on the founder's own machine.

Bugs #53, #54, #55 are flagged P1 "dark loops" in V3 §5 and the QA plan in V3 §8 lists their integration tests as must-haves before any public claim.

### 1.3 — Why together

Three of the four bugs (#45, #53, #54) are structurally "event happens → event not recorded." Investigating them in a single pass lets us generalize the doctor-probe pattern once. #55 rides along because its fix is 3 lines in daemon init and the integration test naturally verifies it.

## 2. Architecture

Four focused fixes, one PR on feature branch `sp1/dark-loops`, five commits (4 fix + integration test). Doctor probes deferred per §11.2. No new crates, no new abstractions. Each fix extends an existing pattern.

| Bug | Pattern | Primary file(s) | Est. |
|---|---|---|---|
| #55 | Direct populator invocation | `crates/daemon/src/bin/forge-daemon.rs`, `crates/daemon/src/skills.rs` | 0.5 day |
| #45 | Writer-channel (copy of CompileContext at `handler.rs:2762-2772`) | `crates/daemon/src/server/handler.rs` | 1 day |
| #54 | Direct `UPDATE` on `tool.use_count` | `crates/daemon/src/extractor.rs`, `crates/daemon/src/db/manas.rs` | 1 day |
| #53 | New `WriteCommand::RecordExtraction` | `crates/daemon/src/writer.rs`, `crates/daemon/src/extractor.rs`, `crates/daemon/src/db/metrics.rs` | 2 days |

**Commit order** (simplest → complex, so each lands green before the next and any interrupt leaves a useful partial state):

1. **`fix(skills): auto-index skill_registry on daemon start (#55)`**
2. **`fix(injection): record proactive context injections (#45)`**
3. **`fix(tools): increment per-tool use_count in extractor (#54)`**
4. **`fix(extraction): record extraction metrics via writer channel (#53)`**
5. **`test(sp1): e2e_sp1_dark_loops integration test`**

Deferred (follow-up PR tentatively "Doctor-Probes-SP1b"): `feat(doctor): add SP1 fail-red probes`.

**Scope exclusions** (deliberate):

- No refactor of existing writer-channel code.
- No new hook endpoints on the Claude Code app side (#54 reuses extractor; #45 uses existing handlers).
- No changes to `stats`/`tools`/`skills-list`/`context-stats` CLI output format beyond what's required to surface newly-recorded metrics.
- No auto-registration of unknown tool names (noted in §4.3).

## 3. Components

### 3.1 — #55 Skill-Registry Auto-Populate

**Location**: daemon bootstrap (`crates/daemon/src/bin/forge-daemon.rs` or the `DaemonState::new()` flow — confirmed during implementation) + existing populator in `crates/daemon/src/skills.rs`.

**Change**: After DB + migrations are ready, resolve `skills_directory` from config (cascade: env `FORGE_SKILLS_DIR` → config file → default `~/.forge/skills` → project `skills/`), then call:

```rust
match crate::skills::refresh_skills(&conn, &skills_dir) {
    Ok(n)  => tracing::info!(skills = n, path = %skills_dir.display(), "Skill registry populated"),
    Err(e) => tracing::warn!(error = %e, path = %skills_dir.display(),
                             "Skill auto-index failed; registry will be empty until RefreshSkillsIndex called"),
}
```

Daemon boot continues on error. No new config keys.

**Why not background worker**: directory of ~15 skills, idempotent upsert — startup scan is O(ms). A periodic worker is over-engineering.

### 3.2 — #45 Proactive Context Injection Recording

**Location**: `crates/daemon/src/server/handler.rs:1985, 2004, 2037` — three return-paths of `build_proactive_context()`.

**Change**: After constructing `proactive_context` and before returning the Response, emit:

```rust
if let Some(tx) = &state.writer_tx {
    let chars: usize = proactive_context.iter().map(|i| i.content.len()).sum();
    let summary = proactive_context
        .iter()
        .map(|i| format!("{}:{}", i.knowledge_type, i.content.len()))
        .collect::<Vec<_>>()
        .join(",");
    let _ = tx.try_send(WriteCommand::RecordInjection {
        session_id: session_id.clone(),
        hook_event: "PreBashChecked".to_string(),  // site-specific: PreBashChecked / PostBashCheck / PostEditCheck
        context_type: "proactive".to_string(),
        content_summary: summary,
        chars_injected: chars,
    });
}
```

`hook_event` differs per site. No `writer.rs` changes — the `RecordInjection` command already does the right thing.

**Why `context_type: "proactive"`**: distinguishes from CompileContext (`"full_context"`) so downstream analytics can split effectiveness by source. Cheap to add now, painful later.

### 3.3 — #54 Per-Tool Usage Counter

**Location**: `crates/daemon/src/extractor.rs` around lines 267-295 (existing tool-use detection) and `crates/daemon/src/db/manas.rs:338-345` (`record_tool_use`).

**Change to extractor chunk parsing**: extend the chunk struct (wherever `has_tool_use: bool` lives) to also carry `tool_names: Vec<String>`. Parse from transcript `<tool_use name="X">` pattern. In the existing tool-count increment branch:

```rust
for tool_name in &chunk.tool_names {
    let tool_id = slugify(tool_name);  // or get_tool_id_by_name if lookup is needed
    match record_tool_use(&locked.conn, &tool_id) {
        Ok(true)  => {},
        Ok(false) => tracing::debug!(tool = %tool_id, "tool not in registry — skipping counter"),
        Err(e)    => tracing::warn!(error = %e, tool = %tool_id, "record_tool_use failed"),
    }
}
```

**Tool-ID resolution**: verify during implementation whether `tool.id` is a slug of `tool.name` or requires a separate lookup. If lookup needed, add `get_tool_id_by_name()` helper in `db/manas.rs`.

**Why direct `UPDATE`**: single-row idempotent increment on a table no other writer touches. Writer-channel would be overkill.

**Coordination with Phase 2A-4c1** (see §11.1): the benchmarking team is landing a NEW `session_tool_call` table (row-per-invocation log) via `Request::RecordToolUse`. That surface is complementary — aggregate counter (`tool.use_count`) vs per-call row. Both justified today. Post-Phase 2A-4c2 (hook-driven ingestion), consider deriving `tool.use_count` from `SELECT tool_name, COUNT(*) FROM session_tool_call GROUP BY tool_name` as a future optimization. SP1 does NOT touch 2A-4c1's code or schema.

### 3.4 — #53 Extraction Metrics Recording

**Location**: `crates/daemon/src/writer.rs` (enum + match arm), `crates/daemon/src/extractor.rs` (call site), `crates/daemon/src/db/metrics.rs` (new helper — or extend existing metrics module).

**Changes**:

1. **New `WriteCommand` variant** in `writer.rs`:
```rust
RecordExtraction {
    session_id: String,
    memories_created: usize,
    tokens_in: u64,
    tokens_out: u64,
    cost_cents: u64,
    error: Option<String>,
}
```

2. **New DB helper** `db::metrics::record_extraction(conn, ...)` writes to `metrics` with `metric_type='extraction'`; token/cost/error fields serialized as `meta` JSON.

3. **Match arm** in writer actor — call helper, `let _ =` on error (same pattern as `RecordInjection`).

4. **Extractor call sites** at `extractor.rs` success path (~744) and error path (~793-799) — emit `tx.try_send(WriteCommand::RecordExtraction { ... })` with appropriate fields.

5. **No stats query change**: `ops.rs:1403-1405` already reads `WHERE metric_type='extraction'`. Once we INSERT, `stats` CLI lights up automatically.

**Why writer-channel here**: the extractor runs as a background worker with its own conn; routing metric writes through the writer actor avoids conn contention with the existing `ops::remember()` writes in the same batch.

## 4. Data Flow

Each fix completes a feedback loop from event → record → queryable state. Read-side queries require no change.

### 4.1 — #55 skills populator

```
daemon start
  → DB + migrations ready
  → resolve skills_dir (env → config → default)
  → refresh_skills(&conn, &skills_dir)
      → walk skills/*/SKILL.md
      → upsert_skill() per file (INSERT OR UPDATE ON CONFLICT)
  → log "Indexed N skills"
                            (runtime) forge-next skills-list → SELECT ... FROM skill_registry → non-empty
```

### 4.2 — #45 proactive injection

```
Claude Code hook (PreBashChecked/PostBashCheck/PostEditCheck)
  → HTTP /api handler
  → build_proactive_context() returns Vec<ProactiveInjection>
  → tx.try_send(WriteCommand::RecordInjection { context_type: "proactive", ... })  ← NEW
  → Response returned to hook

writer actor (async)
  → record_injection_with_size() → INSERT INTO context_effectiveness
                            (runtime) forge-next context-stats → counter > 0
```

### 4.3 — #54 per-tool counter

```
transcript arrives at extractor
  → parse chunks, extract tool_names
  → for each tool_name: record_tool_use(&conn, &tool_id)
      → UPDATE tool SET use_count = use_count + 1, last_used = NOW WHERE id = ?
  → (existing) increment session.tool_use_count
                            (runtime) forge-next tools → per-tool use_count > 0
```

### 4.4 — #53 extraction metrics

```
extractor batch completes
  → on Success: tx.try_send(RecordExtraction { memories_created, tokens, cost, error: None })
  → on Error:   tx.try_send(RecordExtraction { memories_created: 0, tokens: 0, cost: 0, error: Some(e) })

writer actor (async)
  → db::metrics::record_extraction() → INSERT INTO metrics (metric_type='extraction', ...)
                            (runtime) forge-next stats → Extractions: N (E errors)
```

## 5. Error Handling

Guiding principle: **counter writes are best-effort**. A dropped tick is acceptable; a broken extractor or hook path is not. Match existing `RecordInjection` semantics everywhere.

### 5.1 — `try_send` failure (writer channel full or dead)

All writer-channel fixes use `let _ = tx.try_send(...)`. Counter tick lost silently; no upstream impact. Identical to the existing `RecordInjection` pattern.

### 5.2 — Skill directory missing or unreadable (#55)

Log `warn!`, continue boot. `skills-list` stays empty; the `skills_registry_empty` doctor probe surfaces it.

### 5.3 — Unknown tool name in transcript (#54)

`record_tool_use` returns `Ok(false)` if 0 rows match. Log `debug!` (not warn — avoid spam from third-party tool names), skip counter.

**Explicit non-goal**: auto-registering unknown tools. Separate bug, needs schema decisions.

### 5.4 — Extraction error path records failure (#53)

Error extractions MUST write a metric row with `error: Some(e)`. Otherwise the 24h error count reads 0 even when extractor is crash-looping — strictly worse than the current silent state.

### 5.5 — Writer command volume

Back-of-envelope: 8 active sessions × ~5 proactive events/min × 1 `RecordInjection` ≈ 40 writes/min + ~1 `RecordExtraction`/min. Per-tool UPDATEs (direct, not via writer) add ~10/min. Well under existing writer throughput.

### 5.6 — Schema migrations

None required. Every table and column is already present:
- `metrics` — exists, receives new `metric_type='extraction'` rows.
- `context_effectiveness` — exists, `RecordInjection` already writes.
- `tool.use_count` — exists, currently all 0.
- `skill_registry` — exists, currently empty.

Pure wiring fixes.

## 6. Testing

Three layers: unit (per fix, TDD red-green-refactor), integration (per loop + composite), doctor probes (fail-red in production).

### 6.1 — Unit tests (TDD, one per fix)

Each commit adds a failing test → implementation until test passes → refactor with `simplify` skill → commit.

| Fix | Test file | Test name | Assertion |
|---|---|---|---|
| #55 | `skills.rs` `#[cfg(test)]` | `test_auto_populate_skill_registry_on_init` | After init with tempdir of 3 fixture skills, `list_skills().count == 3` |
| #45 | `handler.rs` `#[cfg(test)]` | `test_proactive_context_records_injection` | Driving PreBashChecked handler in test harness → `context_effectiveness` row with `context_type='proactive'` |
| #54 | `extractor.rs` `#[cfg(test)]` | `test_extractor_records_per_tool_use_counter` | Sample transcript with 3 tool_use blocks (Bash, Read, Edit) → each tool's `use_count` incremented |
| #53 | `extractor.rs` + `writer.rs` `#[cfg(test)]` | `test_extraction_success_records_metric` + `test_extraction_error_records_metric` | Both paths write `metrics` row with `metric_type='extraction'`; error row carries error in meta |

Unit tests use in-memory SQLite + `tempfile::TempDir` per daemon conventions.

### 6.2 — Integration test `e2e_sp1_dark_loops`

One integration test in `crates/daemon/tests/`. Boots a real daemon against a temp DB + temp skills dir, then drives each loop:

```
1. Boot daemon with skills_directory=tempdir/skills (3 fixtures)
   → assert forge-next skills-list count == 3                          (#55)

2. POST /api SessionStart hook
   → assert context_injections >= 1 with context_type='full_context'   (baseline, already works)

3. POST /api PreBashChecked hook (triggers build_proactive_context)
   → assert context_injections >= 2 with context_type='proactive' row  (#45 NEW)

4. Write sample transcript with 2 tool_use blocks (Bash, Read)
   → bounded poll until extractor cycle completes
   → assert tool.use_count[Bash] >= 1, tool.use_count[Read] >= 1       (#54)
   → assert metrics table has extraction row with memories_created > 0 (#53)

5. forge-next doctor
   → assert all 4 SP1 probes are GREEN
```

This is the V3 §8 "must-have before public claim" composite. No merge without it passing.

### 6.3 — Doctor probes (per V3 §8) — **DEFERRED**

> **STATUS**: Deferred from SP1 to a follow-up PR (tentatively "Doctor-Probes-SP1b"). Rationale: the housekeeping + doctor-observability plan (`docs/superpowers/plans/2026-04-16-housekeeping-and-doctor-observability.md`) proposes a Doctor enhancement (Task 4) that also modifies the Doctor handler/response. Coordinating avoids merge conflict and duplicated work. The probes table below is kept as reference for the follow-up PR.

Planned (for the follow-up PR, not this one):

| Probe | Condition | Severity | Message |
|---|---|---|---|
| `context_pipeline_dark` | `injections_24h == 0 AND sessions_active > 0` | ERROR | "No context injections in 24h — core value loop dark" |
| `extractor_starving` | `extractions_24h == 0 AND messages_24h > 0` | ERROR | "Extractor has messages but produced 0 extractions" |
| `tools_cold` | `SUM(tool.use_count) == 0 AND sessions_active > 0` | WARN | "All tool counters at 0 despite active sessions" |
| `skills_registry_empty` | `skill_registry_count == 0 AND skills_directory_exists` | ERROR | "Skill registry empty but skills dir populated — auto-index may have failed" |

Probes run inside `doctor` invocations, not scheduled.

### 6.4 — Dogfood validation

Before closing the PR:

1. Rebuild: `cargo build --release -p forge-daemon && cargo install --path crates/daemon --force`
2. Restart daemon: `forge-next restart` (graceful drain).
3. Fresh Claude Code session — trigger each flow:
   - Any `PreToolUse(Bash)` call → confirm proactive injection row appears.
   - Any tool invocation → confirm `forge-next tools` shows non-zero.
   - Wait 1 extractor cycle → confirm `forge-next stats` shows non-zero 24h extraction.
   - Confirm `forge-next skills-list` returns ≥15.
4. `forge-next doctor` — probes not yet installed in this PR; verify counters manually via `context-stats`/`stats`/`tools`/`skills-list`. Doctor probes ship in the follow-up PR per §6.3 + §11.2.
5. Paste output into PR description.

Live telemetry is the final gate — passing tests without lit counters on the real daemon is a fail.

## 7. Quality Gates

Applied at each commit and at PR level per user instruction.

### 7.1 — Per-commit gates (TDD discipline)

For each of the 4 fix-commits:

1. **RED**: write the failing unit test. Verify it fails with the expected error (not a compile error — an assertion failure).
2. **GREEN**: minimum implementation to pass the test. Resist scope creep.
3. **`simplify` skill**: run on the changed code. The skill reviews changed code for reuse, quality, and efficiency, then fixes any issues found.
4. **Adversarial Codex review**: `codex exec` with adversarial prompt on the diff. Codex looks for:
   - Missed edge cases
   - Hidden concurrency issues
   - Test coverage gaps
   - "What would break this in production?"
5. **Fix findings**: iterate until Codex agrees the commit is defensible.
6. **Lint gate**: `cargo clippy --workspace -- -W clippy::all -D warnings` returns 0 warnings.
7. **Test gate**: `cargo test --workspace` passes 100%.
8. **Commit**.

### 7.2 — Trailing commit (integration test)

Same cycle: RED → GREEN → simplify → Codex adversarial → lint → test → commit. Doctor probes deferred per §6.3 + §11.2.

### 7.3 — PR-level gates

Before opening the PR:

1. **Full test run**: `cargo test --workspace` green.
2. **Integration test**: `e2e_sp1_dark_loops` green in isolation.
3. **Clippy**: 0 warnings.
4. **Dogfood**: §6.4 checklist complete, output pasted into PR description.
5. **`simplify` on full PR diff**: final pass across all 6 commits.
6. **Adversarial Codex review of PR**: `codex exec` with "adversarial reviewer" prompt over the full diff. Ask Codex:
   - What would make this regress in 3 months?
   - Are the doctor probes actually fail-red or can they silently degrade?
   - Is there a concurrency scenario under load where the writer-channel fixes drop ticks non-deterministically?
   - Is the integration test a genuine e2e or does it mock the critical boundary?
7. **Fix Codex findings** or justify rejection in PR description.

### 7.4 — Merge gate

- All CI green.
- Codex adversarial review "passed" or explicitly accepted rejections.
- Dogfood counters lit on live daemon for ≥1 hour after merge (catch-the-production-regression window).

## 8. Success Criteria

SP1 is "done" when all of the following hold on the dogfood daemon rebuilt from the merged commit:

- [ ] `forge-next context-stats` reports `Injections > 0` with at least one `context_type='proactive'` row (#45)
- [ ] `forge-next stats` reports `Extractions: N (E errors)` with N > 0 for any 24h window after extractor ran (#53)
- [ ] `forge-next tools` reports at least one tool with `used: >0x` (#54)
- [ ] `forge-next skills-list` returns ≥10 skills (#55)
- [ ] `cargo test --workspace` 100% passing
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` 0 warnings
- [ ] PR merged with adversarial Codex review attached

## 9. Out of Scope

Explicit, to prevent scope creep during implementation:

- Rewriting existing writer-channel code or changing the actor boundary.
- Adding new hook endpoints on Claude Code side.
- Auto-registering unknown tool names in the registry (#54 related but separate).
- Tool-usage attribution across time (histogram, burst detection) — a future analytics feature.
- Healing scheduler (#56 — belongs to SP3).
- Multi-tenant org_id plumbing on these surfaces (belongs to SP2).
- Any CLI output format change beyond what's strictly needed to surface new rows.
- **Doctor probes** (§6.3) — deferred to follow-up PR per §11.2.
- **Any modification to `crates/daemon/src/db/ops.rs`** — currently contains uncommitted 2A-4c1 T3 work. SP1 must not touch it. (The read-side `stats` query at `ops.rs:1403-1405` already needs no change for #53; this is safe.)
- **Any modification to 2A-4c1's code or schema** (`session_tool_call` table, `Request::RecordToolUse`, `Request::ListToolCalls`, `ToolCallRow` type, `tool_use_recorded` event).

## 11. Coordination with Parallel Work

Two active daemon workstreams share branches with SP1. This section exists because the day this spec was written, four active commits were in flight on master from a parallel team.

### 11.1 — Phase 2A-4c1: Tool-Use Recording

**Status at spec time (2026-04-20)**:
- T1 committed (`ebeaf01`): `ToolCallRow` shared type in `core::types`.
- T2 committed (`31d13f6`): `session_tool_call` table schema + 3 indexes.
- T3 in progress (uncommitted in working tree of `crates/daemon/src/db/ops.rs`, 302 line diff): `ops::list_tool_calls`.
- 10 more tasks remain in their plan (`docs/superpowers/plans/2026-04-19-forge-tool-use-recording.md`).

**What 2A-4c1 builds**:
- New append-only `session_tool_call` table (row-per-invocation log).
- `Request::RecordToolUse` + `Request::ListToolCalls` protocol endpoints.
- `tool_use_recorded` event emission (non-authoritative broadcast).

**Relationship to SP1 #54**:
- SP1 #54 ticks an **aggregate counter** on the existing `tool` table (`tool.use_count`), which `forge-next tools` reads.
- 2A-4c1 stores a **row-per-invocation log** with rich fields (`agent`, `tool_name`, `tool_args`, `tool_result_summary`, `success`, `user_correction_flag`).
- These are different surfaces solving different jobs today. Both are justified.
- Post-Phase 2A-4c2 (hook-driven ingestion lands), a future ticket could derive `tool.use_count` from `SELECT tool_name, COUNT(*) FROM session_tool_call GROUP BY tool_name` — out of scope for SP1.

**Constraints on SP1**:
- **MUST NOT modify `crates/daemon/src/db/ops.rs`** — 2A-4c1 T3 is live there. SP1 already has no planned ops.rs change; this is hardened as a rule in §9.
- **MUST NOT modify 2A-4c1's schema, Request variants, or Response types.**
- **Handler.rs coordination**: 2A-4c1 T4-T9 will add new handler arms (`handle_record_tool_use`, `handle_list_tool_calls`). SP1 #45 modifies existing return-paths at `handler.rs:1985/2004/2037`. Unlikely to conflict; rebase daily.

### 11.2 — Housekeeping + Doctor Observability (2026-04-16 plan)

**Status at spec time**: `docs/superpowers/plans/2026-04-16-housekeeping-and-doctor-observability.md` is untracked in git. Unclear whether active, paused, or abandoned. Contains 5 tasks including **Task 4: Doctor enhancement** (adds `version` + raw-layer counts + session stats into `Doctor` response).

**Impact on SP1**:
- SP1 §6.3 originally planned to add 4 fail-red doctor probes (`context_pipeline_dark`, `extractor_starving`, `tools_cold`, `skills_registry_empty`).
- Both would modify `handle_doctor` + `ResponseData::Doctor`. Coordination required.

**Resolution**:
- **Doctor probes deferred** from SP1 to a follow-up PR ("Doctor-Probes-SP1b") to open after the housekeeping plan's status is confirmed (active → coordinate; abandoned → proceed solo).
- SP1 still validates counter movement via the integration test in §6.2 — the doctor probes serve ongoing production monitoring, not SP1 acceptance.
- The probes table in §6.3 stays in-spec as a reference for the follow-up PR.

### 11.3 — Branch + rebase policy for SP1

- **Feature branch**: `sp1/dark-loops`. All SP1 commits land here first.
- **No direct master commits** during SP1 implementation.
- **Rebase from `origin/master` before each fix-commit** (daily minimum). Catches 2A-4c1 progress + any other parallel merges.
- **Pre-PR rebase + full test re-run + dogfood validation.**
- **PR target**: `master`, after spec + plan are complete and all fixes green on the branch.
- **Merge conflict policy**: if 2A-4c1 work conflicts with SP1, resolve by preserving both and re-running the integration test. Do not delete 2A-4c1's additions under any circumstance.

## 12. References

- `forge-app-private/product/engineering/daemon-team/SESSION-GAPS.md` — bugs #45, #53, #54, #55 and their dogfood evidence.
- `forge-app-private/product/engineering/daemon-team/DAEMON-STRATEGY-V3-2026-04-20.md` — strategic framing, W4 bet designation, QA plan in §8.
- `forge-app-private/product/cross-team/HANDOFF.md` — Session 17 summary.
- `forge/CLAUDE.md` — daemon conventions (tracing, error handling, test placement).
- Existing code references (verified 2026-04-20):
  - `handler.rs:1985, 2004, 2037` — proactive injection return-points (to modify)
  - `handler.rs:2762-2772` — CompileContext RecordInjection pattern (to copy)
  - `writer.rs:210-226` — RecordInjection match arm (pattern for RecordExtraction)
  - `extractor.rs:267-295` — tool-use detection path (to extend for #54)
  - `extractor.rs:~744, ~793-799` — extraction success/error paths (to instrument for #53)
  - `skills.rs:66-109` — existing `index_skills_directory` populator (to invoke at boot)
  - `skills.rs:111-129` — existing `upsert_skill` helper
  - `db/manas.rs:316-335` — `list_tools` query (read-side, no change)
  - `db/manas.rs:338-345` — existing `record_tool_use` (to call)
  - `ops.rs:1403-1405` — stats query for extractions (read-side, no change)

---

*End of spec. Next: writing-plans skill produces the implementation plan with per-commit TDD tasks.*
