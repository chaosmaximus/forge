# Forge-Valence-Flipping Implementation Plan (Phase 2A-4a)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the daemon capability to explicitly flip a user preference's valence (positive ↔ negative) while preserving the original as flipped-history, so agents see both the new stance AND the prior stance's context.

**Architecture:** A new `Request::FlipPreference` wraps three DB operations (INSERT new + UPDATE old + INSERT edge) in an explicit `conn.transaction()`. The old memory gets a new `valence_flipped_at` timestamp column marking the flip event. Recall grows an opt-in `include_flipped` flag that extends BM25/vector predicates (not post-filter). CompileContext renders a `<preferences-flipped>` XML section with both old and new valences inline.

**Tech Stack:** Rust workspace (crates/core, crates/daemon), rusqlite 0.32 with bundled SQLite 3.46+, ULID for IDs, serde_json for payloads, `tracing` for logs, `anyhow` at the application layer, thiserror for library errors.

**Spec:** [docs/superpowers/specs/2026-04-17-forge-valence-flipping-design.md](../specs/2026-04-17-forge-valence-flipping-design.md) (v2a, both reviewers approved at commit `7cea246`).

**Master context:** [docs/benchmarks/forge-identity-master-design.md](../../benchmarks/forge-identity-master-design.md) §5 2A-4a (v6a at commit `eac6eb9`).

---

## Deviations from spec (explicit, approved)

1. **`store_memory_raw` → `remember_raw` (existing)** — `crates/daemon/src/db/ops.rs:125` already has a no-dedup variant of `remember()`. Reuse it directly; do NOT add a new function with a different name. The spec anticipated this: §4.3 says "if it doesn't already exist, the first TDD task adds it."

2. **Add `ops::fetch_memory_by_id()` helper** — the spec references this function but it doesn't exist in the codebase. T0 adds it as a new helper returning `rusqlite::Result<Option<Memory>>`. Reused by both the refactored `Request::Supersede` handler (T1) and the new `Request::FlipPreference` handler (T6).

3. **Task ordering:** spec lists T0 as "prereq"; the compile-time chain requires T3 (schema ALTER) + T4 (Memory struct fields) to precede T0's full fetch-helper implementation since `fetch_memory_by_id` reads the new columns. The plan reorders: T3 → T4 → T0 → T1 → T2 → T5 → T6 → T7 → T8 → T9 → T10 → T11 → T12 → T13 → T14. Spec task names preserved for traceability.

---

## File structure

| File | Responsibility |
|------|----------------|
| `crates/core/src/types/memory.rs` | Add `superseded_by` + `valence_flipped_at` fields to `Memory` struct with `#[serde(default, skip_serializing_if = "Option::is_none")]`. |
| `crates/core/src/protocol/request.rs` | Add `FlipPreference` and `ListFlipped` variants; extend `Recall` with `include_flipped: Option<bool>` field. |
| `crates/core/src/protocol/response.rs` | Add `PreferenceFlipped`, `FlippedList`, `FlippedMemory` variants/types. |
| `crates/core/src/protocol/contract_tests.rs` | Extend parameterized test vector with 3 new entries + 2 Recall variants (backward compat + include_flipped=true). |
| `crates/daemon/src/db/schema.rs` | Add `ALTER TABLE memory ADD COLUMN valence_flipped_at TEXT` + partial index. |
| `crates/daemon/src/db/ops.rs` | Add `fetch_memory_by_id()`, `supersede_memory_impl()`, `list_flipped_with_targets()`; update row-mapping SELECT fragments that build `Memory` to read the two new columns. |
| `crates/daemon/src/server/handler.rs` | Refactor `Request::Supersede` to call `supersede_memory_impl()`; add `Request::FlipPreference` and `Request::ListFlipped` handlers; thread `include_flipped` through all Recall paths. |
| `crates/daemon/src/recall.rs` | Extend `hybrid_recall()` signature with `include_flipped: bool`; extend BM25 + vector query predicates; add `<preferences-flipped>` XML section in `compile_dynamic_suffix`; extend `compile_dynamic_suffix` signature with `organization_id`. |
| `crates/daemon/tests/flip_preference_flow.rs` | New integration test (T12): Remember → Flip → ListFlipped → Recall(include_flipped) → CompileContext. |

---

## Commit discipline

- **One logical change per commit.** Each task below produces exactly one commit unless noted.
- **Message format:** `feat(scope): description` or `refactor(scope): description` or `test(scope): description`.
- **Scope:** `core` for `crates/core`, `daemon` for `crates/daemon`, `schema` for DB migrations, `recall` for recall logic, `spec` for docs.
- **Before every commit:** run `cargo fmt --all` (silent when clean) and `cargo clippy --workspace -- -W clippy::all -D warnings` (must be 0 warnings).

---

## Task 3: Schema migration — add `valence_flipped_at` column + partial index

**Files:**
- Modify: `crates/daemon/src/db/schema.rs:1203` (add new ALTER after the existing `superseded_by` ALTER)

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/db/schema.rs` inside the existing `#[cfg(test)] mod tests` block (or create one if none exists — check first with `grep -n 'mod tests' crates/daemon/src/db/schema.rs`):

```rust
#[test]
fn test_memory_schema_has_valence_flipped_at_column() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(memory)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    assert!(
        cols.contains(&"valence_flipped_at".to_string()),
        "memory table missing valence_flipped_at column; columns: {cols:?}"
    );
}

#[test]
fn test_memory_schema_has_valence_flipped_at_partial_index() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let indexes: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='memory'")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    assert!(
        indexes.contains(&"idx_memory_valence_flipped_at".to_string()),
        "memory table missing idx_memory_valence_flipped_at; indexes: {indexes:?}"
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-daemon --lib db::schema::tests::test_memory_schema_has_valence_flipped_at -- --nocapture`
Expected: FAIL with `memory table missing valence_flipped_at column`.

- [ ] **Step 3: Add the ALTER + partial index**

In `crates/daemon/src/db/schema.rs`, immediately AFTER line 1203 (`let _ = conn.execute("ALTER TABLE memory ADD COLUMN superseded_by TEXT", []);`), add:

```rust
    // Phase 2A-4a: valence_flipped_at marks preferences that have been superseded
    // via Request::FlipPreference (as opposed to plain Supersede). Used by
    // CompileContext's <preferences-flipped> section and the ListFlipped endpoint.
    let _ = conn.execute(
        "ALTER TABLE memory ADD COLUMN valence_flipped_at TEXT",
        [],
    );
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_memory_valence_flipped_at
             ON memory(valence_flipped_at)
             WHERE valence_flipped_at IS NOT NULL",
        [],
    )?;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-daemon --lib db::schema::tests::test_memory_schema_has_valence_flipped_at -- --nocapture`
Expected: PASS (both tests).

Then run the full schema tests to verify no regressions: `cargo test -p forge-daemon --lib db::schema`
Expected: PASS with 0 failures.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/db/schema.rs
git commit -m "feat(schema): add valence_flipped_at column + partial index (2A-4a T3)"
```

---

## Task 4: Memory struct fields (`superseded_by`, `valence_flipped_at`)

**Files:**
- Modify: `crates/core/src/types/memory.rs:60` (add 2 fields inside `Memory` struct, just before `organization_id` or at end)
- Modify: `crates/core/src/types/memory.rs:67-97` (update `Memory::new()` constructor to initialize new fields)

- [ ] **Step 1: Write the failing test**

Add to `crates/core/src/types/memory.rs` inside `#[cfg(test)] mod tests` (or at the end of the file — verify a `mod tests` exists with `grep -n 'mod tests' crates/core/src/types/memory.rs`; create one if missing):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_struct_has_valence_flipped_at_and_superseded_by_fields() {
        let m = Memory::new(MemoryType::Preference, "test", "test content");
        assert_eq!(m.superseded_by, None);
        assert_eq!(m.valence_flipped_at, None);
    }

    #[test]
    fn test_memory_struct_roundtrips_superseded_by_and_valence_flipped_at() {
        let mut m = Memory::new(MemoryType::Preference, "test", "test content");
        m.superseded_by = Some("01JABCDEF".to_string());
        m.valence_flipped_at = Some("2026-04-17 14:22:00".to_string());

        let json = serde_json::to_string(&m).unwrap();
        let decoded: Memory = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.superseded_by, Some("01JABCDEF".to_string()));
        assert_eq!(decoded.valence_flipped_at, Some("2026-04-17 14:22:00".to_string()));
    }

    #[test]
    fn test_memory_struct_deserializes_old_json_without_new_fields() {
        // Backward compat: old JSON payloads lacking the fields MUST still deserialize.
        let old_json = r#"{
            "id": "01JABC",
            "memory_type": "preference",
            "title": "test",
            "content": "test",
            "confidence": 0.9,
            "status": "active",
            "project": null,
            "tags": [],
            "embedding": null,
            "created_at": "2026-04-17 00:00:00",
            "accessed_at": "2026-04-17 00:00:00",
            "valence": "neutral",
            "intensity": 0.0
        }"#;
        let m: Memory = serde_json::from_str(old_json).unwrap();
        assert_eq!(m.superseded_by, None);
        assert_eq!(m.valence_flipped_at, None);
    }

    #[test]
    fn test_memory_struct_omits_new_fields_when_none() {
        // Forward compat: when None, the serialized JSON must NOT contain these keys
        // (so older deserializers don't trip on unexpected nulls).
        let m = Memory::new(MemoryType::Preference, "test", "test");
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("superseded_by"), "should omit superseded_by when None; json: {json}");
        assert!(!json.contains("valence_flipped_at"), "should omit valence_flipped_at when None; json: {json}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-core --lib types::memory::tests -- --nocapture`
Expected: FAIL with compile error — `superseded_by` / `valence_flipped_at` not fields of `Memory`.

- [ ] **Step 3: Add fields to `Memory` struct**

In `crates/core/src/types/memory.rs`, modify the `Memory` struct (ending at line 61). Add the two new fields AFTER `organization_id`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub content: String,
    pub confidence: f64,
    pub status: MemoryStatus,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub embedding: Option<Vec<f32>>,
    pub created_at: String,
    pub accessed_at: String,
    #[serde(default = "default_valence")]
    pub valence: String,
    #[serde(default)]
    pub intensity: f64,
    #[serde(default)]
    pub hlc_timestamp: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub access_count: u64,
    #[serde(default)]
    pub activation_level: f64,
    #[serde(default)]
    pub alternatives: Vec<String>,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    /// Phase 2A-4a: pointer from a superseded memory to its replacement. Mirrors
    /// the DB column added in Phase 2A-0 (superseded_by). Omitted in JSON when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Phase 2A-4a: set when this memory was flipped (i.e. replaced via
    /// Request::FlipPreference). Distinguishes flip-supersede from plain supersede.
    /// Format matches forge_core::time::now_iso(): "YYYY-MM-DD HH:MM:SS".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valence_flipped_at: Option<String>,
}
```

Then update `Memory::new()` (line 67-97) to initialize both fields to `None`. In the struct literal, add at the end (after `organization_id: None,`):

```rust
            organization_id: None,
            superseded_by: None,
            valence_flipped_at: None,
        }
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-core --lib types::memory::tests -- --nocapture`
Expected: PASS (4 tests).

Then run the full workspace to surface any compile errors from other producers of `Memory`:
Run: `cargo build --workspace`
Expected: success. The added fields have `#[serde(default)]`, and the struct literal update covers `Memory::new()`. Any OTHER `Memory { ... }` literal constructions in the codebase will fail to compile now and must be fixed in this same task (use `..Default::default()` is not possible without `Default` — instead, explicitly add `superseded_by: None, valence_flipped_at: None,` to each).

