# Adversarial review — 2A-6 spec v1

**Target:** `docs/superpowers/specs/2026-04-26-multi-agent-coordination-bench-design.md`
**Target SHA:** `d64fe83`
**Date:** 2026-04-26
**Reviewer:** claude (general-purpose)
**Verdict:** `not-lockable`

## Findings count

| Severity | Count |
|----------|-------|
| BLOCKER  | 4     |
| HIGH     | 3     |
| MEDIUM   | 3     |
| LOW      | 0     |

## Yaml + summary

See sibling YAML file `2026-04-26-p3-3-2a-6-spec-multi-agent-coordination.yaml` for the structured findings.

## Reviewer reasoning trace (lightly edited)

### BLOCKER-1 / BLOCKER-2 — schema cardinality

Read `db/schema.rs:720-738` for the CREATE block:

```sql
CREATE TABLE IF NOT EXISTS session_message (
    id TEXT PRIMARY KEY,
    from_session TEXT NOT NULL,
    to_session TEXT NOT NULL,
    kind TEXT NOT NULL,
    topic TEXT NOT NULL DEFAULT '',
    parts TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending',
    in_reply_to TEXT,
    project TEXT,
    timeout_secs INTEGER,
    created_at TEXT NOT NULL,
    delivered_at TEXT,
    expires_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_msg_to ON session_message(to_session, status);
CREATE INDEX IF NOT EXISTS idx_msg_from ON session_message(from_session);
CREATE INDEX IF NOT EXISTS idx_msg_reply ON session_message(in_reply_to);
```

13 columns in the base CREATE.

Then `db/schema.rs:1107` adds:

```sql
ALTER TABLE session_message ADD COLUMN meeting_id TEXT
CREATE INDEX IF NOT EXISTS idx_msg_meeting ON session_message(meeting_id)
```

So **14 columns + 4 indexes** post-migration. Spec claims 11 + 3 — both wrong.

### BLOCKER-3 — reviewer error (recorded for honesty)

Reviewer claimed `session.organization_id` does not exist. Counter-verification at `db/schema.rs:864` shows:

```rust
let _ = conn.execute(
    "ALTER TABLE session ADD COLUMN organization_id TEXT DEFAULT 'default'",
    [],
);
```

The column IS present via ALTER. Spec was correct. v2 adds a parenthetical cite to the ALTER line so future readers don't repeat the verification mistake.

### BLOCKER-4 — cross-project corpus math

Spec says "6 cross-project messages exist". Computation:

- 6 sessions, 5 senders per recipient × 2 messages each = 10 incoming per inbox.
- Per inbox: 2 same-project peers × 2 messages = 4 same-project; 3 other-project peers × 2 = 6 cross-project.
- 6 inboxes × 6 cross-project = **36 cross-project messages total**.

Spec is off by 6×.

### HIGH-1 — D1 denominator brittleness

Hardcoded denominator "50" assumes D1 runs first. v2 should compute denominator at D1 runtime as `(SELECT COUNT(*) FROM session_message) - 10`, OR assert pre-D1 row count via infra check 6.

### HIGH-2 — D5 sentinel-hash row pinning

Probe 6 + 7 say "sentinel-row hash unchanged" but don't pin the row id. With ~36 mutations before D5, the row must be one of the 60 seeded that no prior dim touches. v2 must pin (e.g., `seed_msg_planner_alpha_to_generator_alpha_0`) + add an invariant that probe 6/7 do not touch this id.

### HIGH-3 — Grant/Revoke citation

Spec lists these "at sessions.rs". Actual:

- `crates/core/src/protocol/request.rs:535,542` — variants
- `crates/daemon/src/server/handler.rs` — dispatch handlers
- `crates/daemon/src/sessions.rs:571` — `check_a2a_permission` (gate logic only)

### MED-1 — D5 probe 2 boundary

Spec uses 65000 bytes. Source check at `sessions.rs:375` is `parts_json.len() > 65536`. 65000 < 65536 so probe passes trivially. To pin the boundary, test exactly 65536 bytes (last legal size).

### MED-2 — D5 probe 1 error string

Source error at `sessions.rs:377` is exactly: `"message parts exceed 64KB limit"`. Spec text says "exceed 64KB limit" — works as `.contains()`. v2 makes the substring assertion explicit.

### MED-3 — recon HEAD stale

Recon table dated "HEAD `1377ee1`"; current HEAD at spec commit time is `d64fe83`. Re-stamp at T1.

## Math sanity (verified)

- Composite weights: 0.20 + 0.15 + 0.15 + 0.20 + 0.15 + 0.15 = 1.00 ✓
- ULID via `Ulid::new().to_string()` = 26 chars ✓
- Broadcast fan-out = 2 same-project peers ✓ (verified `sessions.rs:386-394`)
- 60 seeded messages = 6 sessions × 10 incoming = 6 senders × 10 outgoing ✓ (symmetric)
- `respond_to_message` orig.from↔orig.to inversion in NEW row ✓

## Resolution path

Per project policy + 2A-5 v1 precedent: **Path A — rewrite spec to v2** addressing all 4 BLOCKERs, 3 HIGHs, 3 MEDs in one commit. Re-dispatch adversarial review on v2.
