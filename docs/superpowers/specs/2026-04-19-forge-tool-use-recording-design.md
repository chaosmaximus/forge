# Forge-Tool-Use-Recording (Phase 2A-4c1) — Design Specification

**Phase:** 2A-4c1 of Phase 2A-4 Forge-Identity master decomposition
**Date:** 2026-04-19
**Parent master:** `docs/benchmarks/forge-identity-master-design.md` v6a §5 2A-4c1, §13 resolution index
**Prior phase:** 2A-4b Recency-weighted Preference Decay shipped on 2026-04-19 (HEAD `21aa115`).
**Follow-on phase:** 2A-4c2 Phase 23 Behavioral Skill Inference (consolidator + `<skills>` renderer + canonical fingerprint).
**Spec version:** v1 (pre-adversarial-review)

## 1. Goal

Ship the substrate needed for Phase 2A-4c2's behavioral skill inference and Phase 2A-4d's Dimension 5 bench:

1. `session_tool_call` table — append-only, session-scoped, org-scoped, non-unique on `(agent, tool_name)`.
2. `Request::RecordToolUse` — write a tool-call row, atomically, with strict validation.
3. `Request::ListToolCalls` — read tool-call rows, session-scoped, org-scoped, newest-first.
4. `tool_use_recorded` event emission per successful write (fire-and-forget, minimal payload).

Explicitly out of scope in this sub-phase: skill inference logic (2A-4c2), Claude Code hook plumbing (2A-4c2 dogfood), `<skills>` renderer changes (2A-4c2), canonical SHA-256 fingerprinting (2A-4c2), production `user_correction_flag` producer (post-c2; bench-seeded in c1), pagination cursors (YAGNI; 500-row cap suffices).

## 2. Architecture

**Append-only child table.** `session_tool_call` is a new session-scoped child relation, following the `context_effectiveness` precedent (`schema.rs:1167–1180`). No foreign-key enforcement (SQLite FKs are off project-wide), but application-level validation requires `session_id` exists in the `session` table before insert.

**Strict cross-org scoping.** Mirrors `FlipPreference` / `ReaffirmPreference` (`handler.rs:871, 1041`). `organization_id` is derived from `session_id` via `get_session_org_id(...)` at write time and at read time. No row bypasses org scope. Deviation from master §5 line 126: `Request::ListToolCalls` narrows `session_id` from `Option<String>` to required `String` — documented in §10.

**Thin ops layer.** Handler owns serialization + size validation; ops layer owns SQL only. Same layering as `ops::remember_raw` (2A-4a T0) and `ops::supersede_memory_impl` (2A-4a T1).

**Event emission.** `handle_record_tool_use` emits `tool_use_recorded` post-INSERT via `events::emit` (fire-and-forget broadcast). Payload excludes `tool_args` + `tool_result_summary` (size / PII). Matches precedent from `memory_created`, `preference_flipped`, `preference_reaffirmed`.

**No scoring-surface touched.** Recall, decay, and hybrid_recall are untouched. No regression-guard bench reruns needed.

## 3. Schema

### 3.1 New table

```sql
CREATE TABLE IF NOT EXISTS session_tool_call (
    id                    TEXT PRIMARY KEY,           -- ULID
    session_id            TEXT NOT NULL,
    agent                 TEXT NOT NULL,
    tool_name             TEXT NOT NULL,
    tool_args             TEXT NOT NULL,              -- canonical JSON, serde_json::to_string()
    tool_result_summary   TEXT NOT NULL,              -- free-form string
    success               INTEGER NOT NULL,           -- 0 or 1
    user_correction_flag  INTEGER NOT NULL DEFAULT 0, -- 0 or 1
    organization_id       TEXT NOT NULL DEFAULT 'default',
    created_at            TEXT NOT NULL               -- wall-clock "YYYY-MM-DD HH:MM:SS"
);
```

### 3.2 Indexes (both non-unique per master §5 line 124)

