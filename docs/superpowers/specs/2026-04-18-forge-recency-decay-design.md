# Phase 2A-4b — Recency-weighted Preference Decay (design)

**Status:** DRAFT v1 — 2026-04-18. Brainstorming-skill output. Ready for two adversarial reviews (Claude + codex CLI) before writing-plans.
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
- New `<preferences>` section in `compile_dynamic_suffix` at `recall.rs:749+`. Always emitted (bare `<preferences/>` when empty — per master D4). Up to 5 entries, budget-accounted. Age buckets: `1d / 1w / 1mo / 6mo / stale`.
- Excluded-layers key: `"preferences"` (snake_case; matches existing pattern)

**Cargo bench feature declaration**
- Add `[features]\nbench = []` to both `crates/core/Cargo.toml` and `crates/daemon/Cargo.toml`

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
let half_life = ctx_config.recall.preference_half_life_days; // or equivalent — see §8
for result in &mut results {
    result.score *= ops::recency_factor(&result.memory, half_life);
}
```

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
/// This is the single source of truth for both:
/// - Post-RRF ranking in `recall.rs`
/// - Type-dispatched confidence decay in `decay_memories`
/// - The bench-only `Request::ComputeRecencyFactor` (must be bit-exact per parity test)
pub fn recency_factor(memory: &Memory, preference_half_life_days: f64) -> f64 {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as f64;

    let anchor_str = if memory.memory_type == MemoryType::Preference {
        memory.reaffirmed_at.as_deref().unwrap_or(&memory.created_at)
    } else {
        &memory.created_at
    };

    let anchor_secs = parse_timestamp_to_epoch(anchor_str).unwrap_or(now_secs);
    let days = ((now_secs - anchor_secs) / 86400.0).max(0.0);

    if memory.memory_type == MemoryType::Preference {
        let half_life = preference_half_life_days.max(1.0);
        2_f64.powf(-days / half_life)
    } else {
        (-0.1 * days).exp()
    }
}
```

### Expected values (master Dim 6a spec, ±0.0001 absolute)

With `preference_half_life_days = 14.0`:

| days_since_pref_age | Formula | Factor |
|---------------------|---------|--------|
| 1                   | 2^(-1/14)   | **0.9517** |
| 14                  | 2^(-14/14)  | **0.5000** |
| 90                  | 2^(-90/14)  | **0.01161** |
| 180                 | 2^(-180/14) | **0.000135** |

These values are locked for 2A-4d Dim 6a direct formula probe. If the half-life constant changes in a future phase, Dim 6a bench expected values must be recomputed.

### Fader (`decay_memories`) type-dispatch

```rust
// crates/daemon/src/db/ops.rs:837-883 (pre-2A-4b fader)

// BEFORE: single formula for all types
let effective = confidence * (-0.03 * days_since).exp();
if effective < 0.1 {
    // UPDATE status = 'faded'
} else if days_since > 1.0 {
    // UPDATE confidence = effective
}

// AFTER: type-dispatched; reads memory_type from a column SELECT
// Pseudo-SQL (actual structure: select memory_type too, branch in Rust)
for (id, memory_type, confidence, accessed_at, reaffirmed_at, created_at) in &rows {
    let effective = if memory_type == "preference" {
        // Use type-dispatched formula — but fader's anchor is pref-age (not accessed_at)
        // because touch() no longer updates pref accessed_at
        let anchor = reaffirmed_at.as_deref().unwrap_or(&created_at);
        let anchor_secs = parse_timestamp_to_epoch(anchor).unwrap_or(now_secs);
        let days = ((now_secs - anchor_secs) / 86400.0).max(0.0);
        confidence * 2_f64.powf(-days / preference_half_life_days.max(1.0))
    } else {
        // Non-preferences: unchanged fader formula on accessed_at
        let accessed_secs = parse_timestamp_to_epoch(accessed_at).unwrap_or(now_secs);
        let days_since = ((now_secs - accessed_secs) / 86400.0).max(0.0);
        confidence * (-0.03 * days_since).exp()
    };

    if effective < 0.1 && memory_type != "preference" {
        // UPDATE status = 'faded' — prefs exempt from hard-fade
        faded_count += 1;
    } else if /* meaningful decay threshold */ {
        // UPDATE confidence = effective
    }
}
```

