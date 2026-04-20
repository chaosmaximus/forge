# Forge-Tool-Use-Recording (Phase 2A-4c1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the `session_tool_call` substrate (schema + atomic `Request::RecordToolUse` + snapshot-consistent `Request::ListToolCalls` + event emission) so that Phase 2A-4c2 can implement behavioral skill inference on top.

**Architecture:** Append-only SQLite child table scoped to sessions. Writes fuse validation + persistence into a single atomic `INSERT ... SELECT FROM session WHERE id = ?` (eliminates TOCTOU). Reads run inside a single transaction for snapshot consistency. Cross-org scoping mirrors `FlipPreference` / `ReaffirmPreference` (target-session org consistency, not cross-caller isolation). Errors return `Response::Error { message: String }` with documented `<error_code>:` prefixes.

**Tech Stack:** Rust 2021, rusqlite, serde, serde_json, tokio broadcast, ulid. Tests: `cargo test --workspace`. Lint: `cargo clippy --workspace -- -W clippy::all -D warnings`. Fmt: `cargo fmt --all`.

**Spec:** `docs/superpowers/specs/2026-04-19-forge-tool-use-recording-design.md` v3 (commit `b1ad7d9`).

---

## File map

| File | Responsibility | Action |
|------|----------------|--------|
| `crates/core/src/types/tool_call.rs` | `ToolCallRow` shared type (DB rows + protocol responses) | CREATE |
| `crates/core/src/types/mod.rs` | Register `tool_call` submodule + re-export | MODIFY |
| `crates/daemon/src/db/schema.rs` | `session_tool_call` table + 3 indexes + rollback recipe test | MODIFY |
| `crates/daemon/src/db/ops.rs` | `ops::list_tool_calls` helper (+ L1 tests) | MODIFY |
| `crates/core/src/protocol/request.rs` | `RecordToolUse` + `ListToolCalls` variants + `default_empty_args` helper | MODIFY |
| `crates/core/src/protocol/response.rs` | `ResponseData::ToolCallRecorded` + `ResponseData::ToolCallList` | MODIFY |
| `crates/core/src/protocol/contract_tests.rs` | Round-trip tests for new variants + error prefixes | MODIFY |
| `crates/daemon/src/server/handler.rs` | `handle_record_tool_use`, `handle_list_tool_calls` — 4 task-tranches: stubs → atomic write → validation → events → read | MODIFY |
| `crates/daemon/tests/record_tool_use_flow.rs` | End-to-end integration test | CREATE |
| `docs/benchmarks/results/forge-tool-use-recording-2026-04-19.md` | Live-daemon dogfood + results doc | CREATE |

---

## Glossary of conventions used throughout this plan

- Error messages are raw strings: `"unknown_session: <session_id>"`, `"payload_too_large: tool_args: 65536"`, `"empty_field: tool_name"`, `"invalid_field: <field>: control_character"`, `"limit_too_large: requested <n>, max 500"`, `"internal_error: <sanitized>"`.
- All timestamps use `forge_core::time::now_iso()` → `"YYYY-MM-DD HH:MM:SS"` (wall clock).
- All IDs use `ulid::Ulid::new().to_string()`.
- All database tests use an in-memory SQLite DB; migration is applied via `crate::db::schema::ensure_schema(&conn)?`.
- Every task ends with `cargo fmt --all`, `cargo clippy --workspace -- -W clippy::all -D warnings`, and `cargo test --workspace` all clean before commit.
- Commit subject format: `feat(2A-4c1 T<n>): <one-line summary>` or `test(2A-4c1 T<n>): ...` or `fix(2A-4c1 T<n>): ...`. Commits include a `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>` trailer.

---

## Task 1: `ToolCallRow` shared type

**Files:**
- Create: `crates/core/src/types/tool_call.rs`
- Modify: `crates/core/src/types/mod.rs` (add module + re-export)

- [ ] **Step 1.1: Write the failing test**

Append to `crates/core/src/types/tool_call.rs` (new file):

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallRow {
    pub id: String,
    pub session_id: String,
    pub agent: String,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub tool_result_summary: String,
    pub success: bool,
    pub user_correction_flag: bool,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_row_roundtrips_via_serde_json() {
        let row = ToolCallRow {
            id: "01KPK000".to_string(),
            session_id: "01KPG000".to_string(),
            agent: "claude-code".to_string(),
            tool_name: "Read".to_string(),
            tool_args: serde_json::json!({"file_path": "/tmp/a"}),
            tool_result_summary: "ok".to_string(),
            success: true,
            user_correction_flag: false,
            created_at: "2026-04-19 12:34:56".to_string(),
        };
        let s = serde_json::to_string(&row).unwrap();
        let back: ToolCallRow = serde_json::from_str(&s).unwrap();
        assert_eq!(row, back);
    }
}
```

- [ ] **Step 1.2: Add module registration + re-export**

Edit `crates/core/src/types/mod.rs`. Current file:

```rust
pub mod code;
pub mod entity;
pub mod manas;
pub mod memory;
pub mod reality_engine;
pub mod session;
pub mod team;
// …
pub use memory::{Memory, MemoryStatus, MemoryType};
// …
```

Add after `pub mod team;`:

```rust
pub mod tool_call;
```

Add after the `pub use memory::...` line:

```rust
pub use tool_call::ToolCallRow;
```

- [ ] **Step 1.3: Run the test to verify it passes**

```bash
cargo test -p forge-core tool_call_row_roundtrips_via_serde_json
```

Expected: 1 passed.

- [ ] **Step 1.4: Fmt + clippy + full test**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace
```

Expected: 0 fmt diffs, 0 clippy warnings, all tests pass.

- [ ] **Step 1.5: Commit**

```bash
git add crates/core/src/types/tool_call.rs crates/core/src/types/mod.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T1): ToolCallRow shared type in core::types

Add ToolCallRow struct (id, session_id, agent, tool_name, tool_args,
tool_result_summary, success, user_correction_flag, created_at) with
serde derives for wire/DB round-trip. Re-exported through
forge_core::types::ToolCallRow.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Schema migration — `session_tool_call` table + 3 indexes

**Files:**
- Modify: `crates/daemon/src/db/schema.rs`

- [ ] **Step 2.1: Locate the ensure_schema function**

```bash
grep -n "fn ensure_schema\|pub fn ensure_schema" crates/daemon/src/db/schema.rs | head -3
```

Note the line where the current `ensure_schema` body emits its last `CREATE TABLE ... CREATE INDEX ...` block (near the `notification_tuning` block around line 1108, or wherever migrations land today — use that as the insertion point for the new block).

- [ ] **Step 2.2: Write the failing migration test**

Append to `crates/daemon/src/db/schema.rs` in the `#[cfg(test)] mod tests` block (or the file's existing test module — search for `mod tests`):

```rust
#[test]
fn session_tool_call_table_and_three_indexes_exist_after_migration() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    ensure_schema(&conn).unwrap();

    // Table present
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session_tool_call'",
            [], |row| row.get(0),
        ).unwrap();
    assert_eq!(count, 1, "session_tool_call table should exist");

    // Three indexes present
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='session_tool_call'
         ORDER BY name",
    ).unwrap();
    let names: Vec<String> = stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap().filter_map(|r| r.ok()).collect();
    assert!(names.contains(&"idx_session_tool_name_agent".to_string()),
            "missing idx_session_tool_name_agent; got {:?}", names);
    assert!(names.contains(&"idx_session_tool_org_session_created".to_string()),
            "missing idx_session_tool_org_session_created; got {:?}", names);
    assert!(names.contains(&"idx_session_tool_session".to_string()),
            "missing idx_session_tool_session; got {:?}", names);
}
```

- [ ] **Step 2.3: Run the failing test**

```bash
cargo test -p forge-daemon session_tool_call_table_and_three_indexes_exist_after_migration
```

Expected: FAIL — "session_tool_call table should exist" (count == 0).

- [ ] **Step 2.4: Add the migration block inside `ensure_schema`**

Inside `ensure_schema`, after the last existing `CREATE TABLE` block (e.g., right before the closing `Ok(())`), add:

```rust
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS session_tool_call (
            id                    TEXT PRIMARY KEY,
            session_id            TEXT NOT NULL,
            agent                 TEXT NOT NULL,
            tool_name             TEXT NOT NULL,
            tool_args             TEXT NOT NULL,
            tool_result_summary   TEXT NOT NULL,
            success               INTEGER NOT NULL,
            user_correction_flag  INTEGER NOT NULL DEFAULT 0,
            organization_id       TEXT NOT NULL DEFAULT 'default',
            created_at            TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_session_tool_session
            ON session_tool_call (session_id, created_at);
        CREATE INDEX IF NOT EXISTS idx_session_tool_name_agent
            ON session_tool_call (agent, tool_name);
        CREATE INDEX IF NOT EXISTS idx_session_tool_org_session_created
            ON session_tool_call (organization_id, session_id, created_at DESC);
    ",
    )?;
```

- [ ] **Step 2.5: Run the test to verify it passes**

```bash
cargo test -p forge-daemon session_tool_call_table_and_three_indexes_exist_after_migration
```

Expected: 1 passed.

- [ ] **Step 2.6: Fmt + clippy + full test**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace
```

- [ ] **Step 2.7: Commit**

```bash
git add crates/daemon/src/db/schema.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T2): session_tool_call schema + 3 indexes

Idempotent CREATE TABLE IF NOT EXISTS for session_tool_call
(id TEXT PRIMARY KEY, session_id, agent, tool_name, tool_args
canonical JSON, tool_result_summary, success, user_correction_flag,
organization_id DEFAULT 'default', created_at wall-clock) plus:

