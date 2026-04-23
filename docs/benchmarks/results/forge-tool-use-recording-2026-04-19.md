# Forge-Tool-Use-Recording (Phase 2A-4c1) — Results

**Phase:** 2A-4c1 of Phase 2A-4 Forge-Identity master decomposition.
**Date:** 2026-04-19 (spec) → 2026-04-23 (ship).
**Parent design:** `docs/superpowers/specs/2026-04-19-forge-tool-use-recording-design.md` (v3, `b1ad7d9`)
**Implementation plan:** `docs/superpowers/plans/2026-04-19-forge-tool-use-recording.md` (`ba9f503`)
**Parent master:** `docs/benchmarks/forge-identity-master-design.md` §5 2A-4c1
**HEAD at ship time:** `887f1a1` — `chore(2A-4c1): address adversarial review — H1/H2/H4/M1/M2`
**Prior phase:** 2A-4b Recency-weighted Preference Decay shipped 2026-04-19 (HEAD `21aa115`).

## Summary

**SHIPPED.** 2A-4c1 adds the `session_tool_call` append-only table, `Request::RecordToolUse`
(atomic INSERT-SELECT), `Request::ListToolCalls` (snapshot-consistent read), and the
`tool_use_recorded` event. Substrate ready for 2A-4c2 Phase 23 (Behavioral Skill Inference)
and 2A-4d Dim 5 bench.

**Tests:** 1352 lib tests + 2 integration tests passing (up from 1294 at 2A-4b baseline).
Clippy clean. Fmt clean.

**No regression-guard benches run** — c1 touches no scoring / recall / decay surfaces.

Live-daemon dogfood (HTTP at port 8430, `git_sha: 887f1a1`):
- A) Record → Ok with ULID id + created_at ✓
- B) List → Ok with 1 row, `tool_args` round-trips as nested JSON ✓
- C) Unknown session → Error `"unknown_session: 01NONEXISTENT0000000000000"` ✓
- D) Payload too large (65 537-byte `tool_args`) → Error `"payload_too_large: tool_args: 65536"` ✓
- E) Control char in session_id (``) → Error `"invalid_field: session_id: control_character"` ✓

## What shipped

| Task | Scope | Commit |
|------|-------|--------|
| T1 | `ToolCallRow` shared type in `core::types::tool_call` | `ebeaf01` |
| T2 | `session_tool_call` schema + 3 indexes | `31d13f6` |
| T3 | `ops::list_tool_calls` + 9 L1 tests | `fad0120` + `86b8a6b` |
| T4 | Request + Response variants + handler stubs | `bc7f05d` |
| T5 | `handle_record_tool_use` atomic INSERT-SELECT happy path | `7118434` |
| T6 | `handle_record_tool_use` validation (8 error paths) | `b22fbb2` |
| T7 | `handle_record_tool_use` event emission | `9371708` |
| T8 | `handle_list_tool_calls` snapshot-consistent read (5 tests) | `bc049f6` |
| T9 | `handle_list_tool_calls` validation + scope pins (6 tests) | `9d2b1e9` |
| T10 | Integration `record_tool_use_flow.rs` (2 tests) | `c99b0da` |
| T11 | Rollback recipe test on populated DB (with pre-assertion) | `86c216b` |
| Review | H1/H2/H4/M1/M2 fixups (NO BLOCKERS found) | `887f1a1` |
| T12 | Live-daemon dogfood + this results doc | *this commit* |

Five commits in T7–T11 + one review-fixup commit = six commits on top of T1–T6.
Combined `origin/master..HEAD` diff: 31 files changed, ~3 650 insertions.

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
unreachable (PK + `WHERE id=?2` guarantees ≤ 1 source row), logged as `internal_error`.

This eliminates the TOCTOU race flagged in spec v1 review.

## Target-session org consistency (spec §10.2–10.3)

`RecordToolUse` writes with `organization_id = COALESCE(session.organization_id, 'default')`
at INSERT time. The row's org is guaranteed to match the target session's current org.

`ListToolCalls` derives `target_session_org` from the target session
(`SELECT organization_id FROM session WHERE id = ?`) inside a transaction, then scans with
`WHERE organization_id = ?target_session_org AND session_id = ?`. Rows from other orgs
can never leak; rows from other sessions in the same org can never leak.

