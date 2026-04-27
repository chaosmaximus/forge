# Handoff — Pre-release audit Phases 1-4 closed; 128 findings tracked — 2026-04-27

**Tracking ledger:** `docs/superpowers/audits/2026-04-27-tracking-ledger.md` — every one of the 147 findings with status (✅ fixed / 🟡 deferred / ❌ false positive / ⏳ pending). Read this first to know exactly what's done and what's queued.

**Public HEAD:** `5432641` (Phase 4 close).
**Working tree:** clean.
**Version:** `v0.6.0-rc.3` (release stack still DEFERRED — halt for sign-off).
**Plan A:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Halt:** **YES** — pre-release audit drain partially closed (5/6 CRITICAL + 14/31 HIGH addressed); Phases 5/6/7 queued for next session.

## This session's deltas (5 commits + 6 parallel audits)

### Pre-release audit (Phase 0)

`be8f5ea` — 6 parallel `general-purpose` agents covered docs/CLI/dead-code/harness/DB/observability+UX. **147 findings** (6 CRITICAL · 31 HIGH · 55 MED · 40 LOW · 15 NIT). Written to `docs/superpowers/audits/2026-04-27-zr-close-pre-release-{A,B,C,D,E,F}-*.yaml` + `2026-04-27-pre-release-audit-synthesis.md`.

### Phase 1 — CRITICAL fixes

`386d32f` — 5 of 5 verified CRITICAL findings (E-3 was a false positive: agent missed the four `zr_c3_*` regression tests).

| ID | What |
|----|------|
| **E-1** | `apply_runtime_pragmas` now sets `PRAGMA foreign_keys=ON`. Pre-fix every `delete_document` orphaned `raw_chunks` rows because the cascade was unenforced. |
| **E-2** | `sync_export` soft-scope `(project=? OR project IS NULL OR project='')` replaced with strict `(project=? OR project='_global_')` per W29 sentinel pattern. |
| **E-4** | `store_project` `INSERT OR REPLACE` → `INSERT … ON CONFLICT(id) DO UPDATE` so a path-collision against a different id surfaces SQLITE_CONSTRAINT instead of silently DELETEing the older row. |
| **F-CRIT-1** | First-run BROKEN — `~/.forge/` not pre-created. Fix: `std::fs::create_dir_all(forge_dir())` before `OpenOptions::open` in CLI's daemon-spawn. |
| **F-CRIT-2** | `Request::Inspect` added to `is_read_only()`. Pre-fix `observe --shape row-count` permanently returned `stale=true, rows=[]` because Inspect routed through writer-actor with `metrics: None`. |

### Phase 2 — CLI HIGH (5)

`12e0466` — protocol_hash bump `c6eadd8e89e3…` → `0ad998ba944d…`.

| ID | What |
|----|------|
| **B-HIGH-1** | `register-session --role` wired through `Request::RegisterSession.role` (`#[serde(default)]`) to `session.role`. 32 call sites swept. |
| **B-HIGH-2** | `team create --parent` wired through `Request::CreateTeam.parent_team_id` to `team.parent_team_id`. |
| **B-HIGH-3** | `cleanup-sessions --older-than` typo no longer collapses to 0 (would end ALL sessions). Hard exit(2) on parse failure. |
| **B-HIGH-4** | `recall --since` typo no longer collapses to "now" (zero rows). Hard exit(2) with help. |
| **B-HIGH-5** | `version` removed from --help category roadmap (no such subcommand); `init` + `consolidate` added (real subcommands previously missing from the roadmap). |

### Phase 3 — DB HIGH (4 of 5; E-8 deferred)

`b8f7fb9`.

| ID | What |
|----|------|
| **E-5** | `memory_vec.store_embedding` validates 768-dim. Pre-fix any-dim byte slice corrupted the vec0 table silently. |
| **E-6** | audit_log triggers via `?` instead of `let _ =`. Tampering protection now surfaces failures at startup. |
| **E-7** | `register_session` `INSERT OR REPLACE` → `ON CONFLICT(id) DO UPDATE` preserving lifecycle columns (tool_use_count, budget_spent, working_set, parent_session_id, team_id, user_id, organization_id, reality_id). |
| **E-9** | Composite `idx_kpi_events_type_timestamp` index for kpi_reaper Pass A. |

