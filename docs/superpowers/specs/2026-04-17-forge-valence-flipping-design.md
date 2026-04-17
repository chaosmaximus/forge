# Forge-Valence-Flipping (Phase 2A-4a) — Detailed Design (v2)

**Status:** DRAFT v2 — addresses 5 CRITICAL + 6 HIGH from first-pass adversarial reviews (Claude + codex). Major design changes: bypass `ops::remember()` dedup via direct INSERT; wrap all flip operations in explicit `conn.transaction()`; thread `include_flipped` through `hybrid_recall()` signature; add `superseded_by` + `valence_flipped_at` to `Memory` struct with fetch-accessor updates; synthesize new-memory `content` instead of cloning; add cross-org scope guard; align `now_iso()` format (no T, no Z); drop CLI subcommand scope from this phase.
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
ALTER TABLE memory ADD COLUMN valence_flipped_at TEXT;
CREATE INDEX IF NOT EXISTS idx_memory_valence_flipped_at ON memory(valence_flipped_at) WHERE valence_flipped_at IS NOT NULL;
```

**Why only one column, not two:** a flipped memory is semantically a superseded memory with valence-change metadata. The existing `superseded_by TEXT` column (already present per exploration at `schema.rs:1203`) already stores the pointer to the replacement memory. Introducing a redundant `flipped_to_id` column would create dual-truth risk (the two could diverge) and a second index. Detection of flipped memories uses the composite predicate `status='superseded' AND valence_flipped_at IS NOT NULL AND memory_type='preference'`.

**Migration idempotency** follows the existing codebase pattern: SQLite does NOT support `ALTER TABLE ADD COLUMN IF NOT EXISTS`. Forge's existing migrations (see `schema.rs:1203` for `superseded_by`) use `let _ = conn.execute("ALTER TABLE memory ADD COLUMN ...", []);` — swallowing the "duplicate column name" error via discard. 2A-4a follows the same pattern. The `CREATE INDEX IF NOT EXISTS ... WHERE ...` partial index IS supported (SQLite 3.8+; bundled SQLite 3.46+ via rusqlite 0.32).

**Partial index note:** this is the first partial index in the codebase (grep confirmed 0 matches for `CREATE INDEX ... WHERE` in `schema.rs`). Acceptable new pattern, but 2A-4d's implementation plan should add a note when using the same pattern.

**Rollback recipe** (SQLite 3.46+ bundled via rusqlite 0.32 supports `ALTER TABLE DROP COLUMN`):
```sql
DROP INDEX IF EXISTS idx_memory_valence_flipped_at;
ALTER TABLE memory DROP COLUMN valence_flipped_at;
```
Acceptance test per master §9 deliverable 8: forward-migrate, insert 1 row with `valence_flipped_at = '2026-04-17 00:00:00'` (SQLite format — no T, no Z), rollback, verify normal queries still work against the residual column-less schema.

## 3. `supersede_memory_impl()` helper extraction

**New function in `crates/daemon/src/db/ops.rs`** (first TDD task):

```rust
/// Mark `old_id` as superseded by `new_id`. Creates a 'supersedes' edge from new to old.
/// If `valence_flipped_at` is `Some(ts)`, additionally sets that column — this is the
/// flip-specific codepath used by `FlipPreference`.
///
/// Caller is responsible for transaction scope. Both statements (UPDATE memory, INSERT edge)
/// execute as direct rusqlite calls; wrap them in `conn.transaction()` at the caller level
/// for atomicity (see §4.3 for the FlipPreference handler's transaction usage).
///
/// Error distinction: returns `OpError::OldMemoryNotActive { id }` if no row matched the
/// UPDATE (either id doesn't exist, status != 'active', or organization_id scope failed);
/// returns `OpError::DbError` for any rusqlite failure. The composite error preserves the
/// existing handler's behavior of rejecting the Supersede when the old memory isn't active.
pub fn supersede_memory_impl(
    conn: &Connection,
    old_id: &str,
    new_id: &str,
    organization_id: Option<&str>,
    valence_flipped_at: Option<&str>,
) -> Result<(), OpError>
```

**Implementation (Rust, all-numbered placeholders):**

```rust
pub fn supersede_memory_impl(
    conn: &Connection,
    old_id: &str,
    new_id: &str,
    organization_id: Option<&str>,
    valence_flipped_at: Option<&str>,
) -> Result<(), OpError> {
    let now = forge_core::time::now_iso();
    let org = organization_id.unwrap_or("default");
    
    // Statement 1: UPDATE the old memory — branched by flip vs plain supersede.
    let rows_updated = if let Some(flip_ts) = valence_flipped_at {
        conn.execute(
            "UPDATE memory
                SET status = 'superseded',
                    superseded_by = ?1,
                    valence_flipped_at = ?2
              WHERE id = ?3
                AND status = 'active'
                AND COALESCE(organization_id, 'default') = ?4",
            rusqlite::params![new_id, flip_ts, old_id, org],
        )?
    } else {
        conn.execute(
            "UPDATE memory
                SET status = 'superseded',
                    superseded_by = ?1
              WHERE id = ?2
                AND status = 'active'
                AND COALESCE(organization_id, 'default') = ?3",
            rusqlite::params![new_id, old_id, org],
        )?
    };
    
    if rows_updated == 0 {
        return Err(OpError::OldMemoryNotActive { id: old_id.to_string() });
    }
    
    // Statement 2: the 'supersedes' edge (new --[supersedes]--> old).
    let edge_id = ulid::Ulid::new().to_string();
    conn.execute(
        "INSERT OR IGNORE INTO edge
             (id, from_id, to_id, edge_type, properties, created_at, valid_from)
         VALUES (?1, ?2, ?3, 'supersedes', '{}', ?4, ?4)",
        rusqlite::params![edge_id, new_id, old_id, now],
    )?;
    
    Ok(())
}
```

**All SQL placeholders are numbered** (`?1, ?2, ...`) matching rusqlite's `params!` macro convention. No bare `?` placeholders (v1 spec mixed these, which would fail to parse).

**`OpError` variants added** in the same file:
```rust
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("old memory not found or not active: {id}")]
    OldMemoryNotActive { id: String },
    #[error(transparent)]
    DbError(#[from] rusqlite::Error),
    // ... existing variants preserved
}
```

**Handler refactor (second TDD task):** `Request::Supersede` at `handler.rs:718-786` is rewritten to:
1. Load `old_memory` via `fetch_memory_by_id(conn, old_id)`; if `None` → `Response::Error("old memory not found or not active: {old_id}")` (matches the helper's OldMemoryNotActive error via explicit mapping).
2. Load `new_memory` via `fetch_memory_by_id(conn, new_id)`; if `None` → `Response::Error("new memory not found: {new_id}")`.
3. Derive `org_id` from `old_memory.organization_id` (preserves existing cross-consistency check that new_memory.organization_id matches).
4. Begin transaction: `let tx = conn.transaction()?;`
5. Call `ops::supersede_memory_impl(&tx, old_id, new_id, org_id.as_deref(), None)`.
6. `tx.commit()?;`
7. Emit `"memory_superseded"` event via `crate::events::emit(&state.events, "memory_superseded", json!({...}));` (note: `state.events`, not `&tx` as v1 incorrectly showed).
8. Return `ResponseData::Superseded { old_id, new_id }` as before.

The current-handler distinction between "old not found" and "new not found" error messages is PRESERVED by separate `fetch_memory_by_id` checks in steps 1 and 2 BEFORE calling the helper. The helper's single error type (`OldMemoryNotActive`) is only hit for race-condition losers (another session flipped/superseded between step 1 and step 4). That race case returns `"old memory no longer active (concurrent change): {old_id}"` from handler.

This refactor lands **before** FlipPreference in the implementation plan. Every existing Supersede handler test must still pass after T1 and T2.

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
    flipped_at: String,  // "YYYY-MM-DD HH:MM:SS" (SQLite format, no T, no Z — matches forge_core::time::now_iso())
},
```