**Decay write-back threshold:** use the same `days > 1.0` rule for both types. At 14-day half-life, 1-day decay gives factor `0.9517` (4.8% drop) which is meaningful enough to write back. No separate threshold for prefs; keeps the code branching minimal.

**Fader anchor for prefs = `coalesce(reaffirmed_at, created_at)`** (NOT `accessed_at`). This is critical: after `touch()` exemption lands, `accessed_at` is static for prefs, so using `accessed_at` would give correct decay initially but would silently wedge on any future code that re-enables pref access updates.

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

### Parity test

An explicit test in `ops.rs` mod tests:
1. Seed 2 memories — one `memory_type = 'preference'`, one `memory_type = 'decision'`
2. Call `touch()` on both IDs
3. Assert: preference's `accessed_at` unchanged; decision's `accessed_at` updated
4. Assert: preference's `access_count` unchanged; decision's `access_count` incremented

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
- Fetch memory via `ops::fetch_memory_by_id`
- Validate `memory_type = 'preference'`
- Validate `status = 'active'` (reject if superseded, faded, reverted, conflict)
- Validate cross-org (caller org must match memory's `organization_id`)
- Atomic tx: UPDATE memory SET `reaffirmed_at = now_iso()` WHERE id = ?
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
| Wrong status | `"memory not active (status: {status}, id: {memory_id})"` |
| Cross-org denied | `"cross-org reaffirm denied"` |
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

### Age bucket mapping

```rust
fn pref_age_bucket(days: f64) -> &'static str {
    if days <= 1.0 { "1d" }
    else if days <= 7.0 { "1w" }
    else if days <= 30.0 { "1mo" }
    else if days <= 180.0 { "6mo" }
    else { "stale" }
}
```

### Budget accounting

Same pattern as `<preferences-flipped>` (2A-4a):
- `context_budget` = `ctx_config.budget_chars`
- Each entry ≤ ~200 bytes; 5 entries ≤ ~1000 bytes
- Skip entries that would overflow remaining budget
- Always emit opening/closing tags (even bare `<preferences/>`) — satisfies master infrastructure assertion 10

### Query

```sql
SELECT id, title, valence, intensity, created_at, reaffirmed_at
FROM memory
WHERE memory_type = 'preference'
  AND status = 'active'
  AND COALESCE(organization_id, 'default') = COALESCE(?1, 'default')
ORDER BY COALESCE(reaffirmed_at, created_at) DESC
LIMIT 5
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

1. **`ops::recency_factor` formula** — preferences at `{1, 14, 90, 180}` days match `{0.9517, 0.5, 0.01161, 0.000135}` within 1e-4
2. **`ops::recency_factor` non-prefs** — lesson at `{1, 10, 30}` days matches `{0.905, 0.368, 0.050}` within 1e-3
3. **`ops::recency_factor` reaffirmed overrides created_at** — seed pref with `created_at=-100d`, `reaffirmed_at=-2d`, assert factor ≈ `2^(-2/14) ≈ 0.905`
4. **`touch()` exemption** — preference `accessed_at` unchanged, decision `accessed_at` updated after `touch()`
5. **`decay_memories` pref formula** — pref with confidence 0.9, created_at=-30d, half_life=14 → stored confidence ≈ `0.9 × 2^(-30/14) ≈ 0.206`
6. **`decay_memories` pref hard-fade exemption** — pref with decayed confidence 0.05 stays `status='active'` (not `'faded'`)
7. **`decay_memories` non-pref unchanged** — lesson with confidence 0.9, accessed_at=-30d → stored confidence ≈ `0.9 × exp(-0.03×30) ≈ 0.366`
8. **Age bucket mapping** — `pref_age_bucket({0.5, 1, 1.5, 7, 8, 30, 31, 180, 181})` → `{"1d", "1d", "1w", "1w", "1mo", "1mo", "6mo", "6mo", "stale"}`

### Handler tests (per Request variant)

9. **ReaffirmPreference happy path** — seed pref, Reaffirm, assert `reaffirmed_at` equals `now_iso()` within 2s
10. **ReaffirmPreference 5 validation paths** — memory_id not found, wrong type, status not active, cross-org, tx failure (induce via closed DB)
11. **ReaffirmPreference event emission** — event name `"preference_reaffirmed"`, payload shape, emitted post-commit (subscriber receives after UPDATE succeeds)
12. **ComputeRecencyFactor happy path** — returns factor matching formula
13. **ComputeRecencyFactor + recency_factor parity** — bit-exact equality via `.to_bits()` comparison

### Integration test (`tests/recency_decay_flow.rs`)

14. End-to-end:
    a. Remember a preference (title="prefers vim", memory_type=preference, valence=positive, intensity=0.8)
    b. Backdate `created_at` to 90 days ago via direct SQL UPDATE (test-only helper)
    c. CompileContext → assert `<preferences>` section contains entry with `age="6mo"`
    d. ReaffirmPreference(id)
    e. CompileContext → assert `<preferences>` section contains entry with `age="1d"`
    f. Recall("prefers vim") with include_flipped=false → returns pref
    g. Capture score S1
    h. Re-seed pref with age 1 day (fresh)
    i. Recall("prefers vim") → score S2
    j. Assert S2 > S1 (fresh ranks higher than reaffirmed — but both decay by the same formula now)

### Schema rollback test

15. **`tests/recency_decay_rollback.rs`** — ALTER TABLE DROP COLUMN reaffirmed_at + drop index (if any); verify fresh schema query after rollback

### Regression-guard bench sweeps (MANDATORY pre-merge)

16. **Forge-Context 5 seeds** — see §10
17. **Forge-Consolidation 5 seeds** — see §10

### Dogfood test (live daemon)

18. **Live daemon rebuild + restart + verify preserved state** — see §13

---

## 12. TDD task sequence (writing-plans input)

18 tasks, ordered for compile-time dependencies:

**T0 — Cargo `bench` feature declaration** (prereq)
- Add `[features]\nbench = []` to `crates/core/Cargo.toml`
- Add `[features]\nbench = ["forge-core/bench"]` to `crates/daemon/Cargo.toml`
- `cargo build --workspace --features bench` succeeds

**T1 — Schema: add `reaffirmed_at` column**
- ALTER TABLE in `crates/daemon/src/db/schema.rs` with Phase 2A-4b banner
- No partial index (recall doesn't filter on reaffirmed_at)
- `forge_db_schema_creates_reaffirmed_at` test

**T2 — Memory struct: add `reaffirmed_at` field**
- `crates/core/src/types/memory.rs` — `#[serde(default, skip_serializing_if = "Option::is_none")] pub reaffirmed_at: Option<String>`
- Update `Memory::new()` constructor
- Serde round-trip test (None elides, Some emits)

**T3 — `RecallConfig::preference_half_life_days`**
- Add field at `config.rs:464` RecallConfig struct
- Default `14.0`
- `validated()` clamps to `1..=365`
- Config round-trip test

**T4 — `ops::recency_factor()` helper**
- New `pub fn` in `crates/daemon/src/db/ops.rs` near `decay_memories`
- Pure function — no DB, no SQL
- 4 formula-correctness tests (prefs at 1/14/90/180, non-prefs at 1/10/30)
- Reaffirmation override test

**T5 — Request/Response variants + contract tests**
- Add `Request::ReaffirmPreference` unit variant (1 field)
- Add `Request::ComputeRecencyFactor` under `#[cfg(any(test, feature = "bench"))]`
- Add `ResponseData::PreferenceReaffirmed` and `RecencyFactor` (bench-gated)
- Update `contract_tests.rs` with parameterized vectors
- Update `writer::is_read_only` matches!() for ComputeRecencyFactor
- Update `tier::request_to_feature` if needed

**T6 — `touch()` exemption SQL predicate**
- Add `AND memory_type != 'preference'` to `ops::touch()` UPDATE
- Parity test at `ops.rs` mod tests: 2 memories (pref + decision), `touch()` both, assert pref unchanged

**T7 — `decay_memories` type-dispatched formula**
- SELECT now includes `memory_type, reaffirmed_at` columns
- Rust-side branch: pref uses type-dispatched formula with anchor=`coalesce(reaffirmed_at, created_at)`; non-pref unchanged
- Hard-fade skip for prefs: `if effective < 0.1 && memory_type != 'preference' { UPDATE status='faded' }`
- Pref write-back test (confidence 0.9, created_at=-30d → ~0.206)
- Pref hard-fade exemption test (decayed to 0.05, stays 'active')
- Non-pref unchanged test

**T8 — `recall.rs` post-RRF envelope replacement + config threading**
- Thread `preference_half_life_days: f64` through `hybrid_recall*` signatures (mirrors 2A-4a `include_flipped` threading)
- Update inner `hybrid_recall_scoped_org_flipped` + `hybrid_recall_scoped_org` + `hybrid_recall_scoped` + `hybrid_recall` signatures
- Update call sites: `handler.rs` Recall arm reads from `load_config().recall.validated().preference_half_life_days`; BatchRecall arm same; `compile_dynamic_suffix` does NOT call hybrid_recall so no change there; bench harness + tests update
- Remove `let recency_boost = (-0.1 * days_old).exp(); result.score *= 1.0 + recency_boost * 0.5;`
- Replace with `result.score *= ops::recency_factor(&result.memory, preference_half_life_days);`
- Source-level test: old envelope pattern does not appear (satisfies master assertion 14)
- Also: `ops::decay_memories` signature extended with `preference_half_life_days: f64`; consolidator call site at `consolidator.rs:107` updated to load config and pass half_life

**T9 — ReaffirmPreference handler happy path**
- Match arm in `handler.rs` near FlipPreference
- Atomic tx: `UPDATE memory SET reaffirmed_at = now_iso() WHERE id = ?1`
- Post-commit event emit
- Test: seed + Reaffirm + assert reaffirmed_at set

**T10 — ReaffirmPreference 5 validation paths**
- memory_id not found
- Wrong type (not preference)
- Wrong status (superseded, faded, reverted, conflict)
- Cross-org denied
- Tx failure (closed DB simulation)
- 5 validation tests

**T11 — ReaffirmPreference event emission post-commit**
- Subscribe to events before handler call
- Call handler, wait for event
- Assert event name `"preference_reaffirmed"`, payload shape
- Assert event NOT emitted when validation fails

**T12 — ComputeRecencyFactor handler + parity test**
- Match arm at `handler.rs` under `#[cfg(any(test, feature = "bench"))]`
- Fetch memory, call `ops::recency_factor`, return response
- Parity test: call handler → get F1; call `ops::recency_factor` directly → get F2; assert `F1.to_bits() == F2.to_bits()`

**T13 — `<preferences>` XML section in compile_dynamic_suffix**
- Section after `<preferences-flipped>` at `recall.rs:~1770`
- Excluded-layers check: `"preferences"`
- Query: ORDER BY `coalesce(reaffirmed_at, created_at)` DESC LIMIT 5
- Age bucket helper `pref_age_bucket` (private to module)
- Always emit (bare `<preferences/>` when empty) — D4 compliance
- Budget accounting like `<preferences-flipped>`
- Tests: empty corpus (bare `<preferences/>`), 3 prefs (3 entries), 7 prefs (5 entries), budget-exceeded (entries truncated), excluded-layer path (no section)

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
pkill -TERM -f forge-daemon
sleep 2
# Verify new PID
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

3. **`recency_factor` config threading** — post-RRF site in `recall.rs:381` doesn't currently receive `ctx_config`. T8 threads `preference_half_life_days: f64` as a trailing parameter through `hybrid_recall*` (mirrors 2A-4a's `include_flipped` threading). Call site blast radius: `handler.rs` Recall + BatchRecall arms (load config once per request), bench harness, tests. `decay_memories` similarly gains a `preference_half_life_days: f64` parameter; consolidator reads from loaded config when invoking.

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