If the build reveals other literal constructions: grep `Memory\s*\{` across the workspace (`grep -rn "Memory\s*{" crates/ --include="*.rs"`), add the two `None` initializers to each literal, and re-run `cargo build --workspace`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/core/src/types/memory.rs
# Plus any other files touched by the grep-and-fix pass above:
# git add <whatever else>
git commit -m "feat(core): add superseded_by + valence_flipped_at to Memory struct (2A-4a T4)"
```

---

## Task 0: `fetch_memory_by_id()` helper + row-mapper updates

**Purpose:** Centralize the SELECT → Memory mapping so every caller (Supersede handler, FlipPreference handler, row-mapping call sites in recall.rs and elsewhere) reads the new columns consistently. The spec assumed this helper exists; it does not, so we add it here.

**Files:**
- Modify: `crates/daemon/src/db/ops.rs` (add `fetch_memory_by_id()` as a new public function)
- Modify: `crates/daemon/src/db/ops.rs` (update any existing `SELECT ... FROM memory WHERE id = ?` pattern that maps to Memory to include the two new columns)
- Modify: `crates/daemon/src/recall.rs` (update any row-to-Memory mapping in `hybrid_recall_scoped_org` or its helpers to read the new columns — find via `grep -n "superseded_by" crates/daemon/src/recall.rs` to locate existing SELECTs; they MUST now also select `valence_flipped_at`)

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/db/ops.rs` inside its existing `#[cfg(test)] mod tests` block (verify with `grep -n 'mod tests' crates/daemon/src/db/ops.rs`; create one if missing):

```rust
#[test]
fn test_fetch_memory_by_id_returns_memory_when_exists() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let mut m = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Preference,
        "tabs over spaces",
        "prefer tabs",
    );
    m.id = "01JABCDEF".to_string();
    remember(&conn, &m).unwrap();

    let fetched = fetch_memory_by_id(&conn, "01JABCDEF").unwrap();
    assert!(fetched.is_some(), "should fetch memory by id");
    let fetched = fetched.unwrap();
    assert_eq!(fetched.id, "01JABCDEF");
    assert_eq!(fetched.title, "tabs over spaces");
    assert_eq!(fetched.superseded_by, None);
    assert_eq!(fetched.valence_flipped_at, None);
}

#[test]
fn test_fetch_memory_by_id_returns_none_when_absent() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let fetched = fetch_memory_by_id(&conn, "does-not-exist").unwrap();
    assert!(fetched.is_none(), "missing id should return None");
}

#[test]
fn test_fetch_memory_by_id_reads_flipped_columns() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    // Insert a pretend-flipped memory via raw SQL so we control the columns exactly.
    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, superseded_by, valence_flipped_at)
         VALUES (?1, 'preference', 'tabs', 'prefer tabs', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.8, ?2, ?3)",
        rusqlite::params!["01OLDMEM", "01NEWMEM", "2026-04-17 14:22:00"],
    ).unwrap();

    let fetched = fetch_memory_by_id(&conn, "01OLDMEM").unwrap().unwrap();
    assert_eq!(fetched.superseded_by, Some("01NEWMEM".to_string()));
    assert_eq!(fetched.valence_flipped_at, Some("2026-04-17 14:22:00".to_string()));
}

#[test]
fn test_remember_raw_inserts_without_dedup() {
    // Sanity test pinning the existing remember_raw behavior: it must NOT dedup
    // against a matching (title, type, project, org) active memory. FlipPreference
    // depends on this bypass to avoid UPSERTing the old memory.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let mut m1 = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Preference,
        "tabs over spaces",
        "prefer tabs",
    );
    m1.id = "01FIRST".to_string();
    remember(&conn, &m1).unwrap();

    let mut m2 = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Preference,
        "tabs over spaces",  // same title
        "new content",
    );
    m2.id = "01SECOND".to_string();
    remember_raw(&conn, &m2).unwrap();

    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory WHERE title = 'tabs over spaces'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2, "remember_raw must not dedup");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-daemon --lib db::ops::tests::test_fetch_memory_by_id -- --nocapture`
Expected: FAIL with `fetch_memory_by_id not found`.

Run: `cargo test -p forge-daemon --lib db::ops::tests::test_remember_raw_inserts_without_dedup -- --nocapture`
Expected: PASS (confirming remember_raw already exists and bypasses dedup; this is the pin-test).

- [ ] **Step 3: Implement `fetch_memory_by_id()`**

In `crates/daemon/src/db/ops.rs`, add a new public function. Place it after `remember_raw()` (around line 152). This function centralizes the SELECT → Memory mapping with ALL columns including the new ones.

```rust
/// Fetch a single memory by id. Returns None if no row matches.
///
/// Reads every column of the memory table and constructs a full Memory struct,
/// including the Phase 2A-4a `superseded_by` and `valence_flipped_at` columns.
/// Centralizing the mapping here means future column additions only need to
/// touch one place.
pub fn fetch_memory_by_id(
    conn: &Connection,
    id: &str,
) -> rusqlite::Result<Option<forge_core::types::memory::Memory>> {
    use forge_core::types::memory::{Memory, MemoryStatus, MemoryType};
    use rusqlite::OptionalExtension;

    conn.query_row(
        "SELECT id, memory_type, title, content, confidence, status, project, tags,
                created_at, accessed_at, valence, intensity, hlc_timestamp, node_id,
                session_id, access_count, activation_level, alternatives, participants,
                organization_id, superseded_by, valence_flipped_at
           FROM memory
          WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            let memory_type_str: String = row.get(1)?;
            let status_str: String = row.get(5)?;
            let tags_json: String = row.get(7)?;
            let alternatives_json: String = row.get(17)?;
            let participants_json: String = row.get(18)?;

            Ok(Memory {
                id: row.get(0)?,
                memory_type: match memory_type_str.as_str() {
                    "decision" => MemoryType::Decision,
                    "lesson" => MemoryType::Lesson,
                    "pattern" => MemoryType::Pattern,
                    "preference" => MemoryType::Preference,
                    "protocol" => MemoryType::Protocol,
                    other => return Err(rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("unknown memory_type: {other}"),
                        )),
                    )),
                },
                title: row.get(2)?,
                content: row.get(3)?,
                confidence: row.get(4)?,
                status: match status_str.as_str() {
                    "active" => MemoryStatus::Active,
                    "superseded" => MemoryStatus::Superseded,
                    "reverted" => MemoryStatus::Reverted,
                    "faded" => MemoryStatus::Faded,
                    "conflict" => MemoryStatus::Conflict,
                    other => return Err(rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("unknown status: {other}"),
                        )),
                    )),
                },
                project: row.get(6)?,
                tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                embedding: None,  // embeddings live in memory_vec; not part of this view
                created_at: row.get(8)?,
                accessed_at: row.get(9)?,
                valence: row.get(10)?,
                intensity: row.get(11)?,
                hlc_timestamp: row.get(12)?,
                node_id: row.get(13)?,
                session_id: row.get(14)?,
                access_count: row.get::<_, i64>(15)? as u64,
                activation_level: row.get::<_, Option<f64>>(16)?.unwrap_or(0.0),
                alternatives: serde_json::from_str(&alternatives_json).unwrap_or_default(),
                participants: serde_json::from_str(&participants_json).unwrap_or_default(),
                organization_id: row.get(19)?,
                superseded_by: row.get(20)?,
                valence_flipped_at: row.get(21)?,
            })
        },
    )
    .optional()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-daemon --lib db::ops::tests -- --nocapture`
Expected: PASS (all 4 tests: `test_fetch_memory_by_id_returns_memory_when_exists`, `test_fetch_memory_by_id_returns_none_when_absent`, `test_fetch_memory_by_id_reads_flipped_columns`, `test_remember_raw_inserts_without_dedup`).

Then run the full workspace test suite to check for any row-mapping code paths that now fail because they don't know about the new columns:
Run: `cargo test -p forge-daemon --lib`
Expected: PASS. If any failures surface from row-mapping call sites in other modules (e.g., `recall.rs` row mappers, `manas.rs` mappers), update them to read the two new columns from their respective SELECTs.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/db/ops.rs
# Plus any other row-mapping files touched:
# git add crates/daemon/src/recall.rs crates/daemon/src/db/manas.rs
git commit -m "feat(daemon): add ops::fetch_memory_by_id() helper + row-mapper updates (2A-4a T0)"
```

---

## Task 1: Extract `supersede_memory_impl()` helper — plain supersede branch (no flip)

**Files:**
- Modify: `crates/daemon/src/db/ops.rs` (add `supersede_memory_impl()` function + `OpError` enum extension)
- Modify: `crates/daemon/src/server/handler.rs:718-786` (refactor inline `Request::Supersede` to call the helper)

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/db/ops.rs` test block:

```rust
#[test]
fn test_supersede_memory_impl_marks_superseded_creates_edge() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let mut old = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Decision,
        "old",
        "old content",
    );
    old.id = "01OLDID".to_string();
    remember(&conn, &old).unwrap();

    let mut new = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Decision,
        "new",
        "new content",
    );
    new.id = "01NEWID".to_string();
    remember(&conn, &new).unwrap();

    supersede_memory_impl(&conn, "01OLDID", "01NEWID", None, None).unwrap();

    // Old memory status + superseded_by
    let (status, superseded_by): (String, Option<String>) = conn
        .query_row(
            "SELECT status, superseded_by FROM memory WHERE id = ?1",
            rusqlite::params!["01OLDID"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "superseded");
    assert_eq!(superseded_by, Some("01NEWID".to_string()));

    // Supersedes edge created
    let edge_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edge WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'supersedes'",
            rusqlite::params!["01NEWID", "01OLDID"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(edge_count, 1);
}

#[test]
fn test_supersede_memory_impl_rejects_missing_old() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let result = supersede_memory_impl(&conn, "does-not-exist", "also-missing", None, None);
    match result {
        Err(OpError::OldMemoryNotActive { id }) => assert_eq!(id, "does-not-exist"),
        other => panic!("expected OldMemoryNotActive, got {other:?}"),
    }
}

#[test]
fn test_supersede_memory_impl_respects_org_scope() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let mut old = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Decision,
        "old",
        "content",
    );
    old.id = "01OLDID".to_string();
    old.organization_id = Some("org-a".to_string());
    remember(&conn, &old).unwrap();

    // Caller claims org-b; should be rejected.
    let result = supersede_memory_impl(&conn, "01OLDID", "01NEWID", Some("org-b"), None);
    assert!(matches!(result, Err(OpError::OldMemoryNotActive { .. })));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-daemon --lib db::ops::tests::test_supersede_memory_impl -- --nocapture`
