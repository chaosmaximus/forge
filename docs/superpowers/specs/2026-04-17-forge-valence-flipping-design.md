# Forge-Valence-Flipping (Phase 2A-4a) — Detailed Design

**Status:** DRAFT — awaiting two adversarial reviews (Claude + codex CLI).
**Parent:** [docs/benchmarks/forge-identity-master-design.md](../../benchmarks/forge-identity-master-design.md) §5 2A-4a.
**Goal:** Ship the daemon capability to explicitly flip a user preference's valence (positive ↔ negative) while preserving the original as flipped-history.
**Scope:** No auto-flip heuristic; explicit API only. No `ReviveFlipped` / undo. `FlipPreference` validates `memory_type = 'preference'` — non-preference memories cannot be flipped.

---

## 1. Motivation

When a user says "I used to prefer tabs, but the team standardizes on spaces — switch my preference," Forge needs a first-class way to represent the change without losing the history. Storing only the new preference erases the fact that the user changed their mind. Storing both as plain memories (and relying on decay) loses the causal link. Phase 2A-4a introduces a single-source-of-truth mechanism: `Request::FlipPreference` creates a new preference memory with opposite valence and marks the old as `status='superseded' AND valence_flipped_at IS NOT NULL`, linked via the existing `superseded_by` column and a `'supersedes'` edge.

This feature is a building block for Phase 2A-4b (recency-weighted preference decay), 2A-4c2 (Phase 23 behavioral skill inference), and 2A-4d (Forge-Identity bench Dim 4). It ships with unit-test-only coverage — no composite benchmark at this phase.

## 2. Schema changes

**Single ALTER migration** in `crates/daemon/src/db/schema.rs`:

```sql
ALTER TABLE memory ADD COLUMN valence_flipped_at TEXT NULL;
CREATE INDEX IF NOT EXISTS idx_memory_valence_flipped_at ON memory(valence_flipped_at) WHERE valence_flipped_at IS NOT NULL;
```

**Why only one column, not two:** a flipped memory is semantically a superseded memory with valence-change metadata. The existing `superseded_by TEXT NULL` column (already present per exploration at `schema.rs:1203`) already stores the pointer to the replacement memory. Introducing a redundant `flipped_to_id` column would create dual-truth risk (the two could diverge) and a second index. Detection of flipped memories uses the composite predicate `status='superseded' AND valence_flipped_at IS NOT NULL AND memory_type='preference'`.

**No schema version table exists in Forge** — migrations are idempotent `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` patterns in `create_schema()`. Follow that pattern.

**Rollback recipe:**
```sql
-- Rollback 2A-4a (safe if no rows have non-null valence_flipped_at yet):
DROP INDEX IF EXISTS idx_memory_valence_flipped_at;
-- SQLite doesn't support ALTER TABLE DROP COLUMN pre-3.35; fallback is to leave the column.
-- For a full rollback on SQLite < 3.35, recreate memory table without the column.
```
The rollback is a NO-OP on SQLite ≥ 3.35 for the column drop; index drop always works. Acceptance test per master §9 deliverable 8: forward-migrate, insert 1 row with `valence_flipped_at = '2026-04-17T00:00:00Z'`, rollback (drop index only), verify normal queries still work against the residual column.

## 3. `supersede_memory_impl()` helper extraction

**New function in `crates/daemon/src/db/ops.rs`** (first TDD task):

```rust
/// Mark `old_id` as superseded by `new_id`. Creates a 'supersedes' edge from new to old.
/// If `valence_flipped_at` is `Some(ts)`, additionally sets that column — this is the
/// flip-specific codepath used by `FlipPreference`.
///
/// Returns Err if:
/// - The old memory doesn't exist
/// - The old memory is not in 'active' status
/// - The organization_id scope check fails
///
/// This function is the single source of truth for supersede semantics. Both
/// `Request::Supersede` (no flip) and `Request::FlipPreference` (with flip) call it.
pub fn supersede_memory_impl(
    conn: &Connection,
    old_id: &str,
    new_id: &str,
    organization_id: Option<&str>,
    valence_flipped_at: Option<&str>,
) -> Result<(), OpError>
```

SQL emitted by the helper (two statements in a single function; no explicit transaction — caller controls transactions as today):

