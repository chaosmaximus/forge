# Forge-Behavioral-Skill-Inference (Phase 2A-4c2) — Results

**Phase:** 2A-4c2 of Phase 2A-4 Forge-Identity master decomposition.
**Date:** 2026-04-24
**Parent design:** `docs/superpowers/specs/2026-04-23-forge-behavioral-skill-inference-design.md`
**Implementation plan:** `docs/superpowers/plans/2026-04-23-forge-behavioral-skill-inference.md`
**HEAD at ship time:** `90d9b74`
**Prior phase:** 2A-4c1 shipped 2026-04-23 (HEAD `cf74fb3`).

## Summary

**SHIPPED.** Phase 23 `infer_skills_from_behavior` is live. Given ≥ 3
matching clean tool-use fingerprints across sessions, the consolidator
elevates them to the `skill` table and surfaces them in `<skills>` via
`CompileContext`.

**Tests:** 1384 passing (1383 prior + new T9 rollback recipe + T10
regression guards). 1 ignored (pre-existing test_hook_e2e infra).

**Live dogfood (HEAD `90d9b74`, 2026-04-24):**
- 3 sessions registered (`DOGFOOD-2A4C2-SA/SB/SC`) with Read+Edit+Bash
  clean calls via HTTP on port 8430 against isolated `FORGE_DIR`.
- `force_consolidate` returned `consolidation_complete`; daemon log
  recorded `[consolidator] inferred 1 skills from tool-use patterns`.
- `compile_context` rendered:
  `<skill domain="file-ops" inferred_sessions="3">Inferred: Bash+Edit+Read [b9f98611]</skill>`
- PASS.

## What shipped

| Task | Scope | Commit |
|------|-------|--------|
| T1 | `skill` table Phase 23 columns (`agent`, `fingerprint`, `inferred_from`, `inferred_at`) + partial unique index | `c337cbd` |
| T2 | `ConsolidationConfig` skill-inference fields + `validated()` clamps | `7ebeab8` |
| T3 | Pure helpers: `canonical_fingerprint`, `infer_domain`, `format_skill_name` | `909672f` |
| T4 | `infer_skills_from_behavior` orchestrator + 9 L1 tests | `27b77a9` |
| T5 | Phase 23 registered in `run_consolidation` between Phase 17 and Phase 18 | `6b2f6a4` |
| T6 | `Request::ProbePhase` + `PHASE_ORDER` const + handler (test/bench-gated) | `3a87982` |
| T7 | `<skills>` renderer dual-gate (success_count>0 OR inferred_at.is_some()) + `inferred_sessions=""` | `eea150b` |
| T8 | `skill_inference_flow` integration test | `247656c` + `7d1ef2c` |
| T9 | Schema rollback recipe test (SQLite 3.35+ DROP COLUMN) | `f85d15c` |
| T10 | Adversarial review (Claude + Codex parallel) + Codex-H1 / H2 / Claude-B1 hardening + 4 regression tests | `90d9b74` |
| T11 | Live-daemon dogfood + this results doc | — |

## T10 adversarial review outcome

Two subagents ran in parallel: Claude `general-purpose` (full review) and Codex
`codex-rescue` (second opinion with inverted prompt). Raw outputs preserved in
the session transcript.

**Fixed in `90d9b74`:**

- **Codex-H1 (BLOCKER)** — Phase 23 INSERT omitted `skill_type`, inheriting the
  `'procedural'` default. This matched `prune_junk_skills()`'s delete predicate
  (`skill_type != 'behavioral' AND steps='[]' AND description='' AND
  success_count=0`), so a daemon restart would wipe every inferred row.
  *Fix:* Phase 23 INSERT now sets `skill_type='behavioral'` explicitly;
  `prune_junk_skills` additionally exempts `inferred_at IS NOT NULL /
  source='inferred'` as defense-in-depth.
- **Codex-H2 (HIGH)** — Inferred skills stored with `project=NULL` leaked across
  every project via recall's `project IS NULL` filter. *Fix:* query LEFT JOINs
  `session` to carry project through; partial unique index widened to
  `(agent, project, fingerprint)` (renamed `idx_skill_agent_project_fingerprint`);
  project value written as `''` (empty string) for global scope, never NULL —
  SQLite treats each NULL as distinct in unique indexes, which would break
  idempotency.
