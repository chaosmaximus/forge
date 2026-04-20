# Forge-Tool-Use-Recording (Phase 2A-4c1) — Design Specification

**Phase:** 2A-4c1 of Phase 2A-4 Forge-Identity master decomposition
**Date:** 2026-04-19
**Parent master:** `docs/benchmarks/forge-identity-master-design.md` v6a §5 2A-4c1, §13 resolution index
**Prior phase:** 2A-4b Recency-weighted Preference Decay shipped on 2026-04-19 (HEAD `21aa115`).
**Follow-on phase:** 2A-4c2 Phase 23 Behavioral Skill Inference (consolidator + `<skills>` renderer + canonical fingerprint).
**Spec version:** v3 — incorporates Claude second-pass review fixes (v2 commit `1ad3deb`, v1 commit `ceea810`)

## 1. Goal

Ship the substrate needed for Phase 2A-4c2's behavioral skill inference and Phase 2A-4d's Dimension 5 bench:

1. `session_tool_call` table — append-only, session-scoped, target-org consistency-checked, non-unique on `(agent, tool_name)`.
2. `Request::RecordToolUse` — write a tool-call row, atomically (validation + insert in one SQL statement), with strict input validation.
3. `Request::ListToolCalls` — read tool-call rows, session-scoped (required), target-session-org-scoped, newest-first.
4. `tool_use_recorded` event emission per successful write — non-authoritative, fire-and-forget broadcast.

Explicitly out of scope in this sub-phase: skill inference logic (2A-4c2), Claude Code hook plumbing (2A-4c2 dogfood), `<skills>` renderer changes (2A-4c2), canonical SHA-256 fingerprinting (2A-4c2), production `user_correction_flag` producer (post-c2; bench-seeded in c1), pagination cursors (YAGNI; 500-row cap suffices), authenticated caller-session API (Phase 2A-6 per master §8).

## 2. Architecture

**Append-only child table.** `session_tool_call` is a new session-scoped child relation, following the `context_effectiveness` precedent (`schema.rs:1167–1180`). No FK enforcement (SQLite FKs are off project-wide).