Expected: FAIL with `cannot find function supersede_memory_impl` or `cannot find OpError`.

- [ ] **Step 3: Add `OpError` enum + `supersede_memory_impl()` function**

In `crates/daemon/src/db/ops.rs`, near the top (after the `use` statements), add the `OpError` enum if it doesn't already exist (grep first: `grep -n 'enum OpError' crates/daemon/src/db/ops.rs`):

```rust
/// Typed errors returned by library-layer db::ops helpers that want the caller
/// to distinguish semantic failures from raw DB errors.
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("old memory not found or not active: {id}")]
    OldMemoryNotActive { id: String },

    #[error(transparent)]
    DbError(#[from] rusqlite::Error),
}
```

Then add `supersede_memory_impl()` after `fetch_memory_by_id()` (from T0):

```rust
/// Mark `old_id` as superseded by `new_id`. Creates a 'supersedes' edge from new to old.
/// If `valence_flipped_at` is `Some(ts)`, additionally sets that column on the old row;
/// this is the flip-specific codepath used by Request::FlipPreference. When None, behaves
/// as a plain supersede identical to the pre-refactor Request::Supersede logic.
///
/// Caller is responsible for transaction scope. Both statements (UPDATE memory, INSERT edge)
/// execute as direct rusqlite calls; wrap them in `conn.transaction()` at the caller level
/// for atomicity.
///
/// Returns `OpError::OldMemoryNotActive` when no row matched the UPDATE (either id missing,
/// status != 'active', or organization_id scope failed).
pub fn supersede_memory_impl(
    conn: &Connection,
    old_id: &str,
    new_id: &str,
    organization_id: Option<&str>,
    valence_flipped_at: Option<&str>,
) -> Result<(), OpError> {
    let now = forge_core::time::now_iso();
    let org = organization_id.unwrap_or("default");

    // UPDATE branch depends on whether valence_flipped_at is being set.
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
        return Err(OpError::OldMemoryNotActive {
            id: old_id.to_string(),
        });
    }

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

Then refactor `Request::Supersede` handler at `crates/daemon/src/server/handler.rs:718-786`. Replace the existing block with:

```rust
        Request::Supersede { old_id, new_id } => {
            // Derive org scope from the old memory's session (unchanged).
            let supersede_org_id = {
                let mem_session: Option<String> = state
                    .conn
                    .query_row(
                        "SELECT session_id FROM memory WHERE id = ?1",
                        rusqlite::params![old_id],
                        |row| row.get(0),
                    )
                    .ok();
                get_session_org_id(&state.conn, mem_session.as_deref())
            };

            // Pre-fetch to distinguish "old missing" from "new missing" (preserves
            // the current handler's per-ID error message).
            let old = match ops::fetch_memory_by_id(&state.conn, &old_id) {
                Ok(Some(m)) => m,
                Ok(None) => {
                    return Response::Error {
                        message: format!("old memory not found or already superseded: {old_id}"),
                    }
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("supersede failed: {e}"),
                    }
                }
            };
            if old.status != forge_core::types::memory::MemoryStatus::Active {
                return Response::Error {
                    message: format!("old memory not found or already superseded: {old_id}"),
                };
            }
            let new_exists: bool = state
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM memory WHERE id = ?1 AND status = 'active')",
                    rusqlite::params![&new_id],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            if !new_exists {
                return Response::Error {
                    message: format!("new memory not found: {new_id}"),
                };
            }

            match ops::supersede_memory_impl(
                &state.conn,
                &old_id,
                &new_id,
                supersede_org_id.as_deref(),
                None,
            ) {
                Ok(()) => {
                    crate::events::emit(
                        &state.events,
                        "memory_superseded",
                        serde_json::json!({
                            "old_id": old_id,
                            "new_id": new_id,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::Superseded { old_id, new_id },
                    }
                }
                Err(ops::OpError::OldMemoryNotActive { .. }) => Response::Error {
                    message: format!("memory not found or already superseded: {old_id}"),
                },
                Err(ops::OpError::DbError(e)) => Response::Error {
                    message: format!("supersede failed: {e}"),
                },
            }
        }
```

Verify `thiserror` is in `crates/daemon/Cargo.toml` dependencies. Check with `grep thiserror crates/daemon/Cargo.toml`. If missing, add:

```toml
thiserror = { workspace = true }
```

And verify the workspace root `Cargo.toml` declares it under `[workspace.dependencies]`:
```bash
grep thiserror Cargo.toml
```
If absent, add `thiserror = "1.0"` under the workspace deps.

**R1 mitigation (1-line add):** before the inline supersede SQL at `crates/daemon/src/workers/consolidator.rs:1499`, add a comment:

```rust
// TODO(2A-4+): migrate to ops::supersede_memory_impl() — see docs/superpowers/specs/2026-04-17-forge-valence-flipping-design.md §14 R1.
```

Spec §14 R1 calls this out as deliberate deferral; the TODO makes it discoverable via grep later.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-daemon --lib db::ops::tests::test_supersede_memory_impl -- --nocapture`
Expected: PASS (3 tests).

Run the existing Supersede handler tests to verify no regression:
Run: `cargo test -p forge-daemon supersede`
Expected: PASS. Specifically the existing `Request::Supersede` integration tests at `handler.rs:7297` should still pass.

Run the full workspace test suite:
Run: `cargo test --workspace`
Expected: PASS with 0 failures.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/db/ops.rs crates/daemon/src/server/handler.rs crates/daemon/src/workers/consolidator.rs
# If Cargo.toml touched:
# git add crates/daemon/Cargo.toml Cargo.toml
git commit -m "refactor(daemon): extract supersede_memory_impl() helper (2A-4a T1)"
```

---

## Task 2: `supersede_memory_impl()` — flip branch test

**Purpose:** Pin the flip-specific branch behavior (`valence_flipped_at` argument is Some). The helper code in T1 already handles the branching; T2 is a pure test-first pin to catch regressions.

**Files:**
- Modify: `crates/daemon/src/db/ops.rs` (add 1 new test — no new production code required since T1 already implemented the branch)

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/db/ops.rs` test block:

```rust
#[test]
fn test_supersede_memory_impl_with_flip_sets_valence_flipped_at() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let mut old = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Preference,
        "tabs over spaces",
        "prefer tabs",
    );
    old.id = "01OLDID".to_string();
    old.valence = "positive".to_string();
    remember(&conn, &old).unwrap();

    let mut new = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Preference,
        "tabs over spaces (flipped)",
        "[flipped from positive to negative]: prefer tabs",
    );
    new.id = "01NEWID".to_string();
    new.valence = "negative".to_string();
    remember_raw(&conn, &new).unwrap();

    supersede_memory_impl(
        &conn,
        "01OLDID",
        "01NEWID",
        None,
        Some("2026-04-17 14:22:00"),
    )
    .unwrap();

    let (status, superseded_by, flipped_at): (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT status, superseded_by, valence_flipped_at FROM memory WHERE id = ?1",
            rusqlite::params!["01OLDID"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "superseded");
    assert_eq!(superseded_by, Some("01NEWID".to_string()));
    assert_eq!(flipped_at, Some("2026-04-17 14:22:00".to_string()));
}

#[test]
fn test_supersede_memory_impl_without_flip_leaves_valence_flipped_at_null() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    let mut old = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Decision,
        "old",
        "content",
    );
    old.id = "01OLDID".to_string();
    remember(&conn, &old).unwrap();

    supersede_memory_impl(&conn, "01OLDID", "01NEWID", None, None).unwrap();

    let flipped_at: Option<String> = conn
        .query_row(
            "SELECT valence_flipped_at FROM memory WHERE id = ?1",
            rusqlite::params!["01OLDID"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(flipped_at, None);
}
```

- [ ] **Step 2: Run tests to verify they pass (already covered by T1's implementation)**

Run: `cargo test -p forge-daemon --lib db::ops::tests::test_supersede_memory_impl -- --nocapture`
Expected: PASS (2 new tests + 3 from T1 = 5 total).

If `test_supersede_memory_impl_with_flip_sets_valence_flipped_at` fails, T1's branch for `valence_flipped_at = Some(...)` is incorrect — verify the UPDATE branch in the helper.

- [ ] **Step 3: Commit (pin-test only, no new production code)**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/db/ops.rs
git commit -m "test(daemon): pin supersede_memory_impl flip branch behavior (2A-4a T2)"
```

---

## Task 5: Request/Response variants + contract tests

**Files:**
- Modify: `crates/core/src/protocol/request.rs` (add `FlipPreference` + `ListFlipped` variants; extend `Recall` with `include_flipped` field)
- Modify: `crates/core/src/protocol/response.rs` (add `FlippedMemory` type + `PreferenceFlipped` + `FlippedList` variants under `ResponseData`)
- Modify: `crates/core/src/protocol/contract_tests.rs` (extend parameterized test vector)

- [ ] **Step 1: Write the failing test**

Add to `crates/core/src/protocol/contract_tests.rs` inside the existing `test_parameterized_variants_method_names` test (around line 74 — find via `grep -n 'test_parameterized_variants_method_names' crates/core/src/protocol/contract_tests.rs`). Add these entries to the `cases` vec:

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

Also extend the existing `Recall` case to cover `include_flipped`. Find the existing `"recall"` case in the same vec and add a second `Recall` case immediately after it:

```rust
            (
                "recall",
                Request::Recall {
                    query: "test".into(),
                    memory_type: None,
                    project: None,
                    limit: Some(10),
                    layer: None,
                    since: None,
                    include_flipped: Some(true),
                },
            ),
```

(If the existing Recall case doesn't have `include_flipped: None` explicitly, that's expected — `#[serde(default)]` fills it in.)

Additionally, add a new standalone test for the ResponseData variants:

```rust
    #[test]
    fn test_preference_flipped_response_variant_roundtrips() {
        use crate::protocol::response::{Response, ResponseData};
        let resp = Response::Ok {
            data: ResponseData::PreferenceFlipped {
                old_id: "01OLD".into(),
                new_id: "01NEW".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                flipped_at: "2026-04-17 14:22:00".into(),
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, decoded);
    }

    #[test]
    fn test_flipped_list_response_variant_roundtrips() {
        use crate::protocol::response::{FlippedMemory, Response, ResponseData};
        use crate::types::memory::{Memory, MemoryType};
        let mut m = Memory::new(MemoryType::Preference, "tabs", "prefer tabs");
        m.valence_flipped_at = Some("2026-04-17 14:22:00".into());
        m.superseded_by = Some("01NEW".into());
        let resp = Response::Ok {
            data: ResponseData::FlippedList {
                items: vec![FlippedMemory {
                    old: m,
                    flipped_to_id: "01NEW".into(),
                    flipped_at: "2026-04-17 14:22:00".into(),
                }],
            },
        };
        let json = serde_json::to_string(&resp).unwrap();
        let decoded: Response = serde_json::from_str(&json).unwrap();
        assert_eq!(resp, decoded);
    }
```

Also ensure `Response` derives `PartialEq` for the assert — check with `grep -n 'derive.*Response' crates/core/src/protocol/response.rs`. If `PartialEq` is missing, this test's `assert_eq!` won't compile; use a JSON-equality check instead:
```rust
        let reserialized = serde_json::to_string(&decoded).unwrap();
        assert_eq!(json, reserialized);
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-core --lib protocol::contract_tests -- --nocapture`
Expected: FAIL with `variant FlipPreference not found` compile error.

- [ ] **Step 3: Add Request variants**

In `crates/core/src/protocol/request.rs`, after the `Recall` variant (ending around line 67), add the `include_flipped` field to `Recall`:

```rust
    Recall {
        query: String,
        memory_type: Option<MemoryType>,
        project: Option<String>,
        limit: Option<usize>,
        #[serde(default)]
        layer: Option<String>,
        #[serde(default)]
        since: Option<String>,
        /// Phase 2A-4a: when Some(true), include superseded-and-flipped preferences
        /// in the candidate set. Default (None or Some(false)) matches pre-2A-4a behavior.
        #[serde(default)]
        include_flipped: Option<bool>,
    },
```

After `Supersede` (line 72-75), add the two new variants:

```rust
    /// Phase 2A-4a: flip a user preference's valence, preserving the original as flipped-history.
    /// Creates a new memory with `new_valence` and marks the old as superseded with
    /// `valence_flipped_at` set to the flip timestamp.
    FlipPreference {
        memory_id: String,
        new_valence: String,
        new_intensity: f64,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Phase 2A-4a: list preferences whose valence was flipped (i.e. superseded via FlipPreference).
    ListFlipped {
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
```

In `crates/core/src/protocol/response.rs`, add the `FlippedMemory` struct and response variants. Locate the existing `ResponseData` enum (find via `grep -n 'pub enum ResponseData' crates/core/src/protocol/response.rs`); add new variants inside. Also add the `FlippedMemory` struct at the top-level of the module:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlippedMemory {
    pub old: crate::types::memory::Memory,
    pub flipped_to_id: String,
    pub flipped_at: String,
}
```

Inside `ResponseData`, add:

```rust
    /// Phase 2A-4a: response for Request::FlipPreference.
    PreferenceFlipped {
        old_id: String,
        new_id: String,
        new_valence: String,
        new_intensity: f64,
        flipped_at: String,
    },
    /// Phase 2A-4a: response for Request::ListFlipped.
    FlippedList {
        items: Vec<FlippedMemory>,
    },
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-core --lib protocol::contract_tests -- --nocapture`
Expected: PASS.

Run the full workspace: `cargo build --workspace`
Expected: likely fails with "non-exhaustive match on Request/ResponseData" at handler.rs and elsewhere. Run `cargo build --workspace 2>&1 | grep -A2 'non-exhaustive'` to find call sites; temporarily add `_ => todo!("2A-4a T6-T9")` arms at each missing location so the build passes — these will be replaced in T6, T9, T10. Record which handler `match` blocks got the temp `todo!()` in the commit message so the next tasks know where to look.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/core/src/protocol/request.rs crates/core/src/protocol/response.rs crates/core/src/protocol/contract_tests.rs crates/daemon/src/server/handler.rs
git commit -m "feat(core): add FlipPreference/ListFlipped request variants + include_flipped Recall field (2A-4a T5)"
```

---

## Task 6: `Request::FlipPreference` happy path handler

**Files:**
- Modify: `crates/daemon/src/server/handler.rs` (replace the `todo!()` from T5 with a real FlipPreference handler)

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/server/handler.rs`'s existing test module (find via `grep -n '#\[cfg(test)\]' crates/daemon/src/server/handler.rs`):

```rust
    #[test]
    fn test_flip_preference_creates_new_memory_with_opposite_valence() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        // Arrange: store a preference
        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "tabs over spaces",
            "prefer tabs",
        );
        pref.id = "01PREF".to_string();
        pref.valence = "positive".to_string();
        pref.intensity = 0.7;
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        // Act: flip it
        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: Some("team switched to spaces".into()),
            },
        );

        // Assert: response carries the flipped data
        match resp {
            forge_core::protocol::Response::Ok { data } => match data {
                forge_core::protocol::ResponseData::PreferenceFlipped {
                    old_id,
                    new_id,
                    new_valence,
                    new_intensity,
                    flipped_at,
                } => {
                    assert_eq!(old_id, "01PREF");
                    assert_ne!(new_id, "01PREF");
                    assert_eq!(new_valence, "negative");
                    assert!((new_intensity - 0.8).abs() < 1e-9);
                    assert_eq!(flipped_at.len(), 19); // "YYYY-MM-DD HH:MM:SS"
                }
                other => panic!("expected PreferenceFlipped, got {other:?}"),
            },
            forge_core::protocol::Response::Error { message } => {
                panic!("flip failed: {message}")
            }
        }

        // Assert: old memory marked superseded with valence_flipped_at set
        let (status, superseded_by, flipped_at): (String, Option<String>, Option<String>) = state
            .conn
            .query_row(
                "SELECT status, superseded_by, valence_flipped_at FROM memory WHERE id = ?1",
                rusqlite::params!["01PREF"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "superseded");
        assert!(superseded_by.is_some());
        assert!(flipped_at.is_some());

        // Assert: new memory has opposite valence and annotated content
        let new_id = superseded_by.unwrap();
        let new = crate::db::ops::fetch_memory_by_id(&state.conn, &new_id).unwrap().unwrap();
        assert_eq!(new.valence, "negative");
        assert!((new.intensity - 0.8).abs() < 1e-9);
        assert!(new.content.starts_with("[flipped from positive to negative at "));
        assert!(new.content.contains("prefer tabs"));
        assert_eq!(new.status, forge_core::types::memory::MemoryStatus::Active);
        assert_eq!(new.alternatives, Vec::<String>::new());
        assert_eq!(new.participants, Vec::<String>::new());

        // Assert: supersedes edge from new to old
        let edge_count: i64 = state
            .conn
            .query_row(
                "SELECT COUNT(*) FROM edge WHERE from_id = ?1 AND to_id = ?2 AND edge_type = 'supersedes'",
                rusqlite::params![&new_id, "01PREF"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(edge_count, 1);
    }
```

Handler tests construct a fresh in-memory daemon via `DaemonState::new(":memory:")` (see pattern at `handler.rs:5775`). This call internally invokes `create_schema` — no separate schema setup needed. `handle_request` takes `&mut DaemonState` (see `handler.rs:233`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p forge-daemon test_flip_preference_creates_new_memory_with_opposite_valence -- --nocapture`
Expected: FAIL — either compile error on the `todo!()` arm or panic from `todo!()`.

- [ ] **Step 3: Implement the FlipPreference handler arm**

Replace the `todo!("2A-4a T6")` arm in `handle_request` with the real handler. Place it adjacent to the existing `Request::Supersede` arm (around handler.rs:786):

```rust
        Request::FlipPreference {
            memory_id,
            new_valence,
            new_intensity,
            reason,
        } => {
            // Step 2: Validate inputs before touching the DB.
            if !matches!(new_valence.as_str(), "positive" | "negative" | "neutral") {
                return Response::Error {
                    message: format!(
                        "new_valence must be positive | negative | neutral (got: {new_valence})"
                    ),
                };
            }
            if !new_intensity.is_finite() || !(0.0..=1.0).contains(&new_intensity) {
                return Response::Error {
                    message: format!(
                        "new_intensity must be finite in [0.0, 1.0] (got: {new_intensity})"
                    ),
                };
            }

            // Step 3: Load the old preference.
            let old = match ops::fetch_memory_by_id(&state.conn, &memory_id) {
                Ok(Some(m)) => m,
                Ok(None) => {
                    return Response::Error {
                        message: format!("memory_id not found: {memory_id}"),
                    }
                }
                Err(e) => {
                    return Response::Error {
                        message: format!("flip failed: {e}"),
                    }
                }
            };
            if old.memory_type != forge_core::types::memory::MemoryType::Preference {
                let got = format!("{:?}", old.memory_type).to_lowercase();
                return Response::Error {
                    message: format!("memory_type must be preference for flip (got: {got})"),
                };
            }
            if old.status != forge_core::types::memory::MemoryStatus::Active {
                return Response::Error {
                    message: format!("memory already superseded (id: {memory_id})"),
                };
            }

            // Step 4: Cross-org scope guard.
            let caller_org = {
                let mem_session_opt: Option<String> = state
                    .conn
                    .query_row(
                        "SELECT session_id FROM memory WHERE id = ?1",
                        rusqlite::params![&memory_id],
                        |row| row.get(0),
                    )
                    .ok();
                get_session_org_id(&state.conn, mem_session_opt.as_deref())
            };
            if caller_org.as_deref() != old.organization_id.as_deref()
                && caller_org.is_some()
                && old.organization_id.is_some()
            {
                return Response::Error {
                    message: "cross-org flip denied".to_string(),
                };
            }
            if old.valence == new_valence {
                return Response::Error {
                    message: format!("memory already superseded (id: {memory_id})"),
                };
            }

            // Step 5: Timestamp and new-memory synthesis.
            let now = forge_core::time::now_iso();
            let reason_suffix = reason
                .as_ref()
                .map(|r| format!(" (reason: {r})"))
                .unwrap_or_default();

            let new_id = ulid::Ulid::new().to_string();
            let new_content = format!(
                "[flipped from {old_valence} to {new_valence} at {now}]{reason_suffix}: {old_content}",
                old_valence = old.valence,
                new_valence = new_valence,
                now = now,
                reason_suffix = reason_suffix,
                old_content = old.content,
            );
            let new_confidence = old.confidence.max(0.5).min(1.0);

            let new_memory = forge_core::types::memory::Memory {
                id: new_id.clone(),
                memory_type: forge_core::types::memory::MemoryType::Preference,
                title: old.title.clone(),
                content: new_content,
                confidence: new_confidence,
                status: forge_core::types::memory::MemoryStatus::Active,
                project: old.project.clone(),
                tags: old.tags.clone(),
                embedding: None,
                created_at: now.clone(),
                accessed_at: now.clone(),
                valence: new_valence.clone(),
                intensity: new_intensity,
                hlc_timestamp: state.hlc.now(),
                node_id: old.node_id.clone(),
                session_id: old.session_id.clone(),
                access_count: 0,
                activation_level: 0.0,
                alternatives: Vec::new(),
                participants: Vec::new(),
                organization_id: old.organization_id.clone(),
                superseded_by: None,
                valence_flipped_at: None,
            };

            // Step 7: Atomic transaction — INSERT new + UPDATE+edge via helper.
            //
            // We use `conn.transaction()` directly on a fresh short-lived borrow,
            // committing explicitly. On early return from any `?`, the transaction
            // is dropped and auto-rolls-back.
            let result: Result<(), ops::OpError> = (|| {
                let tx = state.conn.unchecked_transaction()?;
                ops::remember_raw(&tx, &new_memory)?;
                ops::supersede_memory_impl(
                    &tx,
                    &old.id,
                    &new_memory.id,
                    old.organization_id.as_deref(),
                    Some(&now),
                )?;
                tx.commit()?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    // Step 8: Emit event AFTER commit.
                    crate::events::emit(
                        &state.events,
                        "preference_flipped",
                        serde_json::json!({
                            "old_id": old.id,
                            "new_id": new_memory.id,
                            "new_valence": new_valence,
                            "new_intensity": new_intensity,
                            "reason": reason.as_deref().unwrap_or(""),
                            "flipped_at": now,
                        }),
                    );
                    Response::Ok {
                        data: ResponseData::PreferenceFlipped {
                            old_id: old.id,
                            new_id: new_memory.id,
                            new_valence,
                            new_intensity,
                            flipped_at: now,
                        },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("flip transaction failed: {e}"),
                },
            }
        }
```

Notes:
- `state.conn` is a direct `Connection` on `DaemonState` (see `handler.rs:13`). `unchecked_transaction()` is the rusqlite method that takes `&Connection` instead of `&mut Connection` and returns a `Transaction` with commit-on-drop-unless-committed semantics; this matches the read-only-ish borrowing pattern used elsewhere in handler arms.
- `state.hlc.now()` is the correct HLC generator (see `sync.rs:38`). Its output format is `"{wall_ms}-{counter:010}-{node_id}"`.
- `node_id` is inherited from the old memory — the flip is a continuation of the same logical thread. If the old memory's `node_id` is empty (e.g. an older memory predating HLC), `remember_raw()` happily stores an empty string; no fallback needed.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p forge-daemon test_flip_preference_creates_new_memory_with_opposite_valence -- --nocapture`
Expected: PASS.

Run full workspace: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/server/handler.rs
git commit -m "feat(daemon): add FlipPreference handler happy path (2A-4a T6)"
```

---

## Task 7: `Request::FlipPreference` validation (5 error paths)

**Files:**
- Modify: `crates/daemon/src/server/handler.rs` (add 5 parameterized tests — the handler code from T6 already implements the branches)

- [ ] **Step 1: Write the failing tests**

Add to the same test module as T6:

```rust
    #[test]
    fn test_flip_preference_rejects_missing_memory() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "does-not-exist".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("memory_id not found"),
                    "expected 'memory_id not found', got: {message}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_non_preference_type() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut decision = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Decision,
            "foo",
            "bar",
        );
        decision.id = "01DEC".to_string();
        crate::db::ops::remember(&state.conn, &decision).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01DEC".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("memory_type must be preference"),
                    "got: {message}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_already_superseded() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        state.conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity)
             VALUES (?1, 'preference', 'x', 'y', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.5)",
            rusqlite::params!["01SUP"],
        ).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01SUP".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("already superseded"),
                    "got: {message}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_invalid_valence() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "x",
            "y",
        );
        pref.id = "01PREF".to_string();
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "happy".into(),
                new_intensity: 0.8,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("new_valence must be positive | negative | neutral"),
                    "got: {message}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }

    #[test]
    fn test_flip_preference_rejects_out_of_range_intensity() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "x",
            "y",
        );
        pref.id = "01PREF".to_string();
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        let resp = handle_request(
            &mut state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "negative".into(),
                new_intensity: 1.5,
                reason: None,
            },
        );
        match resp {
            forge_core::protocol::Response::Error { message } => {
                assert!(
                    message.contains("new_intensity must be finite in [0.0, 1.0]"),
                    "got: {message}"
                );
            }
            other => panic!("expected error, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run tests to verify they pass (T6 already implements these branches)**

Run: `cargo test -p forge-daemon test_flip_preference_rejects -- --nocapture`
Expected: PASS (5 tests).

If any fail, the handler in T6 has a validation gap; review the error-message strings exactly.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/server/handler.rs
git commit -m "test(daemon): pin FlipPreference validation error paths (2A-4a T7)"
```

---

## Task 8: `Request::FlipPreference` event emission

**Files:**
- Modify: `crates/daemon/src/server/handler.rs` (add test for event emission; handler code from T6 already emits)

- [ ] **Step 1: Write the failing test**

Add to handler.rs test module:

```rust
    #[test]
    fn test_flip_preference_emits_preference_flipped_event() {
        let mut state = DaemonState::new(":memory:").expect("DaemonState::new");

        let mut pref = forge_core::types::memory::Memory::new(
            forge_core::types::memory::MemoryType::Preference,
            "tabs over spaces",
            "prefer tabs",
        );
        pref.id = "01PREF".to_string();
        pref.valence = "positive".to_string();
        crate::db::ops::remember(&state.conn, &pref).unwrap();

        // Subscribe to the event channel BEFORE making the request.
        let mut rx = state.events.subscribe();

        handle_request(
            &state,
            forge_core::protocol::Request::FlipPreference {
                memory_id: "01PREF".into(),
                new_valence: "negative".into(),
                new_intensity: 0.8,
                reason: Some("team switched".into()),
            },
        );

        // The emit is synchronous; try_recv should succeed immediately.
        // ForgeEvent has fields { event: String, data: serde_json::Value, timestamp: String }
        // per crates/daemon/src/events.rs:10-14.
        let evt = rx.try_recv().expect("no event received");
        assert_eq!(evt.event, "preference_flipped");
        assert_eq!(evt.data["old_id"], "01PREF");
        assert_eq!(evt.data["new_valence"], "negative");
        assert_eq!(evt.data["reason"], "team switched");
    }
```

If the event envelope uses different field names (e.g., `kind` instead of `event_type`), check via `grep -n 'pub struct Event\|pub event_type\|pub kind' crates/daemon/src/events.rs` and adjust.

- [ ] **Step 2: Run test**

Run: `cargo test -p forge-daemon test_flip_preference_emits_preference_flipped_event -- --nocapture`
Expected: PASS (T6's handler already calls `events::emit` post-commit).

If it fails because the emit is pre-commit OR not called: move the `events::emit` call to the post-commit branch of the `match result` block from T6.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/server/handler.rs
git commit -m "test(daemon): pin preference_flipped event emission (2A-4a T8)"
```

---

## Task 9: `Request::ListFlipped` handler + `ops::list_flipped()` helper

**Files:**
- Modify: `crates/daemon/src/db/ops.rs` (add `list_flipped()` helper returning `Vec<Memory>`)
- Modify: `crates/daemon/src/server/handler.rs` (replace `todo!("2A-4a T9")` with real handler)

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/db/ops.rs` test block:

```rust
#[test]
fn test_list_flipped_returns_only_flipped_memories_ordered_desc() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    // Active preference (not flipped)
    let mut a = forge_core::types::memory::Memory::new(
        forge_core::types::memory::MemoryType::Preference,
        "active",
        "content",
    );
    a.id = "01ACTIVE".to_string();
    remember(&conn, &a).unwrap();

    // Two flipped preferences with different flip timestamps
    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
         VALUES ('01F1', 'preference', 'older flip', 'c1', 0.9, 'superseded', NULL, '[]', '2026-04-15 00:00:00', '2026-04-15 00:00:00', 'positive', 0.5, '2026-04-15 10:00:00', '01N1')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
         VALUES ('01F2', 'preference', 'newer flip', 'c2', 0.9, 'superseded', NULL, '[]', '2026-04-16 00:00:00', '2026-04-16 00:00:00', 'negative', 0.6, '2026-04-17 14:00:00', '01N2')",
        [],
    ).unwrap();

    // Superseded but NOT flipped (no valence_flipped_at)
    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, superseded_by)
         VALUES ('01SUP', 'decision', 'plain supersede', 'c3', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'neutral', 0.0, '01N3')",
        [],
    ).unwrap();

    let flipped = list_flipped(&conn, None, 10).unwrap();
    assert_eq!(flipped.len(), 2, "should return exactly 2 flipped preferences");
    assert_eq!(flipped[0].id, "01F2", "most recent flip first");
    assert_eq!(flipped[1].id, "01F1");
}

