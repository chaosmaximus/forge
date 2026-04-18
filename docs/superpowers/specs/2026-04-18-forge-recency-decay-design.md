# Phase 2A-4b — Recency-weighted Preference Decay (design)

**Status:** DRAFT v2 — 2026-04-19. Addresses 5 CRITICAL + 11 HIGH + 7 MEDIUM/LOW findings from first-pass adversarial reviews (Claude + Codex). Ready for second-pass review or writing-plans.
**Parent plan:** [Phase 2A-4 master design v6](../../benchmarks/forge-identity-master-design.md) §5 "2A-4b — Recency-weighted Preference Decay (daemon feature)" and §13 "Resolve in 2A-4b".
**Predecessor:** Phase 2A-4a Valence Flipping (shipped commit `66f2118`, 29 commits on master).
**Sub-phase scope:** daemon feature; unit + integration tests only; no composite bench score here (that's 2A-4d).
**Dogfood gate:** live daemon rebuild + restart + verify preserved state; `<preferences>` renders; regression-guard benches pass.

---

## 1. Thesis

A preference the user stated yesterday should rank higher than the same-topic preference they stated six months ago. The current post-RRF recency boost at `crates/daemon/src/recall.rs:381-386` treats every memory identically via `result.score *= 1.0 + exp(-0.1 × days_since_created) * 0.5`, which:

1. Envelope `1.0 + × 0.5` caps the spread at `[1.0, 1.5]` — a 180-day-old pref ranks only ~33% below a 0-day-old one, not the half-life-appropriate ~10,000× gap.
2. `accessed_at` self-refreshes via `touch()` at `ops.rs:940` on every recall, so a "stale" preference that's been retrieved once recently looks fresh — the classic retrieval-feedback self-contamination failure.
3. Preference aging has no distinct coefficient. A lesson and a preference decay at the same rate, even though lesson staleness reflects "haven't encountered this situation in a while" and preference staleness reflects "user probably doesn't hold this anymore".

Phase 2A-4b fixes this by introducing:
- Type-dispatched post-RRF recency multiplier (prefs: exponential half-life decay; non-prefs: existing `exp(-0.1 × d)` without the envelope)
- `reaffirmed_at` timestamp (user-controlled, not auto-refreshed)
- `touch()` exemption for preferences (SQL predicate)
- `<preferences>` XML section in CompileContext with age buckets
- `Request::ReaffirmPreference` — user/agent declares "I still hold this"
- `Request::ComputeRecencyFactor` (bench/test-gated) — expose the formula for testable bench scoring
- Prefs exempt from universal hard-fade at 0.1

---

## 2. Scope

### In scope (ships in 2A-4b)

**Schema + data model**
- Add column `memory.reaffirmed_at TEXT NULL` (no index — recall doesn't filter on it; partial index YAGNI)
- Add field `Memory::reaffirmed_at: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`
- **Memory struct field-addition audit (full scope):** every site that constructs a `Memory` literal or maps a SQL row to `Memory` must be updated to populate or carry `reaffirmed_at`. Known sites:
  - `crates/core/src/types/memory.rs:29-69` — Memory struct + `Memory::new()` constructor
  - `crates/daemon/src/server/handler.rs:902-926` — FlipPreference's new-memory struct literal (must set `reaffirmed_at: None`)
  - `crates/daemon/src/db/ops.rs:141-150` — `remember_raw` INSERT column list
  - `crates/daemon/src/db/ops.rs:1047-1094` — `export_memories_org` SELECT/serialization
  - `crates/daemon/src/db/ops.rs:1770-1810` — `find_reconsolidation_candidates` row mapper
  - `crates/daemon/src/db/ops.rs` — `MEMORY_ROW_COLUMNS` const + `map_memory_row()` helper from 2A-4a (extend column list + extraction)
  - `crates/daemon/src/sync.rs:491` — sync UPDATE statement (verify if affected)
  - `crates/daemon/src/db/ops.rs:84` — `remember()` UPSERT path (verify carries `reaffirmed_at`)
  - Any other `Memory { ... }` literal or `from_row` mapper found by full grep before T2 commit
- Audit task assertion: `cargo build --workspace` passes after T2 (compile errors flag missed sites)

**Config**
- Add `RecallConfig::preference_half_life_days: f64 = 14.0` (default 14, validated to `1..=365`)

**Pure function**
- `pub fn ops::recency_factor(memory: &Memory, preference_half_life_days: f64) -> f64` — type-dispatched multiplier

**Formula sites**
- `recall.rs:381-386` post-RRF recency pattern — **replace** `score *= 1.0 + recency_boost * 0.5` with `score *= recency_factor(memory, half_life)`
- `ops::decay_memories` at `ops.rs:837-883` — type-dispatched: prefs use new formula with hard-fade exempt; non-prefs unchanged
- `ops::touch()` at `ops.rs:940-954` — add SQL predicate `AND memory_type != 'preference'`

**New Request variants**
- `Request::ReaffirmPreference { memory_id: String }` — validates `memory_type = 'preference'`, sets `reaffirmed_at = now_iso()`; emits `"preference_reaffirmed"` event post-commit
- `Request::ComputeRecencyFactor { memory_id: String }` under `#[cfg(any(test, feature = "bench"))]` — returns pure `recency_factor` value

**New Response variants**
- `ResponseData::PreferenceReaffirmed { memory_id: String, reaffirmed_at: String }`
- `ResponseData::RecencyFactor { memory_id: String, factor: f64, days_since_anchor: f64, anchor: String }` (bench-gated)

**CompileContext XML**
- New `<preferences>` section in `compile_dynamic_suffix` at `recall.rs:749+` (after `<preferences-flipped>` block, before `</forge-dynamic>` close). Always emitted (bare `<preferences/>` when empty — per master D4). Up to 5 entries, budget-accounted. Age buckets: **conform to master §5 line 102 verbatim**: `1d / 1w / 1mo / 6mo+` (4 buckets). The "6mo+" tail covers >30d through forever.
- Excluded-layers key: `"preferences"` (snake_case). Mandatory follow-up: update the `excluded_layers` documentation comment in `crates/core/src/protocol/request.rs:291-295` to enumerate `"preferences"` and `"preferences_flipped"` (the latter was missed in 2A-4a).
- New ops helper `pub fn list_active_preferences(conn, organization_id, limit) -> Vec<Memory>` mirroring `list_flipped_with_targets` pattern — keeps SQL out of the renderer.

**Cargo bench feature declaration**
- Add `[features]\nbench = []` to `crates/core/Cargo.toml`
- Add `[features]\nbench = ["forge-core/bench"]` to `crates/daemon/Cargo.toml` (forwarding form so the gated core variant is enabled when daemon's bench is enabled)
- T0 prerequisite: verify literal dep name in `crates/daemon/Cargo.toml` (currently `forge-core` per `crates/daemon/Cargo.toml:19`)

**Regression-guard re-calibration (MANDATORY pre-merge)**
- Run full 5-seed Forge-Context calibration sweep
- Run full 5-seed Forge-Consolidation calibration sweep
- Record composite scores before/after in results doc
- Any composite regression below 1.0 → investigate + fix (either tune formula, anchor compatibility mode, or update prior bench expected ranges with documented justification)

**Schema rollback recipe test**
- Symmetric to 2A-4a T13: test that `ALTER TABLE memory DROP COLUMN reaffirmed_at` runs cleanly after populating rows

### Out of scope (explicitly deferred)

- Per-topic half-life (single global value suffices)
- Per-user half-life (2A-6 Forge-Transfer will own multi-user)
- Semantic reaffirmation detection (requires extraction pipeline changes)
- Auto-reaffirm based on repeated recall (reaffirmation is always user/agent-initiated)
- Auto-flip based on decayed confidence (Phase 9a remains diagnostic)
- Retroactive population of `reaffirmed_at` for existing preferences (null on migration, falls back to `created_at` via coalesce)
- Per-domain recency (e.g., tech prefs decay faster than values)
- Universal `touch()` exemption for non-preferences (future phase may consider)
- Decay formula for `lesson` vs `decision` vs `pattern` types (non-prefs share the single `exp(-0.1 × d)` rule)
- Aligning fader constant (`0.03`) with ranker constant (`0.1`) for non-prefs — kept separate, see §4 rationale

---

## 3. Dependencies

**Upstream (prereqs — all shipped):**
- Phase 2A-4a Valence Flipping (superseded_by + valence_flipped_at columns exist; FlipPreference handler pattern to mirror)
- `ops::fetch_memory_by_id` + `map_memory_row` helper from 2A-4a T0
- `ops::supersede_memory_impl` from 2A-4a T1 (no changes needed here; reaffirmation does not supersede)
- `hybrid_recall*` signatures threaded with `include_flipped` (2A-4a T10) — no further changes for 2A-4b
- `forge_core::time::now_iso()` — `"YYYY-MM-DD HH:MM:SS"` format

**Downstream (enabled by 2A-4b):**
- Phase 2A-4d Dim 6a (Direct Formula Probe) uses `Request::ComputeRecencyFactor`
- Phase 2A-4d Dim 3 (Preference time-ordering) relies on the type-dispatched recency multiplier
- Phase 2A-4d Dim 6b (Mixed-corpus ranking) relies on the new decay slope

---

## 4. Timestamp semantics (master §3 restated)

### Anchors

- **Preferences:** `days_since_pref_age = now_utc - coalesce(reaffirmed_at, created_at)`. If `reaffirmed_at IS NULL`, falls back to `created_at`. Immutable after insert (`created_at` never changes; `reaffirmed_at` changes only via `Request::ReaffirmPreference`).
- **Non-preferences:** `days_since_created = now_utc - created_at`. Matches current `recall.rs:384` behavior (except the envelope change — see §5).

### Why `accessed_at` is the wrong anchor for prefs

- `touch()` at `ops.rs:940` updates `accessed_at` on every recall. If a user queries "do I prefer vim or emacs?" today, the response includes their 1-year-old "prefer vim" preference with `accessed_at = today`, making it look fresh. Next recall ranks it above actually-recent preferences. This is the retrieval-feedback self-contamination failure.
- `created_at` is immutable and reflects "when did user first state this?"
- `reaffirmed_at` (new) reflects "when did user most recently confirm this?" — strictly under user/agent control via `Request::ReaffirmPreference`

### Non-preference anchor stays `created_at` — NOT `accessed_at`

The existing code at `recall.rs:383` uses `created_at` (via `ops::parse_timestamp_to_epoch(&result.memory.created_at)`). The fader at `ops.rs:862` uses `accessed_at`. This asymmetry is **preserved**:
- Ranker (per-query de-boost): `created_at` — "how long ago was this memory first committed?"
- Fader (background aging to `'faded'` status): `accessed_at` — "how long since anyone looked at this?"

The asymmetry serves different product signals:
- A decision committed 30 days ago that you reference every day stays ranker-ranked at `exp(-0.1×30) ≈ 0.050` (always de-boosted in recall) but stays unfaded in the fader because `accessed_at` is fresh.
- A decision committed 30 days ago and never accessed decays in both dimensions: ranker de-boosts it, AND the fader eventually marks it `'faded'` when `confidence × exp(-0.03 × days_since_accessed) < 0.1`.

Aligning the fader constant from `0.03` to `0.1` would shift fader half-life from ~23d to ~7d — most of the corpus would fade within 3 weeks of no access. This is a product decision that's **out of scope** for 2A-4b; if future UX analysis suggests faster fading, it's a separate RFC.

### Preference hard-fade exemption

The universal hard-fade threshold at `ops.rs:866` (`if effective < 0.1 { ... set status = 'faded' }`) is **skipped** for preferences:
- A decayed preference with `effective_confidence = 0.05` stays `status = 'active'`
- The ranker naturally de-boosts it via `recency_factor` (factor ≈ 0.01 at 90 days = effectively invisible in ranked results)
- Why not fade? Preferences are identity — even a 6-month-old "prefer vim" is part of the user's identity record. Marking it `'faded'` would exclude it from `CompileContext` and `include_flipped` paths. Recall's recency multiplier handles the "don't surface stale prefs" job by itself.

---

## 5. Formula specs

### Current code (to be replaced)

```rust
// crates/daemon/src/recall.rs:381-386 (pre-2A-4b)
for result in &mut results {
    let created_secs = ops::parse_timestamp_to_epoch(&result.memory.created_at).unwrap_or(0.0);
    let days_old = (now_secs - created_secs).max(0.0) / 86400.0;
    let recency_boost = (-0.1 * days_old).exp();
    result.score *= 1.0 + recency_boost * 0.5;
}
```

### New code (post-2A-4b)

```rust
// crates/daemon/src/recall.rs:~381-386 (post-2A-4b)
let now_secs = current_epoch_secs();  // single timestamp for the whole loop (also passed to recency_factor)
for result in &mut results {
    result.score *= ops::recency_factor(&result.memory, preference_half_life_days, now_secs);
}
```

The `preference_half_life_days: f64` value is threaded as a primitive parameter through `hybrid_recall*` signatures (see §12 T8). Callers (`handler.rs` Recall + BatchRecall) load it once per request from `crate::config::load_config().recall.validated().preference_half_life_days`.

### `ops::recency_factor` helper

```rust
// crates/daemon/src/db/ops.rs (new pub fn, placed near decay_memories for locality)

/// Returns the post-RRF recency multiplier for a memory.
///
/// Type-dispatched:
/// * Preferences: `2^(-days_since_pref_age / half_life)` where
///   `days_since_pref_age = now - coalesce(reaffirmed_at, created_at)`.
/// * Non-preferences: `exp(-0.1 * days_since_created)`.
///
/// **Scope:** consumed by recall.rs post-RRF ranking AND by the bench-only
/// `Request::ComputeRecencyFactor` (must be bit-exact per parity test).
/// **NOT consumed by `decay_memories`** — that helper has different anchors
/// and different constants for non-preferences (0.03 on accessed_at, see §5
/// fader spec) and uses its own inline type-dispatch.
///
/// `now_secs` is passed in (not read from SystemTime here) so the parity test
/// can freeze time and assert bit-exact equality between handler and direct
/// helper invocation. Production callers pass `current_epoch_secs()`.
pub fn recency_factor(memory: &Memory, preference_half_life_days: f64, now_secs: f64) -> f64 {
    let anchor_str = if memory.memory_type == MemoryType::Preference {
        memory.reaffirmed_at.as_deref().unwrap_or(&memory.created_at)
    } else {
        &memory.created_at
    };

    let anchor_secs = parse_timestamp_to_epoch(anchor_str).unwrap_or(now_secs);
    // Clock skew clamp: if anchor is in the future (NTP correction, sync from
    // a node whose wall clock leads ours), days = 0 → factor = 1 ("fresh").
    // Acceptable behavior: clock-corrected memories don't become stale.
    let days = ((now_secs - anchor_secs) / 86400.0).max(0.0);

    if memory.memory_type == MemoryType::Preference {
        let half_life = preference_half_life_days.max(1.0);
        2_f64.powf(-days / half_life)
    } else {
        (-0.1 * days).exp()
    }
}

/// Helper for the production caller — reads SystemTime once per recall call.
pub fn current_epoch_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
```

### Expected values (master Dim 6a spec, ±0.0001 absolute)

With `preference_half_life_days = 14.0`:

| days_since_pref_age | Formula | Factor |
|---------------------|---------|--------|
| 1                   | 2^(-1/14)   | **0.9517** |
| 14                  | 2^(-14/14)  | **0.5000** |
| 90                  | 2^(-90/14)  | **0.01161** |
| 180                 | 2^(-180/14) | **0.0001354** |

These values are locked for 2A-4d Dim 6a direct formula probe. If the half-life constant changes in a future phase, Dim 6a bench expected values must be recomputed. Master design v6 §4 Dim 6a notes ±0.0001 tolerance — for the −180d cell that is roughly ±74% relative due to floor proximity. 2A-4d may want to tighten the −180d tolerance to ±0.00005 or note the asymmetric tolerance explicitly.

### Fader (`decay_memories`) type-dispatch

The fader uses its OWN inline type-dispatch (does NOT call `recency_factor`). Reason: non-preferences use a different constant (0.03 vs 0.1) and a different anchor (`accessed_at` vs `created_at`) — see §4 rationale. Sharing a helper across both call sites would force false unification.

**SELECT shape change is additive:** the existing query at `ops.rs:839` returns 3 columns `(id, confidence, accessed_at)`. The new query keeps those at positions 0-2 and appends new columns at positions 3-5. This avoids breaking existing test sites at `ops.rs:3222, 3271, 3337, 5451` that destructure `(String, f64, String)` (T7 must update them; do not silently leave 3-column tests passing against a 6-column SELECT).

```rust
// crates/daemon/src/db/ops.rs:837-883 (post-2A-4b fader)

// New SELECT — 3 original columns at positions 0-2; new columns appended at 3-5
"SELECT id, confidence, accessed_at,
        memory_type, COALESCE(reaffirmed_at, ''), created_at
 FROM memory WHERE status = 'active' LIMIT ?1"

// Per-row branch (Rust):
for (id, confidence, accessed_at, memory_type, reaffirmed_at_or_empty, created_at) in &rows {
    let (effective, write_back_days) = if memory_type == "preference" {
        // Pref fader: 2^ formula, anchor = coalesce(reaffirmed_at, created_at).
        // Empty-string reaffirmed_at means SQL NULL → fall back to created_at.
        let anchor = if reaffirmed_at_or_empty.is_empty() {
            created_at.as_str()
        } else {
            reaffirmed_at_or_empty.as_str()
        };
        let anchor_secs = parse_timestamp_to_epoch(anchor).unwrap_or(now_secs);
        let days = ((now_secs - anchor_secs) / 86400.0).max(0.0);
        let eff = confidence * 2_f64.powf(-days / preference_half_life_days.max(1.0));
        (eff, days)
    } else {
        // Non-pref fader: UNCHANGED — exp(-0.03 × days_since_accessed).
        let accessed_secs = parse_timestamp_to_epoch(accessed_at).unwrap_or(now_secs);
        let days_since = ((now_secs - accessed_secs) / 86400.0).max(0.0);
        let eff = confidence * (-0.03 * days_since).exp();
        (eff, days_since)
    };

    if effective < 0.1 && memory_type != "preference" {
        // UPDATE status = 'faded' — prefs exempt from hard-fade
        faded_count += 1;
    } else if write_back_days > 1.0 {
        // UPDATE confidence = effective (same threshold for both types)
    }
}
```

**Decay write-back threshold:** `write_back_days > 1.0` for both types (the type-appropriate `days` value). At 14-day half-life, 1-day pref decay gives factor `0.9517` (4.8% drop) which is meaningful enough to write back. Keeps branching minimal.

**Fader anchor for prefs = `coalesce(reaffirmed_at, created_at)`** (NOT `accessed_at`). After `touch()` exemption lands, `accessed_at` is static for prefs — using it would give correct decay initially but silently wedge on any future code that re-enables pref access updates.

---

## 6. `touch()` exemption

### Current code

```rust
// crates/daemon/src/db/ops.rs:940-954 (pre-2A-4b)
pub fn touch(conn: &Connection, ids: &[&str]) {
    for id in ids {
        if let Err(e) = conn.execute(
            "UPDATE memory SET accessed_at = datetime('now'),
             access_count = MIN(access_count + 1, 1000)
             WHERE id = ?1
             AND (accessed_at < datetime('now', '-60 seconds') OR access_count = 0)",
            params![id],
        ) {
            eprintln!("[ops] failed to touch memory {id}: {e}");
        }
    }
}
```

### New code

```rust
// crates/daemon/src/db/ops.rs:940-955 (post-2A-4b)
pub fn touch(conn: &Connection, ids: &[&str]) {
    for id in ids {
        if let Err(e) = conn.execute(
            "UPDATE memory SET accessed_at = datetime('now'),
             access_count = MIN(access_count + 1, 1000)
             WHERE id = ?1
             AND (accessed_at < datetime('now', '-60 seconds') OR access_count = 0)
             AND memory_type != 'preference'",
            params![id],
        ) {
            eprintln!("[ops] failed to touch memory {id}: {e}");
        }
    }
}
```

### Architectural rationale (master §13 N-H1 resolved)

The exemption lives at the mutation point in `ops::touch()`, NOT in `writer.rs`. `writer.rs` receives `ids: Vec<String>` without memory types — adding a type lookup there would mean a SELECT per ID before UPDATE, N+1 style. A SQL predicate inside `touch()` lets SQLite handle the filter atomically in a single UPDATE statement.

The predicate is purely additive — zero-ID and non-preference touches behave identically to pre-2A-4b.

### Tests (multi-layer — direct unit test alone is insufficient)

The `touch()` runtime path that matters is the read-only request path: handlers send touches through the writer actor (`handler.rs:667-676` Recall arm, `handler.rs:2641-2643` CompileContext arm). A direct unit test on `ops::touch()` confirms the SQL predicate works but won't catch misrouting in those handlers. Multi-layer tests required:

**Layer 1 — direct unit test (`ops.rs` mod tests):**
1. Seed 2 memories — `memory_type = 'preference'` and `memory_type = 'decision'`
2. Call `touch()` on both IDs
3. Assert: preference's `accessed_at` unchanged; decision's `accessed_at` updated
4. Assert: preference's `access_count` unchanged; decision's `access_count` incremented
5. Negative control: same memory ID, UPDATE memory_type from 'decision' to 'preference', touch again, assert accessed_at now stops updating — proves predicate fires per-call (not at code-path selection)

**Layer 2 — integration test through `Request::Recall` (`tests/touch_exemption_recall.rs`):**
1. Seed a preference + a decision (both with old `accessed_at`)
2. Call `Request::Recall` matching both
3. Wait for writer actor to drain (sleep or sync barrier)
4. Assert: preference's `accessed_at` unchanged end-to-end; decision's updated

**Layer 3 — integration test through `Request::CompileContext` (`tests/touch_exemption_compile_context.rs`):**
1. Seed a preference + a decision
2. Call `Request::CompileContext`
3. Wait for writer drain
4. Assert: preference's `accessed_at` unchanged end-to-end; decision's updated

---

## 7. Cargo `bench` feature declaration

Neither `crates/core/Cargo.toml` nor `crates/daemon/Cargo.toml` currently declares `[features]`. 2A-4b is the first sub-phase introducing `#[cfg(any(test, feature = "bench"))]`-gated variants (`ComputeRecencyFactor`).

### Core crate

```toml
# crates/core/Cargo.toml — add
[features]
bench = []
```

### Daemon crate

```toml
# crates/daemon/Cargo.toml — add
[features]
bench = ["forge-core/bench"]
```

The daemon's `bench` forwards to core's `bench` so the Request variant is available when the daemon's `bench` feature is enabled.

### Fallback (master §13 note)

If the `bench` feature declaration proves difficult, downgrade to `#[cfg(test)]` only and access via integration tests rather than the feature gate. This design recommends the full feature gate since it unblocks 2A-4d bench ingestion.

---

## 8. New Request / Response variants

### `Request::ReaffirmPreference`

```rust
// crates/core/src/protocol/request.rs
Request::ReaffirmPreference {
    /// The preference memory ID to reaffirm.
    /// Must reference an existing memory with memory_type = 'preference' and
    /// status = 'active' (not superseded, not flipped). Sets the memory's
    /// reaffirmed_at to now_iso(); leaves confidence, valence, and content
    /// unchanged. Intended for user/agent re-statement of an existing preference
    /// ("yes, still prefer vim") without creating a new memory.
    memory_id: String,
},
```

### `Request::ComputeRecencyFactor` (bench-gated)

```rust
// crates/core/src/protocol/request.rs
#[cfg(any(test, feature = "bench"))]
Request::ComputeRecencyFactor {
    /// The memory ID to compute the current recency multiplier for.
    /// Returns the pure ops::recency_factor() value WITHOUT running
    /// BM25, vector search, RRF, graph expansion, or ranking — used by
    /// Phase 2A-4d Dim 6a to test formula correctness directly.
    memory_id: String,
},
```

### `ResponseData::PreferenceReaffirmed`

```rust
// crates/core/src/protocol/response.rs
ResponseData::PreferenceReaffirmed {
    memory_id: String,
    reaffirmed_at: String,  // YYYY-MM-DD HH:MM:SS
},
```

### `ResponseData::RecencyFactor` (bench-gated)

```rust
// crates/core/src/protocol/response.rs
#[cfg(any(test, feature = "bench"))]
ResponseData::RecencyFactor {
    memory_id: String,
    factor: f64,
    days_since_anchor: f64,
    anchor: String,  // "reaffirmed_at" | "created_at"
},
```

### Handler behaviors

**`Request::ReaffirmPreference`:**
- Fetch memory via `ops::fetch_memory_by_id` (read-side validation for clear error messages)
- Validate `memory_type = 'preference'`
- Validate `status = 'active'` (reject if superseded, faded, reverted, conflict)
- Validate cross-org (caller org must match memory's `organization_id`)
- **Compute `now_iso = forge_core::time::now_iso()` in Rust; bind as parameter (NOT inline `now_iso()` SQL — that function does not exist in SQLite)**
- **Atomic tx with in-SQL preconditions to prevent TOCTOU between read and write:**
  ```sql
  UPDATE memory
  SET reaffirmed_at = ?1
  WHERE id = ?2
    AND memory_type = 'preference'
    AND status = 'active'
    AND COALESCE(organization_id, 'default') = COALESCE(?3, 'default')
  ```
- **Treat `rows_updated != 1` as semantic failure** ("row changed underneath" race). Return error: `"reaffirm raced — memory state changed (id: {memory_id})"`. This guards against a Flip/Supersede landing between the read-side validation and the UPDATE.
- Post-commit: emit `"preference_reaffirmed"` event with `{memory_id, reaffirmed_at}`

**`Request::ComputeRecencyFactor`:**
- Fetch memory via `ops::fetch_memory_by_id`
- Read `preference_half_life_days` from config
- Call `ops::recency_factor(memory, half_life)`
- Return `{memory_id, factor, days_since_anchor, anchor}`
- No event emission (read-only operation)
- Add to `writer::is_read_only` matches!()

### Stable error messages (ReaffirmPreference)

| Condition | Message |
|-----------|---------|
| `memory_id` not found | `"memory_id not found: {memory_id}"` |
| Wrong type | `"memory_type must be preference for reaffirm (got: {got})"` |
| Status superseded due to flip | `"preference was flipped — use new id from ListFlipped (id: {memory_id})"` (when `status='superseded' AND valence_flipped_at IS NOT NULL`) |
| Status superseded (non-flip) | `"memory superseded (id: {memory_id})"` |
| Status faded/reverted/conflict | `"memory not active (status: {status}, id: {memory_id})"` |
| Cross-org denied | `"cross-org reaffirm denied"` |
| Race (rows_updated != 1) | `"reaffirm raced — memory state changed (id: {memory_id})"` |
| Transaction failed | `"reaffirm transaction failed: {e}"` |

### Event emission contract

- Event name: `"preference_reaffirmed"` (matches 2A-4a's `"preference_flipped"` convention)
- Payload: `{"memory_id": "...", "reaffirmed_at": "YYYY-MM-DD HH:MM:SS"}`
- Emitted AFTER `tx.commit()` succeeds (tested with explicit post-commit assertion)

---

## 9. `<preferences>` CompileContext XML section

### Schema

```xml
<preferences>
  <pref age="1d" valence="positive" intensity="0.8">
    <topic>{title}</topic>
  </pref>
  <pref age="1w" valence="negative" intensity="0.6">
    <topic>{title}</topic>
  </pref>
  <!-- up to 5 entries total, ordered by coalesce(reaffirmed_at, created_at) DESC -->
</preferences>
```

When empty (no active preferences):

```xml
<preferences/>
```

### Age bucket mapping (conforms to master §5 line 102 — `1d|1w|1mo|6mo+`)

```rust
fn pref_age_bucket(days: f64) -> &'static str {
    if days <= 1.0 { "1d" }
    else if days <= 7.0 { "1w" }
    else if days <= 30.0 { "1mo" }
    else { "6mo+" }
}
```

The "6mo+" bucket is the master's chosen tail label even though it covers 30+ days literally (not strictly 6+ months). Future spec revision may add a finer grading; for 2A-4b the 4-bucket vocabulary is locked to match master contract.

### Budget accounting

Same pattern as `<preferences-flipped>` (2A-4a):
- `context_budget` = `ctx_config.budget_chars`
- Each entry ≤ ~200 bytes; 5 entries ≤ ~1000 bytes
- Skip entries that would overflow remaining budget
- Always emit opening/closing tags (even bare `<preferences/>`) — satisfies master infrastructure assertion 10

### Query (encapsulated in helper, NOT inline in renderer)

The renderer at `recall.rs` calls `ops::list_active_preferences(conn, organization_id, 5)` (mirrors `list_flipped_with_targets` from 2A-4a). The helper returns `Vec<Memory>` (or a lighter `ActivePreference { id, title, valence, intensity, anchor_at }` struct if the bench/tests prefer minimal data).

```rust
// crates/daemon/src/db/ops.rs (new pub fn)

pub fn list_active_preferences(
    conn: &Connection,
    organization_id: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<Memory>> {
    // SQL: same MEMORY_ROW_COLUMNS pattern as list_flipped_with_targets;
    // org filter via COALESCE(organization_id, 'default') = COALESCE(?1, 'default');
    // ORDER BY COALESCE(reaffirmed_at, created_at) DESC LIMIT ?2;
    // status='active' AND memory_type='preference'
    // ...
}
```

### Excluded-layers key

`"preferences"` (snake_case; matches `"preferences_flipped"`, `"active-sessions"`, etc.)

### Section position

After `<preferences-flipped>` and before closing `</forge-dynamic>`. Reason: keeps all pref-related sections adjacent; flipped (history) → preferences (current state).

---

## 10. Regression-guard re-calibration plan

The new type-dispatched post-RRF recency multiplier changes absolute scores for ALL memories, not just preferences. Non-preferences lose the `1.0 + × 0.5` envelope — their new multiplier is the bare `exp(-0.1 × days_since_created)`. This is a score-shape change that could affect:
- Forge-Context (Phase 2A-2): composite 1.00 on 5 seeds, includes layered recall with mixed memory types
- Forge-Consolidation (Phase 2A-3): composite 1.00 on 5 seeds, includes consolidation + recall at various phases

### Sweep requirements

Both benches must re-run fresh 5-seed calibration:

```bash
# Forge-Context
for s in 42 1337 2718 31415 9000; do
  cargo run --release -p forge-daemon --bin forge-bench -- forge-context \
    --seed $s --output bench_results_context_2a4b/seed_$s
done

# Forge-Consolidation
for s in 42 1337 2718 31415 9000; do
  cargo run --release -p forge-daemon --bin forge-bench -- forge-consolidation \
    --seed $s --output bench_results_consolidation_2a4b/seed_$s
done
```

### Pass gate (three tiers)

- **Green** (no action): all 5 seeds ≥ 1.00 composite.
- **Yellow** (document + proceed): any single seed in `[0.98, 1.00)` range, OR up to two seeds in that range if mean across all 5 seeds ≥ 0.99. Record observed values with rationale in results doc.
- **Red** (block merge): any seed < 0.95 composite, OR two-or-more seeds < 0.98, OR mean across 5 seeds < 0.98.

Red-tier resolution paths:
- Verify the drop is caused by the recency formula change (revert that single change, re-run — if composite returns to 1.00, formula is the cause)
- Evaluate product judgment: is the drop acceptable given the improvement in pref-staleness signaling? (escalate to user)
- Update the affected bench fixtures (e.g., all-fresh-memory fixtures) with documented justification
- Results doc includes a before/after composite table:

```
Forge-Context — before/after:
  seed 42    → before 1.000 / after 1.000 / delta  +0.000 / accepted
  seed 1337  → before 1.000 / after 1.000 / delta  +0.000 / accepted
  seed 2718  → before 1.000 / after 0.984 / delta  -0.016 / accepted: reason ...
  seed 31415 → before 1.000 / after 1.000 / delta  +0.000 / accepted
  seed 9000  → before 1.000 / after 1.000 / delta  +0.000 / accepted
  mean delta: -0.003

Forge-Consolidation — before/after:
  (same format)
```

### Blocker

If any composite regresses beyond rationale, 2A-4b **does not merge** until resolved. Resolution paths:
1. Tune the non-pref multiplier envelope (e.g., re-introduce `1.0 + factor * 0.5` for non-prefs only)
2. Anchor a compatibility mode for prior benches (e.g., `--legacy-recency` flag)
3. Update prior benches' expected-score ranges with documented justification in their results docs

---

## 11. Testing strategy

### Unit tests (per-helper, inline mod tests)

All `recency_factor` tests use a frozen `now_secs` to avoid wall-clock drift flakiness.

1. **`ops::recency_factor` prefs** — preferences at `{1, 14, 90, 180}` days match `{0.9517, 0.5000, 0.01161, 0.0001354}` within 1e-4 (note: −180d expected value is 0.0001354 to 4 sig-fig, not 0.000135)
2. **`ops::recency_factor` non-prefs** — lesson at `{1, 10, 30}` days matches `{0.9048, 0.3679, 0.04979}` within 1e-3
3. **`ops::recency_factor` reaffirmed overrides created_at** — seed pref with `created_at=-100d`, `reaffirmed_at=-2d`, assert factor ≈ `2^(-2/14) ≈ 0.9048` within 1e-3
4. **`recency_factor` clock skew clamp** — anchor in the future (now_secs - 86400 = anchor_secs) → factor = 1.0 exactly
5. **`touch()` exemption (Layer 1 — direct unit)** — see §6 multi-layer test plan
6. **`touch()` exemption (Layer 2 — Recall)** — see §6
7. **`touch()` exemption (Layer 3 — CompileContext)** — see §6
8. **`decay_memories` pref formula** — seed pref with `confidence=0.9`, `created_at=-30d`, half_life=14 → stored confidence ≈ `0.9 × 2^(-30/14) ≈ 0.2037` within 1e-3
9. **`decay_memories` pref hard-fade exemption** — seed pref with `confidence=0.9`, `created_at=-58d` (so effective ≈ `0.9 × 2^(-58/14) ≈ 0.052 < 0.1`), assert status stays `'active'` AND stored confidence updated to ~0.052 (write-back path triggered, fade-path skipped)
10. **`decay_memories` non-pref unchanged** — lesson with `confidence=0.9`, `accessed_at=-30d` → stored confidence ≈ `0.9 × exp(-0.03×30) ≈ 0.3661` within 1e-3
11. **`decay_memories` reaffirmed pref uses reaffirmed_at** — pref with `created_at=-180d`, `reaffirmed_at=-2d`, half_life=14 → stored confidence reflects 2-day decay not 180-day decay
12. **Age bucket mapping** — `pref_age_bucket({0.5, 1, 1.5, 7, 8, 30, 31, 180, 365})` → `{"1d", "1d", "1w", "1w", "1mo", "1mo", "6mo+", "6mo+", "6mo+"}`

### Handler tests (per Request variant)

13. **ReaffirmPreference happy path** — seed pref, Reaffirm, assert `reaffirmed_at` equals `now_iso()` within 2s; also `rows_updated == 1`
14. **ReaffirmPreference validation paths (7 cases per stable error table):** memory_id not found, wrong type, status superseded due to flip (with valence_flipped_at set), status superseded non-flip, status faded, cross-org denied, race (rows_updated != 1 — induce by deleting row between fetch and UPDATE)
15. **ReaffirmPreference TOCTOU guard** — fetch validation passes, then directly UPDATE the row's status to 'superseded' before the helper's UPDATE runs (test-only race window). Assert helper returns "race" error and does NOT mutate `reaffirmed_at`
16. **ReaffirmPreference event emission** — event name `"preference_reaffirmed"`, payload shape, emitted post-commit (subscriber receives after UPDATE succeeds). Negative case: validation failure → no event emitted
17. **ComputeRecencyFactor happy path** — returns factor matching formula for prefs and non-prefs
18. **ComputeRecencyFactor + recency_factor parity (bit-exact, frozen-time)** — call handler with frozen `now_secs`, get F1; call `ops::recency_factor` directly with same frozen `now_secs`, get F2; assert `F1.to_bits() == F2.to_bits()`. Both helpers must accept `now_secs` as parameter to make this test possible.

### Integration test (`tests/recency_decay_flow.rs`)

19. End-to-end:
    a. Remember a preference (title="prefers vim", memory_type=preference, valence=positive, intensity=0.8)
    b. Backdate `created_at` to 90 days ago via direct SQL UPDATE (test-only helper)
    c. CompileContext → assert `<preferences>` section contains entry with `age="6mo+"`
    d. ReaffirmPreference(id)
    e. CompileContext → assert `<preferences>` section contains entry with `age="1d"`
    f. Recall("prefers vim") with include_flipped=false → returns pref
    g. Capture score S1
    h. Re-seed pref with age 1 day (fresh)
    i. Recall("prefers vim") → score S2
    j. Assert S2 > S1 (fresh ranks higher than reaffirmed — but both decay by the same formula now)

### Schema rollback test

20. **`tests/recency_decay_rollback.rs`** — ALTER TABLE DROP COLUMN reaffirmed_at; verify fresh schema query after rollback (no index to drop since we didn't create one)

### Regression-guard bench sweeps (MANDATORY pre-merge)

21. **Forge-Context 5 seeds** — see §10
22. **Forge-Consolidation 5 seeds** — see §10

### Dogfood test (live daemon)

23. **Live daemon rebuild + restart + verify preserved state** — see §13

---

## 12. TDD task sequence (writing-plans input)

19 tasks (T0-T18), ordered for compile-time dependencies:

**T0 — Cargo `bench` feature declaration** (prereq)
- Verify literal dep name in `crates/daemon/Cargo.toml:19` is `forge-core` (currently confirmed)
- Add `[features]\nbench = []` to `crates/core/Cargo.toml`
- Add `[features]\nbench = ["forge-core/bench"]` to `crates/daemon/Cargo.toml`
- Smoke test: `cargo build --workspace` succeeds; `cargo build --workspace --features bench` succeeds; `cargo build -p forge-daemon --features bench` enables the bench-gated variants

**T1 — Schema: add `reaffirmed_at` column**
- ALTER TABLE in `crates/daemon/src/db/schema.rs` with Phase 2A-4b banner
- No partial index (recall doesn't filter on reaffirmed_at)
- `forge_db_schema_creates_reaffirmed_at` test

**T2 — Memory struct: add `reaffirmed_at` field + full audit**
- `crates/core/src/types/memory.rs` — `#[serde(default, skip_serializing_if = "Option::is_none")] pub reaffirmed_at: Option<String>`
- Update `Memory::new()` constructor
- **Audit and update ALL Memory construction/mapping sites** (compile errors will flag missed sites):
  - `crates/daemon/src/server/handler.rs:902-926` — FlipPreference's new-memory struct literal (add `reaffirmed_at: None`)
  - `crates/daemon/src/db/ops.rs:84-100` — `remember()` UPSERT path (verify SELECT and UPDATE paths carry `reaffirmed_at`)
  - `crates/daemon/src/db/ops.rs:141-150` — `remember_raw` INSERT column list (extend column list AND VALUES)
  - `crates/daemon/src/db/ops.rs:1047-1094` — `export_memories_org` SELECT/serialization
  - `crates/daemon/src/db/ops.rs:1770-1810` — `find_reconsolidation_candidates` row mapper
  - `crates/daemon/src/db/ops.rs` — `MEMORY_ROW_COLUMNS` const + `map_memory_row()` helper from 2A-4a (extend column list to include `reaffirmed_at`; extend extraction)
  - `crates/daemon/src/sync.rs:491` — sync UPDATE statement (extend if needed; verify)
  - **Audit method**: full `git grep -n "Memory {"` AND `git grep -n "from_row\|map_memory_row"` and visit every match
- Serde round-trip test (None elides, Some emits)
- Acceptance: `cargo build --workspace` and `cargo test --workspace` pass

**T3 — `RecallConfig::preference_half_life_days`**
- Add field at `config.rs:464` RecallConfig struct
- Default `14.0`
- `validated()` clamps to `1..=365`
- Config round-trip test

**T4 — `ops::recency_factor()` helper + `current_epoch_secs()`**
- New `pub fn recency_factor(memory: &Memory, preference_half_life_days: f64, now_secs: f64) -> f64` in `crates/daemon/src/db/ops.rs` near `decay_memories`
- New `pub fn current_epoch_secs() -> f64` (single-line wrapper around SystemTime::now)
- Pure function — no DB, no SQL
- Tests using frozen `now_secs`:
  - 4 pref-formula tests at days {1, 14, 90, 180} → {0.9517, 0.5, 0.01161, 0.0001354}
  - 3 non-pref tests at days {1, 10, 30} → {0.9048, 0.3679, 0.04979}
  - Reaffirmation override test
  - Clock-skew clamp test (anchor in future → factor = 1.0)
- Acceptance: tests pass; helper signature accepts `now_secs` (parity test will use this)

**T5 — Request/Response variants + contract tests + REQUIRED routing updates**
- Add `Request::ReaffirmPreference { memory_id: String }` (write — routes through writer actor)
- Add `Request::ComputeRecencyFactor { memory_id: String }` under `#[cfg(any(test, feature = "bench"))]` (read-only — does NOT route through writer)
- Add `ResponseData::PreferenceReaffirmed { memory_id, reaffirmed_at }`
- Add `ResponseData::RecencyFactor { memory_id, factor, days_since_anchor, anchor }` (bench-gated)
- Update `contract_tests.rs` with parameterized vectors for both
- **MANDATORY: Update `writer::is_read_only` matches!() at `crates/daemon/src/server/writer.rs:55-85`** — `ReaffirmPreference` is a WRITE (omit from is_read_only or list explicitly false); `ComputeRecencyFactor` is READ-ONLY (add to is_read_only matches!())
- **MANDATORY: Update `tier::request_to_feature` at `crates/daemon/src/server/tier.rs:294-295`** for both variants (exhaustive match)
- **MANDATORY: Update excluded_layers documentation in `crates/core/src/protocol/request.rs:291-295`** to enumerate `"preferences"` (this 2A-4b adds) and `"preferences_flipped"` (missed in 2A-4a)

**T6 — `touch()` exemption SQL predicate + multi-layer tests**
- Add `AND memory_type != 'preference'` to `ops::touch()` UPDATE
- Layer 1 — direct unit test in `ops.rs` mod tests: 2 memories (pref + decision), `touch()` both, assert pref `accessed_at` unchanged + `access_count` unchanged; assert decision updated. Negative control: UPDATE memory_type from 'decision' to 'preference', touch again, assert accessed_at stops updating
- Layer 2 — integration test `tests/touch_exemption_recall.rs`: through `Request::Recall`, assert preference's `accessed_at` unchanged end-to-end after writer drain
- Layer 3 — integration test `tests/touch_exemption_compile_context.rs`: through `Request::CompileContext`, same assertion

**T7 — `decay_memories` type-dispatched formula**
- SELECT now includes `memory_type, reaffirmed_at` columns
- Rust-side branch: pref uses type-dispatched formula with anchor=`coalesce(reaffirmed_at, created_at)`; non-pref unchanged
- Hard-fade skip for prefs: `if effective < 0.1 && memory_type != 'preference' { UPDATE status='faded' }`
- Pref write-back test (confidence 0.9, created_at=-30d → ~0.206)
- Pref hard-fade exemption test (decayed to 0.05, stays 'active')
- Non-pref unchanged test

**T8 — `recall.rs` post-RRF envelope replacement + config threading**
- Thread `preference_half_life_days: f64` through the THREE `hybrid_recall*` signatures (mirrors 2A-4a `include_flipped` threading):
  - `hybrid_recall(...)` (signature at `recall.rs:121`)
  - `hybrid_recall_scoped(...)` (at `recall.rs:148`)
  - `hybrid_recall_scoped_org(...)` (at `recall.rs:178`)
  - **Note:** `hybrid_recall_scoped_org_flipped` does NOT exist as a function. The `_flipped` suffix variant is `ops::recall_bm25_project_org_flipped` (BM25 helper, not hybrid). Do not invent a fictional signature.
- Update call sites:
  - `handler.rs:481` Recall arm — read `crate::config::load_config().recall.validated().preference_half_life_days` once and pass through
  - `handler.rs:635` (or current line for the same Recall path under different scope) — same
  - `handler.rs:3212` BatchRecall arm — same
  - Bench harness call sites in `crates/daemon/src/bench/forge_context.rs` and `forge_consolidation.rs`
  - Test call sites
  - `compile_dynamic_suffix` does NOT call hybrid_recall — no change there
- Replace post-RRF block:
  ```rust
  // Remove:
  // let recency_boost = (-0.1 * days_old).exp();
  // result.score *= 1.0 + recency_boost * 0.5;
  // Replace with:
  let now_secs = ops::current_epoch_secs();
  for result in &mut results {
      result.score *= ops::recency_factor(&result.memory, preference_half_life_days, now_secs);
  }
  ```
- Source-level test: old envelope pattern `"1.0 + recency_boost * 0.5"` does not appear (satisfies master assertion 14)
- Also extend `ops::decay_memories(conn, limit)` → `ops::decay_memories(conn, limit, preference_half_life_days)`; update consolidator call site at `consolidator.rs:107` to load config and pass half_life

**T9 — ReaffirmPreference handler happy path (with TOCTOU-safe SQL)**
- Match arm in `handler.rs` near FlipPreference
- Compute `let now = forge_core::time::now_iso();` in Rust, bind as parameter (NOT inline `now_iso()` SQL — that function does not exist in SQLite)
- Atomic tx with in-SQL preconditions:
  ```sql
  UPDATE memory
  SET reaffirmed_at = ?1
  WHERE id = ?2
    AND memory_type = 'preference'
    AND status = 'active'
    AND COALESCE(organization_id, 'default') = COALESCE(?3, 'default')
  ```
- Treat `rows_updated != 1` as semantic failure — return "race" error (per §8 stable error table)
- Post-commit event emit (subscriber sees AFTER commit succeeds)
- Test: seed + Reaffirm + assert `reaffirmed_at` set within 2s of `now_iso()`; assert `rows_updated == 1`

**T10 — ReaffirmPreference validation paths + TOCTOU race test**
- memory_id not found → `"memory_id not found: {id}"`
- Wrong type → `"memory_type must be preference for reaffirm (got: {got})"`
- Status superseded due to flip (valence_flipped_at IS NOT NULL) → `"preference was flipped — use new id from ListFlipped (id: {id})"`
- Status superseded non-flip → `"memory superseded (id: {id})"`
- Status faded/reverted/conflict → `"memory not active (status: {status}, id: {id})"`
- Cross-org denied → `"cross-org reaffirm denied"`
- TOCTOU race: read-side validation passes, then UPDATE memory SET status='superseded' before helper UPDATE → assert returns `"reaffirm raced — memory state changed (id: {id})"` AND `reaffirmed_at` NOT mutated
- Tx failure (closed DB simulation) → `"reaffirm transaction failed: {e}"`
- 8 validation tests

**T11 — ReaffirmPreference event emission post-commit**
- Subscribe to events before handler call
- Call handler, wait for event
- Assert event name `"preference_reaffirmed"`, payload shape
- Assert event NOT emitted when validation fails

**T12 — ComputeRecencyFactor handler + bit-exact parity test (frozen time)**
- Match arm at `handler.rs` under `#[cfg(any(test, feature = "bench"))]`
- Fetch memory, read config for `preference_half_life_days`, compute `now_secs = ops::current_epoch_secs()` once per handler call, call `ops::recency_factor(memory, half_life, now_secs)`, return response
- **Parity test (frozen time):** call handler with seeded memory + injected `now_secs = X`; call `ops::recency_factor(&same_memory, half_life, X)` directly; assert `F1.to_bits() == F2.to_bits()`. Frozen time prevents wall-clock drift between calls. **The handler must accept (or be testable with) an injected `now_secs` for this to work** — either via a test-only `recency_factor_at` helper, or by passing `now_secs` as a hidden parameter (test-cfg only)
- Acceptance: parity test never flakes across 100 consecutive runs

**T13 — `<preferences>` XML section in compile_dynamic_suffix + ops helper**
- New helper `ops::list_active_preferences(conn, organization_id, limit) -> rusqlite::Result<Vec<Memory>>` (mirrors `list_flipped_with_targets` pattern; SQL stays in ops.rs)
- Section position: after `<preferences-flipped>` block, before `</forge-dynamic>` close (use textual landmarks not line numbers — line numbers shift)
- Excluded-layers check: `"preferences"` (snake_case)
- Renderer calls `ops::list_active_preferences`, iterates, applies `pref_age_bucket` (private fn in `recall.rs`), XML-escapes title, budget-accounts
- Always emit (bare `<preferences/>` when empty) — D4 compliance
- Budget accounting like `<preferences-flipped>` from 2A-4a
- Tests: empty corpus (bare `<preferences/>`), 3 prefs (3 entries), 7 prefs (5 entries truncated), budget-exceeded (entries truncated), excluded-layer path (no section), reaffirmed pref ordered first
- Conformance: bucket vocabulary `1d / 1w / 1mo / 6mo+` (4 buckets per master)

**T14 — Integration test** (`tests/recency_decay_flow.rs`)
- Remember pref → backdate → CompileContext shows "6mo" → ReaffirmPreference → CompileContext shows "1d"
- Recall scoring delta between aged and fresh prefs

**T15 — Schema rollback recipe test**
- ALTER TABLE DROP COLUMN reaffirmed_at
- Verify rollback runs cleanly in fresh DB + populated DB

**T16 — Regression-guard Forge-Context 5 seeds**
- Run all 5 seeds at HEAD (post-2A-4b)
- Compare composites against archived pre-2A-4b results
- Update bench results doc

**T17 — Regression-guard Forge-Consolidation 5 seeds**
- Run all 5 seeds at HEAD (post-2A-4b)
- Compare composites against archived pre-2A-4b results
- Update bench results doc

**T18 — Live daemon dogfood + results doc**
- Rebuild `forge-daemon` release binary
- SIGTERM live daemon; watchdog restarts with new binary
- Verify preserved state (memory count unchanged)
- HTTP curl sequence:
  - Remember pref
  - ReaffirmPreference (via HTTP `/api` POST)
  - Verify `<preferences>` in CompileContext response
- Write `docs/benchmarks/results/forge-recency-decay-2026-04-18.md`

---

## 13. Dogfood sequence (T18 detail)

```bash
# Pre-rebuild state capture
curl -sX POST localhost:8420/api -H 'Content-Type: application/json' \
  -d '{"method":"recall_stats","params":{}}' > /tmp/forge_pre_2a4b.json

# Build new binary
cargo build --release -p forge-daemon

# Restart daemon (watchdog handles this if supervised)
# If NOT supervised by launchd/systemd/etc, manually restart after pkill:
pkill -TERM -f forge-daemon
sleep 2
# Check if watchdog restarted automatically
NEW_PID=$(pgrep -f forge-daemon | head -1)
if [ -z "$NEW_PID" ]; then
  echo "No watchdog detected — starting daemon manually"
  nohup ~/.cargo/bin/forge-daemon > ~/.forge/logs/daemon.log 2>&1 &
  sleep 3
fi
ps aux | grep forge-daemon

# State preservation check
curl -sX POST localhost:8420/api -H 'Content-Type: application/json' \
  -d '{"method":"recall_stats","params":{}}' > /tmp/forge_post_2a4b.json
diff /tmp/forge_pre_2a4b.json /tmp/forge_post_2a4b.json  # should only differ in uptime fields

# Seed a preference (Remember)
curl -sX POST localhost:8420/api -H 'Content-Type: application/json' \
  -d '{"method":"remember","params":{"memory_type":"preference","title":"test-2a4b","content":"dogfood","valence":"positive","intensity":0.8}}'

# Reaffirm it
curl -sX POST localhost:8420/api -H 'Content-Type: application/json' \
  -d '{"method":"reaffirm_preference","params":{"memory_id":"<ID>"}}'

# CompileContext — verify <preferences> renders
curl -sX POST localhost:8420/api -H 'Content-Type: application/json' \
  -d '{"method":"compile_context","params":{"agent":"claude-code"}}' | grep -A5 '<preferences'

# Doctor check
curl -sX POST localhost:8420/api -H 'Content-Type: application/json' \
  -d '{"method":"doctor"}'

# Log check (no ERROR lines)
tail -100 ~/.forge/logs/daemon.log | grep -iE 'error|panic' || echo "No errors"
```

Success criteria:
- No ERROR logs
- `<preferences>` section renders with the seeded pref at `age="1d"`
- `reaffirmed_at` populated correctly
- Doctor check passes
- Memory count in `/tmp/forge_post_2a4b.json` matches pre-rebuild + 1

---

## 14. §13 resolutions pinned

| §13 item | 2A-4b resolution |
|----------|------------------|
| N-H1 `touch()` exemption layer | SQL predicate `AND memory_type != 'preference'` at `ops.rs:touch()` |
| N-H8 non-pref decay rate reconciliation | Keep separate: fader 0.03 on accessed_at (non-prefs); ranker 0.1 on created_at (non-prefs); documented rationale |
| D7 graph-expanded composition | Inherit from pipeline — same recency_factor applies to graph-expanded rows (no special casing) |
| D2 preference half-life default | 14 days |
| D8 parity test idiom | `ComputeRecencyFactor` handler output == `ops::recency_factor()` direct call, bit-exact via `.to_bits()` |
| Cargo `bench` feature declaration | `[features]\nbench = []` in core; `[features]\nbench = ["forge-core/bench"]` in daemon |
| ReaffirmPreference non-preference validation | Same 5-path validation as FlipPreference (2A-4a) |

---

## 15. Non-goals (explicit deferrals)

- Per-topic half-life (single global value suffices)
- Per-user half-life (2A-6 Forge-Transfer)
- Semantic reaffirmation detection
- Auto-reaffirm on repeated recall
- Auto-flip based on decayed confidence
- Retroactive `reaffirmed_at` population
- Per-domain recency weighting
- Universal `touch()` exemption
- `lesson` vs `decision` decay differentiation
- Aligning fader/ranker constants for non-prefs

---

## 16. Open risks

1. **`decay_memories` SQL shape change** — adding `memory_type, reaffirmed_at` to the SELECT requires a schema-forward query. If a production daemon upgrades from a pre-2A-4b schema (missing `reaffirmed_at`), the SELECT fails. Mitigation: migration adds the column BEFORE the fader-update deploys. Order: T1 (schema) → T7 (fader). Already enforced by TDD sequence.

2. **Regression-guard fixtures may shift** — Forge-Context and Forge-Consolidation calibrations may be sensitive to the envelope change. If composite drops, investigate whether (a) fixture was depending on specific score shape, or (b) feature genuinely regresses something. Mitigation: Pre-compute score distribution on a sample seed before full sweep to catch obvious issues early.

3. **`recency_factor` config threading** — post-RRF site in `recall.rs:381` doesn't currently receive any config. T8 threads `preference_half_life_days: f64` as a trailing primitive parameter through the THREE `hybrid_recall*` signatures (`hybrid_recall`, `hybrid_recall_scoped`, `hybrid_recall_scoped_org`). Call sites (`handler.rs` Recall + BatchRecall arms) load config once per request via `crate::config::load_config().recall.validated().preference_half_life_days`. **Note:** `ContextConfig` and `RecallConfig` are siblings on `ForgeConfig` (config.rs:144-149) — NOT nested. Do not write `ctx_config.recall.X`.

6. **Memory struct field-addition blast radius** — adding `reaffirmed_at` touches at minimum 7 sites (struct, `Memory::new()`, FlipPreference literal in handler.rs, remember+remember_raw in ops.rs, export_memories_org, find_reconsolidation_candidates, MEMORY_ROW_COLUMNS+map_memory_row). Compile errors flag missed sites at T2 acceptance gate.

7. **Cargo feature dep name brittleness** — `daemon/Cargo.toml`'s `bench = ["forge-core/bench"]` requires the dep key to be literally `forge-core`. Verified at `crates/daemon/Cargo.toml:19` as of v2 design write. T0 prerequisite step re-verifies before adding the feature.

8. **TOCTOU race on ReaffirmPreference** — read-side validation runs before UPDATE; a Flip/Supersede could land between. Mitigation: in-SQL preconditions on the UPDATE + `rows_updated == 1` semantic check (see §8 for SQL).

4. **Cargo feature forwarding** — `daemon/Cargo.toml bench = ["forge-core/bench"]` requires `forge-core` dep to be named exactly `forge-core`. Verify dep name before T0.

5. **Dogfood timing sensitivity** — the "1d" bucket depends on `days <= 1.0`. A dogfood run at exactly the 1d boundary could be ambiguous. Mitigation: dogfood test accepts `"1d"` OR `"1w"` for age just after reaffirm (the test is "reaffirm moves the pref into the most recent bucket" — exact bucket name isn't the critical signal).

---

## 17. Success criteria

- `cargo test --workspace` green
- `cargo clippy --workspace -- -W clippy::all -D warnings` zero warnings
- `cargo fmt --all -- --check` clean
- `cargo build --workspace --features bench` compiles
- All 18 TDD tasks pass two-stage review (spec compliance + code quality)
- Regression-guard: Forge-Context + Forge-Consolidation composites ≥ 0.98 on all 5 seeds each (or documented acceptable drift)
- Live daemon dogfood succeeds: state preserved, `<preferences>` renders with correct age bucket, ReaffirmPreference HTTP round-trip works
- Memory handoff written for Phase 2A-4c1 (next in master sequence)

---

## 18. Deliverables

1. This design doc (committed before TDD starts)
2. Two adversarial reviews (Claude + codex CLI) on v1 — address findings before writing-plans
3. Implementation plan at `docs/superpowers/plans/2026-04-18-forge-recency-decay.md`
4. 18 TDD commits on master (subagent-driven-development)
5. `tests/recency_decay_flow.rs` integration test
6. `tests/recency_decay_rollback.rs` schema rollback test
7. Regression-guard updates to bench results docs
8. Live daemon dogfood results doc at `docs/benchmarks/results/forge-recency-decay-2026-04-18.md`
9. Memory handoff file `project_phase_2a4b_complete_2026_04_18.md`
10. MEMORY.md entry with "START HERE NEXT SESSION" → 2A-4c1

---

## 19. Changelog

- **v1 (2026-04-18):** Initial brainstorm output. Addresses master §5 2A-4b scope + §13 resolutions assigned to 2A-4b. Locks D2 (half-life=14), D8 (parity test idiom), and all sub-phase design decisions. Ready for adversarial reviews.

- **v2 (2026-04-19):** Addresses 5 CRITICAL + 11 HIGH + 7 MEDIUM/LOW findings from first-pass adversarial reviews (Claude + Codex):
  - **C1 (both):** ReaffirmPreference SQL — `now_iso()` is a Rust helper not SQLite; in-SQL preconditions added (type, status, org); `rows_updated != 1` treated as race; new "race" stable error message; new "preference was flipped" stable error message for the flipped-status hint.
  - **C2 (Claude):** `ctx_config.recall.preference_half_life_days` doesn't exist — ContextConfig and RecallConfig are siblings on ForgeConfig. T8 threads `preference_half_life_days: f64` as primitive parameter; callers load via `load_config()`.
  - **C3 (Claude):** `hybrid_recall_scoped_org_flipped` doesn't exist as a function. T8 enumerates the 3 real `hybrid_recall*` variants and notes the misnomer.
  - **C4 (Claude):** `decay_memories` SELECT shape change is now additive — original 3 columns at positions 0-2; new columns appended at 3-5. T7 must update the 4 existing decay_memories tests.
  - **C5 (Claude):** Expected −180d formula value corrected to 0.0001354 (4 sig-fig).
  - **H1 (Codex):** `recency_factor` scope narrowed to recall + ComputeRecencyFactor only. Fader uses its own inline type-dispatch (different anchors, different constants).
  - **H2 (Claude):** Hard-fade test uses seed values (`confidence=0.9, created_at=-58d`) not abstract result.
  - **H3 (Claude):** `touch()` test gains negative-control sub-case (mutate type, re-touch).
  - **H4 (Claude):** Clock-skew clamp documented in `recency_factor` doc-comment.
  - **H5 (Claude):** "preference was flipped" stable error message hints at ListFlipped path.
  - **H6 (Claude):** T0 prereq verifies dep name in `crates/daemon/Cargo.toml:19`.
  - **H7 (Claude):** `recency_factor` accepts `now_secs` parameter; parity test uses frozen time for bit-exact equality. `current_epoch_secs()` helper added for production callers.
  - **H8 (Codex):** Memory struct audit task expanded — enumerated 7 mandatory sites plus full grep methodology.
  - **H9 (Codex):** Cargo feature plan §2 corrected to match §7 (daemon forwards `bench = ["forge-core/bench"]`).
  - **H10 (Codex):** XML age bucket vocabulary conformed to master `1d / 1w / 1mo / 6mo+` (4 buckets); `pref_age_bucket` simplified.
  - **H11 (Codex):** `touch()` regression guard expanded to 3 layers — direct unit + Recall integration + CompileContext integration.
  - **M1 (Claude):** New `ops::list_active_preferences` helper introduced; SQL moved out of renderer.
  - **M2 (Claude):** `touch()` empty-list path documented as harmless.
  - **M3 (Claude):** T13 uses textual landmarks ("after `<preferences-flipped>`, before `</forge-dynamic>` close") not line numbers.
  - **M4 (Claude):** Test #2 expected non-pref values updated to 4-sig-fig (0.9048 / 0.3679 / 0.04979).
  - **M5 (Claude):** Dogfood manual-restart branch added (no-watchdog fallback with `nohup`).
  - **M6 (Codex):** writer/tier routing tasks made mandatory (not optional) in T5.
  - **M7 (Codex):** `excluded_layers` doc update at `request.rs:291-295` made mandatory in T5.
  - Header v1 → v2; status updated; counts: 19 TDD tasks (was 18; T6 multi-layer + T10 expanded validation).
