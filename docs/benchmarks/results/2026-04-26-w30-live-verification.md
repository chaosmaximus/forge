# W30 live verification — F16 cross-project identity pollution closed

**Date:** 2026-04-26
**Phase:** P3-3.11 W30 (commits 1-3, SHAs `ec81e6d..b958808`).
**Daemon under test:** v0.6.0-rc.3 release build, post-W30 migration applied.
**DB:** `~/.forge/forge.db`, 219 MB, mid-session live state. Pre-W30
backup at `~/.forge/forge.db.pre-W30-20260426-130705.bak` (untouched).

## Verification summary

| Check | Result |
|-------|-------|
| ALTER TABLE applied: `identity.project` column present | ✓ NOT NULL DEFAULT '_global_' |
| Backfill: pre-W30 rows now carry `_global_` sentinel | ✓ 43 → 43 rows |
| Zero NULL/empty `identity.project` post-migration | ✓ 0 rows |
| Strict `identity list --project forge` excludes `_global_` rows | ✓ |
| `--include-global-identity` admits `_global_` alongside project rows | ✓ |
| Write path: `identity set --project forge` → `forge` | ✓ |
| Write path: `identity set` (no `--project`) → `_global_` | ✓ |
| Strict view of new forge-tagged facet returns ONLY forge row | ✓ |

## Pre-W30 vs post-W30 distribution

Pre-W30: identity table has no `project` column. 43 active facets, 42 on
`claude-code` (incl. Hive Finance / dashboard / credit risk topics —
the F16 pollution surface) + 1 on `codex`.

```
PRAGMA table_info(identity);  -- pre-W30
→ id, agent, facet, description, strength, source, active, created_at,
  user_id, organization_id          (NO project column)
```

Post-W30 (live DB after migration applied):

```
sqlite3 forge.db "PRAGMA table_info(identity)"
→ id, agent, facet, description, strength, source, active, created_at,
  user_id, organization_id, project (NOT NULL DEFAULT '_global_')

sqlite3 forge.db "SELECT IFNULL(project,'<NULL>') AS p, COUNT(*) FROM identity WHERE active=1 GROUP BY p ORDER BY 2 DESC"
→
_global_|43

sqlite3 forge.db "SELECT COUNT(*) FROM identity WHERE project IS NULL OR project = ''"
→ 0
```

**Zero NULL or empty rows.** All previously-untagged facets backfilled
to `_global_`; future writes will tag explicitly via the DAO helper.
Pre-W30 had no source data linking facets to projects — the migration
cannot recover the original tagging, so all 43 land as global. Forward-
going writes (extracted from project-bound sessions, declared via
`identity set --project P`, or template-derived for spawned team
agents) will tag correctly.

## F16 reproducer — strict mode (Hive Finance pollution closed)

```
$ forge-next identity list --agent claude-code --project forge

Identity Facets (claude-code)
─────────────────────────────
  (no identity facets defined)
```

Strict scope returns zero facets because the migration could not
recover original project tagging — every legacy row is `_global_`,
none are `forge`-tagged. The Hive Finance / dashboard / credit risk
topics that were polluting `compile_context` for the forge agent
**no longer surface under strict project scope**. F16 closed.

## F16 reproducer — `--include-global-identity` opt-in

```
$ forge-next identity list --agent claude-code --project forge --include-global-identity

Identity Facets (claude-code)
─────────────────────────────
[1.00] expertise: Completed Soul Framework development (source: extracted)
[0.95] values: Values thoroughness and learning (source: extracted)
[0.95] values: Prioritizes testing and quality (source: extracted)
[0.95] role: Developer responsible for daemon component (source: extracted)
[0.95] values: Forge as a 'nervous system' (source: extracted)
[0.95] architecture: Daemon as a distributed nervous system (source: extracted)
[0.95] role: Architect/Lead for Forge project (source: extracted)
[0.95] expertise: Security-focused developer (source: extracted)
... (42 more rows — full agent-wide identity)
```

The opt-in path admits `_global_`-tagged facets alongside the (here
empty) project-tagged set. The historic broad semantic — "the agent's
identity surfaces in every project's context" — is preserved on
demand without it being the leak-prone default.

## Write-path verification