- idx_session_tool_session (master §5)
- idx_session_tool_name_agent (master §5)
- idx_session_tool_org_session_created (query-serving; §3.3 dev #4)

All NOT NULL columns with storage-strict defaults; wire-level
optionality handled by serde defaults on Request in T4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: `ops::list_tool_calls` + L1 tests

**Files:**
- Modify: `crates/daemon/src/db/ops.rs`

- [ ] **Step 3.1: Add imports at the top of ops.rs**

Existing `crates/daemon/src/db/ops.rs` line 1 imports:

```rust
use forge_core::types::{CodeFile, CodeSymbol, Memory, MemoryStatus, MemoryType};
```

Change to:

```rust
use forge_core::types::{CodeFile, CodeSymbol, Memory, MemoryStatus, MemoryType, ToolCallRow};
```

- [ ] **Step 3.2: Write the first failing L1 test for `list_tool_calls`**

Append to the existing `#[cfg(test)] mod tests` block in `ops.rs` (search for `mod tests {` in that file):

```rust
#[test]
fn list_tool_calls_orders_newest_first_with_id_tiebreaker() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();

    // Seed a session so org derivation is consistent (default org).
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('SESS1', 'claude-code', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();

    // Insert 3 calls with increasing ids; same second (monotonic id tie-breaks).
    for (i, id) in ["01A", "01B", "01C"].iter().enumerate() {
        conn.execute(
            "INSERT INTO session_tool_call
                (id, session_id, agent, tool_name, tool_args, tool_result_summary,
                 success, user_correction_flag, organization_id, created_at)
             VALUES (?1, 'SESS1', 'claude-code', 'Read', '{}', 'ok', 1, 0, 'default',
                     '2026-04-19 12:00:00')",
            rusqlite::params![id],
        ).unwrap();
        let _ = i; // silence unused warning
    }

    let rows = crate::db::ops::list_tool_calls(&conn, "default", "SESS1", None, 10).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["01C", "01B", "01A"],
               "must order by created_at DESC, id DESC");
}
```

- [ ] **Step 3.3: Run the failing test**

```bash
cargo test -p forge-daemon list_tool_calls_orders_newest_first_with_id_tiebreaker
```

Expected: FAIL — `list_tool_calls` not found.

- [ ] **Step 3.4: Implement `ops::list_tool_calls`**

Add this function to `crates/daemon/src/db/ops.rs` (anywhere at top level in the module — follow the placement pattern near other session-touching helpers):

```rust
pub fn list_tool_calls(
    conn: &rusqlite::Connection,
    organization_id: &str,
    session_id: &str,
    agent_filter: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<ToolCallRow>> {
    use rusqlite::types::ToSql;

    let (sql, boxed_params): (&'static str, Vec<Box<dyn ToSql>>) = match agent_filter {
        Some(agent) => (
            "SELECT id, session_id, agent, tool_name, tool_args, tool_result_summary,
                    success, user_correction_flag, created_at
             FROM session_tool_call
             WHERE organization_id = ?1 AND session_id = ?2 AND agent = ?3
             ORDER BY created_at DESC, id DESC
             LIMIT ?4",
            vec![
                Box::new(organization_id.to_string()),
                Box::new(session_id.to_string()),
                Box::new(agent.to_string()),
                Box::new(limit as i64),
            ],
        ),
        None => (
            "SELECT id, session_id, agent, tool_name, tool_args, tool_result_summary,
                    success, user_correction_flag, created_at
             FROM session_tool_call
             WHERE organization_id = ?1 AND session_id = ?2
             ORDER BY created_at DESC, id DESC
             LIMIT ?3",
            vec![
                Box::new(organization_id.to_string()),
                Box::new(session_id.to_string()),
                Box::new(limit as i64),
            ],
        ),
    };

    let mut stmt = conn.prepare(sql)?;
    let params_refs: Vec<&dyn ToSql> = boxed_params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        let tool_args_text: String = row.get(4)?;
        let tool_args: serde_json::Value = serde_json::from_str(&tool_args_text).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                4, rusqlite::types::Type::Text, Box::new(e),
            )
        })?;
        Ok(ToolCallRow {
            id: row.get(0)?,
            session_id: row.get(1)?,
            agent: row.get(2)?,
            tool_name: row.get(3)?,
            tool_args,
            tool_result_summary: row.get(5)?,
            success: row.get::<_, i64>(6)? != 0,
            user_correction_flag: row.get::<_, i64>(7)? != 0,
            created_at: row.get(8)?,
        })
    })?;
    rows.collect()
}
```

- [ ] **Step 3.5: Run the test to verify it passes**

```bash
cargo test -p forge-daemon list_tool_calls_orders_newest_first_with_id_tiebreaker
```

Expected: 1 passed.

- [ ] **Step 3.6: Add 7 more L1 tests (one RED/GREEN cycle each)**

Append each of the following to the `mod tests` block of `ops.rs`, running between each insert to confirm RED (test fails when the tested condition is subtly tweaked) then GREEN. For this plan we batch them; in execution add + run one at a time.

```rust
#[test]
fn list_tool_calls_respects_limit() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    for i in 0..5 {
        conn.execute(
            "INSERT INTO session_tool_call VALUES (?1, 'S', 'a', 'T', '{}', '', 1, 0, 'default',
             '2026-04-19 12:00:00')",
            rusqlite::params![format!("ID{i}")],
        ).unwrap();
    }
    let rows = crate::db::ops::list_tool_calls(&conn, "default", "S", None, 3).unwrap();
    assert_eq!(rows.len(), 3);
}

#[test]
fn list_tool_calls_filters_by_agent() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'alice', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('B', 'S', 'bob', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();

    let rows = crate::db::ops::list_tool_calls(&conn, "default", "S", Some("alice"), 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent, "alice");
}

#[test]
fn list_tool_calls_returns_empty_when_org_mismatch() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'a', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();

    let rows = crate::db::ops::list_tool_calls(&conn, "other_org", "S", None, 10).unwrap();
    assert_eq!(rows.len(), 0);
}

#[test]
fn list_tool_calls_returns_empty_when_session_mismatch() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'a', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();

    let rows = crate::db::ops::list_tool_calls(&conn, "default", "OTHER", None, 10).unwrap();
    assert_eq!(rows.len(), 0);
}

#[test]
fn list_tool_calls_persists_user_correction_flag_true_and_false() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'a', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('B', 'S', 'a', 'T', '{}', '', 1, 1, 'default',
         '2026-04-19 12:00:01')", []).unwrap();

    let rows = crate::db::ops::list_tool_calls(&conn, "default", "S", None, 10).unwrap();
    let flags: Vec<bool> = rows.iter().map(|r| r.user_correction_flag).collect();
    assert_eq!(flags, vec![true, false],
               "rows in DESC order by created_at; B (flag=true) first, A (flag=false) second");
}

#[test]
fn list_tool_calls_handles_concurrent_inserts_during_read_snapshot() {
    // Reads inside a transaction see a stable snapshot even with concurrent writes.
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'a', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();

    let tx = conn.unchecked_transaction().unwrap();
    // Read one row inside the transaction.
    let rows_before = crate::db::ops::list_tool_calls(&tx, "default", "S", None, 10).unwrap();
    assert_eq!(rows_before.len(), 1);

    // Insert another row via the same connection outside the open transaction — SQLite in-memory
    // serialises, but we verify the in-tx read is stable against its own snapshot.
    // NOTE: for in-memory single-connection tests this collapses; the real guarantee is
    // tested at L2 under multi-connection conditions (T8 #25 list_tool_calls_snapshot_consistency_under_concurrent_writes).
    tx.commit().unwrap();

    conn.execute(
        "INSERT INTO session_tool_call VALUES ('B', 'S', 'a', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:05')", []).unwrap();
    let rows_after = crate::db::ops::list_tool_calls(&conn, "default", "S", None, 10).unwrap();
    assert_eq!(rows_after.len(), 2);
}

#[test]
fn list_tool_calls_propagates_corrupt_tool_args_as_conversion_error() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::schema::ensure_schema(&conn).unwrap();
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    // Manually insert malformed tool_args.
    conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'a', 'T', 'not json', '', 1, 0,
         'default', '2026-04-19 12:00:00')", []).unwrap();

    let err = crate::db::ops::list_tool_calls(&conn, "default", "S", None, 10).unwrap_err();
    assert!(matches!(err, rusqlite::Error::FromSqlConversionFailure(_, _, _)),
            "expected FromSqlConversionFailure, got {err:?}");
}
```

- [ ] **Step 3.7: Run all L1 tests**

```bash
cargo test -p forge-daemon list_tool_calls_
```

Expected: 7 passed (plus the first test from Step 3.2 = 8 total).

- [ ] **Step 3.8: Fmt + clippy + full test**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace
```

- [ ] **Step 3.9: Commit**

```bash
git add crates/daemon/src/db/ops.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T3): ops::list_tool_calls + 8 L1 tests

Thin ops helper: scoped read from session_tool_call with optional
agent filter, ORDER BY created_at DESC, id DESC (ULID tiebreaker
for sub-second monotonicity), LIMIT bound. Parses tool_args TEXT
back to serde_json::Value, surfaces corruption as
FromSqlConversionFailure (never silent).

L1 tests: ordering, limit, agent filter, org-scope, session-scope,
user_correction_flag round-trip both values, snapshot-transaction
shape, corrupt-args propagation.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Protocol bundle — Request + Response + contract tests + handler stubs

**Files:**
- Modify: `crates/core/src/protocol/request.rs`
- Modify: `crates/core/src/protocol/response.rs`
- Modify: `crates/core/src/protocol/contract_tests.rs`
- Modify: `crates/daemon/src/server/handler.rs` (stub arms only)

- [ ] **Step 4.1: Add `default_empty_args` helper + both Request variants**

In `crates/core/src/protocol/request.rs`:

Add near the top of the file (after existing imports):

```rust
fn default_empty_args() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}
```

Add two new variants to the `Request` enum (place them near `ReaffirmPreference` / `FlipPreference` to keep session-type variants together):

```rust
    RecordToolUse {
        session_id: String,
        agent: String,
        tool_name: String,
        #[serde(default = "default_empty_args")]
        tool_args: serde_json::Value,
        #[serde(default)]
        tool_result_summary: String,
        success: bool,
        #[serde(default)]
        user_correction_flag: bool,
    },
    ListToolCalls {
        session_id: String,
        #[serde(default)]
        agent: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    },
```

- [ ] **Step 4.2: Add `ResponseData` variants**

In `crates/core/src/protocol/response.rs`, add to the `ResponseData` enum (near `PreferenceReaffirmed` variant):

```rust
    ToolCallRecorded {
        id: String,
        created_at: String,
    },
    ToolCallList {
        calls: Vec<forge_core::types::ToolCallRow>,
    },
```