### 4.3 Handler algorithm (transactional)

Implemented in `crates/daemon/src/server/handler.rs`:

```text
 1. Parse request: memory_id, new_valence, new_intensity, reason.
 2. Validate inputs:
    - new_valence must be "positive" | "negative" | "neutral" → else Response::Error.
    - new_intensity must be finite AND in [0.0, 1.0] → else Response::Error.
 3. Load old memory (read-only, before tx): fetch_memory_by_id(&conn, memory_id).
    - If not found → Response::Error("memory_id not found: {memory_id}").
    - If memory_type != Preference → Response::Error("memory_type must be preference
      for flip (got: {type})").
    - If status != Active → Response::Error("memory already superseded (id: {memory_id})").
 4. Cross-org scope guard:
    - Derive caller_org via state.session_registry or the request-scoped org context.
      (If no session context exists — e.g., bench harness direct call — fall back to
      old.organization_id; this matches existing Supersede's self-consistency check.)
    - Assert caller_org matches old.organization_id → else Response::Error with
      "cross-org flip denied". Also assert new_valence != old.valence (noop-flip
      detection; return same error as already-superseded to avoid no-op creation).
 5. Capture now = forge_core::time::now_iso(). Format: "YYYY-MM-DD HH:MM:SS" (no T, no Z).
 6. Synthesize NEW memory fields (do NOT blindly clone):
    - id = ulid::Ulid::new().to_string()
    - memory_type = MemoryType::Preference
    - title = old.title  (topic unchanged)
    - content = format!(
        "[flipped from {old_valence} to {new_valence} at {now}]{reason_suffix}: {old_content}",
        old_valence = old.valence,
        new_valence = new_valence,
        now = now,
        reason_suffix = reason.as_ref().map(|r| format!(" (reason: {r})")).unwrap_or_default(),
        old_content = old.content,
      )
      // Resulting content e.g.: "[flipped from positive to negative at 2026-04-17 14:22:00]
      // (reason: team switched to spaces): prefer tabs for readability"
      // The leading annotation marks the flip event explicitly — LLM readers see context;
      // BM25 ranking distinguishes old from new because content prefix differs.
    - tags = old.tags.clone()  (topic tags preserved)
    - project = old.project.clone()
    - organization_id = old.organization_id.clone()
    - session_id = session_id_from_request_context().unwrap_or(old.session_id.clone())
      // new memory attributes to the flipping session, not the original
    - embedding = None (re-embedded by indexer worker after insert; the new content is
      semantically distinct from old.content because of the flip annotation)
    - alternatives = vec![]  (reset — old's alternatives don't apply to the flipped preference)
    - participants = vec![]  (reset — same reasoning)
    - confidence = old.confidence.max(0.5).min(1.0)  (D2 revision: inherit, but floor
      at 0.5 and cap at 1.0; rationale per §4.4)
    - valence = new_valence.clone()
    - intensity = new_intensity
    - created_at = now.clone()
    - accessed_at = now.clone()
    - status = MemoryStatus::Active
    - access_count = 0 (fresh)
    - activation_level = 0.0 (fresh)
    - hlc_timestamp = hlc::tick()  (existing pattern in ops::remember)
    - node_id = state.node_id.clone()
 7. Atomic transaction wraps steps 7a-7c:
      let tx = state.conn.transaction()?;
 7a.  ops::store_memory_raw(&tx, &new_memory)?;
      // NOTE: uses store_memory_raw (bypasses dedup) to avoid B-C1 (remember()'s dedup
      // would match old_memory and UPSERT it instead of creating a new row). If
      // store_memory_raw doesn't exist yet, the first TDD task adds it; see §12.
 7b.  ops::supersede_memory_impl(&tx, &old.id, &new_memory.id,
                                  old.organization_id.as_deref(), Some(&now))?;
 7c.  tx.commit()?;
      // If any step fails, the transaction auto-rolls back; no manual forget() needed.
      // Failure → Response::Error("flip transaction failed: {error}").
 8. Emit event AFTER commit:
      crate::events::emit(&state.events, "preference_flipped", json!({
          "old_id": old.id,
          "new_id": new_memory.id,
          "new_valence": new_valence,
          "new_intensity": new_intensity,
          "reason": reason.as_deref().unwrap_or(""),
          "flipped_at": now,
      }));
 9. Return Response::Ok {
      data: ResponseData::PreferenceFlipped {
          old_id: old.id,
          new_id: new_memory.id,
          new_valence,
          new_intensity,
          flipped_at: now,
      }
    };
```