#[test]
fn test_list_flipped_respects_limit() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    for i in 0..5 {
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
             VALUES (?1, 'preference', ?2, 'c', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.5, ?3, ?4)",
            rusqlite::params![format!("01F{i}"), format!("flip {i}"), format!("2026-04-17 0{i}:00:00"), format!("01N{i}")],
        ).unwrap();
    }

    let flipped = list_flipped(&conn, None, 2).unwrap();
    assert_eq!(flipped.len(), 2);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-daemon --lib db::ops::tests::test_list_flipped -- --nocapture`
Expected: FAIL with `cannot find function list_flipped`.

- [ ] **Step 3: Implement `list_flipped()` + handler arm**

In `crates/daemon/src/db/ops.rs`, add after `supersede_memory_impl`:

```rust
/// List preferences whose valence was flipped. Filters on
/// `valence_flipped_at IS NOT NULL AND memory_type = 'preference'`.
/// Ordered by `valence_flipped_at DESC` (most recent first).
///
/// `organization_id`: when `Some`, restricts to that org; when `None`, returns
/// flipped memories across all orgs (caller is responsible for scope enforcement).
/// `limit`: clamped to [1, 100].
pub fn list_flipped(
    conn: &Connection,
    organization_id: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<forge_core::types::memory::Memory>> {
    let org = organization_id.unwrap_or("default");
    let clamped_limit = limit.clamp(1, 100) as i64;

    let mut stmt = conn.prepare(
        "SELECT id FROM memory
          WHERE valence_flipped_at IS NOT NULL
            AND memory_type = 'preference'
            AND (?1 IS NULL OR COALESCE(organization_id, 'default') = ?1)
          ORDER BY valence_flipped_at DESC
          LIMIT ?2",
    )?;
    let ids: Vec<String> = stmt
        .query_map(rusqlite::params![org, clamped_limit], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut results = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(m) = fetch_memory_by_id(conn, &id)? {
            results.push(m);
        }
    }
    Ok(results)
}
```

In `crates/daemon/src/server/handler.rs`, replace `todo!("2A-4a T9")` for `ListFlipped`:

```rust
        Request::ListFlipped { agent: _, limit } => {
            let effective_limit = limit.unwrap_or(20);
            match ops::list_flipped(&state.conn, None, effective_limit) {
                Ok(memories) => {
                    let items: Vec<forge_core::protocol::response::FlippedMemory> = memories
                        .into_iter()
                        .map(|m| {
                            let flipped_to_id = m.superseded_by.clone().unwrap_or_default();
                            let flipped_at = m.valence_flipped_at.clone().unwrap_or_default();
                            forge_core::protocol::response::FlippedMemory {
                                old: m,
                                flipped_to_id,
                                flipped_at,
                            }
                        })
                        .collect();
                    Response::Ok {
                        data: ResponseData::FlippedList { items },
                    }
                }
                Err(e) => Response::Error {
                    message: format!("list_flipped failed: {e}"),
                },
            }
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-daemon --lib db::ops::tests::test_list_flipped -- --nocapture`
Expected: PASS (2 tests).

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/db/ops.rs crates/daemon/src/server/handler.rs
git commit -m "feat(daemon): add ListFlipped handler + ops::list_flipped() helper (2A-4a T9)"
```

---

## Task 10: Thread `include_flipped` through `hybrid_recall()`

**Purpose:** Flipped preferences are `status='superseded'`. BM25 hard-filters `status='active'` in its SQL, so flipped memories never reach the post-RRF stage where a retain-filter could act. The fix: add `include_flipped: bool` to `hybrid_recall()` and extend the BM25/vector predicates.

**Files:**
- Modify: `crates/daemon/src/recall.rs:187` (`hybrid_recall`)
- Modify: `crates/daemon/src/recall.rs:211` (`hybrid_recall_scoped`)
- Modify: `crates/daemon/src/recall.rs:234` (`hybrid_recall_scoped_org`)
- Modify: BM25 query inside `hybrid_recall_scoped_org` (find via `grep -n "status = 'active'" crates/daemon/src/recall.rs`)
- Modify: `crates/daemon/src/server/handler.rs:480, 633, 2979` (pass `include_flipped.unwrap_or(false)` into each call)
- Modify: internal test call sites in `recall.rs` at lines 1978, 2008, 2041, 2067, 2120, 3259, 3297, 3320, 3819 (add `false` as new param)

- [ ] **Step 1: Write the failing tests**

Add to `crates/daemon/src/recall.rs` test module:

```rust
    #[test]
    fn test_recall_default_excludes_flipped_prefs() {
        let conn = setup();

        // Insert a flipped preference. The FTS triggers (memory_fts_insert) auto-populate memory_fts.
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
             VALUES ('01F', 'preference', 'tabs', 'prefer tabs for flipping tests', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.5, '2026-04-17 14:00:00', '01N')",
            [],
        ).unwrap();

        let results = hybrid_recall(&conn, "tabs flipping", None, None, None, 10, false);
        assert!(
            !results.iter().any(|r| r.memory.id == "01F"),
            "flipped preference surfaced when include_flipped=false"
        );
    }

    #[test]
    fn test_recall_include_flipped_surfaces_flipped_prefs() {
        let conn = setup();

        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
             VALUES ('01F', 'preference', 'tabs', 'prefer tabs for flipping tests', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.5, '2026-04-17 14:00:00', '01N')",
            [],
        ).unwrap();

        let results = hybrid_recall(&conn, "tabs flipping", None, None, None, 10, true);
        assert!(
            results.iter().any(|r| r.memory.id == "01F"),
            "flipped preference NOT surfaced when include_flipped=true"
        );
    }

    #[test]
    fn test_recall_include_flipped_does_not_surface_non_preference_superseded() {
        let conn = setup();

        // Superseded decision (no valence_flipped_at) — must STAY hidden even when include_flipped=true
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, superseded_by)
             VALUES ('01D', 'decision', 'migrate', 'migrate auth provider', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'neutral', 0.0, '01N')",
            [],
        ).unwrap();

        let results = hybrid_recall(&conn, "migrate auth", None, None, None, 10, true);
        assert!(
            !results.iter().any(|r| r.memory.id == "01D"),
            "non-preference superseded should NOT be surfaced by include_flipped"
        );
    }
```

The `setup()` helper at `crates/daemon/src/recall.rs:1960` handles sqlite-vec init + schema creation. FTS is kept in sync via SQL triggers (`memory_fts_insert` at schema.rs:334), so raw `INSERT INTO memory` in tests is sufficient — no manual FTS populate needed.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-daemon --lib recall -- --nocapture`
Expected: FAIL — compile error on `hybrid_recall` taking 7 args instead of 6.

- [ ] **Step 3: Thread `include_flipped` through signatures + BM25 predicate**

In `crates/daemon/src/recall.rs`, update the three `hybrid_recall*` functions:

```rust
pub fn hybrid_recall(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    memory_type: Option<&MemoryType>,
    project: Option<&str>,
    limit: usize,
    include_flipped: bool,
) -> Vec<MemoryResult> {
    hybrid_recall_scoped(
        conn,
        query,
        query_embedding,
        memory_type,
        project,
        limit,
        None,
        include_flipped,
    )
}

pub fn hybrid_recall_scoped(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    memory_type: Option<&MemoryType>,
    project: Option<&str>,
    limit: usize,
    reality_id: Option<&str>,
    include_flipped: bool,
) -> Vec<MemoryResult> {
    hybrid_recall_scoped_org(
        conn,
        query,
        query_embedding,
        memory_type,
        project,
        limit,
        reality_id,
        None,
        include_flipped,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn hybrid_recall_scoped_org(
    conn: &Connection,
    query: &str,
    query_embedding: Option<&[f32]>,
    memory_type: Option<&MemoryType>,
    project: Option<&str>,
    limit: usize,
    reality_id: Option<&str>,
    org_id: Option<&str>,
    include_flipped: bool,
) -> Vec<MemoryResult> {
    // ... existing body ...
}
```

Inside `hybrid_recall_scoped_org`, the BM25 candidate query currently hard-filters `m.status = 'active'`. Find the SQL literal via `grep -n "status = 'active'" crates/daemon/src/recall.rs`. Replace the predicate with a conditional built in Rust:

```rust
    let status_predicate = if include_flipped {
        "(m.status = 'active' OR (m.status = 'superseded' AND m.valence_flipped_at IS NOT NULL AND m.memory_type = 'preference'))"
    } else {
        "m.status = 'active'"
    };

    // Then in the SQL string, replace the inlined "m.status = 'active'" with a format! that injects status_predicate.
    // Example:
    let bm25_sql = format!(
        "SELECT m.id, m.memory_type, ... FROM memory m
         JOIN memory_fts ON memory_fts.rowid = m.rowid
         WHERE memory_fts MATCH ?1 AND {status_predicate}
         ORDER BY bm25(memory_fts) LIMIT ?2"
    );
    // ... conn.prepare(&bm25_sql) ...
```

Similarly update the vector candidate path. The vector query joins on `memory_vec` → `memory`, and filters `m.status = 'active'` via the same predicate pattern.

Update all EXTERNAL call sites:

In `crates/daemon/src/server/handler.rs`:
- Line 480: `hybrid_recall(&state.conn, &query, None, memory_type.as_ref(), project.as_deref(), limit, include_flipped.unwrap_or(false))`
- Line 633: `hybrid_recall(&state.conn, &query, None, memory_type.as_ref(), project.as_deref(), limit, include_flipped.unwrap_or(false))`
- Line 2979: same pattern

Grab the `include_flipped` from the Recall request pattern destructuring:
```rust
Request::Recall { query, memory_type, project, limit, layer, since, include_flipped } => {
```

Update all INTERNAL test call sites (recall.rs lines 1978, 2008, 2041, 2067, 2120, 3259, 3297, 3320, 3819) to add `false` as the new trailing parameter.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-daemon --lib recall::tests::test_recall_default_excludes_flipped_prefs -- --nocapture`
Run: `cargo test -p forge-daemon --lib recall::tests::test_recall_include_flipped_surfaces_flipped_prefs -- --nocapture`
Run: `cargo test -p forge-daemon --lib recall::tests::test_recall_include_flipped_does_not_surface_non_preference_superseded -- --nocapture`
Expected: PASS (3 tests).

Run the full workspace: `cargo test --workspace`
Expected: PASS with 0 failures.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/recall.rs crates/daemon/src/server/handler.rs
git commit -m "feat(recall): thread include_flipped through hybrid_recall signatures (2A-4a T10)"
```

---

## Task 11: `<preferences-flipped>` XML section in CompileContext

**Files:**
- Modify: `crates/daemon/src/db/ops.rs` (add `list_flipped_with_targets()` helper that JOINs old+new)
- Modify: `crates/daemon/src/recall.rs:776` (extend `compile_dynamic_suffix` signature with `organization_id: Option<&str>`; add `<preferences-flipped>` rendering)
- Modify: all 10 call sites of `compile_dynamic_suffix` to pass the new `organization_id` argument

- [ ] **Step 1: Write the failing tests**

Add to `crates/daemon/src/recall.rs` test module:

```rust
    #[test]
    fn test_preferences_flipped_section_omitted_when_empty() {
        let conn = setup();

        let ctx_config = crate::config::ContextConfig::default();
        let (suffix, _) = compile_dynamic_suffix(&conn, "claude-code", None, &ctx_config, &[], None, None, None);

        assert!(
            !suffix.contains("<preferences-flipped>"),
            "section should be omitted when no flipped prefs exist"
        );
    }

    #[test]
    fn test_preferences_flipped_section_renders_both_valences() {
        let conn = setup();

        // Old (flipped) preference
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
             VALUES ('01OLD', 'preference', 'tabs over spaces', 'c', 0.9, 'superseded', NULL, '[]', '2026-04-15 00:00:00', '2026-04-15 00:00:00', 'positive', 0.7, '2026-04-17 14:22:00', '01NEW')",
            [],
        ).unwrap();
        // New (active) preference
        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity)
             VALUES ('01NEW', 'preference', 'tabs over spaces', 'c2', 0.9, 'active', NULL, '[]', '2026-04-17 14:22:00', '2026-04-17 14:22:00', 'negative', 0.8)",
            [],
        ).unwrap();

        let ctx_config = crate::config::ContextConfig::default();
        let (suffix, _) = compile_dynamic_suffix(&conn, "claude-code", None, &ctx_config, &[], None, None, None);

        assert!(suffix.contains("<preferences-flipped>"), "section missing; suffix: {suffix}");
        assert!(suffix.contains("old_valence=\"positive\""), "old valence missing");
        assert!(suffix.contains("new_valence=\"negative\""), "new valence missing");
        assert!(suffix.contains("at=\"2026-04-17 14:22:00\""), "timestamp missing");
        assert!(suffix.contains("<topic>tabs over spaces</topic>"), "topic missing");
    }

    #[test]
    fn test_preferences_flipped_section_respects_excluded_layers() {
        let conn = setup();

        conn.execute(
            "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
             VALUES ('01OLD', 'preference', 't', 'c', 0.9, 'superseded', NULL, '[]', '2026-04-15 00:00:00', '2026-04-15 00:00:00', 'positive', 0.7, '2026-04-17 14:22:00', '01NEW')",
            [],
        ).unwrap();

        let ctx_config = crate::config::ContextConfig::default();
        let excluded = vec!["preferences_flipped".to_string()];
        let (suffix, _) = compile_dynamic_suffix(&conn, "claude-code", None, &ctx_config, &excluded, None, None, None);

        assert!(
            !suffix.contains("<preferences-flipped>"),
            "section should be suppressed when layer is excluded"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p forge-daemon --lib recall::tests::test_preferences_flipped -- --nocapture`
Expected: FAIL — compile error on `compile_dynamic_suffix` taking 8 args vs 7.

- [ ] **Step 3: Implement**

In `crates/daemon/src/db/ops.rs`, add after `list_flipped()`:

```rust
pub struct FlippedWithTarget {
    pub old_id: String,
    pub old_title: String,
    pub old_valence: String,
    pub old_flipped_at: String,
    pub new_id: String,
    pub new_valence: String,
}

/// JOIN-based fetch returning both the flipped (old) preference AND the
/// current (new) preference's valence in one row, so CompileContext can
/// render both without a second lookup.
pub fn list_flipped_with_targets(
    conn: &Connection,
    organization_id: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<FlippedWithTarget>> {
    let org = organization_id.unwrap_or("default");
    let clamped_limit = limit.clamp(1, 100) as i64;
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
    let rows = stmt.query_map(rusqlite::params![org, clamped_limit], |row| {
        Ok(FlippedWithTarget {
            old_id: row.get(0)?,
            old_title: row.get(1)?,
            old_valence: row.get(2)?,
            old_flipped_at: row.get(3)?,
            new_id: row.get(4).unwrap_or_default(),
            new_valence: row.get(5).unwrap_or_default(),
        })
    })?;
    rows.collect()
}
```

In `crates/daemon/src/recall.rs`, extend `compile_dynamic_suffix` signature (line 776):

```rust
pub fn compile_dynamic_suffix(
    conn: &Connection,
    agent: &str,
    project: Option<&str>,
    ctx_config: &crate::config::ContextConfig,
    excluded_layers: &[String],
    session_id: Option<&str>,
    focus: Option<&str>,
    organization_id: Option<&str>,
) -> (String, Vec<String>) {
```

Inside the function, after all existing dynamic sections and before the closing `</forge-dynamic>` tag, add:

```rust
    // Phase 2A-4a: <preferences-flipped> section — shows flipped preferences with both
    // old and new valence rendered inline so the LLM has full context without a
    // follow-up lookup.
    if !excluded_layers.iter().any(|l| l == "preferences_flipped") {
        let flipped = crate::db::ops::list_flipped_with_targets(
            conn,
            organization_id,
            5,
        )
        .unwrap_or_default();
        if !flipped.is_empty() {
            let mut pf_xml = String::from("<preferences-flipped>");
            for item in &flipped {
                let entry = format!(
                    "\n  <flip at=\"{at}\" old_valence=\"{ov}\" new_valence=\"{nv}\">\n    <topic>{topic}</topic>\n  </flip>",
                    at = xml_escape(&item.old_flipped_at),
                    ov = xml_escape(&item.old_valence),
                    nv = xml_escape(&item.new_valence),
                    topic = xml_escape(&item.old_title),
                );
                if used + pf_xml.len() + entry.len() + "\n</preferences-flipped>\n".len() < budget {
                    pf_xml.push_str(&entry);
                } else {
                    break;
                }
            }
            pf_xml.push_str("\n</preferences-flipped>\n");
            used += pf_xml.len();
            xml.push_str(&pf_xml);
        }
    }
```

Now update ALL 10 call sites of `compile_dynamic_suffix`:

Run `grep -n "compile_dynamic_suffix(" crates/daemon/src/recall.rs` to list call sites. For each one, add `None` (or a real `organization_id` if the caller has one) as the new trailing argument. Example at line 1788:
```rust
// Before:
compile_dynamic_suffix(conn, agent, project, &ctx_config, &[], None, None);
// After:
compile_dynamic_suffix(conn, agent, project, &ctx_config, &[], None, None, None);
```

Also check call sites OUTSIDE recall.rs: `grep -rn 'compile_dynamic_suffix(' crates/daemon/src/ | grep -v recall.rs`. Update each.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p forge-daemon --lib recall::tests::test_preferences_flipped -- --nocapture`
Expected: PASS (3 tests).

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/db/ops.rs crates/daemon/src/recall.rs
# Plus any other files touched by call-site updates outside recall.rs
git commit -m "feat(recall): add <preferences-flipped> XML section with old+new valence (2A-4a T11)"
```

---

## Task 12: Integration test — end-to-end flow

**Files:**
- Create: `crates/daemon/tests/flip_preference_flow.rs`

- [ ] **Step 1: Write the test**

```rust
//! Integration test: Remember → FlipPreference → ListFlipped → Recall(include_flipped) → CompileContext.

use forge_core::protocol::{Request, Response, ResponseData};
use forge_core::types::memory::MemoryType;

mod common;

#[test]
fn test_flip_preference_end_to_end_flow() {
    let harness = common::DaemonHarness::spawn();

    // 1. Remember a preference
    let resp = harness.call(Request::Remember {
        memory_type: MemoryType::Preference,
        title: "tabs over spaces".into(),
        content: "prefer tabs for readability".into(),
        confidence: Some(0.9),
        tags: Some(vec!["formatting".into()]),
        project: Some("forge".into()),
        metadata: None,
    });
    let old_id = match resp {
        Response::Ok { data: ResponseData::Stored { id } } => id,
        other => panic!("remember failed: {other:?}"),
    };

    // 2. Flip it
    let resp = harness.call(Request::FlipPreference {
        memory_id: old_id.clone(),
        new_valence: "negative".into(),
        new_intensity: 0.8,
        reason: Some("team convention".into()),
    });
    let new_id = match resp {
        Response::Ok { data: ResponseData::PreferenceFlipped { new_id, new_valence, .. } } => {
            assert_eq!(new_valence, "negative");
            new_id
        }
        other => panic!("flip failed: {other:?}"),
    };
    assert_ne!(new_id, old_id);

    // 3. ListFlipped should return the old
    let resp = harness.call(Request::ListFlipped {
        agent: None,
        limit: Some(10),
    });
    match resp {
        Response::Ok { data: ResponseData::FlippedList { items } } => {
            assert_eq!(items.len(), 1);
            assert_eq!(items[0].old.id, old_id);
            assert_eq!(items[0].flipped_to_id, new_id);
        }
        other => panic!("list_flipped failed: {other:?}"),
    }

    // 4. Recall(include_flipped = false) → should NOT return old
    let resp = harness.call(Request::Recall {
        query: "tabs readability".into(),
        memory_type: None,
        project: None,
        limit: Some(10),
        layer: None,
        since: None,
        include_flipped: None,
    });
    match resp {
        Response::Ok { data: ResponseData::Memories { results, count: _ } } => {
            assert!(
                !results.iter().any(|m| m.memory.id == old_id),
                "old memory should NOT be recalled by default"
            );
        }
        other => panic!("recall failed: {other:?}"),
    }

    // 5. Recall(include_flipped = true) → SHOULD return old
    let resp = harness.call(Request::Recall {
        query: "tabs readability".into(),
        memory_type: None,
        project: None,
        limit: Some(10),
        layer: None,
        since: None,
        include_flipped: Some(true),
    });
    match resp {
        Response::Ok { data: ResponseData::Memories { results, count: _ } } => {
            assert!(
                results.iter().any(|m| m.memory.id == old_id),
                "old memory should be recalled when include_flipped=true"
            );
        }
        other => panic!("recall failed: {other:?}"),
    }

    // 6. CompileContext should include <preferences-flipped>.
    // Request::CompileContext fields (from request.rs:263-282): agent, project, static_only,
    // excluded_layers, session_id, focus — all Option<_>.
    let resp = harness.call(Request::CompileContext {
        agent: Some("claude-code".into()),
        project: Some("forge".into()),
        static_only: None,
        excluded_layers: None,
        session_id: None,
        focus: None,
    });
    match resp {
        Response::Ok { data: ResponseData::CompiledContext { context, dynamic_suffix, .. } } => {
            // CompiledContext fields: context, static_prefix, dynamic_suffix, layers_used, chars.
            // The <preferences-flipped> section is part of the dynamic suffix.
            assert!(
                context.contains("<preferences-flipped>") || dynamic_suffix.contains("<preferences-flipped>"),
                "context missing <preferences-flipped>: {}",
                context.chars().take(500).collect::<String>()
            );
            let target = if dynamic_suffix.contains("<preferences-flipped>") { &dynamic_suffix } else { &context };
            assert!(target.contains("old_valence=\"positive\""));
            assert!(target.contains("new_valence=\"negative\""));
        }
        other => panic!("compile_context failed: {other:?}"),
    }
}
```

Note: `common::DaemonHarness` is a test helper that spawns a daemon and supports sending Request values directly. Check whether it exists: `ls crates/daemon/tests/common.rs`. If it exists, use it; the pattern above is the intended API. If it doesn't exist, either (a) create a minimal one that sets up an in-memory DB and dispatches through `handle_request`, or (b) use an existing integration test file's harness pattern — check `crates/daemon/tests/` for existing files: `ls crates/daemon/tests/`.

If `Request::CompileContext` has a different shape, check with `grep -n 'CompileContext {' crates/core/src/protocol/request.rs` and adjust.

- [ ] **Step 2: Run test**

Run: `cargo test -p forge-daemon --test flip_preference_flow -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/tests/flip_preference_flow.rs
git commit -m "test(daemon): add FlipPreference integration flow test (2A-4a T12)"
```

---

## Task 13: Rollback recipe test

**Purpose:** Validate the rollback SQL documented in spec §2 actually works — forward-migrate, insert a row, rollback, verify residual queries still function.

**Files:**
- Modify: `crates/daemon/src/db/schema.rs` (add 1 test)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn test_valence_flipped_at_rollback_recipe_works() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::create_schema(&conn).unwrap();

    // Insert a row with valence_flipped_at set
    conn.execute(
        "INSERT INTO memory (id, memory_type, title, content, confidence, status, project, tags, created_at, accessed_at, valence, intensity, valence_flipped_at, superseded_by)
         VALUES ('01F', 'preference', 't', 'c', 0.9, 'superseded', NULL, '[]', '2026-04-17 00:00:00', '2026-04-17 00:00:00', 'positive', 0.5, '2026-04-17 14:00:00', '01N')",
        [],
    ).unwrap();

    // Execute rollback (SQLite 3.35+ supports DROP COLUMN)
    conn.execute("DROP INDEX IF EXISTS idx_memory_valence_flipped_at", []).unwrap();
    conn.execute("ALTER TABLE memory DROP COLUMN valence_flipped_at", []).unwrap();

    // Verify remaining queries still work
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory WHERE id = '01F'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // Verify the column is gone
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(memory)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(!cols.contains(&"valence_flipped_at".to_string()));
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p forge-daemon --lib db::schema::tests::test_valence_flipped_at_rollback -- --nocapture`
Expected: PASS (SQLite 3.46 bundled supports DROP COLUMN).

If it fails with `near "DROP": syntax error`, the bundled SQLite is < 3.35; check `rusqlite`'s version via `cargo tree -p rusqlite`. Minimum required is 3.35.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
git add crates/daemon/src/db/schema.rs
git commit -m "test(schema): validate valence_flipped_at rollback recipe (2A-4a T13)"
```

---

## Task 14: Live daemon dogfood via HTTP curl

**Purpose:** Exercise the feature end-to-end against a real daemon binary on port 8430. No CLI subcommands in 2A-4a scope; HTTP only.

**Prereqs:**
- `cargo test --workspace` is green
- `cargo clippy --workspace -- -W clippy::all -D warnings` is clean
- `cargo fmt --all` is clean

- [ ] **Step 1: Rebuild daemon binary**

Run: `cargo build --release -p forge-daemon`
Expected: success, produces `target/release/forge-daemon`.

- [ ] **Step 2: Gracefully replace the running daemon**

Check the currently-running daemon PID: `ps aux | grep 'target/release/forge-daemon' | grep -v grep`
If one is running, SIGTERM it: `kill <PID>` and wait 2 seconds for graceful shutdown.

Start the new binary: `nohup target/release/forge-daemon > /tmp/forge-daemon.log 2>&1 &`
Verify it's listening: `curl -sf http://localhost:8430/api -X POST -H 'Content-Type: application/json' -d '{"method":"health"}'`
Expected: `{"status":"ok","data":{...}}` JSON response with a memory count.

Verify state is preserved (compare to pre-rebuild counts): the memory/embedding/edge counts in the health response should match pre-rebuild values within normal drift.

- [ ] **Step 3: Store a preference**

```bash
curl -sf http://localhost:8430/api -X POST -H 'Content-Type: application/json' -d '{
  "method": "remember",
  "params": {
    "memory_type": "preference",
    "title": "dogfood tabs pref",
    "content": "prefer tabs for readability (2A-4a dogfood)",
    "confidence": 0.9,
    "project": "forge-dogfood"
  }
}' | jq .
```
Expected: `{"status":"ok","data":{"Remembered":{"id":"01...<ulid>..."}}}`

Save the returned id (call it `$OLD_ID`) for the next step.

- [ ] **Step 4: Flip it**

```bash
OLD_ID="<paste the id from step 3>"
curl -sf http://localhost:8430/api -X POST -H 'Content-Type: application/json' -d "{
  \"method\": \"flip_preference\",
  \"params\": {
    \"memory_id\": \"$OLD_ID\",
    \"new_valence\": \"negative\",
    \"new_intensity\": 0.8,
    \"reason\": \"2A-4a dogfood — team convention switch\"
  }
}" | jq .
```
Expected: `{"status":"ok","data":{"PreferenceFlipped":{"old_id":"...","new_id":"...","new_valence":"negative","new_intensity":0.8,"flipped_at":"2026-04-17 HH:MM:SS"}}}`

- [ ] **Step 5: ListFlipped**

```bash
curl -sf http://localhost:8430/api -X POST -H 'Content-Type: application/json' -d '{
  "method": "list_flipped",
  "params": {"limit": 5}
}' | jq .
```
Expected: `FlippedList` with at least 1 item containing the `dogfood tabs pref` memory.

- [ ] **Step 6: CompileContext includes `<preferences-flipped>`**

```bash
curl -sf http://localhost:8430/api -X POST -H 'Content-Type: application/json' -d '{
  "method": "compile_context",
  "params": {"agent": "claude-code", "project": "forge-dogfood"}
}' | jq -r '.data.CompiledContext.context' | grep -A3 preferences-flipped
```
Expected: `<preferences-flipped>` element with an entry `at="YYYY-MM-DD HH:MM:SS" old_valence="positive" new_valence="negative"` and the topic.

- [ ] **Step 7: Check doctor + logs**

```bash
curl -sf http://localhost:8430/api -X POST -H 'Content-Type: application/json' -d '{"method":"doctor"}' | jq .
tail -50 /tmp/forge-daemon.log | grep -i error
```
Expected: doctor returns healthy summary; no ERROR-level log entries from the flip operations.

- [ ] **Step 8: Document the dogfood run**

Write a short results file at `docs/benchmarks/results/forge-valence-flipping-2026-04-17.md`:
```markdown
# Phase 2A-4a Forge-Valence-Flipping Dogfood Results

**Date:** 2026-04-17
**Daemon binary:** commit `<HEAD SHA>` (rebuilt from source)
**Steps exercised:** remember → flip_preference → list_flipped → compile_context

## Result

All 6 HTTP steps returned expected JSON shapes. CompileContext rendered
`<preferences-flipped>` with both valences inline. No ERROR-level log entries.

State preservation verified: pre-rebuild memory count $PRE matched post-rebuild count $POST.
```

Fill in `<HEAD SHA>`, `$PRE`, `$POST` with real values from the run.

- [ ] **Step 9: Memory handoff + task update**

Create a new handoff memory file documenting completion. Mark task #204 completed; keep #205-#207 pending.

- [ ] **Step 10: Commit**

```bash
git add docs/benchmarks/results/forge-valence-flipping-2026-04-17.md
git commit -m "docs(bench): Phase 2A-4a Forge-Valence-Flipping dogfood results"
```

---

## Gate checklist (from spec §16)

Before declaring Phase 2A-4a complete:

- [ ] All T0–T14 checkboxes above are ticked
- [ ] `cargo test --workspace` is GREEN (0 failures)
- [ ] `cargo clippy --workspace -- -W clippy::all -D warnings` is CLEAN (0 warnings)
- [ ] `cargo fmt --all` is CLEAN (no diff)
- [ ] Rollback recipe test (T13) passes on bundled SQLite
- [ ] Dogfood run (T14) renders `<preferences-flipped>` in live CompileContext
- [ ] Memory handoff file created at `memory/project_phase_2a4a_complete_2026_04_17.md`
- [ ] `docs/benchmarks/forge-identity-master-design.md` §5 2A-4a lock still accurate (no drift)
- [ ] All 11 commits on master; pushed to origin

---

## Spec coverage map (self-review)

| Spec section | Covered by task |
|--------------|-----------------|
| §2 Schema changes (ALTER + partial index) | T3 |
| §2 Rollback recipe | T13 |
| §3 `supersede_memory_impl()` helper | T1 (plain branch), T2 (flip branch) |
| §3 OpError variants | T1 |
| §3 Supersede handler refactor | T1 |
| §4.1-4.2 FlipPreference/response shapes | T5 |
| §4.3 FlipPreference handler algorithm | T6 |
| §4.3 Cross-org scope guard | T6 (step 4 in handler) |
| §4.3 Transaction atomicity | T6 (unchecked_transaction() wrapper) |
| §4.4 D2 confidence floor | T6 (new_confidence line) |
| §5 ListFlipped variant + response | T5, T9 |
| §6 Recall include_flipped threading | T10 |
| §6.2 Memory struct field additions | T4 |
| §6.2 Fetch-accessor updates | T0 |
| §7 `<preferences-flipped>` XML section | T11 |
| §7 `list_flipped_with_targets` JOIN helper | T11 |
| §7 `compile_dynamic_suffix` signature extension | T11 |
| §8 `"preference_flipped"` event | T8 (post-commit emission from T6) |
| §9 Validation error messages | T7 |
| §10 Contract tests | T5 |
| §11 T0-T14 TDD sequence | all tasks |
| §12 Files touched | header "File structure" table |
| §13 Out-of-scope non-goals | not implemented (correct) |
| §14 R1-R5 risks | R1 TODO comment in consolidator.rs (T14 or earlier); R5 concurrent-flip test in T7 |
| §15 Open decisions | OD1+OD2 resolved in T4/T6; OD3 deferred to 2A-4d |

**Self-review notes:**

- **R1 (consolidator inline supersede):** add a one-line TODO comment in `crates/daemon/src/workers/consolidator.rs:1499-1510` as part of T1's commit (before the inline supersede SQL): `// TODO(2A-4+): migrate to ops::supersede_memory_impl(). See docs/superpowers/specs/2026-04-17-forge-valence-flipping-design.md §14 R1.` This is 1 added line, same commit as the T1 refactor. The consolidator's inline SQL is NOT refactored in 2A-4a (out of scope).
- **R5 (concurrent flip race):** not a standalone test in the plan; covered implicitly by the UPDATE's `status='active'` predicate returning rows_updated=0 on the second writer (returns `OldMemoryNotActive`). The second FlipPreference call then returns `"memory already superseded (id: ...)"` per the handler's error mapping. If full coverage is wanted, add a 6th validation test in T7: `test_concurrent_flip_preference_only_first_succeeds`.
- **Spec §14 R3 (serialization size overhead):** already mitigated by `skip_serializing_if = "Option::is_none"` on both `Memory.superseded_by` and `Memory.valence_flipped_at` from T4.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-17-forge-valence-flipping.md`. Two execution options:

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task (T3 → T4 → T0 → T1 → T2 → T5 → T6 → T7 → T8 → T9 → T10 → T11 → T12 → T13 → T14); spec-compliance review + code-quality review after each; automatic re-dispatch on failures.

**2. Inline Execution** — execute tasks in this same session with checkpoints for review between tasks.

Which approach?