```sql
-- Statement 1: UPDATE the old memory
-- (a) plain supersede:
UPDATE memory
SET status = 'superseded', superseded_by = ?2
WHERE id = ?1
  AND status = 'active'
  AND (? IS NULL OR organization_id = ?);

-- (b) flip (valence_flipped_at is Some):
UPDATE memory
SET status = 'superseded', superseded_by = ?2, valence_flipped_at = ?3
WHERE id = ?1
  AND status = 'active'
  AND (? IS NULL OR organization_id = ?);
```

If the UPDATE affects zero rows, return `OpError::MemoryNotFoundOrNotActive { id }`.

```sql
-- Statement 2: the 'supersedes' edge (unchanged direction: new --[supersedes]--> old)
INSERT OR IGNORE INTO edge (id, from_id, to_id, edge_type, properties, created_at, valid_from)
VALUES (?, ?, ?, 'supersedes', '{}', ?, ?);
```

**Handler refactor (second TDD task):** `Request::Supersede` at `handler.rs:718-786` is rewritten to:
1. Load `(old_memory, org_id)` for validation/diagnostic
2. Call `ops::supersede_memory_impl(conn, old_id, new_id, org_id, None)`
3. Emit `"memory_superseded"` event as before
4. Return `ResponseData::Superseded { old_id, new_id }` as before

This refactor lands **before** FlipPreference in the implementation plan, so Supersede's existing behavior is preserved by tests throughout the change.

## 4. `Request::FlipPreference`

### 4.1 Variant shape

In `crates/core/src/protocol/request.rs`:

```rust
FlipPreference {
    memory_id: String,
    new_valence: String,       // "positive" | "negative" | "neutral"
    new_intensity: f64,         // 0.0..=1.0
    reason: Option<String>,     // optional human-readable; stored in event payload only
},
```

### 4.2 Response shape

In `crates/core/src/protocol/response.rs`:

```rust
PreferenceFlipped {
    old_id: String,
    new_id: String,
    new_valence: String,
    new_intensity: f64,
    flipped_at: String,  // ISO UTC timestamp
},
```

### 4.3 Handler algorithm

Implemented in `crates/daemon/src/server/handler.rs`:

```text
1. Parse request: memory_id, new_valence, new_intensity, reason.
2. Validate:
   - new_valence must be "positive" | "negative" | "neutral" → else Response::Error.
   - new_intensity must be finite and in [0.0, 1.0] → else Response::Error.
3. Load old memory from DB:
   - If not found → Response::Error("memory_id not found").
   - If memory_type != Preference → Response::Error("memory_type must be preference for flip").
   - If status != Active → Response::Error("memory already superseded").
4. Capture now = forge_core::time::now_iso().
5. Construct new memory:
   - id = ULID
   - memory_type = Preference
   - title, content, tags, project, organization_id, session_id = clone from old
   - embedding = clone from old (avoid re-embedding identical content)
   - alternatives, participants = clone from old
   - confidence = old.confidence  [D2 resolution: inherit]
   - valence = new_valence
   - intensity = new_intensity
   - created_at = now
   - accessed_at = now
   - status = MemoryStatus::Active
   - (reaffirmed_at is not set in 2A-4a; that column doesn't exist yet — it ships in 2A-4b)
6. Insert new memory via ops::store_memory(conn, &new_memory).
7. Call ops::supersede_memory_impl(conn, &old.id, &new.id, old.organization_id.as_deref(),
                                     Some(&now)).
   - If Err, roll back step 6 by ops::forget_memory(conn, &new.id).
     (No explicit transaction wrapper; the rollback is best-effort.)
8. Emit event:
   events::emit(&tx, "preference_flipped", json!({
       "old_id": &old.id,
       "new_id": &new.id,
       "new_valence": &new_valence,
       "new_intensity": new_intensity,
       "reason": reason.as_deref().unwrap_or(""),
       "flipped_at": &now
   }));
9. Return Response::Ok { data: PreferenceFlipped { ... } }.
```

**On the lack of an explicit transaction:** the existing Supersede handler at `handler.rs:718-786` does not wrap its UPDATE + edge insert in an explicit transaction either. Consistency follows from SQLite WAL + the operations being append-only or idempotent. 2A-4a preserves this pattern. If a future phase introduces explicit transactions, 2A-4a migrations are compatible.