**E-8 deferred** to v0.6.1: code_search `LIKE '%pattern%'` has no covering index; FTS5 over code_symbol is the right fix but needs its own design pass.

### Phase 4 — Observability + UX HIGH (5)

`5432641`.

| ID | What |
|----|------|
| **F-HIGH-1** | Grafana label-key drift fixed: dashboard JSON + alert YAML now use emit-side labels (`phase`, `kind`, `action`, `table`) instead of stale (`phase_name`, `error_kind`, `outcome`, `layer`). `outcome` group-by dropped (label doesn't exist). |
| **F-HIGH-2** | 9 alert runbooks updated to use real CLI commands (`observe --shape phase-run-summary`, `tail ~/.forge/daemon.log`, `forge-next restart`, `curl /metrics`). |
| **F-HIGH-3** | `HttpConfig::default { enabled: true }`. Loopback-only bind keeps it safe; first-time users now get `/metrics` + `/inspect` + `/api` without a config edit. 4 tests updated. |
| **F-HIGH-4** | README:100 "8-layer knowledge graph" → "8-layer Manas memory + entity/edge knowledge graph" (memory architecture vs storage layout disambiguation). |
| **F-HIGH-5** | `deploy/prometheus.yml` shipped (was referenced by `deploy/docker-compose.yml --profile monitor` but missing — `docker compose up -d` hard-failed). |

## State in one paragraph

**HEAD `ce1acce`. Pre-release audit Phases 0-4 closed (5 phase commits + 1 audit-suite + 1 HANDOFF + this tracking ledger commit).** **20 fixed + 1 false positive + 1 deferred = 22 of 147 findings closed**; **125 pending** (16 HIGH + 54 MED + 40 LOW + 15 NIT). Doctor green. Clippy 0 warnings; full daemon test suite at 1647/1647. Harness-sync + protocol-hash (`0ad998ba944d…`) + license-manifest + review-artifacts (28 reviews) all OK. The full status table is in `docs/superpowers/audits/2026-04-27-tracking-ledger.md`.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 5432641 (5 phase commits + audit synthesis)
git status --short                                 # expect clean
forge-next doctor                                  # verify daemon spawns
bash scripts/check-harness-sync.sh                 # all 4 sanity gates
bash scripts/check-protocol-hash.sh                # 0ad998ba944d…
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh

# Resume options (pre-release path):
# A) Phases 5-7 — deal with the 17 remaining HIGH findings (recommended)
# B) Adversarial review of Phases 1-4 diff first, then 5-7
# C) Skip 5-7, go straight to #101 release stack
# D) Backlog drain — MED/LOW/NIT (110 items)
```

## Remaining work

The **tracking ledger** (`docs/superpowers/audits/2026-04-27-tracking-ledger.md`) is the source of truth for what's done and what's pending. Phases 5-12 below summarize the resolution roadmap; the ledger has the full per-finding detail.

* **Phase 5** — Dead code (2 HIGH): C-HIGH-1 (delete `migrate.rs` 150 LOC + matching test_wave3 entries), C-HIGH-2 (delete `create_meeting_with_voting` 40 LOC).
* **Phase 6** — Harness drift (10 HIGH): D-01..D-10. 6 skills + 3 agents reference fictional CLI commands (`forge scan/research/verify/query/review/test run`); harness-sync regex hole; `.mcp.json` advertised but missing; wrong slash syntax `/forge:new`; fictional `TaskCreate` tool.
* **Phase 7** — Docs (4 HIGH): DOCS-A-001 (fake `forge` binary), DOCS-A-002 (wrong memory-type list), DOCS-A-003 (wrong socket filename), DOCS-A-004 (gRPC "Planned" but shipped).
* **Phase 8** — Adversarial review on Phases 1-7 combined diff (Plan A §6 mandatory).
* **Phase 9** — Fix-wave for review findings.
* **Phase 10** — MED batch (54 remaining): grouped by domain — DB MEDs (10 — incl. let_=conn.execute audit pattern E-13/E-15/E-16; backup pruner E-11; multi-org boundary E-15), CLI MEDs (7), obs MEDs (12 — incl. ForgeWorkerDown alert untriggerable F-MED-11; OTLP silent disable F-MED-9), docs MEDs (10), C MEDs (7), D MEDs (8).
* **Phase 11** — LOW + NIT triage (40 + 15 = 55): each "fix vs document" review.
* **Phase 12** — HANDOFF rewrite + halt for #101 release stack.

Direct links:

* `docs/superpowers/audits/2026-04-27-tracking-ledger.md` — every finding's status
* `docs/superpowers/audits/2026-04-27-pre-release-audit-synthesis.md` — original synthesis
* `docs/superpowers/audits/2026-04-27-zr-close-pre-release-{A,B,C,D,E,F}-*.yaml` — full per-audit detail with rationale

## Cumulative pending work

### Halt path (immediate)

* **#101** — P3-4 release v0.6.0 stack. Multi-OS verify + tag + GitHub release + marketplace bundle + branch protection. Last thing per `feedback_release_stack_deferred.md`. Re-opens after pre-release audit closes.

### Pre-release audit residue

* 17 HIGH remaining (Phases 5/6/7 above)
* 110 MED/LOW/NIT (110 items across the 6 audit YAMLs)
* 1 deferred CRITICAL — E-3 was actually a false positive

### Wave Z + Y + X deferred (review residue from prior sessions)

* **#216 #217 #218 #219 #238** — TOCTOU msg hygiene, project rename/delete CLI, doctor backup XDG, cc-voice §1.2 e2e test, writer_tx route for compile-context.

### v0.6.1 follow-ups

* **#202 #233 #68** — `notify::Watcher` event-driven freshness; domain="unknown" upgrade in indexer; CI bench-fast gate promotion (BLOCKED on GHA billing).
* 9 pre-iteration deferrals per `docs/operations/v0.6.0-pre-iteration-deferrals.md`.
* **E-8** (audit deferral) — FTS5 over code_symbol for `code_search` perf.
* Remaining 110 MED/LOW/NIT from pre-release audit.

## Halt-and-ask map

1. **HALT now.** Pre-release audit Phases 1-4 are complete. Phases 5-7 + adversarial review + #101 release stack remain.
2. **Halt only on:** non-clean working tree across a wave boundary; review verdict `not-lockable`; surprise architectural blocker; cc-voice Round 4 feedback.
3. **AFTER user sign-off:** open Phases 5-7 → adversarial review → #101 release stack.

## Adversarial reviews this session

* `docs/superpowers/audits/2026-04-27-zr-close-pre-release-{A,B,C,D,E,F}-*.yaml` — 6 parallel audits, 147 findings.
* Synthesis at `docs/superpowers/audits/2026-04-27-pre-release-audit-synthesis.md`.

## Auto-memory state

No new auto-memories saved this session. Established patterns applied:

* `feedback_sqlite_no_reverse_silent_migration_failure.md` — informed E-6 audit_log trigger fix.
* `feedback_insert_or_replace_data_loss_on_unique_index.md` — informed E-4 store_project + E-7 register_session fixes.
* `feedback_sentinel_replacement_for_soft_scope_leak.md` — informed E-2 sync_export fix.
* `feedback_table_rename_four_quadrant_state_matrix.md` — already saved last session, reused for ZR review.
* `feedback_release_stack_deferred.md` — informs the post-audit halt path.

## Daemon-binary state

Daemon respawn from current HEAD `5432641` not yet performed — production binary still on prior session's HEAD. Next dogfood pass should rebuild release at `5432641` and respawn before #101.

## One-line summary

**HEAD `ce1acce` (about to bump on the ledger commit). Pre-release audit: 6 parallel agents, 147 findings; **20 fixed + 1 false positive + 1 deferred = 22 closed, 125 pending** — full status in `docs/superpowers/audits/2026-04-27-tracking-ledger.md`. 1647/1647 daemon tests, all 4 sanity gates green, protocol_hash 0ad998ba944d…. Halt for sign-off → resume with Phases 5-12 → adversarial review → #101.**