- **Claude-B1 (hardening)** — `canonical_fingerprint` documented `arg_keys` as
  pre-sorted but didn't defend against a forgetful future caller. *Fix:*
  defensive re-sort of per-call keys inside the helper + regression test.

**Accepted as carry-forwards:**

- **Codex-H3** — Fingerprint ignores arg values by design (Read("/tmp/a") vs
  Read("/prod/secret") collide intentionally). Splitting shape-vs-behavior
  fingerprints is 2P-1b scope.
- **Codex-MED** — `inferred_from` grows monotonically across sessions; no
  windowed pruning of aged-out session IDs. Follow-up: recompute from current
  window each run, or move observations to a separate table.
- **Codex-LOW** — `skills_inferred` count is not in `ConsolidationComplete`
  response; only emitted via `eprintln!` (which reaches stderr but not
  `tracing`). Follow-up: add `skills_inferred: usize` response field and
  `tracing::info!` event.
- **Claude-H1** — `Request::ProbePhase` is `#[cfg(any(test, feature = "bench"))]`
  by intentional design per spec T6 (test-gated assertion 9; not a runtime
  invariant probe).
- **Claude-H2** — `json_each(skill.inferred_from)` on a malformed row would
  error. Column DEFAULT is `'[]'` and all writers emit valid JSON, so only
  manually-corrupted rows hit this. Edge case; a defensive
  `WHERE json_valid(...)` is cheap but not shipped here.
- **Claude-H3** — Phase 23 runs between Phase 17 and Phase 18 despite its
  numeric label. Naming smell, not correctness bug.

## Known carry-forwards

Rolled into the 2P-1b backlog in `HANDOFF.md`:

1. Windowed pruning of `inferred_from` (Codex-MED).
2. `skills_inferred` exposed in `ConsolidationComplete` + structured telemetry
   (Codex-LOW).
3. Shape-vs-behavior fingerprint split (Codex-H3).
4. Defensive `json_valid` guard on UPDATE merge (Claude-H2).
5. Phase-number vs orchestrator-position alignment (Claude-H3) — rename to
   Phase 17.5 or extend `PHASE_ORDER` to list every phase.

## Commands used

Dogfood reproducer (`/tmp/dogfood-2a4c2.sh`) runs against an isolated
`FORGE_DIR` on port 8430 so the user's live daemon is untouched:

```bash
export FORGE_DIR=/tmp/forge-dogfood-2a4c2
export LD_LIBRARY_PATH=/mnt/colab-disk/DurgaSaiK/forge/forge/.tools/onnxruntime-linux-x64-1.23.0/lib
/mnt/colab-disk/DurgaSaiK/forge/forge/target/release/forge-daemon &

DAEMON=http://127.0.0.1:8430/api
for SID in DOGFOOD-2A4C2-SA DOGFOOD-2A4C2-SB DOGFOOD-2A4C2-SC; do
  curl -sS $DAEMON -d "{\"method\":\"register_session\",\"params\":{\"id\":\"$SID\",\"agent\":\"claude-code\"}}"
  for CALL in '"tool_name":"Read","tool_args":{"file_path":"/tmp/a"}' \
              '"tool_name":"Edit","tool_args":{"file_path":"/tmp/a","old_string":"x","new_string":"y"}' \
              '"tool_name":"Bash","tool_args":{"command":"cargo test"}'; do
    curl -sS $DAEMON -d "{\"method\":\"record_tool_use\",\"params\":{\"session_id\":\"$SID\",\"agent\":\"claude-code\",$CALL,\"success\":true}}"
  done
done

curl -sS $DAEMON -d '{"method":"force_consolidate"}'
curl -sS $DAEMON -d '{"method":"compile_context","params":{"agent":"claude-code"}}' \
  | python3 -c "import json,sys; print(json.load(sys.stdin)['data']['context'])" \
  | grep -E 'inferred_sessions|Inferred:'
```

Expected final line:
`<skill domain="file-ops" inferred_sessions="3">Inferred: Bash+Edit+Read [<hash>]</skill>`

## Unblocks

Phase **2A-4d Forge-Identity Bench** is now unblocked. The observability
carry-forwards (Codex-LOW) should land early in 2P-1b so the bench can read
`skills_inferred` directly from the response instead of scraping daemon
stderr.