**On atomicity via transaction:** v1 spec followed the existing Supersede handler's non-transactional pattern. v2 upgrades to an explicit `conn.transaction()` wrapper because FlipPreference has a MULTI-statement sequence (INSERT new + UPDATE old + INSERT edge) that MUST all succeed together. Supersede's existing non-transactional behavior is still acceptable for its 2-statement case (UPDATE + INSERT edge) — T2's refactor keeps the atomicity choice at the handler level, not the helper level. Handler can choose whether to wrap in a transaction; the helper works correctly under either.

**The `store_memory_raw()` helper:** if it doesn't already exist, the first TDD task (T0) adds it as a no-dedup variant of `ops::remember()`:

```rust
pub fn store_memory_raw(conn: &Connection, memory: &Memory) -> Result<(), OpError> {
    // Same INSERT as remember()'s non-dedup path, but ALWAYS takes the INSERT branch.
    // Used by FlipPreference where dedup against the to-be-superseded old memory
    // would produce incorrect behavior.
    // ... (exact SQL mirrors remember()'s INSERT)
}
```

### 4.4 Rationale for D2 (confidence inherit with 0.5 floor)

User flipping a preference does NOT mean they're less sure of the new position — they're just as confident in the new valence as they were in the old. Inheriting confidence preserves this semantic.