```sql
CREATE INDEX IF NOT EXISTS idx_session_tool_session
    ON session_tool_call (session_id, created_at);

CREATE INDEX IF NOT EXISTS idx_session_tool_name_agent
    ON session_tool_call (agent, tool_name);
```

### 3.3 Deviations from master v6a §5

| Deviation | Rationale |
|-----------|-----------|
| Added `organization_id TEXT NOT NULL DEFAULT 'default'` column | Master did not specify cross-org scoping. We mirror `FlipPreference` / `ReaffirmPreference` for consistency (see §5 cross-org handling). |
| `tool_args` and `tool_result_summary` declared `NOT NULL` | Prevents the "missing vs empty" ambiguity; callers send `{}` or `""` explicitly. |
| Column order matches `context_effectiveness` precedent | Visual consistency with other append-only session child tables. |

### 3.4 Migration pattern

Idempotent `CREATE TABLE IF NOT EXISTS` + `CREATE INDEX IF NOT EXISTS`. No data backfill (new table). Matches `schema.rs:384–911` forward-only pattern.

### 3.5 Rollback recipe

```sql
DROP INDEX IF EXISTS idx_session_tool_name_agent;
DROP INDEX IF EXISTS idx_session_tool_session;
DROP TABLE IF EXISTS session_tool_call;
```

Validated by `test_session_tool_call_rollback_recipe_works` (§7 T12).

## 4. Protocol

### 4.1 `Request::RecordToolUse`

File: `crates/core/src/protocol/request.rs`. NOT feature-gated.

```rust
RecordToolUse {
    session_id: String,
    agent: String,
    tool_name: String,
    tool_args: serde_json::Value,
    tool_result_summary: String,
    success: bool,
    #[serde(default)]
    user_correction_flag: bool,
},
```

### 4.2 `Request::ListToolCalls`

```rust
ListToolCalls {
    session_id: String,                  // REQUIRED — narrower than master for cross-org safety (§10)
    #[serde(default)]
    agent: Option<String>,               // AND-filter within session
    #[serde(default)]
    limit: Option<usize>,                // None → 50; > 500 rejected
},
```

### 4.3 Responses

File: `crates/core/src/protocol/response.rs`.

```rust
ToolCallRecorded {
    id: String,          // new ULID
    created_at: String,  // wall-clock
},

ToolCallList {
    calls: Vec<ToolCallRow>,
},
```

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

`organization_id` is intentionally NOT included in `ToolCallRow` — the caller's org is already known (rows are filter-scoped to caller_org at query time).

### 4.5 New error variants

File: `crates/core/src/protocol/errors.rs` (extend existing enum).

| Variant | Meaning | Triggered by |
|---------|---------|--------------|
| `UnknownSession { session_id }` | Session row does not exist | `RecordToolUse`, `ListToolCalls` |
| `PayloadTooLarge { field, max_bytes }` | `field` value exceeds 64 KB serialized | `RecordToolUse` (`tool_args`, `tool_result_summary`) |
| `LimitTooLarge { requested, max }` | Requested limit > 500 | `ListToolCalls` |
| `EmptyField { field }` | Required string is empty | `RecordToolUse` (`tool_name`, `agent`) |

`CrossOrgDenied` — reuse existing variant from FlipPreference / ReaffirmPreference if present; add if absent (same shape: `{ session_id }`).

### 4.6 Contract tests

File: `crates/core/src/protocol/contract_tests.rs`. Parameterized vectors for:

1. `RecordToolUse` round-trip — all fields present.
2. `RecordToolUse` defaults — omit `user_correction_flag`, verify deserializes as `false`.
3. `ListToolCalls` round-trip — only required `session_id`; omit `agent` and `limit`.
4. `ListToolCalls` full — all optional fields populated.
5. `ToolCallRecorded` response round-trip.
6. `ToolCallList` response round-trip — empty Vec and 3-element Vec.
7. `ToolCallRow` round-trip — verify `tool_args` round-trips as `serde_json::Value`.
8. Error variants — `UnknownSession`, `PayloadTooLarge`, `LimitTooLarge`, `EmptyField` round-trip.