**Atomic write — `INSERT ... SELECT`.** Validation (session existence + org derivation) is fused into the INSERT statement — there is no separate preflight read. This eliminates the TOCTOU race between validation and insert (codex BLOCKER #2 fix). The `organization_id` of the new row is sourced from the target session's `organization_id` column inside the same statement.

**Target-session org consistency, NOT cross-caller isolation.** The stored row's `organization_id` is guaranteed to match the target `session_id`'s current `organization_id`. This prevents type-confusion attacks (writing org-A rows tagged to an org-B session). It does **NOT** provide cross-caller isolation — any caller able to obtain a valid `session_id` can write to or read from that session's tool calls. True cross-caller isolation requires authenticated caller-session at the protocol layer (Phase 2A-6 per master §8). See §10 + §11.

**Snapshot-consistent read.** `ListToolCalls` runs the session-existence check + scoped SELECT inside a single `unchecked_transaction` so the org-derivation and the row scan see one snapshot.

**Thin ops layer.** Handler owns input validation (string sanitization, size caps, JSON serialization); ops layer owns SQL only. Same layering as `ops::remember_raw` (2A-4a T0) and `ops::supersede_memory_impl` (2A-4a T1).

**String-based error contract.** All validation failures return `Response::Error { message: String }`. Error messages follow a stable `"<error_code>: <human_detail>"` convention (see §4.5) so callers can programmatically match on the prefix. No new typed-error enum (the codebase has none — Claude+Codex BLOCKER #1 fix).

**Event emission.** `handle_record_tool_use` emits `tool_use_recorded` only after the INSERT succeeds (`rows_affected == 1`), via `events::emit` (fire-and-forget broadcast). Payload excludes `tool_args` + `tool_result_summary` + `user_correction_flag` (size / PII / c2 contract). Events are **non-authoritative** — the DB is the source of truth (§9).

**No scoring-surface touched.** Recall, decay, and hybrid_recall are untouched. No regression-guard bench reruns needed.

## 3. Schema

### 3.1 New table

```sql
CREATE TABLE IF NOT EXISTS session_tool_call (
    id                    TEXT PRIMARY KEY,           -- ULID
    session_id            TEXT NOT NULL,
    agent                 TEXT NOT NULL,
    tool_name             TEXT NOT NULL,
    tool_args             TEXT NOT NULL,              -- canonical JSON (serde_json::to_string)
    tool_result_summary   TEXT NOT NULL,              -- free-form string
    success               INTEGER NOT NULL,           -- 0 or 1
    user_correction_flag  INTEGER NOT NULL DEFAULT 0, -- 0 or 1 (downstream SQL must use !=0/=0, not =1)
    organization_id       TEXT NOT NULL DEFAULT 'default',
    created_at            TEXT NOT NULL               -- wall-clock "YYYY-MM-DD HH:MM:SS"
);
```

### 3.2 Indexes

Two from master §5 (both non-unique — tool calls can repeat) plus one query-serving index added to address codex MEDIUM #10 (the actual query pattern is `WHERE organization_id = ? AND session_id = ? [AND agent = ?] ORDER BY created_at DESC`):

```sql
-- Master §5 line 122
CREATE INDEX IF NOT EXISTS idx_session_tool_session
    ON session_tool_call (session_id, created_at);

-- Master §5 line 123
CREATE INDEX IF NOT EXISTS idx_session_tool_name_agent
    ON session_tool_call (agent, tool_name);

-- Query-serving — added by spec §3.3 deviation #4
CREATE INDEX IF NOT EXISTS idx_session_tool_org_session_created
    ON session_tool_call (organization_id, session_id, created_at DESC);
```

### 3.3 Deviations from master v6a §5

| # | Deviation | Rationale |
|---|-----------|-----------|
| 1 | Added `organization_id TEXT NOT NULL DEFAULT 'default'` column | Master did not specify cross-org scoping. We add it for target-session org consistency (§10). New rows always have a non-null org because SELECTed from `session.organization_id` via `COALESCE(..., 'default')`. |
| 2 | `tool_args`, `tool_result_summary`, `user_correction_flag` declared `NOT NULL` (with `DEFAULT 0` on the flag) | Prevents the missing-vs-empty ambiguity in storage. Wire-level optionality preserved via `#[serde(default)]` on the Request (§4.1). |
| 3 | Column order matches `context_effectiveness` precedent | Visual consistency with other append-only session child tables. |
| 4 | Added third index `idx_session_tool_org_session_created (organization_id, session_id, created_at DESC)` | Master indexes don't cover the actual query pattern; this index is required for `ListToolCalls` to perform at scale (10M+ rows). Non-master, non-controversial. |
| 5 | `user_correction_flag` storage caveat (column comment) | SQLite has no boolean type. Downstream SQL filters MUST use `!= 0` / `= 0`, not `= 1` / `<> 1`, for correctness against arbitrary integer values. |

### 3.4 Migration pattern

Idempotent `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS`. No data backfill (new table). Matches `schema.rs:384–911` forward-only pattern.

### 3.5 Rollback recipe

```sql
DROP INDEX IF EXISTS idx_session_tool_org_session_created;
DROP INDEX IF EXISTS idx_session_tool_name_agent;
DROP INDEX IF EXISTS idx_session_tool_session;
DROP TABLE IF EXISTS session_tool_call;
```

Validated by `test_session_tool_call_rollback_recipe_works` against a populated test database (§7 T11).

## 4. Protocol

### 4.1 `Request::RecordToolUse`

File: `crates/core/src/protocol/request.rs`. NOT feature-gated.

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
```

with the helper

```rust
fn default_empty_args() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}
```

Wire-level: omitting `tool_args` deserializes to `{}`; omitting `tool_result_summary` deserializes to `""`; omitting `user_correction_flag` deserializes to `false`. This matches master §5 line 117 nullable-args expectation while keeping storage strictly NOT NULL.

### 4.2 `Request::ListToolCalls`

```rust
ListToolCalls {
    session_id: String,                  // REQUIRED — narrower than master for target-session-org safety (§10)
    #[serde(default)]
    agent: Option<String>,               // AND-filter within session
    #[serde(default)]
    limit: Option<usize>,                // None → 50; > 500 rejected
},
```

### 4.3 `ResponseData` variants

File: `crates/core/src/protocol/response.rs`. Variants of the existing `ResponseData` enum (NOT top-level `Response` variants). Mirrors `ResponseData::PreferenceReaffirmed` precedent.

```rust
pub enum ResponseData {
    // … existing variants …
    ToolCallRecorded {
        id: String,          // new ULID
        created_at: String,  // wall-clock
    },
    ToolCallList {
        calls: Vec<ToolCallRow>,
    },
}
```

Successful responses are wrapped in `Response::Ok { data: ResponseData::ToolCallRecorded { … } }` / `Response::Ok { data: ResponseData::ToolCallList { … } }` per the existing enum pattern at `response.rs:1195-1199`.

### 4.4 Shared type

File: `crates/core/src/types/tool_call.rs` (NEW module; re-exported through `crates/core/src/types/mod.rs`).

```rust
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
```

`organization_id` intentionally excluded — rows are always pre-filtered to the target session's org.

### 4.5 Error message convention

All validation failures return `Response::Error { message: String }`. Messages follow the convention `"<error_code>: <human_detail>"` so callers can programmatically prefix-match. No new typed enum is introduced (the codebase doesn't have one; the existing `Response::Error { message: String }` is the wire-level error surface — see `response.rs:1199`).

| Error code prefix | Human detail | Triggered by |
|------------------|--------------|--------------|
| `unknown_session:` | `<session_id>` | `RecordToolUse`, `ListToolCalls` — session row does not exist OR exists but `INSERT ... SELECT FROM session WHERE id=? AND organization_id=?` returned 0 rows. |
| `payload_too_large:` | `<field>: <max_bytes>` | `RecordToolUse` — `tool_args` serialized > 65536 UTF-8 bytes OR `tool_result_summary` > 65536 UTF-8 bytes. Size is measured in bytes, NOT chars. |
| `limit_too_large:` | `requested <n>, max <m>` | `ListToolCalls` — `limit > 500`. |
| `empty_field:` | `<field>` | `RecordToolUse` — `tool_name` or `agent` is empty after trimming, or contains only whitespace. |
| `invalid_field:` | `<field>: <reason>` | `RecordToolUse` — `tool_name`, `agent`, or `session_id` contains a `\0` or other non-printable control character (codex MEDIUM #8). |
| `internal_error:` | `<sanitized message>` | Any rusqlite error other than `QueryReturnedNoRows` (claude HIGH #6 fix — distinguish DB faults from logical missing-row). |

These message strings are tested in §4.6 contract tests. No HTTP-status differentiation; the daemon currently returns 200 OK for all `Response::Error` payloads (existing behavior).

### 4.6 Contract tests

File: `crates/core/src/protocol/contract_tests.rs`. Parameterized vectors for:

1. `Request::RecordToolUse` round-trip — all fields present.
2. `Request::RecordToolUse` defaults — omit `tool_args`, `tool_result_summary`, `user_correction_flag`; verify they deserialize to `{}`, `""`, `false`.
3. `Request::ListToolCalls` round-trip — only required `session_id`; omit `agent` and `limit`.
4. `Request::ListToolCalls` full — all optional fields populated.
5. `ResponseData::ToolCallRecorded` round-trip wrapped in `Response::Ok`.
6. `ResponseData::ToolCallList` round-trip — empty Vec and 3-element Vec.
7. `ToolCallRow` round-trip — verify `tool_args` round-trips as `serde_json::Value`.
8. `Response::Error` round-trip with each of the 6 message-code prefixes.

## 5. Handler behavior

### 5.1 `handle_record_tool_use` — atomic INSERT … SELECT

```
Step 1. Validate strings (pre-DB):
    a. trim_str(tool_name); if trimmed empty   → "empty_field: tool_name"
    b. trim_str(agent);     if trimmed empty   → "empty_field: agent"
    c. if any of (session_id, tool_name, agent) contains '\0' or any ch < 0x20 (other than '\t')
                                                → "invalid_field: <field>: control_character"
    d. canonical = serde_json::to_string(&tool_args)?;
       if canonical.len() > 65536 (UTF-8 byte count of the serialized form)
                                                → "payload_too_large: tool_args: 65536"
    e. if tool_result_summary.len() > 65536 (UTF-8 byte count, not char count — non-ASCII
       characters consume more than one byte each)
                                                → "payload_too_large: tool_result_summary: 65536"

Step 2. Generate id + timestamp:
    let id = ulid::Ulid::new().to_string();
    let created_at = forge_core::time::now_iso();   // "YYYY-MM-DD HH:MM:SS"

Step 3. Atomic INSERT — validation + insert in one SQL statement (codex BLOCKER #2 fix):
    let n = conn.execute(
        "INSERT INTO session_tool_call
            (id, session_id, agent, tool_name, tool_args, tool_result_summary,
             success, user_correction_flag, organization_id, created_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                COALESCE(s.organization_id, 'default'), ?9
         FROM session s
         WHERE s.id = ?2",
        params![id, session_id, agent, tool_name,
                canonical, tool_result_summary,
                success as i64, user_correction_flag as i64, created_at],
    )?;
    if n == 0 → "unknown_session: <session_id>"
    if n > 1  → unreachable (PK + WHERE id=? guarantees ≤ 1 source row); treat as internal_error

Step 4. Emit event (post-INSERT-success only):
    events::emit(&state.events, "tool_use_recorded", json!({
        "id":          id,
        "session_id":  session_id,
        "agent":       agent,
        "tool_name":   tool_name,
        "success":     success,
        "created_at":  created_at,
    }));

Step 5. Return Response::Ok { data: ResponseData::ToolCallRecorded { id, created_at } };
```

Notes:
- The INSERT-SELECT is one statement; SQLite auto-commits. There is no preflight SELECT, so no TOCTOU window.
- `n == 0` collapses two distinct cases into one error (session never existed OR session row was deleted between client send and daemon execute). The combined error is correct because both warrant identical caller behavior (re-establish the session, retry).
- Event emits ONLY after `n == 1` confirmed. If the broadcast channel is full, the emit silently drops (existing project-wide contract). Tested in §7 #3 + #4.

### 5.2 `handle_list_tool_calls` — snapshot-consistent read

```
Step 1. Validate + normalize limit:
    let limit = match params.limit {
        None       => 50,
        Some(0)    => 50,                                   // 0 ≡ default
        Some(n) if n > 500 → "limit_too_large: requested <n>, max 500"
        Some(n)    => n,
    };

Step 2. Validate session_id is non-empty + control-char free (Step 1c semantics applied to session_id and the optional agent filter).

Step 3. Open snapshot-consistent transaction:
    let tx = conn.unchecked_transaction()?;

Step 4. Derive caller_org from session (one query inside tx):
    let caller_org: String = match tx.query_row(
        "SELECT COALESCE(organization_id, 'default') FROM session WHERE id = ?1",
        params![session_id], |row| row.get(0),
    ) {
        Ok(s) => s,
        Err(rusqlite::Error::QueryReturnedNoRows) → "unknown_session: <session_id>"
        Err(e) → "internal_error: <sanitized>"               // claude HIGH #6 fix
    };

Step 5. Scan rows (inside same tx, snapshot-consistent):
    let rows = ops::list_tool_calls(&tx, &caller_org, &session_id,
                                    params.agent.as_deref(), limit)?;
    tx.commit()?;

Step 6. Return Response::Ok { data: ResponseData::ToolCallList { calls: rows } };
```

The transaction guarantees `caller_org` and the row scan see one snapshot — even under concurrent UPDATE/DELETE on `session.organization_id` or DELETE on the session.

## 6. Ops layer

### 6.1 `ops::record_tool_use`

DELETED — Step 3 of §5.1 inlines the atomic INSERT-SELECT directly in the handler. There is no thin ops helper for the write path (the helper would have provided no abstraction beyond the SQL itself, and inlining makes the atomicity contract auditable in one place).

### 6.2 `ops::list_tool_calls`

```rust
pub fn list_tool_calls(
    conn: &Connection,                   // accepts both Connection and &Transaction
    organization_id: &str,
    session_id: &str,
    agent_filter: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<ToolCallRow>> {
    let (sql, params): (&'static str, Vec<Box<dyn ToSql>>) = match agent_filter {
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
    let params_refs: Vec<&dyn ToSql> = params.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(ToolCallRow {
            id:                   row.get(0)?,
            session_id:           row.get(1)?,
            agent:                row.get(2)?,
            tool_name:            row.get(3)?,
            tool_args:            serde_json::from_str(&row.get::<_, String>(4)?)
                                    .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                                        4, rusqlite::types::Type::Text, Box::new(e)))?,
            tool_result_summary:  row.get(5)?,
            success:              row.get::<_, i64>(6)? != 0,
            user_correction_flag: row.get::<_, i64>(7)? != 0,
            created_at:           row.get(8)?,
        })
    })?;
    rows.collect()
}
```

`ORDER BY created_at DESC, id DESC` is a tiebreaker for identical timestamps (wall-clock can produce duplicates within the same second) — Codex MEDIUM #9 fix to make ordering deterministic. ULID `id` is monotonic in time at millisecond granularity, so ordering by `id DESC` as a secondary key approximates sub-second ordering.

A JSON-parse failure on a stored row (corruption) propagates as `rusqlite::Error::FromSqlConversionFailure` — the handler maps this to `"internal_error: corrupt tool_args row <id>"`.

## 7. Testing strategy

### 7.1 L1 — Ops unit tests (`crates/daemon/src/db/ops.rs`, 8 tests)

1. `list_tool_calls_orders_newest_first_with_id_tiebreaker`
2. `list_tool_calls_respects_limit`
3. `list_tool_calls_filters_by_agent`
4. `list_tool_calls_returns_empty_when_org_mismatch`
5. `list_tool_calls_returns_empty_when_session_mismatch`
6. `list_tool_calls_handles_concurrent_inserts_during_read` — start a write task, ensure the read sees a stable snapshot inside its transaction.
7. `list_tool_calls_persists_user_correction_flag_true_and_false_via_atomic_insert` (drives the atomic INSERT-SELECT path used in §5.1 Step 3)
8. `list_tool_calls_propagates_corrupt_tool_args_as_conversion_error` — manually insert a malformed `tool_args` TEXT, expect `FromSqlConversionFailure`.

### 7.2 L2 — Request-path handler tests (`crates/daemon/src/server/handler.rs` cfg(test) module, 26 tests)

`RecordToolUse` (15):

1. `record_tool_use_happy_path_returns_id_and_created_at` — verifies `Response::Ok { data: ResponseData::ToolCallRecorded { id, created_at } }`.
2. `record_tool_use_persists_all_fields_roundtrip_via_list`
3. `record_tool_use_emits_tool_use_recorded_event_only_after_insert_succeeds` — assert event NOT emitted on validation error or `unknown_session`.
4. `record_tool_use_event_payload_excludes_args_result_correction`
5. `record_tool_use_rejects_unknown_session` — message starts with `"unknown_session: "`.
6. `record_tool_use_rejects_session_deleted_between_client_send_and_daemon_execute` — DELETE the session row, then call `RecordToolUse` with that session_id; expect `unknown_session` (atomic INSERT-SELECT proves no orphan row written).
7. `record_tool_use_rejects_tool_args_over_64kb_after_serialization`
8. `record_tool_use_rejects_tool_result_summary_over_64kb`
9. `record_tool_use_rejects_empty_tool_name`
10. `record_tool_use_rejects_whitespace_only_tool_name` — codex MEDIUM #8 fix.
11. `record_tool_use_rejects_empty_agent`
12. `record_tool_use_rejects_control_character_in_session_id` — `"abc\0xyz"` → `"invalid_field: session_id: control_character"`.
13. `record_tool_use_accepts_unicode_in_tool_name_and_agent` — affirmative test that emoji + non-ASCII letters work.
14. `record_tool_use_defaults_user_correction_flag_to_false_when_omitted_in_json`
15. `record_tool_use_writes_org_id_from_target_session_not_caller` — pre-seed two sessions in different orgs; record into session B; verify the persisted row's `organization_id` matches session B's org.

`ListToolCalls` (11):

16. `list_tool_calls_happy_path_returns_newest_first`
17. `list_tool_calls_defaults_limit_to_50_when_none`
18. `list_tool_calls_rejects_limit_over_500`
19. `list_tool_calls_treats_limit_zero_as_default_50`
20. `list_tool_calls_rejects_unknown_session` — message starts with `"unknown_session: "`.
21. `list_tool_calls_agent_filter_narrows_result`
22. `list_tool_calls_rejects_control_character_in_session_id`
23. `list_tool_calls_rejects_control_character_in_agent_filter`
24. `list_tool_calls_returns_only_target_session_org_rows` — seed rows in two orgs both tagged to the same session_id (impossible normally; manually insert), verify only target-session-org rows returned.
25. `list_tool_calls_snapshot_consistency_under_concurrent_writes` — start a writer task during read, expect a stable snapshot.
26. `list_tool_calls_does_not_leak_other_sessions_in_same_org` — caller queries session A; rows for session B (same org) MUST NOT appear in result.

### 7.3 L3 — Integration + rollback (3 tests)

27. `record_tool_use_flow_end_to_end` — `crates/daemon/tests/record_tool_use_flow.rs` (NEW): start session → record 3 calls (success, failure, correction-flagged) → ListToolCalls with session filter → ListToolCalls with session+agent filter → verify 3 rows in DESC order with correct fields including `tool_args` round-trip → emit event verified via subscriber.
28. `record_tool_use_writes_target_session_org_id_not_caller_org_id` — Codex/Claude HIGH #8 + claim correction (not "rejects cross-org" — verifies the row's `organization_id` is sourced from the target session). Two sessions in two different orgs; record into session B; verify row's `organization_id == session_B.organization_id`.
29. `test_session_tool_call_rollback_recipe_works` — schema.rs, forward migration → INSERT 5 rows → DROP 3 INDEXes + DROP TABLE → verify all 4 drops succeed and table absent. Validates against a populated DB.

**Total: 37 tests** (up from 31 in v1; Codex/Claude HIGH coverage gaps closed).

### 7.4 Live-daemon dogfood (T12 results doc)

Default daemon port is **8420** (`crates/daemon/src/config.rs:90`, `CLAUDE.md`). 2A-4b results doc used `8430` for a custom-port dogfood; this spec uses the canonical default.

```bash
# Rebuild forge-daemon at HEAD:
cargo build --release --bin forge-daemon

# Shutdown + restart via curl shutdown + SIGTERM + nohup (2A-4a/4b precedent)

DAEMON=http://127.0.0.1:8420/api
SID=$(curl -sS -X POST $DAEMON -d '{
  "method":"start_session","params":{"agent":"claude-code","project":"forge-test"}
}' | jq -r '.data.id')

# Step 1 — Record a tool call
curl -sS -X POST $DAEMON -d "{
  \"method\":\"record_tool_use\",
  \"params\":{
    \"session_id\":\"$SID\",\"agent\":\"claude-code\",\"tool_name\":\"Read\",
    \"tool_args\":{\"file_path\":\"/tmp/a\"},\"tool_result_summary\":\"ok\",
    \"success\":true,\"user_correction_flag\":false
  }}" | jq .