**Floor at 0.5 (v2 revision):** the reviewer B-H5 correctly noted that once 2A-4b ships (recency-weighted preference decay), a 180-day-old preference with decayed `confidence = 0.3` would pass that decayed value onto the new flipped memory. The new memory's `created_at` is now (fresh), but `confidence = 0.3` makes it rank like a stale pref. User experience: "I just flipped this preference; why does it rank weakly?" Floor of 0.5 is the middle ground between (a) inheriting raw old confidence (could be 0.01) and (b) resetting to default 0.9 (ignores user's prior stated confidence). A fresh user-stated preference goes through `Remember` at default 0.9; an automatic valence flip from new evidence is Phase 9a-driven (deferred); `FlipPreference` as user-initiated lands at ≥ 0.5 to signal "just stated but with some prior context."

Formula: `new.confidence = old.confidence.max(0.5).min(1.0)`.

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

### 6.1 Architectural decision: thread `include_flipped` through `hybrid_recall()` signature

The v1 spec claimed a post-processing retain filter at `recall.rs:313` would suffice. Review B-C4 (both reviewers) correctly flagged this is wrong: BM25 hard-filters `m.status = 'active'` at `ops.rs:266-303`; flipped memories (status='superseded') never enter the candidate list in the first place. Post-retain is a no-op because there's nothing to retain.

**Correct architecture:** add a `include_flipped: bool` parameter to `hybrid_recall()` and propagate into the BM25 + vector candidate queries. The BM25 query (`ops::search_memories_bm25`) gets a predicate change:

```sql
-- When include_flipped = false (current behavior):
WHERE m.status = 'active'
  AND m.memory_type = COALESCE(?2, m.memory_type)
  -- ... other existing filters bound as ?3, ?4 ...

-- When include_flipped = true:
WHERE (
    m.status = 'active'
    OR (m.status = 'superseded' AND m.valence_flipped_at IS NOT NULL AND m.memory_type = 'preference')
  )
  AND m.memory_type = COALESCE(?2, m.memory_type)
  -- ... other existing filters bound as ?3, ?4 ...
```

(Placeholder numbering illustrative; the implementation plan locks the exact bindings in T10. Pattern matches the numbered-placeholder rule from §3.)

The vector search (`vec::search_vectors` via `memory_vec`) has no status column, but its results get mapped back to `memory` via JOIN; the post-RRF step filters by `status IN ('active', 'superseded_flipped_preference_pseudo-status')` per the same rule. Details are in the 2A-4a implementation plan's T10 tasks.

**Call-site blast radius (explicit):** `hybrid_recall()` is called from:
- `handler.rs:480` — `Request::Recall { layer: None }` path → pass `include_flipped.unwrap_or(false)`
- `handler.rs:633` — `Request::Recall { layer: Some("experience") }` → pass `include_flipped.unwrap_or(false)`
- `handler.rs:2979` — similar location → pass `include_flipped.unwrap_or(false)`
- Bench/test call sites → default `false`

All call sites are updated in T10. Integration tests verify that the existing callers' behavior is unchanged when `include_flipped = false`.

### 6.2 `Memory` struct field additions

Two new fields on `Memory` struct in `crates/core/src/types/memory.rs`:

```rust
pub struct Memory {
    // ... existing fields ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valence_flipped_at: Option<String>,
}
```

**`skip_serializing_if = "Option::is_none"`** ensures older CLI deserializers don't see unexpected null fields in payloads where the old memory wasn't flipped/superseded.

