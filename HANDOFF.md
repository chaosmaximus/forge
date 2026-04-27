# Handoff — Pre-release audit Phases 1-10 (partial) closed; 91 findings pending — 2026-04-28

**Tracking ledger:** `docs/superpowers/audits/2026-04-27-tracking-ledger.md` — every one of the 147 findings with status (✅ fixed / 🟡 deferred / ❌ false positive / ⏳ pending). Read this first to know exactly what's done and what's queued.

**Public HEAD:** `<bumped on next commit>`. Working tree clean after the ledger commit.
**Version:** `v0.6.0-rc.3` (release stack still DEFERRED — halt for sign-off).
**Plan A:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Halt:** **YES** — Phase 10 partially closed (18 of 54 MEDs fixed); Phases 10D-10G + 11 + 12 queued.

## This session's deltas (2026-04-28)

8 commits + 2 background-agent dispatches. All 6 CRITICAL + 30 HIGH closed. 19 MEDs closed.

| Phase | Commit | Scope |
|-------|--------|-------|
| **5** | `92b15c9` | Dead code: deleted `migrate.rs` (150 LOC) + `create_meeting_with_voting` (40 LOC) + matching test_wave3 entries. -217 lines. |
| **6** | `37bd99a` | Harness drift: 6 skills + 3 agents rewritten away from fictional `forge scan/research/verify/query/review/build/plan/recall`. Extended `check-harness-sync.sh` regex to catch bare-`forge X` form. Drift fixture pinned. README/skill prose tweaks to suppress new false positives. |
| **7** | `45ffc9a` | Docs HIGH: memory-type list (`fact`/`entity`/`skill` → real 5-variant `MemoryType`), socket name (`daemon.sock` → `forge.sock`), gRPC "Planned" → real shipped binding, `forge scan` removed from getting-started. |
| **Phase 8** | review (`docs/superpowers/reviews/2026-04-28-pre-release-phases-5-7-adversarial.{md,yaml}`) | Verdict `lockable-with-fixes`: 1 HIGH (SKIP_CLI_TOKENS over-skip) + 2 LOW (regex limitations). |
| **9** | `5f3b93b` + `493baad` | Phase 8 fix-wave: pruned SKIP_CLI_TOKENS to `("binary" "cli")` (was masking `plugin/skill/memory/session` fictional drift). Added drift fixture entry + assertion to pin. Documented LOW-1/LOW-2 as known limitations in script comments. |
| **10A** | `bc795a8` | 5 of 10 DB MEDs: E-12 composite `idx_session_agent_project_status`; E-13/E-15/E-16 `let _ = conn.execute(...)` → `?` propagation (per `feedback_sqlite_no_reverse_silent_migration_failure.md`); E-19 `unwrap_or(false)` probe → `?`. |
| **10B** | `6a33c6b` (agent dispatch) | 8 of 8 harness MEDs: D-11 (front-matter on forge-build-workflow), D-12 (agent-content scan in harness-sync gate + 2 fixture assertions), D-13/D-14 verified-no-op, D-15 (drop empty owner.email), D-16 (LICENSES.yaml covers skills/agents), D-17 (forge-verify project-conventions detection), D-18 (FORGE_HOOK_VERBOSE in forge-setup + forge-verify Troubleshooting). |
| **10C** | `4f339fd` + ed | 5 of 10 docs MEDs: DOCS-A-005..A-008 (stale version/endpoint/worker/test counts), A-011 (cargo install clarifies 3 binaries from 2 crates). |

## State in one paragraph

**HEAD bumps with the ledger commit. Pre-release audit Phases 0-10 (partial) closed (8 phase commits + 2 doc commits + 1 review YAML).** **54 fixed + 1 false positive + 1 deferred = 56 of 147 findings closed**; **91 pending** (36 MED + 40 LOW + 15 NIT). All 5 CRITICAL + all 30 HIGH closed. Doctor green. Clippy 0 warnings. Daemon schema unit tests 35/35 pass. Harness-sync (158 JSON + 108 CLI) + protocol-hash (`0ad998ba944d…`) + license-manifest (3 files) + review-artifacts (29 reviews) all OK. 12/12 fixture tests pass. The full status table is in `docs/superpowers/audits/2026-04-27-tracking-ledger.md`.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -15                              # 8 phase commits + 2 doc commits since 1682f26
git status --short                                 # expect clean
forge-next doctor                                  # verify daemon spawns
bash scripts/check-harness-sync.sh                 # all 4 sanity gates
bash scripts/check-protocol-hash.sh                # 0ad998ba944d…
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh
bash tests/scripts/test-harness-sync.sh            # 12/12 fixture tests