# Step 2 — List for the session
curl -sS -X POST $DAEMON -d "{
  \"method\":\"list_tool_calls\",\"params\":{\"session_id\":\"$SID\"}
}" | jq .

# Step 3 — Validation: unknown session
curl -sS -X POST $DAEMON -d '{
  "method":"list_tool_calls","params":{"session_id":"01NONEXISTENT0000000000000"}
}' | jq .
# Expect: {"status":"error","message":"unknown_session: 01NONEXISTENT0000000000000"}

# Step 4 — Validation: payload too large
LARGE=$(python3 -c 'import json; print(json.dumps({"x":"A"*65537}))')
curl -sS -X POST $DAEMON -d "{
  \"method\":\"record_tool_use\",\"params\":{
    \"session_id\":\"$SID\",\"agent\":\"x\",\"tool_name\":\"x\",
    \"tool_args\":$LARGE,\"tool_result_summary\":\"\",
    \"success\":true
  }}" | jq .
# Expect: {"status":"error","message":"payload_too_large: tool_args: 65536"}

# Step 5 — Validation: control character in session_id
curl -sS -X POST $DAEMON -d '{
  "method":"list_tool_calls","params":{"session_id":"abc\u0000xyz"}
}' | jq .
# Expect: {"status":"error","message":"invalid_field: session_id: control_character"}
```

## 8. Error handling + edge cases

| Case | Returned `Response` |
|------|---------------------|
| `session_id` not in `session` table | `Error { message: "unknown_session: <session_id>" }` |
| Session deleted between client send and daemon execute | `Error { message: "unknown_session: <session_id>" }` (atomic INSERT-SELECT proves no orphan) |
| `tool_args` > 65536 bytes after `serde_json::to_string` (UTF-8 byte count) | `Error { message: "payload_too_large: tool_args: 65536" }` |
| `tool_result_summary` > 65536 UTF-8 bytes (not chars — Unicode > 1 byte/codepoint counts) | `Error { message: "payload_too_large: tool_result_summary: 65536" }` |
| Empty `tool_name` | `Error { message: "empty_field: tool_name" }` |
| Whitespace-only `tool_name` | `Error { message: "empty_field: tool_name" }` |
| Empty `agent` | `Error { message: "empty_field: agent" }` |
| Control char (`\0`, `< 0x20` ex `\t`) in `session_id`/`agent`/`tool_name` | `Error { message: "invalid_field: <field>: control_character" }` |
| Unicode in `tool_name`/`agent` (emoji, accents) | Accepted |
| `tool_args` is valid JSON but a non-object (array, string, number, null) | Accepted — the column is generic JSON, not object-typed |
| Concurrent INSERTs from two sessions / two clients | Both succeed; no uniqueness constraint |
| `ListToolCalls` with `agent` filter unknown to session | Returns empty Vec |
| `ListToolCalls` with `limit = 0` | Treated as default 50 |
| `ListToolCalls` with `limit > 500` | `Error { message: "limit_too_large: requested <n>, max 500" }` |
| Two `ListToolCalls` rows with identical `created_at` (sub-second) | Tie-broken by `id DESC` (ULID monotonic) |
| JSON corruption in stored `tool_args` TEXT | `Error { message: "internal_error: corrupt tool_args row <id>" }` |
| rusqlite error other than `QueryReturnedNoRows` (DB locked, IO, schema corruption) | `Error { message: "internal_error: <sanitized>" }` (claude HIGH #6 fix — distinguish DB faults from missing-row) |
| Legacy session row with `organization_id IS NULL` | Treated as `'default'` org (existing project-wide `COALESCE(..., 'default')` precedent — see §11.4 acknowledgment) |

## 9. Event emission contract

**Event name:** `tool_use_recorded`

**Payload shape:**

```json
{
  "id":         "01KPK...",
  "session_id": "01KPG...",
  "agent":      "claude-code",
  "tool_name":  "Read",
  "success":    true,
  "created_at": "2026-04-19 12:34:56"
}
```

**Excluded:** `tool_args`, `tool_result_summary`, `user_correction_flag`, `organization_id`.

**Rationale for exclusions:**
- `tool_args` + `tool_result_summary`: size (up to 64 KB each × many calls) would drown the 256-slot broadcast channel; subscribers can query via `ListToolCalls` on demand.
- `user_correction_flag`: c1 seeds it bench-only; production signal is deferred; exposing it now would create a contract to honor later.
- `organization_id`: subscribers already know their org; redundant.

**Emission timing:** ONLY after the atomic INSERT-SELECT confirms `rows_affected == 1`. Validation errors and `unknown_session` paths do NOT emit.

**Drop behavior — non-authoritative.** If the broadcast channel is full, `events::emit` silently drops (existing `let _ = tx.send(...)` pattern at `events.rs:24`). Subscribers that fall behind lose events; subsequent events continue to broadcast. **Events are non-authoritative — the `session_tool_call` table is the source of truth.** Any consumer that needs guaranteed completeness MUST reconcile via `ListToolCalls`. This is documented in the results doc (T12).

**Cross-org broadcast caveat.** The broadcast channel is NOT org-scoped — `tool_use_recorded` events are visible to all subscribers regardless of `organization_id`. The event payload includes `session_id`, which a subscriber could correlate back to an org via `ListSession` or DB inspection. This matches the existing `memory_created` precedent and is a known limitation of the fire-and-forget broadcast model.

## 10. Master-deviation: target-session org consistency, not cross-org isolation

### 10.1 What changed from v1

v1 spec used the language "strict cross-org scoping" + "cross-org rejection." v2 reframes: the property we actually provide is **target-session org consistency**, which is weaker than (and frequently confused with) cross-caller isolation.

### 10.2 What we provide

For both `RecordToolUse` and `ListToolCalls`:

- **Write path:** the persisted row's `organization_id` is sourced atomically from the target session's `organization_id` column (via `INSERT ... SELECT FROM session WHERE id = ?session_id`). It is impossible to write a row whose org differs from the target session's org. This protects against type-confusion attacks (writing an org-A-tagged row into an org-B session).
- **Read path:** `ListToolCalls` derives `caller_org` from the target session's `organization_id` column and filters rows by `WHERE organization_id = ?caller_org`. Rows are guaranteed to be from the same org as the target session — but, by construction, no other org's rows could ever exist for this session_id (write path enforces consistency).

### 10.3 What we do NOT provide

- **Cross-caller isolation.** Any caller able to obtain a valid `session_id` (e.g., by guessing, scraping, or being told) can `RecordToolUse` and `ListToolCalls` against that session. There is no authenticated caller context to compare against the target session's owner.
- **Intra-org caller isolation.** Within a single org, a caller for session A can `ListToolCalls` for session B (also in the same org) if they know session B's id. The result set is correctly scoped to session B, but the access is unauthorized in a multi-user-per-org deployment.

### 10.4 Why we narrow `ListToolCalls.session_id` from master `Option<String>` to required `String`

If `session_id` were optional and the handler had to derive `caller_org` from the `agent` filter alone, it would have to pick one org by heuristic (e.g., "most recent session for that agent"). If the agent exists across multiple orgs, the handler picks a session from one — and silently filters the result set to that one org. The caller has no way to know which org was selected, and no way to ask for their own. Without authenticated caller context, agent-only queries are unsafe.

Requiring `session_id` makes `caller_org` derivable from one query, deterministically, with no heuristic.

### 10.5 Future relaxation path

Once authenticated caller-session lands (Phase 2A-6 per master §8), the protocol can relax to:

```rust
ListToolCalls {
    session_id: Option<String>,        // filter
    agent: Option<String>,             // filter
    organization_id: Option<String>,   // filter; defaults to authenticated caller's org
    limit: Option<usize>,
}
```

with `caller_org` derived from the authenticated caller-session, not the target. Documented as a phase-wide follow-up (§11.1).

## 11. Architectural limitations + follow-ups

### 11.1 No cross-caller isolation in c1 — same architectural class as 2A-4a/4b

`RecordToolUse` and `ListToolCalls` derive the caller's effective org from the target `session_id`'s own `organization_id` column. This is **the same architectural class** as `FlipPreference` and `ReaffirmPreference`. The 2A-4b T9 BLOCKER fix did NOT close this gap — it added a same-org consistency check. True cross-caller isolation requires authenticated caller-session at the protocol layer.

**Status:** **Phase-wide follow-up, blocking Phase 2A-6 (Multi-user isolation per master §8).** All four write-path requests (`Flip`, `Reaffirm`, `RecordToolUse`, `ListToolCalls`) need the same migration when caller-session auth ships.

### 11.2 No retroactive import / multi-agent correlation / hook plumbing

Master §5 line 130 and §8 line 210 explicitly defer:
- Retroactive tool-use import from transcripts.
- Multi-agent tool-use correlation.
- Real-observation testing via Claude Code hook plumbing (→ 2A-4c2 dogfood).

### 11.3 No production `user_correction_flag` producer in c1

Master §13 line 304 ("lock one of a/b/c") is resolved to **bench-only seeding**. No Claude Code hook heuristic (premature), no `Request::FlagToolUseCorrection` retrofit variant (new API surface for unclear need), no auto-detection (speculative without real data). Revisit after 2A-4c2 dogfood data arrives.

### 11.4 Legacy NULL `organization_id` sessions treated as `'default'`

Sessions migrated before the `organization_id` column was added (`schema.rs:810-812` ALTER) may have `organization_id IS NULL`. The atomic INSERT-SELECT and the snapshot-read both use `COALESCE(s.organization_id, 'default')`, so NULL is silently treated as `'default'` org. This matches the existing project-wide convention (FlipPreference / ReaffirmPreference both `COALESCE(..., 'default')` — `handler.rs:1049`). It is a **known same-org ambiguity** for legacy data: pre-migration sessions and explicitly-default-org sessions share an org boundary. Backfilling NULL → 'default' on existing sessions is a Phase 2A-6 housekeeping concern.

### 11.5 No fingerprint / canonicalization in c1

Master §5 2A-4c2 owns SHA-256 canonical fingerprinting. c1 stores `tool_args` as serialized JSON via `serde_json::to_string` (which does NOT sort keys). c2 will read `tool_args` TEXT, canonicalize (e.g., via `serde_json::Value` → sorted `BTreeMap` → re-serialize), then hash. c1 does NOT pre-sort, because that would lock the canonicalization scheme in c1 before c2 can design it.

### 11.6 No pagination cursor — read-many strategy is "one query, capped"

500-row LIMIT is the c1 cap. There is no `before_created_at` parameter. If a caller needs more than 500 rows, they must wait for c2/c3 to design pagination, or run their own DB query. For 2A-4d Dim 5 + dogfood, 500 rows per call is well above expected workload.

### 11.7 `user_correction_flag` semantics for c2 — row-level vs session-level

Master §13 line 305 explicitly defers this decision to c2: when Phase 23 reads `session_tool_call`, does `user_correction_flag = 1` exclude (a) only that specific row, or (b) the entire session's tool calls? **C1 stores the flag at the row level and takes no position on c2's filter semantics.** C2 must lock this decision in its design spec.

### 11.8 Wall-clock `ORDER BY created_at` is not strictly monotonic

`created_at` is a TEXT field with format `"YYYY-MM-DD HH:MM:SS"` from `core::time::now_iso()`. Lexicographic ordering on this format is correct only when the system clock is monotonic. NTP corrections, VM migrations, and clock skew can produce out-of-order timestamps within a single second. The `id` column (ULID) IS millisecond-monotonic by design, so the secondary sort `ORDER BY created_at DESC, id DESC` recovers approximate sub-second ordering.

**For c2 Phase 23 fingerprinting,** if strict monotonic ordering is required, c2 should `ORDER BY id DESC` (drop the wall-clock primary key). c1 keeps wall-clock primary because it matches the precedent for `context_effectiveness` and is human-readable in dogfood.

### 11.9 Event broadcast leaks `session_id` to all org subscribers

`tool_use_recorded` events broadcast unfiltered to all subscribers. The payload includes `session_id`, which can be correlated to an org via `ListSession` or DB. Existing `memory_created` precedent has the same property. Org-scoped event filtering is a Phase 2A-6 concern.

### 11.10 No protocol-level rate limiting

A misbehaving caller can flood `RecordToolUse` calls. The 256-slot broadcast channel will drop events, but the underlying INSERTs will succeed and consume disk. Rate limiting is out of scope for c1 + c2 + c3; it is a deployment-layer concern.

## 12. TDD task sequence (12 tasks)

v3 folds v2's thin standalone "default_empty_args helper" task into the protocol bundle task (Claude v2-review NEW LOW #4 — cleaner boundary); total drops from 13 to 12 tasks.

| Task | Scope | Files |
|------|-------|-------|
| **T1** | `ToolCallRow` type + `core::types::tool_call` module + re-export + roundtrip serde test | `crates/core/src/types/tool_call.rs` (new), `crates/core/src/types/mod.rs`, `crates/core/src/lib.rs` |
| **T2** | Schema migration — `session_tool_call` table + 3 indexes (idempotent) | `crates/daemon/src/db/schema.rs` |
| **T3** | `ops::list_tool_calls` + 8 L1 tests | `crates/daemon/src/db/ops.rs` |
| **T4** | Protocol bundle — `Request::RecordToolUse` (with `default_empty_args` helper + `#[serde(default)]`) + `Request::ListToolCalls` + `ResponseData::ToolCallRecorded` + `ResponseData::ToolCallList` + 8 contract tests + handler stub arms returning `Response::Error { message: "unimplemented" }` (Claude HIGH #7 — keep code compilable through to T5) | `crates/core/src/protocol/{request,response,contract_tests}.rs`, `crates/daemon/src/server/handler.rs` (stub arms) |
| **T5** | `handle_record_tool_use` — atomic INSERT-SELECT + happy path + persistence test (1 test) | `crates/daemon/src/server/handler.rs` |
| **T6** | `handle_record_tool_use` validation — 8 error paths (unknown session, deleted-mid-call, args/result too large, empty/whitespace name+agent, control chars in session_id/agent/tool_name, unicode-accepted affirmative) | `crates/daemon/src/server/handler.rs` |
| **T7** | `handle_record_tool_use` event emission — `tool_use_recorded` post-INSERT-success only (3 tests: emitted-on-success, NOT-emitted-on-validation-error, NOT-emitted-on-unknown-session) | `crates/daemon/src/server/handler.rs` |
| **T8** | `handle_list_tool_calls` happy path + transaction-wrapped read + filter + limit + ordering + tiebreaker (5 tests) | `crates/daemon/src/server/handler.rs` |
| **T9** | `handle_list_tool_calls` validation + target-session-org behavior (6 tests: limit_too_large, unknown_session, control_char in session_id, control_char in agent_filter, target-session-org-only-rows, no-leak-other-sessions-same-org) | `crates/daemon/src/server/handler.rs` |
| **T10** | Integration test `record_tool_use_flow.rs` (NEW) + `record_tool_use_writes_target_session_org_id_not_caller_org_id` | `crates/daemon/tests/record_tool_use_flow.rs` |
| **T11** | Schema rollback recipe test against populated DB | `crates/daemon/src/db/schema.rs` |
| **T12** | Live-daemon dogfood (port 8420) + results doc | `docs/benchmarks/results/forge-tool-use-recording-2026-04-19.md` |