### 4.4 Rationale for D2 (confidence inherit)

User flipping a preference does NOT mean they're less sure of the new position — they're just as confident in the new valence as they were in the old. Inheriting confidence preserves this semantic. A fresh user-stated preference goes through `Remember` at default 0.9 confidence. An automatic valence flip from new evidence is a separate path (Phase 9a-driven, deferred). `FlipPreference` is the user-initiated path where confidence should carry over unchanged.

## 5. `Request::ListFlipped`

### 5.1 Variant + response

```rust
// Request
ListFlipped {
    agent: Option<String>,        // currently informational; all memories accessible to one agent
    limit: Option<usize>,          // default 20, clamped to [1, 100]
},

// Response
FlippedList {
    items: Vec<FlippedMemory>,
},

pub struct FlippedMemory {
    pub old: Memory,                 // the original preference (now superseded)
    pub flipped_to_id: String,       // == old.superseded_by (de-aliased for convenience)
    pub flipped_at: String,          // ISO UTC == old.valence_flipped_at
}
```

Note: `FlippedMemory.flipped_to_id` is derived from `old.superseded_by` in the handler, not stored separately. The field is informational for consumers; the underlying source is `old.superseded_by`.

### 5.2 Handler

```rust
// Pseudo-SQL (actual code uses rusqlite::params!)
SELECT id, memory_type, title, content, confidence, status, project, tags,
       created_at, accessed_at, valence, intensity, /* full memory columns */
       valence_flipped_at, superseded_by, organization_id
FROM memory
WHERE valence_flipped_at IS NOT NULL
  AND memory_type = 'preference'
  AND (?1 IS NULL OR organization_id = ?1)
ORDER BY valence_flipped_at DESC
LIMIT ?2;
```

The `agent` parameter is informational and not used for filtering in this phase (no per-agent memory scope exists in the current schema; the `organization_id` scope is the actual access boundary).

## 6. `Request::Recall` `include_flipped` extension

In `crates/core/src/protocol/request.rs`, extend the `Recall` variant with:

```rust
Recall {
    // ... existing fields ...
    #[serde(default)]
    include_flipped: Option<bool>,  // None or Some(false) = exclude; Some(true) = include
},
```

**`#[serde(default)]`** ensures backward compatibility: existing JSON payloads without the field still deserialize successfully (Option<bool>::None).

**Handler change:** In `crates/daemon/src/server/handler.rs:420-690` (the Recall handler span), after `hybrid_recall()` returns results at `recall.rs:313`, the post-processing step filters on status. Today:

```rust
results.retain(|r| r.memory.status == MemoryStatus::Active);
```

becomes (when `include_flipped` is `Some(true)`):

```rust
results.retain(|r| {
    match r.memory.status {
        MemoryStatus::Active => true,
        MemoryStatus::Superseded => {
            include_flipped
                && r.memory.valence_flipped_at.is_some()  // new field on Memory struct
                && r.memory.memory_type == MemoryType::Preference
        }
        _ => false,
    }
});
```

When `include_flipped` is `None` or `Some(false)`, the filter is unchanged (only active memories surface). This preserves backward compatibility for every current caller.

**`Memory::valence_flipped_at` field:** A new nullable field on the `Memory` struct in `crates/core/src/types/memory.rs`. Type: `pub valence_flipped_at: Option<String>`. Serde default empty-null. Hydrated from the new DB column in `fetch_memory_by_id()` and similar accessors.

## 7. CompileContext `<preferences-flipped>` XML section

In `crates/daemon/src/recall.rs` `compile_dynamic_suffix()`, after the existing dynamic sections (lessons, skills, etc.), add:

```rust
// Preferences-flipped (greenfield; budget-accounted)
if !excluded_layers.iter().any(|l| l == "preferences_flipped") {
    let flipped = ops::list_flipped(conn, organization_id.as_deref(), 5).unwrap_or_default();
    if !flipped.is_empty() {  // omit entirely when empty
        let mut pf_xml = String::from("<preferences-flipped>");
        for item in &flipped {
            let entry = format!(
                "\n  <flip at=\"{}\" old_valence=\"{}\" new_valence_query_hint=\"superseded_by={}\">{}</flip>",
                xml_escape(&item.old.valence_flipped_at.as_deref().unwrap_or("")),
                xml_escape(&item.old.valence),
                xml_escape(&item.flipped_to_id),
                xml_escape(&item.old.title),
            );
            if used + pf_xml.len() + entry.len() < budget {
                pf_xml.push_str(&entry);
            } else {
                break;  // budget exceeded; stop adding flips but still close the tag
            }
        }
        pf_xml.push_str("\n</preferences-flipped>\n");
        used += pf_xml.len();
        xml.push_str(&pf_xml);
    }
    // else: omit entirely (matches the "omit empty" emit policy locked in master §12 D4 rationale)
}
```

**Budget accounting:** consistent with existing `<lessons>` and `<skills>` sections — each entry is checked against the remaining budget before append; if an entry wouldn't fit, the section stops (leaving previously-added entries intact) and closes cleanly. Maximum section size is bounded by `limit=5 × per-entry byte count (~200 bytes) ≈ 1KB`, a small fraction of the typical 8–16KB context budget.

**XML escaping:** reuse the existing `xml_escape()` helper (verified to exist at `recall.rs` — if not under that name, the detailed implementation task adds it as a local helper or imports from `forge_core::util` if available). Applied to `valence_flipped_at`, `old.valence`, `flipped_to_id`, and `old.title`.

**`excluded_layers` key:** `"preferences_flipped"` (snake_case matching existing "decisions", "lessons", "skills" pattern).

## 8. Event emission

Precedent at `crates/daemon/src/server/handler.rs:767-773` emits `"memory_superseded"` via `events::emit()`. 2A-4a adds a new event name `"preference_flipped"` with payload:

```json
{
    "old_id": "01J...",
    "new_id": "01J...",
    "new_valence": "negative",
    "new_intensity": 0.8,
    "reason": "team switched to spaces",
    "flipped_at": "2026-04-17T14:22:00Z"
}
```

**Emission site:** directly after the successful supersede_memory_impl() call in the FlipPreference handler, before the Response::Ok return.

**No new `Request::Notification`-style structured variant** — consistent with how `memory_superseded` is handled today. Subscribers (HUD, CLI) opt in via the broadcast event channel.

**`Request::Supersede` also continues to emit** `"memory_superseded"` (unchanged).

## 9. Validation and error shape

All errors returned as `Response::Error { message: String }` with stable message strings for test assertions:

| Condition | Message |
|-----------|---------|
| `memory_id` doesn't exist | `"memory_id not found: {id}"` |
| Memory not a preference | `"memory_type must be preference for flip (got: {actual_type})"` |
| Memory already superseded | `"memory already superseded (id: {id})"` |
| Invalid valence | `"new_valence must be positive | negative | neutral (got: {value})"` |
| Invalid intensity | `"new_intensity must be finite in [0.0, 1.0] (got: {value})"` |

Same `Response::Error` pattern for ListFlipped (only storage errors; no user-input validation beyond clamp).

## 10. Contract tests

In `crates/core/src/protocol/contract_tests.rs`, add to the parameterized test vector:

```rust
(
    "flip_preference",
    Request::FlipPreference {
        memory_id: "01JABCDEF".into(),
        new_valence: "negative".into(),
        new_intensity: 0.8,
        reason: Some("team switched to spaces".into()),
    },
),
(
    "list_flipped",
    Request::ListFlipped {
        agent: Some("claude-code".into()),
        limit: Some(10),
    },
),
```

**Recall extension:** the existing Recall test case(s) are extended to cover the new field. Use two cases: one with `include_flipped: None` (backward compat), one with `include_flipped: Some(true)`. Ensures serde round-trip works both ways.

**`ResponseData::PreferenceFlipped` and `ResponseData::FlippedList`:** similarly added to the response contract-test vector.

## 11. Test plan (TDD sequence)

Implementation follows strict TDD per master methodology. Test-first, watch RED, minimal implementation, watch GREEN. Tasks ordered so each TDD cycle produces working, committed code:

**T1. supersede_memory_impl helper (no flip)**
- RED: `test_supersede_memory_impl_marks_superseded_creates_edge` — covers the existing Supersede behavior via the new helper
- GREEN: extract helper, wire Supersede handler to call it
- Verification: all existing Supersede handler tests still pass (regression-guard)