## 5. Handler behavior

### 5.1 `handle_record_tool_use`

```
1. Validate inputs (pre-DB):
   a. Serialize tool_args:       let canonical = serde_json::to_string(&tool_args)?;
      If canonical.len() > 65536    → err PayloadTooLarge { field: "tool_args", max_bytes: 65536 }
   b. If tool_result_summary.len() > 65536
                                  → err PayloadTooLarge { field: "tool_result_summary", max_bytes: 65536 }
   c. If tool_name.is_empty()    → err EmptyField { field: "tool_name" }
   d. If agent.is_empty()        → err EmptyField { field: "agent" }

2. Validate session exists (strict):
   let org_id = get_session_org_id_strict(conn, &session_id)?;
     — Returns Err(UnknownSession { session_id }) if not found
     — Returns Ok(String) on hit (never "default" fallback)

3. Generate id + timestamp:
   let id = ulid::Ulid::new().to_string();
   let created_at = core::time::now_iso();        // "YYYY-MM-DD HH:MM:SS"

4. Persist atomically (single INSERT):
   ops::record_tool_use(
       &conn, &id, &session_id, &agent, &tool_name,
       &canonical, &tool_result_summary,
       success, user_correction_flag,
       &org_id, &created_at,
   )?;

5. Emit event (fire-and-forget):
   events::emit(&state.events, "tool_use_recorded", json!({
       "id":          id,
       "session_id":  session_id,
       "agent":       agent,
       "tool_name":   tool_name,
       "success":     success,
       "created_at":  created_at,
   }));
   // Excludes tool_args, tool_result_summary, user_correction_flag from payload
   // (size + PII + correction-signal scoping is internal to 2A-4c2).

6. Return Response::ToolCallRecorded { id, created_at };
```

### 5.2 `handle_list_tool_calls`

```
1. Validate + normalize limit:
   let limit = match params.limit {
       None       => 50,
       Some(0)    => 50,                                      // treat 0 as default
       Some(n) if n > 500 => err LimitTooLarge { requested: n, max: 500 },
       Some(n)    => n,
   };

2. Validate session exists + derive caller_org:
   let caller_org = get_session_org_id_strict(conn, &params.session_id)?;

3. Query (scoped):
   let rows = ops::list_tool_calls(
       &conn, &caller_org, &params.session_id,
       params.agent.as_deref(), limit,
   )?;

4. Return Response::ToolCallList { calls: rows };
```

### 5.3 Helper: `get_session_org_id_strict`

Extend the existing `get_session_org_id` (handler.rs:193–204) with a strict variant:

```rust
fn get_session_org_id_strict(conn: &Connection, session_id: &str)
    -> Result<String, HandlerError>
{
    conn.query_row(
        "SELECT COALESCE(organization_id, 'default') FROM session WHERE id = ?1",
        params![session_id], |row| row.get(0),
    ).map_err(|_| HandlerError::UnknownSession { session_id: session_id.to_string() })
}
```

The existing permissive `get_session_org_id` stays untouched (used by `Remember` + other legacy write paths). The strict version is introduced for `RecordToolUse` + `ListToolCalls` and is available for future tightening.

## 6. Ops layer

### 6.1 `ops::record_tool_use`

```rust
pub fn record_tool_use(
    conn: &Connection,
    id: &str,
    session_id: &str,
    agent: &str,
    tool_name: &str,
    tool_args_canonical: &str,    // pre-serialized + pre-size-validated
    tool_result_summary: &str,
    success: bool,
    user_correction_flag: bool,
    organization_id: &str,
    created_at: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO session_tool_call
            (id, session_id, agent, tool_name, tool_args, tool_result_summary,
             success, user_correction_flag, organization_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id, session_id, agent, tool_name, tool_args_canonical,
            tool_result_summary, success as i64, user_correction_flag as i64,
            organization_id, created_at,
        ],
    )?;
    Ok(())
}
```