**`fetch_memory_by_id()` and related fetch helpers in `db/ops.rs` MUST be updated** to read both columns from the DB row. Currently they read ~15 columns; v2 spec adds 2 more. Every SELECT fragment in `ops.rs` that constructs a Memory from row data needs the additional columns:

- `fetch_memory_by_id` at ops.rs:~X (exact line located during T0)
- `hybrid_recall` row-to-Memory mapper (recall.rs)
- `list_memories_by_project` / similar in manas.rs (if any)

T0's first task: grep `pub fn.*Memory.*conn.*-> Result` in ops.rs and manas.rs; update each to include the new columns.

## 7. CompileContext `<preferences-flipped>` XML section

In `crates/daemon/src/recall.rs` `compile_dynamic_suffix()`, after the existing dynamic sections (lessons, skills, etc.), add:

```rust
// Preferences-flipped (greenfield; budget-accounted)
// Each entry renders BOTH old and new valence directly (no query-hint attribute hack).
// Requires a JOIN in list_flipped() to fetch each flipped memory's superseded_by
// target (the new active memory) and its valence.
if !excluded_layers.iter().any(|l| l == "preferences_flipped") {
    let flipped = ops::list_flipped_with_targets(conn, caller_org_id.as_deref(), 5)
        .unwrap_or_default();
    if !flipped.is_empty() {  // omit entirely when empty
        let mut pf_xml = String::from("<preferences-flipped>");
        for item in &flipped {
            let entry = format!(
                "\n  <flip at=\"{at}\" old_valence=\"{ov}\" new_valence=\"{nv}\">\
                 \n    <topic>{topic}</topic>\
                 </flip>",
                at = xml_escape(item.old_flipped_at.as_str()),
                ov = xml_escape(item.old_valence.as_str()),
                nv = xml_escape(item.new_valence.as_str()),
                topic = xml_escape(item.old_title.as_str()),
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
    // else: omit entirely (matches existing <skills/> emit pattern)
}
```

**New helper `ops::list_flipped_with_targets()`** (instead of v1's `list_flipped`):

```rust
pub struct FlippedWithTarget {
    pub old_id: String,
    pub old_title: String,
    pub old_valence: String,
    pub old_flipped_at: String,
    pub new_id: String,
    pub new_valence: String,
}

pub fn list_flipped_with_targets(
    conn: &Connection,
    organization_id: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<FlippedWithTarget>> {
    // JOIN memory m_old (flipped) with memory m_new (its superseded_by target)
    // to render BOTH valences in the <preferences-flipped> XML directly, avoiding
    // the v1 "query-hint attribute" hack that required a follow-up LLM tool call.
    let org = organization_id.unwrap_or("default");
    let clamped_limit = limit.min(100).max(1) as i64;
    let mut stmt = conn.prepare(
        "SELECT m_old.id, m_old.title, m_old.valence, m_old.valence_flipped_at,
                m_new.id, m_new.valence
           FROM memory m_old
      LEFT JOIN memory m_new ON m_old.superseded_by = m_new.id
          WHERE m_old.valence_flipped_at IS NOT NULL
            AND m_old.memory_type = 'preference'
            AND COALESCE(m_old.organization_id, 'default') = ?1
          ORDER BY m_old.valence_flipped_at DESC
          LIMIT ?2",
    )?;
    // ... map rows ...
}
```

**`caller_org_id` derivation in `compile_dynamic_suffix`:** the function currently doesn't receive `organization_id`. v2 extends its signature: add `organization_id: Option<&str>` as the last parameter (breaking change). All callers (grep `compile_dynamic_suffix\(` to enumerate) get `state.session_registry.get(session_id).map(|s| s.organization_id.as_str())` or `None` fallback.

**XML output example (populated):**

```xml
<preferences-flipped>
  <flip at="2026-04-17 14:22:00" old_valence="positive" new_valence="negative">
    <topic>tabs over spaces</topic>
  </flip>
</preferences-flipped>
```

Agent reading this sees: "At 2026-04-17 14:22:00, user flipped their stance on 'tabs over spaces' from positive to negative." No follow-up lookup needed; information is self-contained.

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
    "flipped_at": "2026-04-17 14:22:00"
}
```

(Timestamp uses `forge_core::time::now_iso()` format: `"YYYY-MM-DD HH:MM:SS"` — no T, no Z — consistent with §4.3 step 5 and §7 XML samples.)

**Emission site:** strictly AFTER `tx.commit()?` succeeds in the FlipPreference handler, before the Response::Ok return. Emitting pre-commit would leak the event even on rollback. Matches §4.3 step 8 sequence.

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

**T0. Prereq: `store_memory_raw()` helper + fetch-accessor field updates**
- RED: `test_store_memory_raw_inserts_without_dedup` — inserts a memory whose (title, type, project, org) matches an existing active memory; asserts two rows exist post-insert
- GREEN: add `store_memory_raw()` to `ops.rs`, identical to `remember()`'s INSERT branch but without the dedup UPSERT check
- Also: grep all `fn .* -> Memory` and `fn .* -> Result<Memory>` in `ops.rs`/`manas.rs`/`recall.rs`; update each to also read `superseded_by` and `valence_flipped_at` columns (once the schema migration lands in T3)
- Verification: existing workspace tests green

**T1. supersede_memory_impl helper (no flip)**
- RED: `test_supersede_memory_impl_marks_superseded_creates_edge` — covers the existing Supersede behavior via the new helper
- GREEN: extract helper, wire Supersede handler to call it inside an explicit `conn.transaction()`
- Verification: all existing Supersede handler tests still pass (regression-guard). Explicit per-ID error handling preserved via pre-helper fetch_memory_by_id checks in the handler.

**T2. supersede_memory_impl helper (with flip)**
- RED: `test_supersede_memory_impl_with_flip_sets_valence_flipped_at`
- GREEN: add the `valence_flipped_at` optional parameter and SQL branch

**T3. Schema migration + index**
- RED: `test_memory_schema_has_valence_flipped_at_column` (introspects SQLite schema via `PRAGMA table_info`)
- GREEN: add the `ALTER TABLE memory ADD COLUMN valence_flipped_at TEXT` (idempotent via `let _ = conn.execute(...)` — SQLite will error on duplicate; error is discarded)
- Also: add the partial index

**T4. Memory struct + serde**
- RED: `test_memory_struct_has_valence_flipped_at_and_superseded_by_fields` (serialize/deserialize round-trip; old JSON without these fields deserializes as None via `#[serde(default)]`)
- GREEN: add both `valence_flipped_at` and `superseded_by` as `Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`

