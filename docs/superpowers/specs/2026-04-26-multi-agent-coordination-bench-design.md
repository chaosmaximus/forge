# Multi-Agent Coordination Bench (2A-6) — Design v1

**Status:** DRAFT v1 — 2026-04-26. Pending adversarial review.
**Phase position:** Second sub-phase of P3-3 (after 2A-5 closed at HEAD `1377ee1`).
**Predecessors:** 2A-5 domain-isolation bench (v2.1 LOCKED + impl shipped).
**Successors:** 2A-7 daemon restart drill, 2C-1/C-2 ops dashboards.

---

## 1. Goal

**Validate FISP-driven multi-agent coordination correctness as a measurable
quality dimension.** Today, the daemon ships an inter-session message
queue (`session_message` table + `Request::SessionSend`/`SessionRespond`/
`SessionMessages`/`SessionAck`) that is the substrate for the planner →
generator → evaluator pipeline pattern adopted in `agents/forge-*.md` +
project protocol "FISP agent orchestration: planner→generator→evaluator
pipeline" (active in this repo). The substrate has unit tests in
`crates/daemon/src/sessions.rs` but no end-to-end correctness probe that
would catch regressions in:

- Cross-session inbox leakage (a malformed `WHERE to_session = ?1` would
  not be caught by single-session unit tests).
- Broadcast project-scoping (the `to="*"` SELECT at `sessions.rs:386-400`
  filters by project + active status; a JOIN regression here is invisible
  to current tests).