If `forge_core::types::ToolCallRow` import-path collides with the crate being `forge-core` itself, use the crate-relative path (`crate::types::ToolCallRow`) — verify by checking how `response.rs` currently imports other shared types.

- [ ] **Step 4.3: Add handler stub arms so workspace compiles**

In `crates/daemon/src/server/handler.rs`, find the big `match request { ... }` inside the dispatch function. Add two new arms alongside the existing ones (near `Request::ReaffirmPreference`):

```rust
        Request::RecordToolUse { .. } => Response::Error {
            message: "unimplemented: record_tool_use (T5)".to_string(),
        },
        Request::ListToolCalls { .. } => Response::Error {
            message: "unimplemented: list_tool_calls (T8)".to_string(),
        },
```

- [ ] **Step 4.4: Add 8 contract tests**

In `crates/core/src/protocol/contract_tests.rs`, append:

```rust
#[test]
fn record_tool_use_roundtrip_all_fields() {
    let req = Request::RecordToolUse {
        session_id: "S".to_string(),
        agent: "a".to_string(),
        tool_name: "T".to_string(),
        tool_args: serde_json::json!({"k": 1}),
        tool_result_summary: "ok".to_string(),
        success: true,
        user_correction_flag: true,
    };
    let s = serde_json::to_string(&req).unwrap();
    let back: Request = serde_json::from_str(&s).unwrap();
    assert_eq!(req, back);
}

#[test]
fn record_tool_use_defaults_when_optional_fields_omitted() {
    let json = r#"{
        "method": "record_tool_use",
        "params": {
            "session_id": "S", "agent": "a", "tool_name": "T",
            "success": true
        }
    }"#;
    // Deserialize through the outer envelope used in the request test module.
    // The exact envelope type (RequestEnvelope / tagged enum) lives in this file already;
    // adapt to the local helper pattern used for other Request variants (check existing tests).
    let req: Request = serde_json::from_str(
        r#"{"type":"record_tool_use","session_id":"S","agent":"a","tool_name":"T","success":true}"#
    ).unwrap();
    if let Request::RecordToolUse { tool_args, tool_result_summary, user_correction_flag, .. } = req {
        assert_eq!(tool_args, serde_json::json!({}));
        assert_eq!(tool_result_summary, "");
        assert!(!user_correction_flag);
    } else {
        panic!("wrong variant");
    }
    let _ = json;
}

#[test]
fn list_tool_calls_roundtrip_required_only() {
    let req = Request::ListToolCalls {
        session_id: "S".to_string(),
        agent: None,
        limit: None,
    };
    let s = serde_json::to_string(&req).unwrap();
    let back: Request = serde_json::from_str(&s).unwrap();
    assert_eq!(req, back);
}

#[test]
fn list_tool_calls_roundtrip_all_fields() {
    let req = Request::ListToolCalls {
        session_id: "S".to_string(),
        agent: Some("a".to_string()),
        limit: Some(100),
    };
    let s = serde_json::to_string(&req).unwrap();
    let back: Request = serde_json::from_str(&s).unwrap();
    assert_eq!(req, back);
}

#[test]
fn tool_call_recorded_response_roundtrip() {
    let resp = Response::Ok {
        data: ResponseData::ToolCallRecorded {
            id: "01K".to_string(),
            created_at: "2026-04-19 12:00:00".to_string(),
        },
    };
    let s = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&s).unwrap();
    assert_eq!(resp, back);
}

#[test]
fn tool_call_list_response_roundtrip_empty_and_three() {
    use forge_core::types::ToolCallRow;
    for rows in [vec![], vec![
        ToolCallRow {
            id: "1".to_string(), session_id: "S".to_string(), agent: "a".to_string(),
            tool_name: "T".to_string(), tool_args: serde_json::json!({}),
            tool_result_summary: "".to_string(), success: true, user_correction_flag: false,
            created_at: "2026-04-19 12:00:00".to_string(),
        }; 3]] {
        let resp = Response::Ok { data: ResponseData::ToolCallList { calls: rows.clone() } };
        let s = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);
    }
}

#[test]
fn response_error_roundtrips_with_all_six_prefixes() {
    let prefixes = [
        "unknown_session: 01K...",
        "payload_too_large: tool_args: 65536",
        "limit_too_large: requested 1000, max 500",
        "empty_field: tool_name",
        "invalid_field: session_id: control_character",
        "internal_error: db locked",
    ];
    for p in prefixes {
        let resp = Response::Error { message: p.to_string() };
        let s = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);
    }
}
```

Note: if the existing contract test file uses a different serde-tag convention (e.g., the Request enum is `#[serde(tag = "type")]` vs `#[serde(tag = "method")]`), adjust the JSON strings in the default test (#2 above) accordingly. Check the existing test in `contract_tests.rs` for an example of the expected tag name.

- [ ] **Step 4.5: Run contract tests + full workspace**

```bash
cargo test -p forge-core record_tool_use_roundtrip list_tool_calls_roundtrip tool_call_recorded tool_call_list response_error_roundtrips
cargo test -p forge-core record_tool_use_defaults
cargo test --workspace
```

Expected: 7 new contract tests pass, workspace builds clean (handler stubs mean no unimplemented arms).

- [ ] **Step 4.6: Fmt + clippy + full test**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace
```

- [ ] **Step 4.7: Commit**

```bash
git add crates/core/src/protocol/ crates/daemon/src/server/handler.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T4): protocol variants + handler stubs

Add Request::RecordToolUse + Request::ListToolCalls with
#[serde(default = "default_empty_args")] on tool_args and
#[serde(default)] on tool_result_summary / user_correction_flag /
agent / limit. ListToolCalls.session_id is REQUIRED (target-session
org safety, spec §10).

Add ResponseData::ToolCallRecorded and ResponseData::ToolCallList
wrapping Vec<ToolCallRow>, matching PreferenceReaffirmed precedent.

Handler stubs return Response::Error "unimplemented: ..." so workspace
compiles through T5-T9 (Claude HIGH #7 fix).

7 new contract tests: Request roundtrips, optional-field defaults,
Response success roundtrips (empty + 3-row Vec), Response::Error
with all 6 documented prefix codes.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `handle_record_tool_use` happy path + atomic INSERT-SELECT

**Files:**
- Modify: `crates/daemon/src/server/handler.rs`

- [ ] **Step 5.1: Write the happy-path L2 test**

Append to the `#[cfg(test)] mod tests` block in `handler.rs` (match the existing pattern used for `record_tool_use` → find where ReaffirmPreference tests live and co-locate):

```rust
#[test]
fn record_tool_use_happy_path_returns_id_and_created_at() {
    let state = crate::server::handler::test_support::new_state();
    // Seed a session (use whatever helper the file already has for test sessions;
    // fallback: direct INSERT).
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('SESS1', 'claude-code', '2026-04-19 10:00:00', 'active', 'acme')",
        [],
    ).unwrap();

    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "SESS1".to_string(),
        agent: "claude-code".to_string(),
        tool_name: "Read".to_string(),
        tool_args: serde_json::json!({"file_path": "/tmp/a"}),
        tool_result_summary: "ok".to_string(),
        success: true,
        user_correction_flag: false,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallRecorded { id, created_at },
        } => {
            assert_eq!(id.len(), 26, "ULID is 26 chars");
            assert!(created_at.starts_with("20"), "created_at is ISO-ish date");

            // Verify the row was actually persisted with the target session's org.
            let org: String = state.conn.query_row(
                "SELECT organization_id FROM session_tool_call WHERE id = ?1",
                rusqlite::params![id], |row| row.get(0),
            ).unwrap();
            assert_eq!(org, "acme", "organization_id is sourced from target session, not default");
        }
        other => panic!("expected ToolCallRecorded, got {other:?}"),
    }
}
```

Note: the names `test_support::new_state` and `dispatch` are placeholders for whichever helpers the file already uses. Search for `fn new_state()` or similar in the test module, or adapt to the existing test harness pattern (e.g., an `AppState` builder + `handle_request(&state, req)`).

- [ ] **Step 5.2: Run the failing test**

```bash
cargo test -p forge-daemon record_tool_use_happy_path_returns_id_and_created_at
```

Expected: FAIL — current stub returns `Response::Error { message: "unimplemented: record_tool_use (T5)" }`.

- [ ] **Step 5.3: Replace the stub with the real handler**

In `crates/daemon/src/server/handler.rs`, find the `Request::RecordToolUse { .. }` stub arm from T4 Step 4.3. Replace with:

```rust
        Request::RecordToolUse {
            session_id, agent, tool_name, tool_args, tool_result_summary,
            success, user_correction_flag,
        } => {
            // Step 1 — basic ULID + timestamp + canonical args.
            let id = ulid::Ulid::new().to_string();
            let created_at = forge_core::time::now_iso();
            let canonical = match serde_json::to_string(&tool_args) {
                Ok(s) => s,
                Err(e) => {
                    return Response::Error {
                        message: format!("internal_error: serde_json::to_string failed: {e}"),
                    }
                }
            };

            // Step 2 — atomic INSERT-SELECT. Validation fused with write.
            let rows = state.conn.execute(
                "INSERT INTO session_tool_call
                    (id, session_id, agent, tool_name, tool_args, tool_result_summary,
                     success, user_correction_flag, organization_id, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                        COALESCE(s.organization_id, 'default'), ?9
                 FROM session s
                 WHERE s.id = ?2",
                rusqlite::params![
                    id, session_id, agent, tool_name,
                    canonical, tool_result_summary,
                    success as i64, user_correction_flag as i64, created_at,
                ],
            );

            match rows {
                Ok(1) => {
                    Response::Ok {
                        data: forge_core::protocol::ResponseData::ToolCallRecorded {
                            id, created_at,
                        },
                    }
                }
                Ok(0) => Response::Error {
                    message: format!("unknown_session: {session_id}"),
                },
                Ok(n) => Response::Error {
                    message: format!("internal_error: INSERT affected {n} rows (expected 1)"),
                },
                Err(e) => Response::Error {
                    message: format!("internal_error: {e}"),
                },
            }
        }
```

- [ ] **Step 5.4: Run the test to verify it passes**

```bash
cargo test -p forge-daemon record_tool_use_happy_path_returns_id_and_created_at
```

Expected: 1 passed.

- [ ] **Step 5.5: Add a second test — persistence roundtrip via list (manual query)**

```rust
#[test]
fn record_tool_use_persists_all_fields_roundtrip_via_direct_select() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'claude-code', '2026-04-19 10:00:00', 'active', 'acme')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(),
        agent: "claude-code".to_string(),
        tool_name: "Bash".to_string(),
        tool_args: serde_json::json!({"cmd": "ls"}),
        tool_result_summary: "ok".to_string(),
        success: false,
        user_correction_flag: true,
    };
    let _ = crate::server::handler::dispatch(&state, req);

    let (agent, tool, args, summary, success, correction, org): (
        String, String, String, String, i64, i64, String
    ) = state.conn.query_row(
        "SELECT agent, tool_name, tool_args, tool_result_summary, success,
                user_correction_flag, organization_id FROM session_tool_call LIMIT 1",
        [], |row| Ok((
            row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
            row.get(5)?, row.get(6)?,
        )),
    ).unwrap();

    assert_eq!(agent, "claude-code");
    assert_eq!(tool, "Bash");
    assert_eq!(args, r#"{"cmd":"ls"}"#);
    assert_eq!(summary, "ok");
    assert_eq!(success, 0);
    assert_eq!(correction, 1);
    assert_eq!(org, "acme");
}
```

- [ ] **Step 5.6: Run the second test**

```bash
cargo test -p forge-daemon record_tool_use_persists_all_fields_roundtrip_via_direct_select
```

Expected: pass.

- [ ] **Step 5.7: Fmt + clippy + full test**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace
```

- [ ] **Step 5.8: Commit**

```bash
git add crates/daemon/src/server/handler.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T5): handle_record_tool_use atomic INSERT-SELECT

