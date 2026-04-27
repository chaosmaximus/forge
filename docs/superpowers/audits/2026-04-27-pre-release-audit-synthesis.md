# Pre-Release Audit Synthesis — v0.6.0 ZR-close

**Date:** 2026-04-27. **Base HEAD:** `1cf9a7d` (post-ZR close).
**Auditors:** 6 parallel `general-purpose` agents (A docs, B CLI, C dead-code, D harness, E DB, F observability/UX).

## Tally

| Audit | CRITICAL | HIGH | MED | LOW | NIT | Total |
|-------|----------|------|-----|-----|-----|-------|
| A docs | 0 | 4 | 10 | 5 | 1 | 20 |
| B CLI | 0 | 5 | 8 | 9 | 2 | 24 |
| C dead-code | 0 | 2 | 7 | 8 | 6 | 23 |
| D harness | 0 | 10 | 8 | 5 | 2 | 25 |
| E DB | 4 | 5 | 10 | 5 | 1 | 25 |
| F UX/obs | 2 | 5 | 12 | 8 | 3 | 30 |
| **Total** | **6** | **31** | **55** | **40** | **15** | **147** |

## CRITICAL findings (verified — implementer + agent both distrusted)

| # | ID | Title | File | Verified |
|---|----|-------|------|----------|
| 1 | **E-1** | `PRAGMA foreign_keys=OFF` in canonical helper — `raw_chunks` orphaned on `delete_document` | `crates/daemon/src/db/pragma.rs:85-113` | ✓ apply_runtime_pragmas has only journal_mode + busy_timeout |
| 2 | **E-2** | `sync_export` soft-scope reintroduces W29 cross-project leak | `crates/daemon/src/sync.rs:244` | ✓ `(project = ? OR project IS NULL OR project = '')` |
| 3 | **E-3** | ZR migration regression test gap | n/a | ✗ **FALSE POSITIVE** — agent missed the 4 `zr_c3_*` tests at schema.rs:3493/3634/3666/3748 |
| 4 | **E-4** | `store_project` INSERT OR REPLACE data-loss on `idx_project_path_unique` collision | `crates/daemon/src/db/ops.rs:2939-2952` | ✓ confirmed — even auto_create_project_if_absent docstring acknowledges the trap exists |
| 5 | **F-CRIT-1** | First-run BROKEN — fresh `$HOME` with no `~/.forge/` makes every CLI invocation fail | `crates/cli/src/client.rs:143` | ✓ no `create_dir_all` before `OpenOptions::create` |
| 6 | **F-CRIT-2** | `observe --shape row-count` permanently broken — `Request::Inspect` missing from `is_read_only()` | `crates/daemon/src/server/writer.rs:70` | ✓ verified — Inspect routes through writer actor where `metrics: None` |

**Net: 5 CRITICAL to fix; E-3 deferred (false positive).**

## HIGH findings — by domain

### Docs (4 HIGH from Audit A)

- **DOCS-A-001** — Docs reference `forge` binary for secret scanning; binary doesn't exist
- **DOCS-A-002** — Memory-type list (`fact`, `entity`, `skill`) doesn't match `MemoryType` enum
- **DOCS-A-003** — Wrong Unix socket filename (`daemon.sock` vs `forge.sock`)
- **DOCS-A-004** — security.md says gRPC is "(Planned)" — it shipped

### CLI (5 HIGH from Audit B)

- **B-HIGH-1** — `register-session --role` parses but discards
- **B-HIGH-2** — `team create --parent` parses but discards
- **B-HIGH-3** — `cleanup-sessions --older-than` silent-destructive `unwrap_or(0)`
- **B-HIGH-4** — `recall --since` silent-destructive `unwrap_or(0)`
- **B-HIGH-5** — `--help` advertises `version` keyword that isn't a subcommand

### Dead code (2 HIGH from Audit C)

- **C-HIGH-1** — `crates/daemon/src/migrate.rs` (150 LOC) is dead production code
- **C-HIGH-2** — `teams::create_meeting_with_voting` (40 LOC) has zero callers

### Harness / Skills / Agents (10 HIGH from Audit D)