**T5. Request/Response variants (contract)**
- RED: `test_parameterized_variants_method_names` fails — `FlipPreference` not found (note: parameterized, not unit — FlipPreference carries fields)
- GREEN: add `Request::FlipPreference`, `Request::ListFlipped`, extend `Request::Recall` with `#[serde(default)] include_flipped: Option<bool>`, add `ResponseData::PreferenceFlipped`, `ResponseData::FlippedList`

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

**T10. Recall: include_flipped (signature threading)**
- RED (three tests): `test_recall_default_excludes_flipped_prefs`, `test_recall_include_flipped_surfaces_flipped_prefs`, `test_recall_include_flipped_does_not_surface_non_preference_superseded`
- GREEN: thread `include_flipped: bool` param through `hybrid_recall()`; update BM25 and vector query status predicates (per §6.1); update every call site (4 known: handler.rs:480, 633, 2979, plus any test/bench calls)
- Verification: existing workspace tests green (with include_flipped=false by default at all existing call sites)

**T11. CompileContext: preferences-flipped section**
- RED (three tests): empty-omitted, single-populated-renders, budget-exceeded-truncates
- GREEN: add section in compile_dynamic_suffix

**T12. Integration harness + cross-crate clippy/test**
- Run `cargo test --workspace && cargo clippy --workspace -- -W clippy::all -D warnings`
- Must be 0 failures, 0 warnings before merge
- Add one integration test at `crates/daemon/tests/flip_preference_flow.rs` that exercises: Remember(preference) → FlipPreference → ListFlipped → Recall with include_flipped → CompileContext containing `<preferences-flipped>`.

**T13. Rollback recipe test**
- RED: test that runs forward migration, inserts a row with `valence_flipped_at`, runs rollback SQL, verifies remaining queries still work