### 6.2 `ops::list_tool_calls`

```rust
pub fn list_tool_calls(
    conn: &Connection,
    organization_id: &str,
    session_id: &str,
    agent_filter: Option<&str>,
    limit: usize,
) -> rusqlite::Result<Vec<ToolCallRow>> {
    let (sql, params): (String, Vec<Box<dyn ToSql>>) = match agent_filter {
        Some(agent) => (
            "SELECT id, session_id, agent, tool_name, tool_args, tool_result_summary,
                    success, user_correction_flag, created_at
             FROM session_tool_call
             WHERE organization_id = ?1 AND session_id = ?2 AND agent = ?3
             ORDER BY created_at DESC
             LIMIT ?4".to_string(),
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
             ORDER BY created_at DESC
             LIMIT ?3".to_string(),
            vec![
                Box::new(organization_id.to_string()),
                Box::new(session_id.to_string()),
                Box::new(limit as i64),
            ],
        ),
    };

    let mut stmt = conn.prepare(&sql)?;
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

A JSON-parse failure on a stored row (corruption) propagates as `rusqlite::Error::FromSqlConversionFailure` — should never happen in practice because `record_tool_use` only accepts valid pre-serialized JSON.

## 7. Testing strategy

### 7.1 L1 — Ops unit tests (`crates/daemon/src/db/ops.rs`, 8 tests)

1. `record_tool_use_inserts_row`
2. `record_tool_use_allows_duplicate_tool_name_agent_across_sessions`
3. `record_tool_use_persists_user_correction_flag_true_and_false`
4. `list_tool_calls_orders_newest_first`
5. `list_tool_calls_respects_limit`
6. `list_tool_calls_filters_by_agent`
7. `list_tool_calls_returns_empty_when_org_mismatch`
8. `list_tool_calls_returns_empty_when_session_mismatch`

### 7.2 L2 — Request-path handler tests (`crates/daemon/src/server/handler.rs` cfg(test) module, 20 tests)

`RecordToolUse` (12):

1. `record_tool_use_happy_path_returns_id_and_created_at`
2. `record_tool_use_persists_all_fields_roundtrip_via_list`
3. `record_tool_use_emits_tool_use_recorded_event_post_commit`
4. `record_tool_use_event_payload_excludes_args_and_result`
5. `record_tool_use_rejects_unknown_session`
6. `record_tool_use_rejects_tool_args_over_64kb`
7. `record_tool_use_rejects_tool_result_summary_over_64kb`
8. `record_tool_use_rejects_empty_tool_name`
9. `record_tool_use_rejects_empty_agent`
10. `record_tool_use_accepts_user_correction_flag_true`
11. `record_tool_use_defaults_user_correction_flag_to_false`
12. `record_tool_use_derives_organization_id_from_session`

`ListToolCalls` (8):

13. `list_tool_calls_happy_path_returns_newest_first`
14. `list_tool_calls_defaults_limit_to_50_when_none`
15. `list_tool_calls_rejects_limit_over_500`
16. `list_tool_calls_treats_limit_zero_as_default_50`
17. `list_tool_calls_rejects_unknown_session`
18. `list_tool_calls_agent_filter_narrows_result`
19. `list_tool_calls_returns_only_caller_org_rows`
20. `list_tool_calls_rejects_cross_org_session`

### 7.3 L3 — Integration + rollback (3 tests)

21. `record_tool_use_flow_end_to_end` — `crates/daemon/tests/record_tool_use_flow.rs` (new): start session → record 3 calls (success, failure, correction-flagged) → ListToolCalls with session filter → ListToolCalls with agent filter → cross-org attempt rejected.
22. `record_tool_use_rejects_cross_org_session` — caller session A, target session B in another org.
23. `test_session_tool_call_rollback_recipe_works` — schema.rs, forward migration + INSERT + DROP INDEX × 2 + DROP TABLE verifies idempotent rollback.

**Total: 31 tests** (within 25-35 target).

### 7.4 Live-daemon dogfood (T13 results doc)

```bash
# Rebuild forge-daemon at HEAD:
cargo build --release --bin forge-daemon