- **D-01–D-05** — 5 skills reference non-existent CLI commands (`forge scan`, `forge research`, `forge verify .`, `forge test run`, `forge query`, `forge review .`)
- **D-06** — Agents reference `data/` and `evaluation-criteria/` dirs that don't exist in repo
- **D-07** — `check-harness-sync.sh` regex misses bare `forge X` invocations (catches only `forge-next`/`forge-cli`)
- **D-08** — forge-setup advertises Stitch MCP via `.mcp.json` (file doesn't exist; `mcpServers: {}`)
- **D-09** — forge-setup uses wrong slash syntax `/forge:new` (should be `/forge:forge-new`)
- **D-10** — Three skills invoke fictional `TaskCreate` tool (real CC tool is `TodoWrite`)

### DB (5 HIGH from Audit E — beyond the CRITICALs)

- **E-5** — `memory_vec.store_embedding` doesn't validate 768-dim (code/raw vec writers do)
- **E-6** — Audit-log triggers created via `let _ = conn.execute_batch(...)` — silent fail risk
- **E-7** — `register_session` INSERT OR REPLACE wipes lifecycle columns on re-register
- **E-8** — `code_search` `name LIKE '%pattern%'` has no index — O(N) at scale
- **E-9** — `kpi_reaper` Pass A missing composite `(event_type, timestamp)` index

### Observability / UX (5 HIGH from Audit F — beyond CRITICALs)

- **F-HIGH-1** — Grafana panels + alerts use label keys (`phase_name`, `error_kind`, `outcome`, `layer`) that don't match emit-side (`phase`, `kind`, `action`, `table`)
- **F-HIGH-2** — All 9 alert runbooks reference non-existent CLI subcommands (`forge-next observe worker-status`, `phase-summary`, `forge-next logs`, `service restart`)
- **F-HIGH-3** — `/metrics` + `/inspect` HTTP unreachable on default install (`http.enabled=false`); first-time Grafana scrape silently fails
- **F-HIGH-4** — "8-layer Manas memory" headline contradicts observability surface, which exposes 11 layers via `row-count` and `forge_table_rows`
- **F-HIGH-5** — `docker compose --profile monitor up -d` hard-fails — `deploy/prometheus.yml` referenced but not shipped

## Fix-wave plan

**Phase 1 — CRITICALs (1 commit):** E-1, E-2, E-4, F-CRIT-1, F-CRIT-2.

**Phase 2 — CLI HIGH (1 commit):** B-HIGH-1..5.

**Phase 3 — DB HIGH (1 commit):** E-5..E-9.

**Phase 4 — Observability/UX HIGH (1 commit):** F-HIGH-1..5.

**Phase 5 — Dead code HIGH (1 commit):** C-HIGH-1, C-HIGH-2.

**Phase 6 — Harness drift HIGH (1 commit):** D-01..D-10.

**Phase 7 — Docs HIGH (1 commit):** DOCS-A-001..A-004.

**Phase 8 — Adversarial review on the full fix-wave + fix-wave for review findings.**

**MEDs / LOWs / NITs (110 items)** are deferred to v0.6.0 backlog with rationale appended to `docs/operations/v0.6.0-pre-iteration-deferrals.md`.

## Cross-cutting observations

1. **The `let _ = conn.execute(...)` anti-pattern is alive.** Despite `feedback_sqlite_no_reverse_silent_migration_failure.md`, several still-shipping migrations + audit-log triggers use it (E-6, E-13, E-15, E-16). The narrow ZR-fw1 LOW-1 fix only patched `idx_project_path_unique`; the full audit was deferred per `v0.6.0-pre-iteration-deferrals.md` entry #13. **The findings here say that defer was wrong** — these are ship-blocking on E-6 (audit-log compliance), surface-blocking on E-15 (multi-org boundary).

2. **The harness-sync gate has a regex hole.** It catches `forge-next`/`forge-cli` invocations but misses bare `forge X` (D-07). 6 skills + 1 agent slip through. The whole point of the gate was to catch this drift.

3. **The README + getting-started promises don't match reality.** Memory-type list wrong, endpoint count stale, version stale. New users will hit immediate doc-vs-reality friction.

4. **Multiple wire-protocol-vs-vocabulary leaks survived ZR.** `notification.reality_id` in JSON wire shape (E-18); cli-reference still describes `--reality` config scope (DOCS-A-009). Per `feedback_project_everywhere_vocabulary.md`, user-facing surfaces should say `project`.

5. **First-run is genuinely broken.** F-CRIT-1 reproduces against a fresh `HOME=$(mktemp -d)`. This is the single highest-impact issue — every new user hits it.

## Carry-forwards (to v0.6.1+)

- `feedback_sqlite_no_reverse_silent_migration_failure.md` — workspace-wide audit (entry #13 of deferrals)
- `feedback_insert_or_replace_data_loss_on_unique_index.md` — `register_session` (E-7) + any other REPLACE on UNIQUE-indexed tables not yet caught
- `feedback_sentinel_replacement_for_soft_scope_leak.md` — fixed for `recall` (W29), now `sync_export` (E-2)
- `feedback_project_everywhere_vocabulary.md` — wire-shape `reality_id` aliases on Notification + CLI scope identifier

## Next actions

1. Synthesis doc (this file) → committed.
2. Phase 1 CRITICAL fix-wave → next.
3. Phases 2–7 in sequence.
4. Adversarial review on combined diff.
5. HANDOFF rewrite. Halt for #101.