Replace T4 stub with atomic INSERT...SELECT FROM session WHERE id=?
(eliminates TOCTOU race between validation and persistence —
Codex BLOCKER #2 fix). organization_id is sourced from target
session via COALESCE (s.organization_id, 'default'); row count 0
→ unknown_session; row count > 1 (unreachable) → internal_error.

ULID generation + wall-clock created_at per existing conventions.
tool_args is canonically serialised once by the handler; ops layer
stays thin.

Tests: happy path returns ULID id + created_at, target-session
organization_id is the one persisted (not the caller's default),
all fields round-trip through a direct SELECT.

Validation + events added in T6/T7.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `handle_record_tool_use` validation — 8 error paths

**Files:**
- Modify: `crates/daemon/src/server/handler.rs`

- [ ] **Step 6.1: Write the `unknown_session` test first (already works — pin it)**

```rust
#[test]
fn record_tool_use_rejects_unknown_session() {
    let state = crate::server::handler::test_support::new_state();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "NONEXISTENT".to_string(),
        agent: "a".to_string(),
        tool_name: "T".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: String::new(),
        success: true,
        user_correction_flag: false,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Error { message } => {
            assert!(message.starts_with("unknown_session: "), "got {message}");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}
```

- [ ] **Step 6.2: Run + verify it passes (the atomic INSERT already rejects)**

```bash
cargo test -p forge-daemon record_tool_use_rejects_unknown_session
```

Expected: pass.

- [ ] **Step 6.3: Add the `session_deleted_between_client_send_and_daemon_execute` test**

```rust
#[test]
fn record_tool_use_rejects_session_deleted_between_validate_and_execute() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    // Simulate the "session deleted between send and execute" case by deleting it
    // before the handler runs. The atomic INSERT-SELECT must reject with unknown_session
    // AND leave no orphan row.
    state.conn.execute("DELETE FROM session WHERE id = 'S'", []).unwrap();

    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(),
        agent: "a".to_string(),
        tool_name: "T".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: String::new(),
        success: true,
        user_correction_flag: false,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    assert!(matches!(resp, forge_core::protocol::Response::Error { ref message } if message.starts_with("unknown_session: ")));

    // Atomic INSERT-SELECT proves no orphan row.
    let count: i64 = state.conn.query_row(
        "SELECT COUNT(*) FROM session_tool_call", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(count, 0, "no row should be inserted when session is missing");
}
```

- [ ] **Step 6.4: Run + verify pass**

```bash
cargo test -p forge-daemon record_tool_use_rejects_session_deleted_between_validate_and_execute
```

Expected: pass (atomic pattern handles this automatically).

- [ ] **Step 6.5: Write the empty/whitespace/control-char validation tests (RED)**

```rust
#[test]
fn record_tool_use_rejects_empty_tool_name() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(), agent: "a".to_string(), tool_name: "".to_string(),
        tool_args: serde_json::json!({}), tool_result_summary: String::new(),
        success: true, user_correction_flag: false,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Error { message } =>
            assert_eq!(message, "empty_field: tool_name"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn record_tool_use_rejects_whitespace_only_tool_name() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(), agent: "a".to_string(), tool_name: "   \t  ".to_string(),
        tool_args: serde_json::json!({}), tool_result_summary: String::new(),
        success: true, user_correction_flag: false,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    assert!(matches!(resp, forge_core::protocol::Response::Error { ref message } if message == "empty_field: tool_name"));
}

#[test]
fn record_tool_use_rejects_empty_agent() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(), agent: "".to_string(), tool_name: "T".to_string(),
        tool_args: serde_json::json!({}), tool_result_summary: String::new(),
        success: true, user_correction_flag: false,
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message } if message == "empty_field: agent"
    ));
}

#[test]
fn record_tool_use_rejects_control_character_in_session_id() {
    let state = crate::server::handler::test_support::new_state();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "abc\0xyz".to_string(), agent: "a".to_string(), tool_name: "T".to_string(),
        tool_args: serde_json::json!({}), tool_result_summary: String::new(),
        success: true, user_correction_flag: false,
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message }
            if message == "invalid_field: session_id: control_character"
    ));
}

#[test]
fn record_tool_use_rejects_tool_args_over_64kb() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let big: String = "A".repeat(70_000);
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(), agent: "a".to_string(), tool_name: "T".to_string(),
        tool_args: serde_json::json!({"x": big}),
        tool_result_summary: String::new(),
        success: true, user_correction_flag: false,
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message }
            if message == "payload_too_large: tool_args: 65536"
    ));
}

#[test]
fn record_tool_use_rejects_tool_result_summary_over_64kb() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(), agent: "a".to_string(), tool_name: "T".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: "B".repeat(70_000),
        success: true, user_correction_flag: false,
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message }
            if message == "payload_too_large: tool_result_summary: 65536"
    ));
}

#[test]
fn record_tool_use_accepts_unicode_in_tool_name_and_agent() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'claude-code', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(),
        agent: "αβγ-😀".to_string(),
        tool_name: "Чтение".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: String::new(),
        success: true, user_correction_flag: false,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    assert!(matches!(resp, forge_core::protocol::Response::Ok { .. }),
            "unicode strings without control chars must be accepted, got {resp:?}");
}
```

- [ ] **Step 6.6: Run all new tests — expect the first 6 to FAIL**

```bash
cargo test -p forge-daemon record_tool_use_rejects_empty record_tool_use_rejects_whitespace record_tool_use_rejects_control record_tool_use_rejects_tool_args record_tool_use_rejects_tool_result record_tool_use_accepts_unicode
```

Expected: 6 FAIL — validation is missing. The last (`accepts_unicode`) should pass since the handler already accepts.

- [ ] **Step 6.7: Add validation in the handler**

In `crates/daemon/src/server/handler.rs`, modify the `Request::RecordToolUse { ... } => { ... }` arm. Insert the validation block BEFORE the `let id = ulid::Ulid::new()...` line (from T5 Step 5.3):

```rust
        Request::RecordToolUse {
            session_id, agent, tool_name, tool_args, tool_result_summary,
            success, user_correction_flag,
        } => {
            // Validation (fail-fast, before any DB touch).
            fn has_control_char(s: &str) -> bool {
                s.chars().any(|c| (c as u32) < 0x20 && c != '\t')
            }
            if tool_name.trim().is_empty() {
                return Response::Error { message: "empty_field: tool_name".to_string() };
            }
            if agent.trim().is_empty() {
                return Response::Error { message: "empty_field: agent".to_string() };
            }
            if has_control_char(&session_id) {
                return Response::Error {
                    message: "invalid_field: session_id: control_character".to_string(),
                };
            }
            if has_control_char(&agent) {
                return Response::Error {
                    message: "invalid_field: agent: control_character".to_string(),
                };
            }
            if has_control_char(&tool_name) {
                return Response::Error {
                    message: "invalid_field: tool_name: control_character".to_string(),
                };
            }
            if tool_result_summary.len() > 65536 {
                return Response::Error {
                    message: "payload_too_large: tool_result_summary: 65536".to_string(),
                };
            }

            // ULID + timestamp + canonical args (serialisation order matters because
            // we check the serialised size).
            let id = ulid::Ulid::new().to_string();
            let created_at = forge_core::time::now_iso();
            let canonical = match serde_json::to_string(&tool_args) {
                Ok(s) => s,
                Err(e) => {
                    return Response::Error {
                        message: format!("internal_error: serde_json::to_string failed: {e}"),
                    }
                }
            };
            if canonical.len() > 65536 {
                return Response::Error {
                    message: "payload_too_large: tool_args: 65536".to_string(),
                };
            }

            // Atomic INSERT-SELECT (unchanged from T5).
            let rows = state.conn.execute(
                "INSERT INTO session_tool_call
                    (id, session_id, agent, tool_name, tool_args, tool_result_summary,
                     success, user_correction_flag, organization_id, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                        COALESCE(s.organization_id, 'default'), ?9
                 FROM session s
                 WHERE s.id = ?2",
                rusqlite::params![
                    id, session_id, agent, tool_name,
                    canonical, tool_result_summary,
                    success as i64, user_correction_flag as i64, created_at,
                ],
            );

            match rows {
                Ok(1) => Response::Ok {
                    data: forge_core::protocol::ResponseData::ToolCallRecorded {
                        id, created_at,
                    },
                },
                Ok(0) => Response::Error {
                    message: format!("unknown_session: {session_id}"),
                },
                Ok(n) => Response::Error {
                    message: format!("internal_error: INSERT affected {n} rows (expected 1)"),
                },
                Err(e) => Response::Error { message: format!("internal_error: {e}") },
            }
        }
```

- [ ] **Step 6.8: Run the 7 validation tests + full test**

```bash
cargo test -p forge-daemon record_tool_use_rejects record_tool_use_accepts_unicode
cargo test --workspace
```

Expected: 7 + 2 (from T5) = 9 `record_tool_use_*` tests pass.

- [ ] **Step 6.9: Fmt + clippy + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace

git add crates/daemon/src/server/handler.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T6): handle_record_tool_use validation (8 error paths)

Validation before any DB touch:
- empty_field: tool_name (empty or whitespace-only)
- empty_field: agent (empty or whitespace-only)
- invalid_field: session_id/agent/tool_name: control_character
  (any char < 0x20 except \t rejects)
- payload_too_large: tool_args: 65536 (measured on serialised form)
- payload_too_large: tool_result_summary: 65536 (UTF-8 byte count)

Unicode (non-control) strings accepted affirmatively.

All error responses are Response::Error { message } with documented
<code>: <detail> prefix (§4.5). No typed error enum — matches the
existing codebase wire format.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `handle_record_tool_use` event emission

**Files:**
- Modify: `crates/daemon/src/server/handler.rs`

- [ ] **Step 7.1: Write the three event tests first (RED)**

```rust
#[test]
fn record_tool_use_emits_tool_use_recorded_event_only_after_insert_succeeds() {
    let state = crate::server::handler::test_support::new_state();
    let mut rx = state.events.subscribe();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'claude-code', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();

    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(),
        agent: "claude-code".to_string(),
        tool_name: "Read".to_string(),
        tool_args: serde_json::json!({"file_path": "/tmp/a"}),
        tool_result_summary: "ok".to_string(),
        success: true,
        user_correction_flag: false,
    };
    let _ = crate::server::handler::dispatch(&state, req);

    let event = rx.try_recv().expect("event must be emitted");
    assert_eq!(event.event, "tool_use_recorded");
    let data = &event.data;
    assert!(data.get("id").is_some());
    assert_eq!(data.get("session_id").and_then(|v| v.as_str()), Some("S"));
    assert_eq!(data.get("agent").and_then(|v| v.as_str()), Some("claude-code"));
    assert_eq!(data.get("tool_name").and_then(|v| v.as_str()), Some("Read"));
    assert_eq!(data.get("success").and_then(|v| v.as_bool()), Some(true));
    assert!(data.get("created_at").and_then(|v| v.as_str()).is_some());
    assert!(data.get("tool_args").is_none(), "tool_args MUST NOT be in event");
    assert!(data.get("tool_result_summary").is_none(), "summary MUST NOT be in event");
    assert!(data.get("user_correction_flag").is_none(), "correction_flag MUST NOT be in event");
}

#[test]
fn record_tool_use_does_not_emit_event_on_validation_error() {
    let state = crate::server::handler::test_support::new_state();
    let mut rx = state.events.subscribe();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "S".to_string(),
        agent: "".to_string(),  // invalid
        tool_name: "T".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: String::new(),
        success: true,
        user_correction_flag: false,
    };
    let _ = crate::server::handler::dispatch(&state, req);
    assert!(rx.try_recv().is_err(), "no event should be emitted on validation error");
}

#[test]
fn record_tool_use_does_not_emit_event_on_unknown_session() {
    let state = crate::server::handler::test_support::new_state();
    let mut rx = state.events.subscribe();
    let req = forge_core::protocol::Request::RecordToolUse {
        session_id: "NONEXISTENT".to_string(),
        agent: "a".to_string(),
        tool_name: "T".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: String::new(),
        success: true,
        user_correction_flag: false,
    };
    let _ = crate::server::handler::dispatch(&state, req);
    assert!(rx.try_recv().is_err(), "no event should be emitted when session is unknown");
}
```

- [ ] **Step 7.2: Run — expect the first to FAIL (no event emitted yet), other two to pass**

```bash
cargo test -p forge-daemon record_tool_use_emits_tool_use_recorded record_tool_use_does_not_emit
```

Expected: 1 FAIL (emits), 2 PASS (the "does not emit" tests trivially pass because we don't emit anything yet).

- [ ] **Step 7.3: Add `events::emit` call after successful INSERT**

In `crates/daemon/src/server/handler.rs`, modify the `Ok(1) => ...` match arm of the INSERT result:

```rust
                Ok(1) => {
                    crate::events::emit(
                        &state.events,
                        "tool_use_recorded",
                        serde_json::json!({
                            "id":         id,
                            "session_id": session_id,
                            "agent":      agent,
                            "tool_name":  tool_name,
                            "success":    success,
                            "created_at": created_at,
                        }),
                    );
                    Response::Ok {
                        data: forge_core::protocol::ResponseData::ToolCallRecorded {
                            id, created_at,
                        },
                    }
                }
```

- [ ] **Step 7.4: Run + verify all 3 event tests pass**

```bash
cargo test -p forge-daemon record_tool_use_emits record_tool_use_does_not_emit
cargo test --workspace
```

Expected: all 3 pass; no regressions.

- [ ] **Step 7.5: Fmt + clippy + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace

git add crates/daemon/src/server/handler.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T7): tool_use_recorded event emission

Post-INSERT-success only. Payload: id, session_id, agent, tool_name,
success, created_at. EXCLUDES tool_args + tool_result_summary (size,
PII) + user_correction_flag (c2 filter contract still unlocked).

Validation errors and unknown_session paths do NOT emit. Broadcast
is fire-and-forget (non-authoritative — session_tool_call table is
the source of truth, per §9).

Tests: emitted-on-success with correct shape (positive + negative
field presence), not-emitted-on-validation-error, not-emitted-on-
unknown-session.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `handle_list_tool_calls` happy path + snapshot-consistent read

**Files:**
- Modify: `crates/daemon/src/server/handler.rs`

- [ ] **Step 8.1: Write 5 happy-path tests (RED)**

```rust
#[test]
fn list_tool_calls_happy_path_returns_newest_first() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    for (i, id) in ["01A", "01B", "01C"].iter().enumerate() {
        state.conn.execute(
            "INSERT INTO session_tool_call VALUES (?1, 'S', 'a', 'T', '{}', '', 1, 0, 'default',
             '2026-04-19 12:00:0?2')",
            rusqlite::params![id, i as i64],
        ).unwrap_or_else(|_| {
            // fallback if the ?2 insertion isn't supported in that position; do literal.
            state.conn.execute(
                &format!("INSERT INTO session_tool_call VALUES ('{id}', 'S', 'a', 'T', '{{}}', '', 1, 0, 'default', '2026-04-19 12:00:0{i}')"),
                [],
            ).unwrap()
        });
    }
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(), agent: None, limit: None,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallList { calls },
        } => {
            let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
            assert_eq!(ids, vec!["01C", "01B", "01A"]);
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn list_tool_calls_defaults_limit_to_50_when_none() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    for i in 0..60 {
        state.conn.execute(
            &format!("INSERT INTO session_tool_call VALUES ('ID{i:03}', 'S', 'a', 'T', '{{}}', '', 1, 0, 'default', '2026-04-19 12:00:00')"),
            [],
        ).unwrap();
    }
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(), agent: None, limit: None,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallList { calls },
        } => assert_eq!(calls.len(), 50),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn list_tool_calls_treats_limit_zero_as_default_50() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    for i in 0..60 {
        state.conn.execute(
            &format!("INSERT INTO session_tool_call VALUES ('ID{i:03}', 'S', 'a', 'T', '{{}}', '', 1, 0, 'default', '2026-04-19 12:00:00')"),
            [],
        ).unwrap();
    }
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(), agent: None, limit: Some(0),
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallList { calls },
        } => assert_eq!(calls.len(), 50, "limit=0 treated as default 50"),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn list_tool_calls_agent_filter_narrows_result() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    state.conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'alice', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();
    state.conn.execute(
        "INSERT INTO session_tool_call VALUES ('B', 'S', 'bob', 'T', '{}', '', 1, 0, 'default',
         '2026-04-19 12:00:00')", []).unwrap();

    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(),
        agent: Some("alice".to_string()),
        limit: None,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallList { calls },
        } => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].agent, "alice");
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn list_tool_calls_tiebreaks_identical_created_at_by_id_desc() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    for id in ["01A", "01B", "01C"] {
        state.conn.execute(
            &format!("INSERT INTO session_tool_call VALUES ('{id}', 'S', 'a', 'T', '{{}}', '', 1, 0, 'default', '2026-04-19 12:00:00')"),
            [],
        ).unwrap();
    }
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(), agent: None, limit: None,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallList { calls },
        } => {
            let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
            assert_eq!(ids, vec!["01C", "01B", "01A"], "tiebreak by id DESC");
        }
        other => panic!("got {other:?}"),
    }
}
```

- [ ] **Step 8.2: Run the tests — expect all to FAIL (stub returns Error)**

```bash
cargo test -p forge-daemon list_tool_calls_happy_path list_tool_calls_defaults_limit list_tool_calls_treats_limit_zero list_tool_calls_agent_filter list_tool_calls_tiebreaks
```

Expected: 5 FAIL.

- [ ] **Step 8.3: Replace the ListToolCalls stub with the real handler**

In `crates/daemon/src/server/handler.rs`, replace the `Request::ListToolCalls { .. } => ...` stub arm with:

```rust
        Request::ListToolCalls { session_id, agent, limit } => {
            fn has_control_char(s: &str) -> bool {
                s.chars().any(|c| (c as u32) < 0x20 && c != '\t')
            }

            // Validation (fail-fast).
            if has_control_char(&session_id) {
                return Response::Error {
                    message: "invalid_field: session_id: control_character".to_string(),
                };
            }
            if let Some(ref a) = agent {
                if has_control_char(a) {
                    return Response::Error {
                        message: "invalid_field: agent: control_character".to_string(),
                    };
                }
            }
            let effective_limit: usize = match limit {
                None => 50,
                Some(0) => 50,
                Some(n) if n > 500 => {
                    return Response::Error {
                        message: format!("limit_too_large: requested {n}, max 500"),
                    };
                }
                Some(n) => n,
            };

            // Open snapshot transaction, derive caller_org, scan, commit.
            let tx = match state.conn.unchecked_transaction() {
                Ok(t) => t,
                Err(e) => return Response::Error { message: format!("internal_error: {e}") },
            };

            let caller_org: String = match tx.query_row(
                "SELECT COALESCE(organization_id, 'default') FROM session WHERE id = ?1",
                rusqlite::params![&session_id],
                |row| row.get::<_, String>(0),
            ) {
                Ok(s) => s,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Response::Error {
                        message: format!("unknown_session: {session_id}"),
                    };
                }
                Err(e) => {
                    return Response::Error { message: format!("internal_error: {e}") };
                }
            };

            let rows = match crate::db::ops::list_tool_calls(
                &tx, &caller_org, &session_id, agent.as_deref(), effective_limit,
            ) {
                Ok(r) => r,
                Err(e) => return Response::Error { message: format!("internal_error: {e}") },
            };

            if let Err(e) = tx.commit() {
                return Response::Error { message: format!("internal_error: {e}") };
            }

            Response::Ok {
                data: forge_core::protocol::ResponseData::ToolCallList { calls: rows },
            }
        }
```

- [ ] **Step 8.4: Run the tests to verify they pass**

```bash
cargo test -p forge-daemon list_tool_calls_happy_path list_tool_calls_defaults_limit list_tool_calls_treats_limit_zero list_tool_calls_agent_filter list_tool_calls_tiebreaks
```

Expected: all 5 pass.

- [ ] **Step 8.5: Fmt + clippy + full test + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace

git add crates/daemon/src/server/handler.rs
git commit -m "$(cat <<'EOF'
feat(2A-4c1 T8): handle_list_tool_calls snapshot-consistent read

Replace T4 stub with transaction-wrapped read:
- Open unchecked_transaction (snapshot isolation).
- SELECT organization_id from target session (QueryReturnedNoRows
  distinguished from other rusqlite errors — Claude HIGH #6 fix).
- Call ops::list_tool_calls inside same tx.
- Commit.

Limit normalisation: None and Some(0) → 50; Some(n>500) →
limit_too_large error. Control-char rejection on session_id and
agent filter. ORDER BY created_at DESC, id DESC tiebreaker via
ops layer.

Tests: happy path DESC ordering, default limit 50, limit=0 as 50,
agent filter narrows, identical-created_at tiebreak by id.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `handle_list_tool_calls` validation + target-session-org behavior

**Files:**
- Modify: `crates/daemon/src/server/handler.rs`

- [ ] **Step 9.1: Write 6 validation + scope tests (RED)**

```rust
#[test]
fn list_tool_calls_rejects_limit_over_500() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(), agent: None, limit: Some(1000),
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message }
            if message == "limit_too_large: requested 1000, max 500"
    ));
}

#[test]
fn list_tool_calls_rejects_unknown_session() {
    let state = crate::server::handler::test_support::new_state();
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "NONEXISTENT".to_string(), agent: None, limit: None,
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message }
            if message.starts_with("unknown_session: ")
    ));
}

#[test]
fn list_tool_calls_rejects_control_character_in_session_id() {
    let state = crate::server::handler::test_support::new_state();
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "abc\0xyz".to_string(), agent: None, limit: None,
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message }
            if message == "invalid_field: session_id: control_character"
    ));
}

#[test]
fn list_tool_calls_rejects_control_character_in_agent_filter() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();
    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(),
        agent: Some("bad\0agent".to_string()),
        limit: None,
    };
    assert!(matches!(
        crate::server::handler::dispatch(&state, req),
        forge_core::protocol::Response::Error { ref message }
            if message == "invalid_field: agent: control_character"
    ));
}

#[test]
fn list_tool_calls_returns_only_target_session_org_rows() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'acme')",
        [],
    ).unwrap();
    // Correctly-tagged row in acme org.
    state.conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'S', 'a', 'T', '{}', '', 1, 0, 'acme',
         '2026-04-19 12:00:00')", []).unwrap();
    // Manually-forged row with wrong org (should not appear).
    state.conn.execute(
        "INSERT INTO session_tool_call VALUES ('B', 'S', 'a', 'T', '{}', '', 1, 0, 'other_org',
         '2026-04-19 12:00:00')", []).unwrap();

    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "S".to_string(), agent: None, limit: None,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallList { calls },
        } => {
            let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
            assert_eq!(ids, vec!["A"], "only target-session-org rows surface");
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn list_tool_calls_does_not_leak_other_sessions_in_same_org() {
    let state = crate::server::handler::test_support::new_state();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('SA', 'a', '2026-04-19 10:00:00', 'active', 'acme')", []).unwrap();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('SB', 'a', '2026-04-19 10:00:00', 'active', 'acme')", []).unwrap();
    state.conn.execute(
        "INSERT INTO session_tool_call VALUES ('A', 'SA', 'a', 'T', '{}', '', 1, 0, 'acme',
         '2026-04-19 12:00:00')", []).unwrap();
    state.conn.execute(
        "INSERT INTO session_tool_call VALUES ('B', 'SB', 'a', 'T', '{}', '', 1, 0, 'acme',
         '2026-04-19 12:00:00')", []).unwrap();

    let req = forge_core::protocol::Request::ListToolCalls {
        session_id: "SA".to_string(), agent: None, limit: None,
    };
    let resp = crate::server::handler::dispatch(&state, req);
    match resp {
        forge_core::protocol::Response::Ok {
            data: forge_core::protocol::ResponseData::ToolCallList { calls },
        } => {
            let ids: Vec<&str> = calls.iter().map(|c| c.id.as_str()).collect();
            assert_eq!(ids, vec!["A"], "listing session SA must not leak SB's rows");
        }
        other => panic!("got {other:?}"),
    }
}
```

- [ ] **Step 9.2: Run the tests to verify they pass**

The handler from T8 already implements all these behaviours (control-char rejection, limit>500 rejection, unknown-session rejection, scoped WHERE clause). They should pass without code changes.

```bash
cargo test -p forge-daemon list_tool_calls_rejects list_tool_calls_returns_only list_tool_calls_does_not_leak
```

Expected: 6 passed (no further handler changes needed).

- [ ] **Step 9.3: Fmt + clippy + full test + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace

git add crates/daemon/src/server/handler.rs
git commit -m "$(cat <<'EOF'
test(2A-4c1 T9): list_tool_calls validation + target-session-org

Six L2 tests pinning behaviour already implemented in T8:
- limit_too_large: requested N, max 500
- unknown_session: <id>
- invalid_field: session_id: control_character
- invalid_field: agent: control_character
- target-session-org-only rows (forged cross-org row invisible)
- no-leak-other-sessions-same-org (session_id scope tight)

The last two tests are the direct pins on the
"target-session org consistency" property from spec §10.3 —
NOT a cross-caller isolation guarantee (see §11.1).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Integration test `record_tool_use_flow.rs`

**Files:**
- Create: `crates/daemon/tests/record_tool_use_flow.rs`

- [ ] **Step 10.1: Write the end-to-end integration test**

Create `crates/daemon/tests/record_tool_use_flow.rs`:

```rust
//! Integration test for Phase 2A-4c1 Forge-Tool-Use-Recording.
//!
//! Exercises the full Request::RecordToolUse + Request::ListToolCalls path
//! end-to-end through the handler, including:
//!   - happy-path record of 3 calls (success, failure, correction-flagged)
//!   - ListToolCalls with session filter (verifies DESC ordering + all fields
//!     round-trip through serde_json::Value including nested tool_args)
//!   - ListToolCalls with session + agent filter
//!   - cross-session no-leak (same-org, different session_id)
//!   - target-session organization_id is sourced from the session, not the caller

use forge_core::protocol::{Request, Response, ResponseData};

fn setup() -> forge_daemon::server::handler::AppState {
    // Use whatever constructor the existing integration tests use; fallback below
    // assumes test_support::new_state() and public state fields.
    forge_daemon::server::handler::test_support::new_state()
}

#[test]
fn record_tool_use_flow_end_to_end() {
    let state = setup();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('SESS1', 'claude-code', '2026-04-19 10:00:00', 'active', 'acme')",
        [],
    ).unwrap();

    // 1. Record 3 calls.
    let calls_to_record = [
        (true, false, "Read", serde_json::json!({"file_path": "/tmp/a"}), "ok"),
        (false, false, "Bash", serde_json::json!({"cmd": "false"}), "exit 1"),
        (true, true,  "Read", serde_json::json!({"file_path": "/tmp/b"}), "ok but corrected"),
    ];
    for (success, correction, tool, args, summary) in calls_to_record {
        let req = Request::RecordToolUse {
            session_id: "SESS1".to_string(),
            agent: "claude-code".to_string(),
            tool_name: tool.to_string(),
            tool_args: args,
            tool_result_summary: summary.to_string(),
            success, user_correction_flag: correction,
        };
        let resp = forge_daemon::server::handler::dispatch(&state, req);
        assert!(matches!(resp,
            Response::Ok { data: ResponseData::ToolCallRecorded { .. } }));
        // Tiny sleep so created_at differs at the second granularity.
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }

    // 2. ListToolCalls session-only, verify 3 rows newest-first.
    let resp = forge_daemon::server::handler::dispatch(&state,
        Request::ListToolCalls {
            session_id: "SESS1".to_string(), agent: None, limit: None,
        });
    let calls = match resp {
        Response::Ok { data: ResponseData::ToolCallList { calls } } => calls,
        other => panic!("got {other:?}"),
    };
    assert_eq!(calls.len(), 3);
    // DESC order: most recent first.
    assert_eq!(calls[0].tool_name, "Read");
    assert!(calls[0].user_correction_flag);
    // tool_args Value round-tripped correctly.
    assert_eq!(calls[0].tool_args, serde_json::json!({"file_path": "/tmp/b"}));

    // 3. ListToolCalls with agent filter (same agent — non-narrowing but exercises path).
    let resp = forge_daemon::server::handler::dispatch(&state,
        Request::ListToolCalls {
            session_id: "SESS1".to_string(),
            agent: Some("claude-code".to_string()),
            limit: None,
        });
    assert!(matches!(resp, Response::Ok { data: ResponseData::ToolCallList { ref calls } } if calls.len() == 3));
}

#[test]
fn record_tool_use_writes_target_session_org_id_not_caller_org_id() {
    // Two sessions in two different orgs. Writing to session_b must tag the row
    // with session_b's org, regardless of any "caller" concept.
    let state = setup();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('SA', 'a', '2026-04-19 10:00:00', 'active', 'org_a')",
        [],
    ).unwrap();
    state.conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('SB', 'a', '2026-04-19 10:00:00', 'active', 'org_b')",
        [],
    ).unwrap();

    // Write into session SB.
    let req = Request::RecordToolUse {
        session_id: "SB".to_string(), agent: "a".to_string(),
        tool_name: "T".to_string(),
        tool_args: serde_json::json!({}),
        tool_result_summary: String::new(),
        success: true, user_correction_flag: false,
    };
    let _ = forge_daemon::server::handler::dispatch(&state, req);

    // Verify the row was stored with org_b, not org_a or 'default'.
    let org: String = state.conn.query_row(
        "SELECT organization_id FROM session_tool_call WHERE session_id = 'SB' LIMIT 1",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(org, "org_b");

    // Verify listing session SB yields the row under org_b.
    let resp = forge_daemon::server::handler::dispatch(&state,
        Request::ListToolCalls {
            session_id: "SB".to_string(), agent: None, limit: None,
        });
    assert!(matches!(resp, Response::Ok { data: ResponseData::ToolCallList { ref calls } } if calls.len() == 1));

    // Verify listing SA returns 0 (no row tagged to org_a for SA).
    let resp = forge_daemon::server::handler::dispatch(&state,
        Request::ListToolCalls {
            session_id: "SA".to_string(), agent: None, limit: None,
        });
    assert!(matches!(resp, Response::Ok { data: ResponseData::ToolCallList { ref calls } } if calls.is_empty()));
}
```

- [ ] **Step 10.2: Check that integration tests compile with the existing test support**

The above assumes `forge_daemon::server::handler::test_support::new_state` and `forge_daemon::server::handler::dispatch` are accessible from integration tests. Open `crates/daemon/tests/` directory (e.g., `recency_decay_flow.rs` from 2A-4b T14) and mirror the import pattern.

```bash
ls crates/daemon/tests/
cat crates/daemon/tests/recency_decay_flow.rs | head -30
```

If the integration-test pattern uses a different setup function (e.g., `forge_daemon::test_support::setup()` or direct `rusqlite::Connection::open_in_memory()` + manual state construction), adapt the `setup()` body accordingly.

- [ ] **Step 10.3: Run the integration test**

```bash
cargo test -p forge-daemon --test record_tool_use_flow
```

Expected: 2 passed.

- [ ] **Step 10.4: Fmt + clippy + full test + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace

git add crates/daemon/tests/record_tool_use_flow.rs
git commit -m "$(cat <<'EOF'
test(2A-4c1 T10): end-to-end record_tool_use_flow integration tests

Two integration tests in tests/record_tool_use_flow.rs:

1. record_tool_use_flow_end_to_end: record 3 calls (success/failure/
   correction-flagged) → ListToolCalls session-only verifies 3 rows
   DESC with tool_args Value round-trip → ListToolCalls + agent
   filter verifies path exercised.

2. record_tool_use_writes_target_session_org_id_not_caller_org_id:
   two sessions (org_a, org_b), write into org_b session, verify row
   persists with org_b (not 'default' or org_a); list from org_a
   session returns empty (no cross-session leak).

This is the direct integration-level pin on the target-session org
consistency property (spec §10.3).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Schema rollback recipe test

**Files:**
- Modify: `crates/daemon/src/db/schema.rs`

- [ ] **Step 11.1: Write the rollback recipe test**

Append to the same `#[cfg(test)] mod tests` block in `schema.rs` as T2:

```rust
#[test]
fn test_session_tool_call_rollback_recipe_works_on_populated_db() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    ensure_schema(&conn).unwrap();

    // Seed a session (prerequisite for foreign-key-like org derivation).
    conn.execute(
        "INSERT INTO session (id, agent, started_at, status, organization_id)
         VALUES ('S', 'a', '2026-04-19 10:00:00', 'active', 'default')",
        [],
    ).unwrap();

    // Populate with 5 tool-call rows.
    for i in 0..5 {
        conn.execute(
            &format!("INSERT INTO session_tool_call VALUES
                ('ID{i}', 'S', 'a', 'T', '{{}}', 'ok', 1, 0, 'default',
                 '2026-04-19 12:00:00')"),
            [],
        ).unwrap();
    }

    // Execute the documented rollback recipe.
    conn.execute_batch(
        "
        DROP INDEX IF EXISTS idx_session_tool_org_session_created;
        DROP INDEX IF EXISTS idx_session_tool_name_agent;
        DROP INDEX IF EXISTS idx_session_tool_session;
        DROP TABLE IF EXISTS session_tool_call;
        ",
    ).unwrap();

    // Table and all three indexes must be gone.
    let row_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session_tool_call'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(row_count, 0, "session_tool_call table should be dropped");

    let idx_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type='index' AND name IN (
             'idx_session_tool_session',
             'idx_session_tool_name_agent',
             'idx_session_tool_org_session_created'
         )",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(idx_count, 0, "all 3 indexes should be dropped");
}
```

- [ ] **Step 11.2: Run the test**

```bash
cargo test -p forge-daemon test_session_tool_call_rollback_recipe_works_on_populated_db
```

Expected: pass (no new code — just a documented-recipe validation).

- [ ] **Step 11.3: Fmt + clippy + full test + commit**

```bash
cargo fmt --all
cargo clippy --workspace -- -W clippy::all -D warnings
cargo test --workspace

git add crates/daemon/src/db/schema.rs
git commit -m "$(cat <<'EOF'
test(2A-4c1 T11): session_tool_call rollback recipe validated

Verifies the documented spec §3.5 rollback sequence works on a
populated database: populate 5 rows → DROP 3 indexes → DROP TABLE,
then assert table + all indexes absent from sqlite_master.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Live-daemon dogfood + results doc

**Files:**
- Create: `docs/benchmarks/results/forge-tool-use-recording-2026-04-19.md`

- [ ] **Step 12.1: Build release daemon**

```bash
cargo build --release --bin forge-daemon
```

Expected: clean build, binary at `target/release/forge-daemon`.

- [ ] **Step 12.2: Shutdown + restart daemon**

```bash
# Shutdown via shutdown endpoint (if available) or SIGTERM, then nohup the new binary.
pkill -TERM forge-daemon 2>/dev/null || true
sleep 2
nohup ./target/release/forge-daemon > /tmp/forge-daemon.log 2>&1 &
sleep 3

# Verify version + git_sha.
curl -sS -X POST http://127.0.0.1:8420/api -d '{"method":"version"}' | jq .
```

Expected JSON with `git_sha` matching `git rev-parse HEAD`.

- [ ] **Step 12.3: Run the dogfood curl sequence**

```bash
DAEMON=http://127.0.0.1:8420/api

# Create a test session.
SID=$(curl -sS -X POST $DAEMON -d '{
  "method":"start_session","params":{"agent":"claude-code","project":"forge-test"}
}' | jq -r '.data.id')
echo "Session: $SID"

# Step A: Record a tool call — expect Ok + id + created_at.
curl -sS -X POST $DAEMON -d "{
  \"method\":\"record_tool_use\",
  \"params\":{
    \"session_id\":\"$SID\",\"agent\":\"claude-code\",\"tool_name\":\"Read\",
    \"tool_args\":{\"file_path\":\"/tmp/a\"},\"tool_result_summary\":\"ok\",
    \"success\":true,\"user_correction_flag\":false
  }}" | tee /tmp/step_a.json | jq .