# Resume options:
# A) Phase 10D-G — drain remaining 36 MEDs (recommended, ~3-4 commits)
# B) Phase 11 — LOW+NIT triage (55 items; each "fix vs document")
# C) Adversarial review on Phase 10 combined diff
# D) Skip residue, go straight to #101 release stack
```

## Remaining work

The **tracking ledger** (`docs/superpowers/audits/2026-04-27-tracking-ledger.md`) is the source of truth. Phases 10D-12 below summarize the resolution roadmap.

* **Phase 10D — CLI MEDs** (~7): B-MED-1 (Skills/Meeting handlers print raw Debug), B-MED-2 (multibyte UTF-8 byte-slice panics — 4 sites), B-MED-4 (`export --format ndjson` silent no-op), B-MED-5/6 (no CLI surface for `UpdateAgentTemplate` / `ListTeamTemplates`), B-MED-7 (`record-tool-use` silently re-encodes malformed JSON), B-MED-8 (`act-notification` with neither --approve/--reject silently rejects).
* **Phase 10E — Obs/UX MEDs** (~11): F-MED-1 (RPATH bakes onnxruntime-1.23.0 paths), F-MED-2 (macOS install error wrong binary name), F-MED-3 (Grafana hardcoded datasource), F-MED-4 (otlp-validation runbook stale CLI ref), F-MED-6 (no `forge-next plugin install/uninstall` despite docs implying it), F-MED-7 (compile-context "9 layers" vs README "8"), F-MED-8 (doctor warns about no embeddings without fix hint), F-MED-9 (OTLP silently disables when endpoint empty), F-MED-10 (`observe row-count` shows misleading `(no rows)` on stale), F-MED-11 (`ForgeWorkerDown` alert untriggerable — gauge always 1), F-MED-12 (no auto-create `~/.forge/config.toml` template).
* **Phase 10F — C MEDs** (~7): wrapper-triplet rot in recall.rs + consolidator.rs (10 + 6 pub fns test-only), `ProjectEngine` trait premature abstraction, `expire_diagnostics` test-only, etc. Mostly safe deletions.
* **Phase 10G — Remaining E (5) + A (4) MEDs** that needed design decisions: E-10 (audit-log read-tracking), E-11 (backup pruner worker), E-14 (sync_import tx pattern), E-17 (NULL line_start coerced to 0), E-18 (notification reality_id wire-shape rename), A-009 (cli-reference reality keys post-ZR), A-010 (task_completion_check undocumented), A-012 (marketplace.json claim), A-014 (internal phase tags in user docs).
* **Phase 11 — LOW + NIT triage** (40 LOW + 15 NIT = 55): each item reviewed for "fix vs document" — some are intentional design choices (E-20 Manas convention, E-24 dim split is documented).
* **Phase 12 — HANDOFF rewrite + halt for #101 release stack.**

Direct links:

* `docs/superpowers/audits/2026-04-27-tracking-ledger.md` — every finding's status
* `docs/superpowers/audits/2026-04-27-pre-release-audit-synthesis.md` — original synthesis
* `docs/superpowers/audits/2026-04-27-zr-close-pre-release-{A,B,C,D,E,F}-*.yaml` — full per-audit detail
* `docs/superpowers/reviews/2026-04-28-pre-release-phases-5-7-adversarial.{md,yaml}` — Phase 8 review

## Cumulative pending work

### Halt path (immediate)

* **#101** — P3-4 release v0.6.0 stack. Multi-OS verify + tag + GitHub release + marketplace bundle + branch protection. Last thing per `feedback_release_stack_deferred.md`.

### Pre-release audit residue

* 36 MED remaining (Phases 10D-G above)
* 40 LOW + 15 NIT (Phase 11 above)

### Wave Z + Y + X deferred (review residue from prior sessions)

* **#216 #217 #218 #219 #238** — TOCTOU msg hygiene, project rename/delete CLI, doctor backup XDG, cc-voice §1.2 e2e test, writer_tx route for compile-context.

### v0.6.1 follow-ups

* **#202 #233 #68** — `notify::Watcher` event-driven freshness; domain="unknown" upgrade in indexer; CI bench-fast gate promotion (BLOCKED on GHA billing).
* 9 pre-iteration deferrals per `docs/operations/v0.6.0-pre-iteration-deferrals.md`.
* **E-8** (audit deferral) — FTS5 over code_symbol for `code_search` perf.

## Halt-and-ask map

1. **HALT now.** Pre-release audit Phases 0-10 (partial) closed. 36 MED + 40 LOW + 15 NIT remain.
2. **Halt only on:** non-clean working tree across a wave boundary; review verdict `not-lockable`; surprise architectural blocker; cc-voice Round 4 feedback.
3. **AFTER user sign-off:** open Phases 10D-G → 11 → adversarial review on combined Phase 10 diff → #101 release stack.

## Adversarial reviews this session

* Phase 8 — `docs/superpowers/reviews/2026-04-28-pre-release-phases-5-7-adversarial.{md,yaml}` — verdict `lockable-with-fixes` (1 HIGH closed in Phase 9, 2 LOW documented).

## Auto-memory state

No new auto-memories saved this session. Established patterns applied:

* `feedback_sqlite_no_reverse_silent_migration_failure.md` — informed E-13/E-15/E-16/E-19 `?` propagation in Phase 10A.
* `feedback_ci_drift_fixture_pattern.md` — informed Phase 6 D-07 + Phase 9 plugin-mask + Phase 10B D-12 fixture additions.
* `feedback_release_stack_deferred.md` — informs the post-audit halt path.

## Daemon-binary state

Daemon respawn from current HEAD not yet performed — production binary still on prior session's HEAD. Next dogfood pass should rebuild release at HEAD and respawn before #101.

## One-line summary

**HEAD (bumps on ledger commit). Pre-release audit: 6 parallel agents found 147 findings; **54 fixed + 1 false positive + 1 deferred = 56 closed, 91 pending** — full status in `docs/superpowers/audits/2026-04-27-tracking-ledger.md`. All CRITICAL + all HIGH closed. 35/35 schema tests, all 4 sanity gates green, 12/12 fixture tests, protocol_hash 0ad998ba944d…. Halt for sign-off → resume with Phases 10D-G → 11 → 12 → #101.**