**T14. Dogfood (HTTP API only — no CLI subcommands in 2A-4a scope)**
- Rebuild `forge-daemon` binary
- Manually via `curl` to port 8430:
  1. `POST /api {"method":"remember","params":{...preference...}}` → store a preference
  2. `POST /api {"method":"flip_preference","params":{"memory_id":"<id>","new_valence":"negative","new_intensity":0.8,"reason":"team convention"}}` → flip it
  3. `POST /api {"method":"list_flipped","params":{"limit":5}}` → verify the flip appears
  4. `POST /api {"method":"compile_context","params":{...}}` → verify `<preferences-flipped>` element renders
- Verify HUD/doctor report sane state post-flip (memory counts, no errors in log)
- **CLI subcommand** (`forge flip-preference`, `forge list-flipped`) **is out of scope for 2A-4a** — deferred to a follow-up task once the HTTP API stabilizes (scope simplification per v2 review).

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
| `crates/daemon/src/recall.rs` | Extend `hybrid_recall()` signature with `include_flipped: bool`; update BM25/vector query predicates; add `<preferences-flipped>` section in `compile_dynamic_suffix` (with extended signature including `organization_id`); update row-to-Memory mapper to read new columns |
| `crates/daemon/src/db/manas.rs` | Update any `fn ... -> Memory` / `fn ... -> Result<Memory>` accessors to read `superseded_by` and `valence_flipped_at` columns (T0 grep-and-update task) |
| `crates/daemon/src/bench/forge_consolidation.rs` | Update row mappers if any use direct SELECT on memory columns (regression-guard) |
| `crates/daemon/tests/flip_preference_flow.rs` | New integration test (T12) |

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
- **v2 (2026-04-17):** Addresses 5 CRITICAL + 6 HIGH + 8 MEDIUM findings from first-pass adversarial reviews.
  - **B-C1 fix:** FlipPreference uses new `store_memory_raw()` helper (T0), bypassing `remember()`'s dedup UPSERT that would merge new back onto old.
  - **B-C2 fix:** All SQL placeholders use numbered form (`?1, ?2, ...`) consistently; reference implementation included in §3.
  - **B-C3 fix:** `now_iso()` format aligned to `"YYYY-MM-DD HH:MM:SS"` (no T, no Z). Samples throughout updated.
  - **B-C4 fix:** `include_flipped` threaded through `hybrid_recall()` signature (§6.1); BM25/vector queries get predicate change; 4 call sites enumerated.
  - **B-C5 fix:** Memory struct gains `superseded_by` AND `valence_flipped_at` with `#[serde(default, skip_serializing_if)]` (§6.2); all fetch accessors updated in T0.
  - **B-H1 fix:** FlipPreference wraps all ops in explicit `conn.transaction()` for atomicity; no manual `forget_memory()` rollback needed.
  - **B-H2 fix:** Embedding NOT cloned; new memory inserted with `embedding: None`, indexer worker re-embeds the new content (which is DIFFERENT from old content — see B-C3/C-H3 fix).
  - **B-H4 fix:** Supersede handler retains per-ID error distinction via pre-helper `fetch_memory_by_id` checks.
  - **B-H5 fix:** Confidence floored at 0.5 (§4.4); new `confidence = old.confidence.max(0.5).min(1.0)`.
  - **C-C1 fix:** Cross-org scope guard added (§4.3 step 4) — caller_org derived from session, compared to old.organization_id, reject if different.
  - **C-H2 fix:** `alternatives` and `participants` reset to empty on flip (not cloned).
  - **C-H3 fix:** New memory `content` synthesized as `format!("[flipped from ... to ... at ...]: old.content")`, marking the flip event explicitly; old and new are semantically distinguishable.
  - **B-M1 fix:** `compile_dynamic_suffix` signature extended with `organization_id: Option<&str>`.
  - **B-M2 fix:** `<preferences-flipped>` renders both valences directly via new `list_flipped_with_targets()` JOIN helper; no `new_valence_query_hint` attribute hack.
  - **E1 fix:** §2 wording corrected — SQLite does NOT support `ALTER TABLE ADD COLUMN IF NOT EXISTS`; migration follows existing pattern (`let _ = conn.execute(...)` swallowing duplicate-column error).
  - **events::emit binding fix:** `&state.events` (not `&tx`).
  - **T5 fix:** test name corrected to `test_parameterized_variants_method_names` (FlipPreference is param variant, not unit).
  - **CLI subcommand scope dropped** from 2A-4a; deferred to follow-up.
  - **T0 added** for `store_memory_raw` helper + fetch-accessor grep-and-update; T1–T14 renumbered accordingly.