**This is NOT cross-caller isolation.** Any caller with a valid `session_id` can read or
write that session's tool calls. Phase 2A-6 (authenticated caller-session API) owns the
caller-isolation property. See spec §11.1.

(Adversarial review finding H2 — renamed from misleading `caller_org` to `target_session_org`
in commit `887f1a1`.)

## Event payload (spec §9)

`tool_use_recorded` event is emitted **only after** a successful INSERT. Payload:

```json
{
  "id":         "<ULID>",
  "session_id": "<sid>",
  "agent":      "<agent>",
  "tool_name":  "<tool>",
  "success":    true|false,
  "created_at": "<iso>"
}
```

Deliberately excluded: `tool_args` (size + PII), `tool_result_summary` (size + PII),
`user_correction_flag` (the c2 filter contract is still unlocked — not ready to lean on
downstream).

Validation errors and `unknown_session` paths do NOT emit. Broadcast is fire-and-forget;
the `session_tool_call` table is the source of truth.

## Error-message convention (stable prefixes)

| Prefix | Meaning |
|--------|---------|
| `unknown_session: <id>` | Target session does not exist, or was deleted mid-call. |
| `payload_too_large: tool_args: 65536` | Serialised `tool_args` > 65 536 UTF-8 bytes. |
| `payload_too_large: tool_result_summary: 65536` | `tool_result_summary` > 65 536 UTF-8 bytes. |
| `empty_field: tool_name` | Empty or whitespace-only `tool_name`. |
| `empty_field: agent` | Empty or whitespace-only `agent`. |
| `invalid_field: <field>: control_character` | `\0` or other `< 0x20` (except `\t`) rejected in `session_id`, `agent`, or `tool_name`. |
| `limit_too_large: requested <n>, max 500` | `ListToolCalls` `limit` > 500. |
| `internal_error: <sanitized>` | Non-`QueryReturnedNoRows` rusqlite fault. |

Callers can prefix-match programmatically.

## Live-daemon dogfood (T12)

Rebuilt + restarted at HEAD `887f1a1`. Exercised the full 2A-4c1 surface via `POST /api`
on `127.0.0.1:8430`. Test session: `SESS-2A4C1-TEST-01` registered via `register_session`.

Daemon version response:

```json
{"status":"ok","data":{"kind":"version","version":"0.4.0","build_profile":"release","target_triple":"aarch64-apple-darwin","rustc_version":"rustc 1.88.0 (6b00bc388 2025-06-23)","git_sha":"887f1a1","uptime_secs":5}}
```

### Step A — Record tool call

Request:

```bash
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"record_tool_use","params":{
    "session_id":"SESS-2A4C1-TEST-01","agent":"claude-code","tool_name":"Read",
    "tool_args":{"file_path":"/tmp/a"},"tool_result_summary":"ok",
    "success":true,"user_correction_flag":false
  }}'
```

Response:

```json
{"status":"ok","data":{"kind":"tool_call_recorded","id":"01KPWV4D8ECXJVDP6W477XF6W8","created_at":"2026-04-23 09:35:57"}}
```

### Step B — List for session

Request:

```bash
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"list_tool_calls","params":{"session_id":"SESS-2A4C1-TEST-01"}
}'
```

Response:

```json
{"status":"ok","data":{"kind":"tool_call_list","calls":[{"id":"01KPWV4D8ECXJVDP6W477XF6W8","session_id":"SESS-2A4C1-TEST-01","agent":"claude-code","tool_name":"Read","tool_args":{"file_path":"/tmp/a"},"tool_result_summary":"ok","success":true,"user_correction_flag":false,"created_at":"2026-04-23 09:35:57"}]}}
```

Note: `tool_args` round-trips as a nested JSON object, not an escaped string — serde_json
`Value` preservation is intact through the full pipeline.

### Step C — Unknown session

Request:

```bash
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"list_tool_calls","params":{"session_id":"01NONEXISTENT0000000000000"}
}'
```

Response:

```json
{"status":"error","message":"unknown_session: 01NONEXISTENT0000000000000"}
```