# Step B: List for the session — expect Ok + 1 row.
curl -sS -X POST $DAEMON -d "{
  \"method\":\"list_tool_calls\",\"params\":{\"session_id\":\"$SID\"}
}" | tee /tmp/step_b.json | jq .

# Step C: Unknown session — expect Error + "unknown_session: ...".
curl -sS -X POST $DAEMON -d '{
  "method":"list_tool_calls","params":{"session_id":"01NONEXISTENT0000000000000"}
}' | tee /tmp/step_c.json | jq .

# Step D: Payload too large — expect Error + "payload_too_large: tool_args: 65536".
LARGE=$(python3 -c 'import json; print(json.dumps({"x":"A"*65537}))')
curl -sS -X POST $DAEMON -d "{
  \"method\":\"record_tool_use\",\"params\":{
    \"session_id\":\"$SID\",\"agent\":\"x\",\"tool_name\":\"x\",
    \"tool_args\":$LARGE,\"tool_result_summary\":\"\",
    \"success\":true
  }}" | tee /tmp/step_d.json | jq .

# Step E: Control char in session_id — expect Error + "invalid_field: session_id: control_character".
curl -sS -X POST $DAEMON -d '{
  "method":"list_tool_calls","params":{"session_id":"abc\u0000xyz"}
}' | tee /tmp/step_e.json | jq .
```

Capture the JSON output of each step; they'll be embedded in the results doc.

- [ ] **Step 12.4: Write the results doc**

Create `docs/benchmarks/results/forge-tool-use-recording-2026-04-19.md`:

```markdown
# Forge-Tool-Use-Recording (Phase 2A-4c1) — Results