**T2. supersede_memory_impl helper (with flip)**
- RED: `test_supersede_memory_impl_with_flip_sets_valence_flipped_at`
- GREEN: add the `valence_flipped_at` optional parameter and SQL branch

**T3. Schema migration + index**
- RED: `test_memory_schema_has_valence_flipped_at_column` (introspects SQLite schema)
- GREEN: add ALTER TABLE statement to `create_schema()` path

**T4. Memory struct + serde**
- RED: `test_memory_struct_has_valence_flipped_at_field` (serialize/deserialize round-trip)
- GREEN: add `pub valence_flipped_at: Option<String>` to `Memory`

**T5. Request/Response variants (contract)**
- RED: `test_unit_variants_method_names` fails — `FlipPreference` not found
- GREEN: add `Request::FlipPreference`, `Request::ListFlipped`, extend `Request::Recall`, add `ResponseData::PreferenceFlipped`, `ResponseData::FlippedList`

**T6. Handler: FlipPreference happy path**
- RED: `test_flip_preference_creates_new_memory_with_opposite_valence`
- GREEN: implement handler; store new memory; call supersede helper
- Verify: test also asserts old memory has `valence_flipped_at` set, `status == Superseded`, `superseded_by == new.id`

**T7. Handler: FlipPreference validation**
- RED (parameterized): 5 tests, one per error condition from §9
- GREEN: implement validation branches

**T8. Handler: FlipPreference event emission**
- RED: `test_flip_preference_emits_preference_flipped_event`
- GREEN: wire `events::emit()` call

**T9. Handler: ListFlipped**
- RED: `test_list_flipped_returns_only_flipped_memories_ordered_desc`
- GREEN: implement `ops::list_flipped()` and handler arm

**T10. Recall: include_flipped**
- RED (two tests): `test_recall_default_excludes_flipped_prefs`, `test_recall_include_flipped_surfaces_flipped_prefs`
- GREEN: extend status filter in hybrid_recall post-processing

**T11. CompileContext: preferences-flipped section**
- RED (three tests): empty-omitted, single-populated-renders, budget-exceeded-truncates
- GREEN: add section in compile_dynamic_suffix

**T12. Integration harness + cross-crate clippy/test**
- Run `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`
- Must be 0 failures, 0 warnings before merge
- Add one integration test at `crates/daemon/tests/flip_preference_flow.rs` that exercises: Remember(preference) → FlipPreference → ListFlipped → Recall with include_flipped → CompileContext containing `<preferences-flipped>`.

**T13. Rollback recipe test**
- RED: test that runs forward migration, inserts a row with `valence_flipped_at`, runs rollback SQL, verifies remaining queries still work

**T14. Dogfood**
- Rebuild `forge-daemon` binary
- Manually: `forge remember --type preference --valence positive "prefer tabs"` then `forge flip-preference <id> --new-valence negative --reason "team convention"` then `forge list-flipped`
- Verify HUD/doctor report sane state

## 12. Files touched (final list)

| File | Change |
|------|--------|
| `crates/core/src/protocol/request.rs` | Add `FlipPreference`, `ListFlipped` variants; extend `Recall` with `include_flipped` field |
| `crates/core/src/protocol/response.rs` | Add `PreferenceFlipped`, `FlippedList`, `FlippedMemory` types |
| `crates/core/src/protocol/contract_tests.rs` | Add 3 new parameterized entries; extend Recall test |
| `crates/core/src/types/memory.rs` | Add `valence_flipped_at: Option<String>` field to `Memory` struct |
| `crates/daemon/src/db/schema.rs` | Add ALTER migration + index for `valence_flipped_at` |
| `crates/daemon/src/db/ops.rs` | Add `supersede_memory_impl()`, `list_flipped()`; extend memory-fetching accessors to read `valence_flipped_at` |
| `crates/daemon/src/server/handler.rs` | Refactor `Request::Supersede` to call helper; add `Request::FlipPreference`, `Request::ListFlipped` handlers; extend Recall handler status filter |
| `crates/daemon/src/recall.rs` | Add `<preferences-flipped>` section in `compile_dynamic_suffix` |
| `crates/daemon/tests/flip_preference_flow.rs` | New integration test (T12) |
| `crates/cli/src/main.rs` + related CLI files | Add `forge flip-preference` and `forge list-flipped` subcommands (for dogfood; scoped narrowly) |