Per-task discipline (inherited from 2A-4a/4b):

- TDD RED → GREEN → REFACTOR.
- Each task: `cargo fmt` + `cargo clippy --workspace -- -W clippy::all -D warnings` + `cargo test --workspace` clean.
- T4 specifically includes the handler-stub arms so the workspace stays compilable between T4 and T9 — no `todo!()` or feature-gate hacks needed (Claude HIGH #7 fix).
- Commit after each GREEN. Review-fix commits tagged to the same task.
- Per-task subagent dispatch: implementer + parallel spec-reviewer + code-quality-reviewer + Codex CLI reviewer.

## 13. Non-goals (explicit)

1. **Phase 23 consolidator** — skill inference logic lives in 2A-4c2.
2. **Canonical SHA-256 fingerprint** — c2 owns; c1 stores raw `serde_json::to_string()` output.
3. **`<skills>` renderer update** — c2 owns `compile_dynamic_suffix` changes.
4. **Claude Code hook ingestion** — c2 dogfood phase.
5. **Production `user_correction_flag` producer** — bench-seeded only in c1.
6. **`user_correction_flag` filter semantics (row-level vs session-level)** — c2 owns the filter design.
7. **Retroactive tool-use import** — master §5 line 130 defers.
8. **Multi-agent tool-use correlation** — master §5 line 130 defers.
9. **Pagination cursors** — 500-row cap suffices for c1 + dogfood.
10. **Tool-call deduplication** — master §5 line 130 defers; c1 stores duplicates as separate rows.
11. **Bench harness seeding of `session_tool_call`** — 2A-4d concern.
12. **Regression-guard bench reruns** — no scoring-surface touched in c1.
13. **Cross-caller isolation** — Phase 2A-6 (master §8) concern.
14. **Authenticated caller-session API** — Phase 2A-6 (master §8) concern.
15. **Event-stream rate limiting / backpressure** — deployment-layer concern.

## 14. Success criteria

1. All 37 tests in §7 pass (~1294 + 37 = ~1331 lib + workspace tests green).
2. `cargo clippy --workspace -- -W clippy::all -D warnings` — 0 warnings at every commit boundary (T1 through T12).
3. `cargo fmt --all` — clean.
4. Live-daemon dogfood (T12) — all 5 curl steps behave as specified in §7.4 against port 8420.
5. Rollback recipe (T11) executes cleanly on a populated test database.
6. Target-session org test (L2 #15 + L3 #28) — written row's `organization_id` matches target session's org.
7. No-orphan-write test (L2 #6) — session deleted between client send and daemon execute → `unknown_session` error AND no row inserted (atomic INSERT-SELECT proof).
8. No-cross-session-leak test (L2 #26) — listing session A in org X does not return rows for session B in org X.
9. Event emission tests (L2 #3 + #4) — `tool_use_recorded` fires post-INSERT-success only with exactly the fields in §9; payload excludes args/result/correction.
10. Snapshot consistency test (L2 #25 + L1 #6) — concurrent writes during a `ListToolCalls` read produce a stable snapshot inside the transaction.
11. Master §6 Schema assertions 5 and 7 verifiable post-merge:
    - Assertion 5: `Request::RecordToolUse`, `Request::ListToolCalls` variants exist (post 2A-4c1) ✓
    - Assertion 7: `session_tool_call` table exists with specified columns and per-session/per-agent indexes (non-unique — tool calls can repeat) (post 2A-4c1) ✓
12. Spec doc committed before implementation starts (2A-4a/4b precedent — v3 commit replaces v2 commit `1ad3deb` which replaced v1 commit `ceea810`).

**Known untested path** (accepted risk, Claude v2-review NEW LOW #2): the `internal_error: <sanitized>` handler branch for non-`QueryReturnedNoRows` rusqlite errors (DB locked, IO fault, schema corruption) is not directly exercised in the test suite — these faults are hard to inject without mocking. The path is tested indirectly at dogfood level; if regression tooling (e.g., `faultinject`) lands in a later phase, add a targeted test then.

## 15. Key code locations (from exploration)

| Location | Purpose |
|----------|---------|
| `crates/daemon/src/db/schema.rs:384–911` | Session schema + migration patterns |
| `crates/daemon/src/db/schema.rs:810-812` | Existing `organization_id` ALTER (DEFAULT 'default'); legacy NULL rows possible |
| `crates/daemon/src/db/schema.rs:1167–1180` | `context_effectiveness` — precedent for append-only session child table |
| `crates/daemon/src/db/ops.rs` | Ops layer — add `list_tool_calls` (read-only; write inlined in handler for atomicity) |
| `crates/daemon/src/server/handler.rs:193–204` | Existing `get_session_org_id` — kept; new strict transaction-based pattern in §5.2 |
| `crates/daemon/src/server/handler.rs:1028-1107` | ReaffirmPreference handler — atomic UPDATE-RETURNING precedent for §5.1 atomic INSERT-SELECT |
| `crates/daemon/src/events.rs:1–30` | `events::emit` — broadcast primitive |
| `crates/core/src/types/memory.rs` | Precedent for shared type in `core::types::*` |
| `crates/core/src/protocol/request.rs:40–250+` | Request enum — add 2 new variants |
| `crates/core/src/protocol/response.rs:1196-1199` | `Response::Ok { data: ResponseData }` + `Response::Error { message: String }` — actual error wire format |
| `crates/core/src/protocol/response.rs:83+` | `ResponseData` enum — add 2 variants alongside `PreferenceReaffirmed` |
| `crates/core/src/protocol/contract_tests.rs:74–600+` | Parameterized contract test precedent |
| `crates/core/src/time.rs::now_iso()` | Wall-clock timestamp in "YYYY-MM-DD HH:MM:SS" format |
| `crates/daemon/src/config.rs:90` | Default daemon port = 8420 |

## 16. Changelog

| Version | Date | Change |
|---------|------|--------|
| v1 | 2026-04-19 | Initial design, pre-adversarial-review. Commit `ceea810`. |
| v2 | 2026-04-19 | Adversarial-review pass (Claude + Codex). Major revisions: (a) reframed "cross-org scoping" as honest "target-session org consistency" (Codex BLOCKER #1, Claude BLOCKER #2); (b) atomic INSERT-SELECT eliminates TOCTOU race (Codex BLOCKER #2); (c) dropped typed `HandlerError` enum + `errors.rs` references (don't exist) — now `Response::Error { message: String }` with documented `<error_code>:` prefix convention (Claude+Codex BLOCKER #1); (d) Response shape uses `ResponseData::ToolCallRecorded`/`ToolCallList` (Codex HIGH #5); (e) port 8420 (was 8430; Claude HIGH #4); (f) added `#[serde(default = "default_empty_args")]` for tool_args (Claude HIGH #5); (g) `get_session_org_id_strict` distinguishes `QueryReturnedNoRows` from other rusqlite errors (Claude HIGH #6); (h) T5 includes handler stubs to keep code compilable (Claude HIGH #7); (i) added 3rd query-serving index (Codex MEDIUM #10); (j) added concurrency tests + control-character tests + atomic-insert-rejects-deleted-session test (Codex HIGH #7, MEDIUM #8); (k) added `ORDER BY created_at DESC, id DESC` tiebreaker for sub-second monotonicity (Codex MEDIUM #9); (l) removed broken pagination claim (Codex MEDIUM #9); (m) §11 expanded with limitations §11.4 NULL org legacy, §11.7 row-vs-session correction semantics (master §13 item 3 unresolved → c2), §11.8 wall-clock not monotonic, §11.9 event broadcast unfiltered, §11.10 no rate limit; (n) test count 31 → 37; (o) success criteria reworded for accuracy (no more "rejects cross-org" claim — now "writes target-session org id" + "no-cross-session-leak"). Commit `1ad3deb`. |
| v3 | 2026-04-20 | Second-pass adversarial-review fixes from Claude (Codex second-pass was killed after hanging 11+ hrs; Claude verified ALL 23 v1 findings as PASS and flagged only minor follow-ups). Changes: (a) clarified `.len()` as **UTF-8 byte count** (not char count) at §5.1 Step 1d-1e + §8 table + §4.5 error-prefix docs — Claude NEW MEDIUM #1; (b) folded thin v2 T3 ("default_empty_args helper standalone") into T4 protocol bundle for cleaner task boundary — Claude NEW LOW #4; task count 13 → 12 with renumbering throughout §12 + §14; (c) added §14 "Known untested path" note acknowledging that the `internal_error:` rusqlite-fault branch is tested only at dogfood level (accepted gap — Claude NEW LOW #2). No scope or architectural changes. |