# Shutdown + restart via curl shutdown + SIGTERM + nohup (2A-4a/4b precedent)

# Step 1 — Record a tool call
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"record_tool_use",
  "params":{
    "session_id":"<sid>","agent":"claude-code","tool_name":"Read",
    "tool_args":{"file_path":"/tmp/a"},"tool_result_summary":"ok",
    "success":true,"user_correction_flag":false
  }}' | jq .

# Step 2 — List for the session
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"list_tool_calls","params":{"session_id":"<sid>"}
}' | jq .

# Step 3 — Validation errors
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"list_tool_calls","params":{"session_id":"nonexistent"}
}' | jq .
# Expect UnknownSession error

curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"record_tool_use","params":{
    "session_id":"<sid>","agent":"x","tool_name":"x",
    "tool_args":<64k+ JSON>,"tool_result_summary":"",
    "success":true
  }}' | jq .
# Expect PayloadTooLarge error
```

## 8. Error handling + edge cases

| Case | Behavior |
|------|----------|
| `session_id` not in `session` table | `Err(UnknownSession { session_id })` — no silent "default" org fallback |
| `tool_args` > 64 KB serialized | `Err(PayloadTooLarge { field: "tool_args", max_bytes: 65536 })` |
| `tool_result_summary` > 64 KB | `Err(PayloadTooLarge { field: "tool_result_summary", max_bytes: 65536 })` |
| Empty `tool_name` or `agent` | `Err(EmptyField { field })` |
| `tool_args` is valid JSON but weird shape (e.g., array) | Accept — daemon does not enforce object-ness |
| Concurrent INSERTs from two sessions | Both succeed; no uniqueness constraint |
| `ListToolCalls` with `agent` filter unknown to session | Returns empty Vec (not an error) |
| `ListToolCalls` with `limit = 0` | Treated as default 50 |
| `ListToolCalls` with `limit > 500` | `Err(LimitTooLarge { requested, max: 500 })` |
| `ListToolCalls` with `session_id` from a different org | Row filter on `organization_id = caller_org` returns empty (strict mode — after unknown/cross-org session detection) |
| JSON corruption in stored `tool_args` TEXT | `rusqlite::Error::FromSqlConversionFailure` propagates |

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

**Emission timing:** POST-INSERT. The `INSERT` is a single atomic statement — SQLite auto-commits. No transaction wrapper needed, unlike FlipPreference's multi-statement `conn.transaction()`.

**Drop behavior:** if the broadcast channel is full, `events::emit` silently drops (existing `let _ = tx.send(...)` pattern). Subscribers that fall behind lose events; subsequent events continue to broadcast. This is the existing project-wide contract.

## 10. Cross-org scoping deviation from master v6a

**Master §5 2A-4c1 line 126:**
> `Request::ListToolCalls { session_id: Option<String>, agent: Option<String>, limit: Option<usize> }` variant for observability

**This spec:** `session_id: String` (required).

**Rationale:**

- Without an authenticated caller-session parameter (phase-wide follow-up flagged at 2A-4b T9 BLOCKER fix), deriving `caller_org` from a free-floating `agent` filter is unsafe — the same agent can exist across multiple orgs, and the handler would have to pick one by heuristic (e.g., "most recent session").
- 2A-4b T9 already narrowed `FlipPreference` / `ReaffirmPreference` beyond master spec by adding strict cross-org guards; precedent is to tighten for safety and document.
- Once authenticated caller-session lands (phase-wide follow-up), `session_id` can be relaxed back to `Option<String>` safely — the handler will derive `caller_org` from the authenticated session, not the parameter.

**Future relaxation path:**

```rust
// Once caller_session is a trustworthy parameter:
ListToolCalls {
    session_id: Option<String>,   // filter
    agent: Option<String>,        // filter
    organization_id: Option<String>, // filter (defaults to caller's org)
    limit: Option<usize>,
}
```

Documented as a **known limitation** in the results doc (T13).

## 11. Architectural limitations + follow-ups

### 11.1 Self-authorizing cross-org (inherited from 2A-4a/4b)

`RecordToolUse` and `ListToolCalls` both derive `caller_org` from the target `session_id`'s own `organization_id` column. This is the same pattern as `FlipPreference` and `ReaffirmPreference`. It assumes the caller has legitimate access to whatever session they pass. Real cross-caller isolation requires an authenticated caller context at the protocol layer.

**Status:** phase-wide follow-up. Flagged at 2A-4b (`project_phase_2a4b_complete_2026_04_19.md`), unchanged by c1.

### 11.2 No retroactive import

Master §5 line 130 and §8 line 210 explicitly defer:
- Retroactive tool-use import from transcripts.
- Multi-agent tool-use correlation.
- Real-observation testing via Claude Code hook plumbing (→ 2A-4c2 dogfood).

### 11.3 No production `user_correction_flag` producer in c1

Master §13 line 304 ("lock one of a/b/c") is resolved to **option (a-minus): bench-only seeding**. No Claude Code hook heuristic (premature), no `Request::FlagToolUseCorrection` retrofit variant (new API surface for unclear need), no auto-detection (speculative without real data). Revisit after 2A-4c2 dogfood data arrives.

### 11.4 No fingerprint / canonicalization in c1

Master §5 2A-4c2 owns SHA-256 canonical fingerprinting. c1 stores `tool_args` as serialized JSON via `serde_json::to_string` (which does NOT sort keys). c2 will read `tool_args` TEXT, canonicalize (e.g., via `serde_json::Value` → sorted `BTreeMap` → re-serialize), then hash. c1 does NOT pre-sort, because that would lock the canonicalization scheme in c1 before c2 can design it.

### 11.5 No pagination cursor

500-row limit plus caller-iterated `created_at` filter on the next call provides sufficient pagination for c1 bench + dogfood. Cursor-based pagination is a 2A-4d concern (scanning many sessions) or later.

## 12. TDD task sequence (13 tasks)

| Task | Scope | Files |
|------|-------|-------|
| **T1** | `ToolCallRow` type + `core::types::tool_call` module + re-export | `crates/core/src/types/tool_call.rs` (new), `crates/core/src/types/mod.rs`, `crates/core/src/lib.rs` |
| **T2** | Schema migration — `session_tool_call` table + 2 indexes | `crates/daemon/src/db/schema.rs` |
| **T3** | `ops::record_tool_use` + 3 L1 tests | `crates/daemon/src/db/ops.rs` |
| **T4** | `ops::list_tool_calls` + 5 L1 tests | `crates/daemon/src/db/ops.rs` |
| **T5** | Protocol bundle — `Request::RecordToolUse` + `Request::ListToolCalls` + `Response::ToolCallRecorded` + `Response::ToolCallList` + 4 error variants + 8 contract tests | `crates/core/src/protocol/{request,response,errors,contract_tests}.rs` |
| **T6** | `handle_record_tool_use` happy path + `get_session_org_id_strict` helper | `crates/daemon/src/server/handler.rs` |
| **T7** | `handle_record_tool_use` validation (6 error paths) | `crates/daemon/src/server/handler.rs` |
| **T8** | `handle_record_tool_use` event emission (`tool_use_recorded` + 2 tests) | `crates/daemon/src/server/handler.rs`, `crates/daemon/src/events.rs` |
| **T9** | `handle_list_tool_calls` happy path + filter + limit + ordering (4 tests) | `crates/daemon/src/server/handler.rs` |
| **T10** | `handle_list_tool_calls` validation + cross-org (4 tests) | `crates/daemon/src/server/handler.rs` |
| **T11** | Integration test `record_tool_use_flow.rs` (NEW) | `crates/daemon/tests/record_tool_use_flow.rs` |
| **T12** | Schema rollback recipe test | `crates/daemon/src/db/schema.rs` |
| **T13** | Live-daemon dogfood + results doc | `docs/benchmarks/results/forge-tool-use-recording-2026-04-19.md` |

Per-task discipline (inherited from 2A-4a/4b):

- TDD RED → GREEN → REFACTOR.
- Each task: `cargo fmt` + `cargo clippy --workspace -- -W clippy::all -D warnings` + `cargo test --workspace` clean.
- Commit after each GREEN. Review-fix commits tagged to the same task.
- Per-task subagent dispatch: implementer + parallel spec-reviewer + code-quality-reviewer + Codex CLI reviewer.

## 13. Non-goals (explicit)

1. **Phase 23 consolidator** — skill inference logic lives in 2A-4c2.
2. **Canonical SHA-256 fingerprint** — c2 owns; c1 stores raw `serde_json::to_string()` output.
3. **`<skills>` renderer update** — c2 owns `compile_dynamic_suffix` changes.
4. **Claude Code hook ingestion** — c2 dogfood phase.
5. **Production `user_correction_flag` producer** — bench-seeded only in c1.
6. **Retroactive tool-use import** — master §5 line 130 defers.
7. **Multi-agent tool-use correlation** — master §5 line 130 defers.
8. **Pagination cursors** — 500-row cap suffices for c1 + dogfood.
9. **Tool-call deduplication** — master §5 line 130 defers; c1 stores duplicates as separate rows.
10. **Bench harness seeding of `session_tool_call`** — 2A-4d concern.
11. **Regression-guard bench reruns** — no scoring-surface touched in c1.

## 14. Success criteria

1. All 31 tests in §7 pass (~1294 + 31 = 1325 lib + workspace tests green).
2. `cargo clippy --workspace -- -W clippy::all -D warnings` — 0 warnings.
3. `cargo fmt --all` — clean.
4. Live-daemon dogfood (T13) — all 4 curl steps behave as specified in §7.4.
5. Rollback recipe (T12) executes cleanly on a populated test database.
6. Cross-org attack test (L2 test #20 + L3 test #22) — rejects cross-org session access.
7. Event emission test (L2 test #3 + #4) — `tool_use_recorded` fires post-INSERT with exactly the fields in §9.
8. Master §6 Schema assertions 5 and 7 verifiable post-merge:
   - Assertion 5: `Request::RecordToolUse`, `Request::ListToolCalls` variants exist (post 2A-4c1) ✓
   - Assertion 7: `session_tool_call` table exists with specified columns and per-session/per-agent indexes (non-unique — tool calls can repeat) (post 2A-4c1) ✓
9. Spec doc committed before implementation starts (2A-4a/4b precedent).

## 15. Key code locations (from exploration)

| Location | Purpose |
|----------|---------|
| `crates/daemon/src/db/schema.rs:384–911` | Session schema + migration patterns |
| `crates/daemon/src/db/schema.rs:1167–1180` | `context_effectiveness` — precedent for append-only session child table |
| `crates/daemon/src/db/ops.rs` | Ops layer — add `record_tool_use` + `list_tool_calls` |
| `crates/daemon/src/server/handler.rs:193–204` | `get_session_org_id` — extend with strict variant |
| `crates/daemon/src/server/handler.rs:871, 1041` | FlipPreference / ReaffirmPreference cross-org precedent |
| `crates/daemon/src/events.rs:1–30` | `events::emit` — broadcast primitive |
| `crates/core/src/types/memory.rs` | Precedent for shared type in `core::types::*` |
| `crates/core/src/protocol/request.rs:40–250+` | Request enum — add 2 new variants |
| `crates/core/src/protocol/contract_tests.rs:74–600+` | Parameterized contract test precedent |
| `crates/core/src/time.rs::now_iso()` | Wall-clock timestamp in "YYYY-MM-DD HH:MM:SS" format |

## 16. Changelog

| Version | Date | Change |
|---------|------|--------|
| v1 | 2026-04-19 | Initial design, pre-adversarial-review. Ready for Claude + Codex dual review pass. |