**Phase:** 2A-4c1 of Phase 2A-4 Forge-Identity master decomposition.
**Date:** 2026-04-19
**Parent design:** `docs/superpowers/specs/2026-04-19-forge-tool-use-recording-design.md` (v3, `b1ad7d9`)
**Parent master:** `docs/benchmarks/forge-identity-master-design.md` §5 2A-4c1
**HEAD:** `<git rev-parse HEAD at ship time>`
**Prior phase:** 2A-4b Recency-weighted Preference Decay shipped on 2026-04-19 (HEAD `21aa115`).

## Summary

**SHIPPED.** 2A-4c1 adds the `session_tool_call` append-only table, `Request::RecordToolUse`
(atomic INSERT-SELECT), `Request::ListToolCalls` (snapshot-consistent read), and the
`tool_use_recorded` event. Substrate ready for 2A-4c2 Phase 23 (Behavioral Skill Inference)
and 2A-4d Dim 5 bench.

Tests: **<base+37> lib + workspace tests passing** (up from 1294 at 2A-4b). Clippy clean.
Fmt clean.

**No regression-guard benches run** — c1 touches no scoring / recall / decay surfaces.

Live-daemon dogfood (HTTP at port 8420):
- Record → Ok with ULID id + created_at ✓
- List → Ok with 1 row, tool_args round-trips as JSON ✓
- Unknown session → Error `"unknown_session: 01NONEXISTENT0000000000000"` ✓
- Payload too large (64k+ tool_args) → Error `"payload_too_large: tool_args: 65536"` ✓
- Control char in session_id → Error `"invalid_field: session_id: control_character"` ✓