### Step D — Payload too large (65 537-byte `tool_args`)

Request (via `curl --data-binary @file` — 65 746 bytes on the wire):

```python
import json
payload = {
  "method":"record_tool_use",
  "params":{
    "session_id":"SESS-2A4C1-TEST-01","agent":"x","tool_name":"x",
    "tool_args":{"x":"A"*65537},"tool_result_summary":"",
    "success":True,"user_correction_flag":False
  }
}
```

Response:

```json
{"status":"error","message":"payload_too_large: tool_args: 65536"}
```

### Step E — Control char in `session_id` (``)

Request:

```bash
curl -sS -X POST http://127.0.0.1:8430/api -d '{
  "method":"list_tool_calls","params":{"session_id":"abcxyz"}
}'
```

Response:

```json
{"status":"error","message":"invalid_field: session_id: control_character"}
```

## Adversarial review — T7-T11 (Claude substitute, Codex CLI unresponsive)

**NO BLOCKERS.** 4 HIGH + 2 MEDIUM findings, all addressed in `887f1a1` except H3 (pre-existing
T5/T6 validation order vs spec §5.1 — outside this PR's diff range, carried to 2A-4c2).

| ID | Severity | Issue | Fix commit |
|----|----------|-------|------------|
| H1 | HIGH | T11 rollback test would pass vacuously on forward-migration regression — `DROP IF EXISTS` silently no-ops when indexes absent. | `887f1a1`: added `idx_count_before == 3` pre-assertion. |
| H2 | HIGH | `caller_org` variable name inverted the security model (derived from target session, not caller). | `887f1a1`: renamed `caller_org` → `target_session_org`; updated comment to reference §10.2-10.3. |
| H3 | HIGH | RecordToolUse validation order deviates from spec §5.1 (ULID generated before `tool_args` size check). | **Not fixed in this PR** — pre-existing in T6 (`b22fbb2`), outside the T7-T11 diff range. Flagged for 2A-4c2 cycle. |
| H4 | HIGH | `has_control_char` defined twice as identical local functions. | `887f1a1`: hoisted to module-scope `fn has_control_char` above `handle_request`. |
| M1 | MEDIUM | T9 test name overstated isolation property (session_id does the filtering, not org). | `887f1a1`: renamed to `list_tool_calls_session_id_scope_excludes_sibling_sessions_within_same_org` with explanatory comment. |
| M2 | MEDIUM | T9 org-only test lacked explanation that the forged-org row is unreachable via the normal write path. | `887f1a1`: added comment clarifying raw-SQL injection + WHERE clause pin. |

## Known carry-forwards (NOT blockers)

1. **H3 validation order** — move `tool_args` serialization/size check before ULID/timestamp
   generation. Trivial reorder, no behavior change except error-priority alignment with spec.
   Owner: 2A-4c2 cycle.
2. **MCP tool 4th slug namespace** — SP1 found `record_tool_names` has only 3 slug candidates
   (bare, `cli:X`, `claude:X`). MCP tools (e.g., `mcp__context7__query-docs`) don't match any.
   Separate from 2A-4c1 (different code path — that's the SP1 transcript-parser counter, not
   the c1 event log). Owner: SP2 or 2A-4c2.
3. **`pre-existing forge_persist_harness` flake** — `SpawnTimeout`. Noted in SP1, still extant.

## Test gates (at HEAD `887f1a1`)

```
cargo test -p forge-daemon --lib            → 1352 passed, 0 failed, 1 ignored
cargo test -p forge-daemon --test record_tool_use_flow
                                            → 2 passed, 0 failed
cargo clippy --workspace -- -W clippy::all -D warnings
                                            → clean
cargo fmt --all                             → clean
```

Integration-test coverage added by this phase: `record_tool_use_flow.rs` (T10),
`e2e_sp1_dark_loops.rs` carried from SP1 (unchanged).

## Ship checklist

- [x] T1-T11 committed.
- [x] Adversarial review completed; findings addressed.
- [x] Live daemon rebuilt at HEAD, dogfood 5-step exercise passing.
- [x] Results doc written (this file).
- [ ] Push to `origin/master` — awaiting user approval.