## 13. Out of scope (explicit non-goals)

- **Auto-flip heuristic** from Phase 9a contradiction diagnostics — master §8 non-goal
- **`ReviveFlipped` / undo** — master §8 non-goal
- **Flipping non-preference memory types** — master §8 non-goal; handler validates and rejects
- **Multi-agent flip broadcasting** — master §8 non-goal
- **`ReaffirmPreference` / recency-weighted decay** — that's 2A-4b scope
- **`<preferences>` dynamic section** — that's also 2A-4b scope (2A-4a only adds `<preferences-flipped>`)
- **Phase 9a changes** — remains diagnostic-only in 2A-4a
- **LLM-driven reason parsing** — `reason` field stored as opaque string in event payload

## 14. Known risks

- **R1 — supersede_memory_impl call-site drift.** The existing `consolidator.rs:1499-1510` has an inline copy of the supersede SQL. 2A-4a does NOT refactor the consolidator's copy (out of scope — 2A-4a only touches the handler). Future phase should migrate consolidator to the helper too. Risk: behavior drift between handler path and consolidator path. Mitigation: add `TODO: migrate to supersede_memory_impl() in future phase` comment at the consolidator's inline SQL.

- **R2 — Rollback incompleteness on SQLite < 3.35.** The ALTER ADD COLUMN can't be fully reversed without a table rebuild on old SQLite. Mitigation: document the limitation; test the rollback on SQLite 3.35+ (the project's declared minimum is 3.35 per master deliverable §9.8 rationale — verify this in 2A-4a design-gate).

- **R3 — `valence_flipped_at` field addition to `Memory` struct changes serialization size.** Every Memory-serializing path gets one more JSON field. Typical increase per-memory: ~25 bytes when null (`"valence_flipped_at":null`). Mitigation: negligible for typical corpus sizes (125 memories × 25 bytes ≈ 3KB); use `#[serde(skip_serializing_if = "Option::is_none")]` to omit when null and reduce the overhead to zero for active memories.

- **R4 — Event subscriber backlog.** If the broadcast channel has slow subscribers, emitting a new event type adds pressure. Mitigation: event emission is fire-and-forget via `broadcast::Sender::send()` which is non-blocking. No change from existing `"memory_superseded"` pattern.

- **R5 — Race condition: concurrent FlipPreference on the same memory.** Two simultaneous flip requests could both see the memory in 'active' status, both try to supersede. SQLite WAL serializes writes; the second update fails (affects 0 rows due to `status = 'active'` predicate) and returns `Err`. Mitigation: first writer wins; second returns `"memory already superseded"`. Test: `test_concurrent_flip_preference_only_first_succeeds` in T7.

## 15. Open decisions

- **OD1 (resolved at master-level D2):** confidence carryover on flip → inherit old.confidence (Option a).
- **OD2 (resolved at master-level D1 simplified):** schema drops `flipped_to_id` column; reuse `superseded_by`.
- **OD3 (deferred to 2A-4d):** how bench tests FlipPreference's effect on recall (include_flipped path) — that integrates with Dim 4 scoring, out of scope here.

## 16. Deliverables (gate checklist)

- [ ] This design doc with two adversarial reviews addressed
- [ ] Implementation plan at `docs/superpowers/plans/2026-04-17-forge-valence-flipping.md`
- [ ] Schema migration rollback recipe tested on fresh DB
- [ ] All T1–T14 tests passing (GREEN)
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` clean
- [ ] `cargo fmt --all` clean
- [ ] Parity test pattern established (documented in-line with first bench-only hook, though 2A-4a itself has none)
- [ ] Dogfood run: flip a real preference on the live daemon; verify CompileContext renders `<preferences-flipped>`
- [ ] Memory handoff file `project_phase_2a4a_complete_YYYY_MM_DD.md` created
- [ ] `docs/benchmarks/forge-identity-master-design.md` §5 2A-4a lock confirmed (no drift from detailed spec)

---

## Changelog

- **v1 (2026-04-17):** Initial detailed design.