## What shipped

| Task | Scope | Commit |
|------|-------|--------|
| T1 | `ToolCallRow` shared type in `core::types::tool_call` | `<sha>` |
| T2 | `session_tool_call` schema + 3 indexes | `<sha>` |
| T3 | `ops::list_tool_calls` + 8 L1 tests | `<sha>` |
| T4 | Request + Response variants + 7 contract tests + handler stubs | `<sha>` |
| T5 | `handle_record_tool_use` atomic INSERT-SELECT happy path | `<sha>` |
| T6 | `handle_record_tool_use` validation (8 error paths) | `<sha>` |
| T7 | `handle_record_tool_use` event emission | `<sha>` |
| T8 | `handle_list_tool_calls` snapshot-consistent read (5 tests) | `<sha>` |
| T9 | `handle_list_tool_calls` validation + scope (6 tests) | `<sha>` |
| T10 | Integration `record_tool_use_flow.rs` (2 tests) | `<sha>` |
| T11 | Rollback recipe test on populated DB | `<sha>` |
| T12 | Live-daemon dogfood + this results doc | `<sha>` |

(Replace `<sha>` with actual commit SHAs after each task's commit.)

## Atomic INSERT-SELECT pattern (canonical)

```sql
INSERT INTO session_tool_call
    (id, session_id, agent, tool_name, tool_args, tool_result_summary,
     success, user_correction_flag, organization_id, created_at)
SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
       COALESCE(s.organization_id, 'default'), ?9
FROM session s
WHERE s.id = ?2
```

Single statement. Validation (session existence + org derivation) fused with persistence.
Row count 0 → `unknown_session`; row count 1 → success + event emission; row count > 1 →
unreachable (PK + WHERE id=?2 guarantees ≤ 1 source row), logged as `internal_error`.

This eliminates the TOCTOU race Codex flagged in v1 review.

## Target-session org consistency

`RecordToolUse` writes with `organization_id = COALESCE(session.organization_id, 'default')`
at INSERT time. The row's org is guaranteed to match the target session's current org.

`ListToolCalls` derives `caller_org` from the target session (`SELECT organization_id FROM
session WHERE id = ?`) inside a transaction, then scans with `WHERE organization_id = ?caller_org
AND session_id = ?`. Rows from other orgs can never leak; rows from other sessions in the
same org can never leak.

This is NOT cross-caller isolation. Any caller with a valid `session_id` can read or write
that session's tool calls. Phase 2A-6 (authenticated caller-session API) owns the caller-
isolation property. See spec §11.1.

## Error-message convention (stable prefixes)

| Prefix | Meaning |
|--------|---------|
| `unknown_session: <id>` | Target session does not exist, or was deleted mid-call. |
| `payload_too_large: tool_args: 65536` | Serialised `tool_args` > 65536 UTF-8 bytes. |
| `payload_too_large: tool_result_summary: 65536` | `tool_result_summary` > 65536 UTF-8 bytes. |
| `empty_field: tool_name` | Empty or whitespace-only. |
| `empty_field: agent` | Empty or whitespace-only. |
| `invalid_field: <field>: control_character` | `\0` or other `< 0x20` (except `\t`) rejected in session_id, agent, or tool_name. |
| `limit_too_large: requested <n>, max 500` | `ListToolCalls` limit > 500. |
| `internal_error: <sanitized>` | Non-`QueryReturnedNoRows` rusqlite fault. |

Callers can prefix-match programmatically.

## Live-daemon dogfood (T12)

Rebuilt + restarted at HEAD. Exercised full 2A-4c1 surface via `POST /api`:

### Step A — Record tool call

```bash
curl -sS -X POST http://127.0.0.1:8420/api -d '{
  "method":"record_tool_use","params":{
    "session_id":"<sid>","agent":"claude-code","tool_name":"Read",
    "tool_args":{"file_path":"/tmp/a"},"tool_result_summary":"ok",
    "success":true,"user_correction_flag":false
  }}'
# → {"status":"ok","data":{"kind":"tool_call_recorded","id":"01K...","created_at":"..."}}
```

(Paste the actual response from /tmp/step_a.json here.)

### Step B — List for session

```bash
curl -sS -X POST http://127.0.0.1:8420/api -d '{
  "method":"list_tool_calls","params":{"session_id":"<sid>"}
}'
# → {"status":"ok","data":{"kind":"tool_call_list","calls":[{...}]}}
```

(Paste from /tmp/step_b.json.)

### Step C — Unknown session

```bash
curl -sS -X POST http://127.0.0.1:8420/api -d '{
  "method":"list_tool_calls","params":{"session_id":"01NONEXISTENT0000000000000"}
}'
# → {"status":"error","message":"unknown_session: 01NONEXISTENT0000000000000"}
```

(Paste from /tmp/step_c.json.)

### Step D — Payload too large

```bash
LARGE=$(python3 -c 'import json; print(json.dumps({"x":"A"*65537}))')
curl -sS -X POST http://127.0.0.1:8420/api -d "{
  \"method\":\"record_tool_use\",\"params\":{
    \"session_id\":\"<sid>\",\"agent\":\"x\",\"tool_name\":\"x\",
    \"tool_args\":$LARGE,\"tool_result_summary\":\"\",
    \"success\":true
  }}"
# → {"status":"error","message":"payload_too_large: tool_args: 65536"}
```

(Paste from /tmp/step_d.json.)

### Step E — Control char in session_id

```bash
curl -sS -X POST http://127.0.0.1:8420/api -d '{
  "method":"list_tool_calls","params":{"session_id":"abc\u0000xyz"}
}'
# → {"status":"error","message":"invalid_field: session_id: control_character"}
```

(Paste from /tmp/step_e.json.)

All 5 steps passed. Daemon version `<version>`, git_sha `<sha>` confirmed via
`{"method":"version"}` endpoint.

## Known limitations (unchanged from spec)

1. **No cross-caller isolation** — spec §11.1. Deferred to Phase 2A-6 (authenticated
   caller-session API).
2. **`internal_error:` path untested at unit level** — spec §14 "Known untested path". DB-
   fault branch exercised only at dogfood; to test directly requires rusqlite mocking / fault
   injection.
3. **`user_correction_flag` bench-seeded only** — spec §11.3. Production producer deferred.
4. **Event broadcast unfiltered** — spec §11.9. `session_id` in payload leaks org membership
   to any subscriber (existing `memory_created` precedent).
5. **Wall-clock not strictly monotonic** — spec §11.8. Tiebroken by `id DESC`.
6. **No pagination cursor** — spec §11.6. 500-row cap suffices for c1.

## Next

**Phase 2A-4c2** (Behavioral Skill Inference) builds on this: consolidator Phase 23 reads
`session_tool_call`, produces SHA-256 canonical fingerprints, dedup-inserts into `skill`
table, updates `<skills>` renderer.

**Phase 2A-4d** (Forge-Identity Bench) then composes 2A-4a + 2A-4b + 2A-4c2 outputs into a
6-dimension composite; Dim 5 asserts against `session_tool_call` fixtures seeded via
`RecordToolUse`.

Known follow-ups carried forward:
- Authenticated caller-session parameter for write-path requests (Flip, Reaffirm, Record).
- `load_config()` hot-path I/O in consolidator Phase 4 (codex v7 LOW since 2A-4b).
- Lock `expected_recall_delta = 0.20` as CLI default for regression CI (since 2A-3 handoff).
```

Fill in the `<sha>` placeholders (12 of them for T1-T12), the HEAD sha, the test count delta, the version, and paste the actual JSON output from `/tmp/step_*.json` files after running Step 12.3.

- [ ] **Step 12.5: Commit**

```bash
git add docs/benchmarks/results/forge-tool-use-recording-2026-04-19.md
git commit -m "$(cat <<'EOF'
docs(2A-4c1 T12): live-daemon dogfood + results doc

Phase 2A-4c1 Forge-Tool-Use-Recording shipped. Results doc captures
task ledger T1-T12 with SHAs, design decisions (atomic INSERT-SELECT,
target-session org consistency, stable error-prefix convention),
live dogfood curl sequence (5 steps: record, list, unknown_session,
payload_too_large, control_char), and known limitations carried
forward to 2A-4c2 / 2A-4d / Phase 2A-6.

No regression-guard benches run — c1 touches no scoring surface.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

- [ ] **Step 12.6: Push all T1-T12 commits to origin**

```bash
git log --oneline "$(git merge-base HEAD origin/master)"..HEAD | head -20
git push origin master
```

Verify the push summary shows 12 (or 12+review-fix) commits pushed.

---

## Self-review checklist

Before invoking this plan, verify:

- [ ] **Spec coverage:** Every §3-§14 spec section maps to at least one task. §3 schema → T2; §4 protocol → T4; §5 handler → T5-T9; §6 ops → T3; §7 tests → T1, T3, T4, T5, T6, T7, T8, T9, T10, T11; §8 edge cases table → T6+T9 validation tests; §9 event → T7; §10 deviation → implicitly via T8+T9 target-org tests + T10 integration; §11 limitations → documented in results doc (T12); §12 task sequence → this plan; §13 non-goals → implicitly by not building those; §14 success criteria → T12 verifies (5 curl steps, 37 test count, clippy/fmt clean). ✓
- [ ] **Placeholder scan:** no "TBD", "TODO", "fill in", "similar to Task N", "add appropriate error handling", "write tests for the above" present in this plan. ✓ (modulo user-adaptable placeholders like `<sha>`, `<version>`, and the "adapt to the existing test harness pattern" note in T5 Step 5.1 — these are flagged as review points, not silent gaps.)
- [ ] **Type consistency:** `ToolCallRow` shape matches across T1 (definition), T3 (ops returns), T4 (response embeds), T10 (integration asserts). Error strings match across T5-T9 + T12 (dogfood + results doc). `default_empty_args` is only referenced in T4 (added there). ✓

Plan complete and saved to `docs/superpowers/plans/2026-04-19-forge-tool-use-recording.md`.