- Authorization enforcement on `ack_messages` and `respond_to_message`
  (both check caller ownership; regression class = "drop the WHERE
  clause").
- The full planner → generator → evaluator causal chain (response messages
  with correct `in_reply_to` round-tripping back to the original sender's
  inbox).

**Before this work:**
- `session_message` table at `db/schema.rs:720-738` has indexes on
  (to_session, status), (from_session), (in_reply_to).
- `sessions.rs::send_message` (line 363), `respond_to_message` (line 430),
  `list_messages` (line 479), `ack_messages` (line 532) implement the
  primitives.
- Single-session happy-path unit tests exist; no cross-session leakage,
  no broadcast precision, no authorization-bypass probes.

**After this work:**
- A new bench `forge-coordination` runs in-process per the Forge-Identity /
  Forge-Isolation precedent. Generates a deterministic 6-session corpus
  (3 roles × 2 projects: planner-α, generator-α, evaluator-α + same triplet
  for β), seeds 60 directed messages + 4 broadcasts + a 4-message pipeline
  chain, and scores 6 dimensions covering inbox precision, send+retrieve
  round-trip integrity, broadcast project-scoping, ack/respond
  authorization, edge-case resilience, and end-to-end pipeline chain
  correctness.
- The bench emits one `kpi_events` row per run with
  `event_type='bench_run_completed'` and
  `metadata_json.bench_name='forge-coordination'`, consumable by the
  `bench_run_summary` `/inspect` shape from Tier 3.
- A new `forge-bench forge-coordination` CLI subcommand mirrors the
  forge-isolation flag layout (`--seed`, `--output`, `--expected-composite`).
- The bench joins the CI matrix as the fourth in-process bench under the
  same `continue-on-error: true` rollout policy until 14 consecutive green
  master runs accumulate (T17 promotion gate from Tier 3 D4 covers this
  bench too).

**Success metric:** a reviewer can answer "did this commit break the FISP
inbox isolation, broadcast scoping, authorization, or planner →
generator → evaluator chain?" by reading a single composite from the
last bench run.

---

## 2. Verified reconnaissance (2026-04-26, HEAD `1377ee1`)

| # | Fact | Evidence |
|---|------|----------|
| 1 | `session_message` table with 11 columns: `id TEXT PRIMARY KEY, from_session TEXT NOT NULL, to_session TEXT NOT NULL, kind TEXT NOT NULL, topic TEXT NOT NULL DEFAULT '', parts TEXT NOT NULL DEFAULT '[]', status TEXT NOT NULL DEFAULT 'pending', in_reply_to TEXT, project TEXT, timeout_secs INTEGER, created_at TEXT NOT NULL, delivered_at TEXT, expires_at TEXT`. | `db/schema.rs:720-734` |
| 2 | 3 indexes: `idx_msg_to (to_session, status)`, `idx_msg_from (from_session)`, `idx_msg_reply (in_reply_to)`. | `db/schema.rs:735-737` |
| 3 | `Request::SessionSend` accepts `to`, `kind ("notification" or "request")`, `topic`, `parts`, `project`, `timeout_secs`, `meeting_id`, `from_session`. | `crates/core/src/protocol/request.rs:488-504` |
| 4 | `Request::SessionRespond` accepts `message_id`, `status` (one of "accepted", "rejected", "completed", "failed" per protocol), `parts`. | `request.rs:505-510` |
| 5 | `Request::SessionMessages` accepts `session_id`, optional `status`, `limit`, `offset`. | `request.rs:511-518` |
| 6 | `Request::SessionAck` accepts `message_ids`, optional `session_id` (caller for ownership check). | `request.rs:519-525` |
| 7 | `sessions::send_message(conn, from_session, to, kind, topic, parts_json, project, timeout_secs, meeting_id) -> Result<String>` returns the new message ID. Validates `parts_json.len() <= 65536` (returns `Err(InvalidParameterName(...))` on overflow). | `sessions.rs:362-380` |
| 8 | Broadcast (`to="*"`) at `sessions.rs:385-413`: SELECTs `id FROM session WHERE status IN ('active', 'idle') AND project = ?1 AND id != ?2` (when project is `Some`); INSERTS one row per recipient. Returns a sentinel `broadcast_id` (its own ULID, NOT inserted as a row). | `sessions.rs:385-414` |
| 9 | Direct send (`to != "*"`) at `sessions.rs:415-423`: single INSERT with `status='pending', created_at=datetime('now')`, optional `expires_at`. Does NOT validate that `to` corresponds to an existing session. | `sessions.rs:415-423` |
| 10 | `respond_to_message(conn, message_id, from_session, status, parts_json)` at line 430-475: SELECTs original message; if original missing returns `Ok(false)`; if `orig.to_session != from_session` returns `Ok(false)` with a stderr WARN; otherwise UPDATES original.status and INSERTS a new message with `kind='response', from_session=responder, to_session=orig.from_session, in_reply_to=orig_id, project=orig.project, status=<the response status>`. | `sessions.rs:430-475` |
| 11 | `list_messages(conn, session_id, status_filter, limit, offset)` at line 479: SELECTs `WHERE to_session = ?1` plus optional `AND status = ?2`, `ORDER BY created_at DESC, LIMIT/OFFSET` capped at 1000. | `sessions.rs:479-528` |
| 12 | `ack_messages(conn, message_ids, caller_session)` at line 532-548: per-id UPDATE `WHERE id = ?1 AND to_session = ?2` setting `status='read', delivered_at=datetime('now')`. Returns total rows affected. Silently no-ops on ID mismatch OR caller-not-recipient. | `sessions.rs:532-548` |
| 13 | `session` table columns (relevant): `id PRIMARY KEY, agent NOT NULL, project, status DEFAULT 'active'` plus migrations adding `working_set, capabilities, current_task, tool_use_count, organization_id, team_id`. Bench needs only `id, agent, project, status, started_at, organization_id`. | `db/schema.rs:410-418` + ALTER blocks |
| 14 | Forge-Isolation precedent at `crates/daemon/src/bench/forge_isolation.rs` (~970 lines): single shared `DaemonState` per seed (§3.7 mandate); `seed_corpus(state, corpus)` direct-INSERT pattern; `run_bench_in_state(state, corpus, seed)` orchestrator that zeros dimensions when infra fails. Identical primitives reusable. | direct read |
| 15 | `bench/common.rs::seeded_rng(seed)` returns `ChaCha20Rng`; `deterministic_embedding(seed_key)` returns 768-dim `Vec<f32>`; `sha256_hex(input)` returns hex digest (used in 2A-5 D5 sentinel-row hash). | `bench/common.rs:11,18,31` |
| 16 | `bench/scoring.rs::composite_score(scores: &[f64], weights: &[f64]) -> f64` lifted in 2A-5 T2.2. Reusable byte-for-byte; no further lift needed. | `bench/scoring.rs` |
| 17 | `bench/telemetry.rs::emit_bench_run_completed` opens short-lived rusqlite WAL connection, single INSERT into `kpi_events`, closes. No-op when `FORGE_DIR` unset. New bench requires only a `bench_name` registry row in `docs/architecture/events-namespace.md`. | `bench/telemetry.rs` |
| 18 | CI bench-fast matrix today (post-2A-5): `[forge-consolidation, forge-identity, forge-isolation]` with `continue-on-error: true`. Adding `forge-coordination` adds ~60s wall-clock (single ubuntu-latest job). | `.github/workflows/ci.yml` |
| 19 | `forge-bench` binary at `crates/daemon/src/bin/forge-bench.rs` dispatches by clap subcommand. Adding `forge-coordination` follows the existing pattern (~30-line clap variant + dispatch fn). | direct read |

T1 re-verifies these at HEAD-current. Specifically grep `respond_to_message` to confirm the in_reply_to inversion logic (orig.from_session ↔ orig.to_session) is unchanged — that's the load-bearing invariant for D6.

---

## 3. Architecture

### 3.1 Six dimensions

| Dim | Name | Probe | Min | Weight |
|-----|------|-------|-----|--------|
| **D1** | `inbox_precision` | For each session S in 6 sessions (3 roles × 2 projects): `list_messages(conn, &S.id, None, 1000, None)`. Foreign-message denominator = total messages in DB minus messages with `to_session=S.id`. Score = 1 − (foreign_observed / max_possible_foreign) averaged. Min 0.95. | 0.95 | 0.20 |
| **D2** | `roundtrip_correctness` | For K=10 trials: `send_message(from, to, kind="notification", topic="t_{idx}", parts_json="[{\"text\":\"p_{idx}\"}]", project=Some("alpha"), None, None)`; immediately `list_messages(conn, &to, Some("pending"), 1000, None)` and find the row by id. Assert (a) row found, (b) from_session=expected, (c) to_session=expected, (d) topic=expected, (e) parts roundtrip-equals, (f) kind=expected, (g) project=expected. Score = pass_count / (K × 7). Min 0.95. | 0.95 | 0.15 |
| **D3** | `broadcast_project_scoping` | For K=4 trials (one per role × project combo): `send_message(from, "*", kind="notification", topic="b_{idx}", parts_json, project=Some(<sender_project>), None, None)`. Pre-broadcast: count matching rows. Post-broadcast: re-count matching rows. Assert (a) delta = 2 (2 same-project peers excluding sender), (b) all delta-rows have `project=<sender_project>`, (c) zero delta-rows have `to_session in <other_project_sessions>`. Score = pass_count / (K × 3). Min 0.95. | 0.95 | 0.15 |
| **D4** | `authorization_enforcement` | Two sub-classes (combined). **Ack ownership:** for K=3 trials, send M from A to B; have C call `ack_messages(conn, &[M.id], &C.id)`; assert (a) returned count = 0, (b) M.status post-call still 'pending'. **Respond authorization:** for K=3 trials, send M (kind='request') from A to B; have C call `respond_to_message(conn, &M.id, &C.id, "completed", "[]")`; assert (a) returns Ok(false), (b) M.status unchanged, (c) no row exists with `in_reply_to=M.id`. Score = pass_count / (3×2 + 3×3) = pass_count / 15. Min 0.95. | 0.95 | 0.20 |
| **D5** | `edge_case_resilience` | 7 sub-probes (see §3.1a). Score = pass_count / 7. Min 0.85. | 0.85 | 0.15 |
| **D6** | `pipeline_chain_correctness` | For K=2 trials (one per project), construct a 4-step pipeline: (1) planner → generator request M1, (2) generator responds to M1 with status='accepted' → creates M2, (3) generator → evaluator request M3, (4) evaluator responds to M3 with status='completed' → creates M4. Assert per-trial: (a) M1.status post-step-2 = 'accepted', (b) M2 exists with `from_session=generator, to_session=planner, kind='response', in_reply_to=M1.id, status='accepted'`, (c) M3.status post-step-4 = 'completed', (d) M4 exists with `from_session=evaluator, to_session=generator, kind='response', in_reply_to=M3.id, status='completed'`, (e) M2 retrievable via `list_messages(planner.id, None, ...)`, (f) M4 retrievable via `list_messages(generator.id, None, ...)`. Score = pass_count / (K × 6). Min 0.90. | 0.90 | 0.15 |

**Composite:** weighted mean across the 6 dims (weights sum to 1.00).
**Pass gate (dual):** composite ≥ 0.95 AND every dim ≥ its min.

D1 + D4 weighted equal at 0.20 because both audit precision/security at
the FISP-receive surface (inbox listing and authorization). D2 0.15, D3
0.15 because they audit functionality + a precision sub-class (broadcast
fan-out) at the FISP-send surface. D5 0.15 catches edge-case bugs. D6 0.15
audits the explicit plan-doc deliverable (planner→generator→evaluator E2E)
even though it's a partial superset of D2+D4 — value is in the chain
walk + in_reply_to round-trip that no other dim probes.

### 3.1a D5 — 7 sub-probes

1. **`payload_size_limit_enforced`** — `send_message(..., parts_json=<65537-byte string>, ...)` returns `Err`. Pass = `Err(InvalidParameterName(_))` containing "exceed 64KB limit".
2. **`payload_at_limit_succeeds`** — `send_message(..., parts_json=<65000-byte JSON>, ...)` returns `Ok(<msg_id>)`. Pass = `Ok(_)` AND new row exists in `session_message` with the returned id. (Boundary case companion to probe 1.)
3. **`send_to_nonexistent_session_no_panic`** — `send_message(from=<real>, to="zzz_nonexistent_session_xxx", ...)` returns `Ok(<msg_id>)` (no recipient validation per fact 9). Pass = `Ok(_)` AND row INSERTED.
4. **`respond_to_nonexistent_message_returns_false`** — `respond_to_message(conn, "zzz_nonexistent_msg_xxx", from_session, "completed", "[]")` returns `Ok(false)` per fact 10 line 473. Pass = `Ok(false)` AND no new rows inserted.
5. **`empty_broadcast_zero_inserts`** — `send_message(from, "*", ..., project=Some("zzz_no_active_sessions"), ...)` returns `Ok(<broadcast_id>)` with 0 INSERTs. Pass = `Ok(_)` AND `session_message` count delta = 0.
6. **`empty_ack_returns_zero`** — `ack_messages(conn, &[], "any_caller")` returns `Ok(0)` (no UPDATEs). Pass = `Ok(0)` AND no row mutations (sentinel-row hash unchanged).
7. **`sql_injection_in_topic_inert`** — `send_message(from, to, kind, topic="alpha'; DROP TABLE session_message;--", parts_json, ...)`. Pass = `Ok(_)` AND `session_message` table still exists (`SELECT 1 FROM session_message LIMIT 1` succeeds) AND sentinel-row hash unchanged. Catches DROP TABLE / DELETE FROM **and** UPDATE-class injection (`UPDATE session_message SET status='read'`). Sentinel hash = SHA-256 of a canary row's `(id, from_session, to_session, kind, topic, parts, status, in_reply_to)`.

7 probes × `pass_count / 7` scoring: single failure = 14% drop (still ≥ 0.85 min, robust to one regression).

### 3.2 Dataset generator

`bench/forge_coordination.rs::generate_corpus(rng: &mut ChaCha20Rng) -> Corpus`:

```
Sessions (6 total):
  Roles × Projects = 3 × 2:
    [planner, generator, evaluator] × [alpha, beta]
    Session id format: format!("{role}_{project}")
    e.g. "planner_alpha", "generator_alpha", "evaluator_alpha",
         "planner_beta",  "generator_beta",  "evaluator_beta"
    Each session: agent="forge-{role}", project="{project}", status='active'
                  started_at="2026-04-26T00:00:00Z", organization_id='default'

Pre-seeded directed messages (60 total):
  For each session S: 10 messages addressed TO S, sent from each of the
  5 other sessions × 2 messages each. Per spec §3.3 D1 the foreign-message
  denominator is `(total_msgs - 10)` per inbox = 50.
  Per-message:
    id: ULID-stable format, deterministically derived from
        (sender_idx, recipient_idx, idx_within_pair)
    from_session: <sender_id>
    to_session: <recipient_id>
    kind: 'notification' (90% of seeded) or 'request' (10%)
    topic: format!("seed_{sender_role}_{recipient_role}_{idx}")
    parts: JSON `[{"text": "<from→to>: m_{idx}"}]`
    status: 'pending'
    project: <recipient_session.project>  (for D3 cross-project audit:
             6 cross-project messages exist where sender.project != recipient.project)
    in_reply_to: NULL
    created_at: 2026-04-26T00:00:00Z + idx seconds (deterministic ordering)

Total static rows in session_message after seeding: 60.

Dynamic operations (run during dim execution; not part of static corpus):
  D2 adds K=10 trial messages.
  D3 adds K=4 broadcast operations (each fans out to 2 same-project peers
     = 8 new rows total).
  D4 adds K=6 messages (3 ack-test + 3 respond-test) plus 0-3 response rows
     depending on bug class.
  D5 adds 1-7 messages depending on probes (most are reverted by sentinel
     check).
  D6 adds K=2 × 4 = 8 messages (2 trials of 4-step pipeline) plus 2 × 2
     response messages = 12 messages.

  Post-bench session_message row count is dim-execution-order-dependent
  but bounded; D1 reads only seeded rows because it runs FIRST (per
  §3.3 dim order rationale).
```

Per the 2A-5 precedent (M4 fix), all corpus content is fully derived by
formula from `(role, project, idx)` triples — no `rand_range` consumption
from `rng`. The `_rng: &mut ChaCha20Rng` parameter is taken for
signature-consistency with other bench harnesses but not consumed.

### 3.3 Score formulas

```text
D1 score per session S:
   inbox = list_messages(&conn, &S.id, None, 1000, None)
   foreign_count = |{ m in inbox : m.to_session != S.id }|
   max_possible_foreign = total_seeded_messages - 10  // 50 per inbox
   score_S = 1 − (foreign_count / max_possible_foreign)
   D1 = mean across 6 sessions.

D2 (K=10 trials):
   subassertions = [from, to, topic, parts, kind, project, row-found]
   pass_count = Σ over trials of |{ a in subassertions : a holds }|
   D2 score = pass_count / (K × 7) = pass_count / 70

D3 (K=4 trials):
   subassertions = [delta=2, all-rows-project-match, no-cross-project-leak]
   pass_count = Σ over trials of |{ a in subassertions : a holds }|
   D3 score = pass_count / (K × 3) = pass_count / 12

D4:
   ack_subassertions per trial = [count=0, status=pending]
   ack_pass_count = Σ over 3 ack trials = at most 3 × 2 = 6
   respond_subassertions per trial = [returns-false, status-unchanged, no-reply-row]
   respond_pass_count = Σ over 3 respond trials = at most 3 × 3 = 9
   D4 score = (ack_pass_count + respond_pass_count) / 15

D5 score = pass_count / 7 (per §3.1a)

D6 (K=2 trials):
   subassertions per trial = [M1.status='accepted', M2 row shape, M3.status='completed',
                              M4 row shape, M2 in planner inbox, M4 in generator inbox]
   pass_count = Σ over 2 trials of |{ a in subassertions : a holds }|
   D6 score = pass_count / (K × 6) = pass_count / 12

Composite = 0.20*D1 + 0.15*D2 + 0.15*D3 + 0.20*D4 + 0.15*D5 + 0.15*D6
```

**Dim execution order (load-bearing):** D1 → D2 → D3 → D4 → D6 → D5.

D1 runs FIRST against the static 60-message corpus (pre-perturbation).
D5 runs LAST because some probes (probe 7 SQL-injection) intentionally
push adversarial input into `session_message`; running D5 before
correctness dims would invalidate sentinel-hash assumptions in those
dims. The order is checked by an infrastructure-stub assertion at bench
startup — not load-bearing for correctness, but documented for clarity.

### 3.4 Infrastructure assertions

8 fail-fast checks before dimensions run (matches 2A-5 cardinality):

1. `session_message_table_exists` — `pragma_table_info('session_message')` returns ≥ 11 rows.
2. `session_message_indexes_present` — `sqlite_master` contains `idx_msg_to`, `idx_msg_from`, `idx_msg_reply`.
3. `session_table_columns_present` — `pragma_table_info('session')` includes `id`, `agent`, `project`, `status`.
4. `seeded_rng_deterministic` — `seeded_rng(42)` produces identical first u64 twice.
5. `corpus_size_matches_spec` — corpus has exactly 6 sessions + 60 directed messages.
6. `session_distribution_correct` — count by (role, project) matches spec (1 each); 10 incoming per session confirmed.
7. `send_message_returns_ulid` — sanity probe: `send_message(...)` returns `Ok(<id>)` AND `id.len() == 26` (ULID length).
8. `respond_to_message_inverts_addressing` — sanity probe: send msg, respond, verify the response row has `from_session=responder, to_session=orig.from_session, in_reply_to=orig.id`.

Any check failing → abort with summary failure (composite=0.0, all dims
zeroed per 2A-5 MED-4 fix precedent).

### 3.5 Telemetry integration

Standard `bench_run_completed` emit at the tail of execution per Tier 3 §3.2.
No new event type. New `dimensions[].name` registry rows added to
`docs/architecture/events-namespace.md`:

```
bench_name: "forge-coordination"
dimensions:
  - inbox_precision
  - roundtrip_correctness
  - broadcast_project_scoping
  - authorization_enforcement
  - edge_case_resilience
  - pipeline_chain_correctness
```

### 3.6 CI integration

Add `forge-coordination` as a fourth matrix entry to
`.github/workflows/ci.yml`'s `bench-fast` job. Same `continue-on-error:
true` rollout policy until 14 consecutive green master runs (T17 promotion
gate from Tier 3 D4 covers this bench too — promotion happens for the
whole matrix at once).

Adds ~60s to bench-CI wall-clock; no impact on the 15-min total CI budget.

### 3.7 Single shared corpus per seed (mirrors 2A-5 §3.7)

**Mandate:** all 6 dims read from a **single shared `DaemonState`** seeded
with the corpus once per `--seed` invocation. Per-dim isolated `:memory:`
DBs (as in `forge_identity.rs:1703 run_dim_isolated`) actively HIDE
cross-dim leakage because each dim sees a fresh slate. For
forge-coordination, D5's SQL-injection probe specifically requires
cross-dim integrity (the sentinel hash compared in D5 must reflect the
state left by D1-D4-D6 — running them in isolation would break the chain
of evidence). Forge-identity's per-dim isolation pattern is appropriate
for *its* property-testing surface; for forge-coordination it's the wrong
primitive.

Implementation: `run_bench(seed)` builds one `DaemonState`, calls
`seed_corpus(&mut state, &corpus)`, then runs `infrastructure_checks` →
D1 → D2 → D3 → D4 → D6 → D5 sequentially against that state.

---

## 4. Architecture decisions

- **D1 — Number of sessions / projects.** 3 roles × 2 projects = 6 sessions.
  Smaller than 2A-5's 6 projects (intentional: coordination semantics test
  via topology, not scale). 60 directed messages keep the bench fast
  (<500ms target).
- **D2 — Send mechanism.** Use `sessions::send_message(...)` directly via
  in-process daemon helpers (same pattern as 2A-5 forge_isolation +
  forge_identity). Don't spawn a daemon subprocess.
- **D3 — Embedding model.** Not used. Coordination bench is purely about
  the FISP message envelope; no semantic search.
- **D4 — Composite weighting.** D1 0.20, D2 0.15, D3 0.15, D4 0.20,
  D5 0.15, D6 0.15. D1 + D4 highest because they audit precision +
  authorization at the FISP-receive surface; D6 = explicit plan-doc
  deliverable but partial superset of D2+D4.
- **D5 — Pass gate.** Composite ≥ 0.95 AND every dim ≥ its min (dual gate;
  matches 2A-5 precedent).
- **D6 — D5 edge cases.** v1 ships 7 probes (payload size, payload at
  limit, nonexistent recipient, nonexistent reply target, empty broadcast,
  empty ack, SQL injection). v2+ candidates: Unicode topic / parts edge,
  concurrent send-and-ack race, expired-message reaper interaction.
- **D7 — Calibration target.** 1.0 composite on all 5 seeds before lock —
  same as 2A-5 (which converged 5/5 first-run). Plan for 0-2 cycles;
  halt-and-flag at 5.
- **D8 — Single shared DaemonState (§3.7).** Mandatory for sentinel-hash
  integrity in D5 + cross-dim signal preservation.
- **D9 — Teams API (`teams.rs`) deferred.** v1 uses `session.project` as
  a lightweight team substitute. The full team API (`list_team_templates`,
  `create_team`, `run_team`, `spawn_agent`, `team_member` table) is more
  surface than this bench needs and is out of scope. v2+ extends with team
  membership probes if a regression class surfaces there.
- **D10 — `meeting_id` deferred.** `Request::SessionSend` accepts
  `meeting_id` for meeting-participant responses; v1 does not exercise this
  field. Defer to a future bench focused on the meeting / poll surface.

---

## 5. Out of scope (with explicit disclaimers)

- **Concurrent send-and-ack stress.** Single-thread, sequential ops. A
  race condition where two callers ack the same message simultaneously is
  not probed; rely on SQLite's per-connection serialization for v1.
- **Expired-message reaper interaction.** `expires_at` field exists; a
  background reaper would mark messages 'expired'. v1 does not test the
  reaper, only the static `expires_at` value persistence.
- **Network probes / multi-daemon FISP.** All in-process. The
  `to_session=remote@other-host` cross-daemon FISP path (HLC-tagged sync)
  is out of scope; covered by `forge-persist` bench's HLC determinism
  audit.
- **Permission gating.** `Request::GrantPermission` / `RevokePermission`
  exist; `send_message` per-call permission checks are at the request
  handler layer, NOT inside `sessions::send_message`. v1 calls the helper
  directly so this layer is not exercised. v2 adds a Dim 7 permission-
  enforcement probe at the request handler boundary.
- **Real LLM calls / agent execution.** Bench never spawns real planner /
  generator / evaluator subprocesses. The "agents" in this bench are
  session ID labels with role-suggestive names; correctness is purely a
  property of the FISP message-queue substrate, not the agents driving it.
- **Tag / FTS leakage at the message layer.** `session_message` does not
  participate in `memory_fts`; no FTS-substring class. Mentioned for
  completeness with the 2A-5 §5 disclaimer.
- **Coverage table — what compile_context-style helpers are NOT exercised:**

  | FISP-adjacent surface | Site | v1 coverage | Reason |
  |---|---|---|---|
  | `sessions::send_message` (direct) | sessions.rs:363 | **covered (D2, D3, D5)** | direct call path |
  | `sessions::send_message` (broadcast) | sessions.rs:385 | **covered (D3)** | broadcast SELECT |
  | `sessions::respond_to_message` | sessions.rs:430 | **covered (D4 respond, D6)** | authorization + chain |
  | `sessions::list_messages` | sessions.rs:479 | **covered (D1, D2, D6)** | inbox precision + roundtrip |
  | `sessions::ack_messages` | sessions.rs:532 | **covered (D4 ack)** | ownership enforcement |
  | Request handler permission gate | server/handler.rs (a2a perm check) | not-covered | v2 follow-up |
  | Meeting-participant response path | sessions.rs (meeting_id branches) | not-covered | v2 follow-up |
  | Background reaper (`expires_at`) | workers/reaper.rs | not-covered | reaper bench candidate |
  | `Request::GrantPermission` / `RevokePermission` | sessions.rs (permission CRUD) | not-covered | permission-bench candidate |

---

## 6. Dependencies / blockers

* **LOCKED:** Forge-Identity bench precedent (master v6 + 2A-4d.3 shipped).
* **LOCKED:** Forge-Isolation bench precedent (2A-5 v2.1 + impl shipped at HEAD `1377ee1`).
* **SHIPPED:** Tier 3 telemetry layer (`bench_run_completed` emit) +
  Tier 3 leaderboard surface (`bench_run_summary` `/inspect` shape).
* **No new schema.** Uses existing `session_message` table + indexes
  + `session` table.
* **No new request variants.** Uses existing `Request::SessionSend`,
  `SessionRespond`, `SessionMessages`, `SessionAck` — but the bench calls
  the underlying `sessions::*` helpers directly (matches 2A-5 pattern).
* **No further `bench/common.rs` / `bench/scoring.rs` lifts needed.** All
  primitives lifted in 2A-5 T2.1 + T2.2 (`deterministic_embedding`,
  `composite_score`, `seeded_rng`, `sha256_hex`) are reusable as-is.

---

## 7. Task breakdown

| Task | Description | Agent-friendly? |
|------|-------------|-----------------|
| **T1** | Re-verify the 19 recon facts at HEAD (whatever HEAD is current at impl time). Specifically grep `respond_to_message` to confirm orig.from_session ↔ orig.to_session inversion is unchanged. Also confirm `session_message` schema column order (load-bearing for direct INSERT). | Yes — recon |
| **T2** | `crates/daemon/src/bench/forge_coordination.rs` skeleton: `CoordinationScore` + `BenchConfig` + 6 dimension stubs returning `DimensionScore { name, score: 0.0, min, pass: false }` + composite scorer call site (uses lifted `bench::scoring::composite_score`) + corpus generator stub returning `Corpus { sessions: vec![], messages: vec![] }` + 8 infrastructure-assertion stubs. Integration test stub running scorer on empty fixtures. **§3.7 mandate: single shared `DaemonState` per seed (no per-dim isolation).** | Yes |
| **T3** | Implement corpus generator (per §3.2). 6 sessions + 60 directed messages, deterministic content (no rand_range consumption). Adds `bench/forge_coordination/corpus.rs` if file size warrants. Implement `seed_corpus(state, corpus)` with direct INSERT INTO session + INSERT INTO session_message. | Yes |
| **T4** | Implement D1 (inbox_precision) + D2 (roundtrip_correctness). | Yes |
| **T5** | Implement D3 (broadcast_project_scoping) + D4 (authorization_enforcement). | Yes |
| **T6** | Implement D5 (edge_case_resilience — 7 probes per §3.1a) + D6 (pipeline_chain_correctness) + 8 infrastructure assertions. | Yes |
| **T7** | `forge-bench forge-coordination` CLI subcommand in `bin/forge-bench.rs` + argument plumbing (seed, output, expected-composite). | Yes |
| **T8** | Wire into `bench/telemetry.rs::emit_bench_run_completed` call path. Add `forge-coordination` row to `docs/architecture/events-namespace.md` per-bench dim registry. | Yes |
| **T9** | Calibration loop: run on 5 seeds, iterate until 1.0 composite (halt-and-flag at 5 cycles per locked decision). | Partially — interactive |
| **T10** | Adversarial review on T1-T9 diff (Claude general-purpose). | Yes |
| **T11** | Address review BLOCKER + HIGH; defer LOW with rationale. | Yes |
| **T12** | `.github/workflows/ci.yml` — add `forge-coordination` to `bench-fast` matrix with `continue-on-error: true`. | Yes |
| **T13** | Results doc at `docs/benchmarks/results/2026-04-XX-forge-coordination-stage2.md` mirroring forge-isolation precedent. | Yes |
| **T14** | Close 2A-6: HANDOFF append, Stage 2 task complete, MEMORY index entry. | Yes |

**Critical path:** T1 → T2 → T3 → {T4, T5 sequential after T3, T6 sequential after T5} → T7 → T8 → T9 → T10 → T11 → T12 → T13 → T14.

**Estimated commits:** 12-15 (depends on calibration cycle count + impl review fix-wave size).

---

## 8. Open questions (v1 → v2 triggers)

1. **Permission-handler gate.** v1 bypasses the request-handler permission
   layer by calling `sessions::send_message` directly. A regression in the
   request handler's permission check (e.g. `Request::SessionSend` arrives
   without a valid `from_agent`/`to_agent` permission) would not be caught.
   v2 candidate: Dim 7 permission_enforcement probe routing through the
   actual `Request::SessionSend` dispatch path.
2. **Concurrent ack race.** Two callers acking the same message in parallel.
   SQLite's transaction isolation makes this benign in practice, but a bug
   class exists where the WHERE clause is dropped and both acks succeed
   AND increment a counter twice. v1 single-thread; v2 candidate.
3. **Meeting-id branch (`Request::SessionSend.meeting_id`).** Auto-records
   the message as a meeting-participant response. v1 always passes
   `meeting_id=None`. v2 candidate: Dim 8 meeting_id_round_trip — verify
   the meeting_record table is populated correctly when `meeting_id` is set.
4. **Wall-clock target.** Forge-isolation at <500ms; forge-coordination
   should be similar (less corpus, simpler ops). Target ≤ 500ms on
   ubuntu-latest. T1 measures; if exceeds 1500ms, demote to nightly.

---

## 9. Acceptance criteria

- [ ] All 6 dimensions land with non-zero implementations.
- [ ] Composite ≥ 0.95 on 5 seeds (calibration locked).
- [ ] 8 infrastructure assertions all pass on a fresh state.
- [ ] `forge-bench forge-coordination --seed 42` runs in < 1.5s on
      ubuntu-latest.
- [ ] `bench_run_completed` event emitted with
      `metadata_json.bench_name='forge-coordination'` and 6-element
      `dimensions[]` array.
- [ ] CI matrix includes the bench under `continue-on-error: true`.
- [ ] Adversarial review verdict `lockable-as-is` or `lockable-with-fixes`
      with all HIGH addressed.
- [ ] Results doc + events-namespace registry updated.
- [ ] `cargo clippy --workspace --features bench --tests -- -W clippy::all -D warnings` clean.

---

## 10. References

- `docs/superpowers/specs/2026-04-25-domain-isolation-bench-design.md`
  — 2A-5 spec v2.1 LOCKED (template for this spec).
- `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier3-design.md`
  — bench harness precedent (v2 LOCKED).
- `docs/benchmarks/forge-identity-master-design.md` v6 — bench-internal pattern source.
- `docs/architecture/events-namespace.md` — `bench_run_completed` v1 contract + per-bench dim registry.
- `crates/daemon/src/bench/{common.rs, scoring.rs, telemetry.rs, forge_identity.rs, forge_isolation.rs}` — implementation precedent.
- `crates/daemon/src/sessions.rs:363,430,479,532` — FISP primitive entrypoints.
- `crates/daemon/src/db/schema.rs:410-420,720-738` — session + session_message DDL.
- `crates/core/src/protocol/request.rs:488-525` — Request::SessionSend/Respond/Messages/Ack.
- `agents/forge-{planner,generator,evaluator}.md` — pipeline pattern source.

---

## Changelog

- **v1 (2026-04-26):** Initial draft. 6 dims (inbox_precision +
  roundtrip_correctness + broadcast_project_scoping +
  authorization_enforcement + edge_case_resilience +
  pipeline_chain_correctness), 60-message + 6-session corpus, 7-probe D5,
  8 infra checks, single-shared `DaemonState` (§3.7) mirroring 2A-5 v2.1.
  Pending adversarial review.