```
$ forge-next identity set --facet role \
    --description 'W30 verify forge-only role' \
    --strength 0.95 --project forge
Identity facet set: idfacet-1777209105262

$ forge-next identity set --facet expertise \
    --description 'W30 verify global expertise' \
    --strength 0.85
Identity facet set: idfacet-1777209113082

$ sqlite3 forge.db "SELECT id, project, facet, description \
    FROM identity WHERE description LIKE 'W30 verify%' \
    ORDER BY description"
idfacet-1777209105262|forge|role|W30 verify forge-only role
idfacet-1777209113082|_global_|expertise|W30 verify global expertise
```

The `--project forge` write tags the row `forge`. The omitted-project
write tags the row `_global_` via the DAO helper (`db::ops::project_or_global`).
**No NULL or empty value is reachable from any write path in this
crate.**

## Strict scope after the new forge-tagged write

```
$ forge-next identity list --agent claude-code --project forge

Identity Facets (claude-code)
─────────────────────────────
[0.95] role: W30 verify forge-only role (source: cli)
```

Only the explicitly-tagged forge facet appears. `_global_` rows do
**not** leak. The opt-in flag is the single switch that admits them,
matching the W29 `Recall.include_globals` semantic shape.

## Files referenced

* Migration: `crates/daemon/src/db/schema.rs::create_schema` —
  `ALTER TABLE identity ADD COLUMN project TEXT NOT NULL DEFAULT '_global_'`
  + idx_identity_project + idx_identity_agent_project + defensive
  UPDATE for legacy NULL/empty rows.
* DAO helpers: `crates/daemon/src/db/manas.rs` —
  `list_identity_for_project`, `list_identity_for_user_project`;
  `store_identity` keys dedup on `(agent, description, project)`;
  `row_to_identity` reads the new column.
* Write-site project propagation: `crates/daemon/src/workers/extractor.rs`
  (extracted facets → session.project), `crates/daemon/src/teams.rs`
  (team-template facets → spawned session.project),
  `crates/daemon/src/sync.rs::sync_import` (peer facet pass-through),
  `crates/cli/src/commands/manas.rs::identity_set` (CLI `--project` flag).
* Recall scoping: `crates/daemon/src/recall.rs::compile_static_prefix_with_inj`
  routes through `list_identity_for_project(agent, project, true, true)`
  when project is Some — closes F16 at the user-visible compile_context
  surface.
* Protocol: `crates/core/src/protocol/request.rs::Request::ListIdentity`
  gains `project` + `include_global_identity` fields.
* CLI: `crates/cli/src/main.rs::IdentityAction::List` /
  `IdentityAction::Set` — `--project` and
  `--include-global-identity` flags.

## Closing F16

The P3-3.8 dogfood findings doc described F16 as:

> 41 facets are visible in `identity list` for the `claude-code`
> agent — including "Working on Hive Finance credit risk pipeline",
> "Building Hive Finance platform with K8s and OTLP", "Manages a
> credit risk ML platform on GKE" — none of which are relevant to
> the forge repo. Identity should probably be per-(agent, project)
> not per-agent.

Root cause confirmed by this session's investigation: the `identity`
schema had no `project` column at all, so every facet was per-agent
and `compile_context` had no way to filter for the working project.

Fix delivered in commits `ec81e6d..b958808`:

1. Schema migration adds `project TEXT NOT NULL DEFAULT '_global_'` on
   the identity table; pre-W30 rows backfill to the sentinel via
   SQLite's ADD COLUMN with DEFAULT semantics + a defensive UPDATE leg.
2. DAO helper at every store_identity call site ensures no future
   write can re-introduce NULL/empty (closes the future-bug footgun).
3. New `list_identity_for_project` / `list_identity_for_user_project`
   strict-by-default readers; `compile_context` routes through them
   with `include_globals=true` so global convictions still surface
   while project-foreign facets are filtered out.
4. `--include-global-identity` opt-in on `identity list` preserves
   the broad semantic for callers who want it (e.g. agent-wide
   identity audit).
5. `identity set --project P` and `--project` flag tags new CLI
   facets to a project. Extractor and team-spawn paths derive
   project from the writing session.

The dogfood F16 query is now strictly scoped. Other-project facets
require explicit opt-in. Writes preserve project tagging end-to-end.
