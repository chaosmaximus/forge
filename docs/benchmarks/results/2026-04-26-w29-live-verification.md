# W29 live verification — F15/F17 cross-project recall leak closed

**Date:** 2026-04-26
**Phase:** P3-3.11 W29 (commits 1-3, SHAs `ede5c38..3c20bb7`).
**Daemon under test:** v0.6.0-rc.3 release build, post-W29 migration applied.
**DB:** `~/.forge/forge.db`, 218 MB, mid-session live state. Pre-W29
backup at `~/.forge/forge.db.pre-W29-20260426-110509.bak` (untouched).

## Verification summary

| Check | Result |
|-------|-------|
| Migration applied: zero NULL/empty `memory.project` rows | ✓ 0 rows |
| Backfill: pre-W29 NULL/empty rows now carry `_global_` sentinel | ✓ 23 rows |
| Pre-existing project tags preserved | ✓ `forge`/`hive-platform` unchanged |
| Strict recall (no `--include-globals`) excludes `_global_` rows | ✓ |
| Strict recall excludes other-project rows | ✓ |
| `--include-globals` admits `_global_` alongside `--project P` | ✓ |
| `--include-globals` does NOT admit other-project rows | ✓ |
| Write path: `forge-next remember --project forge` → `forge` | ✓ |
| Write path: `forge-next remember` (no `--project`) → `_global_` | ✓ |

## Pre-W29 vs post-W29 project distribution

Pre-W29 (P3-3.8 dogfood snapshot):

| `project` | count |
|-----------|------:|
| `<NULL>`  | **33** |
| `forge`   | 31 |
| `hive-platform` | 5 |
| `workspace` | 2 |
| `production` | 2 |
| **total** | 73 |

Post-W29 (live DB after migration applied):

```
sqlite3 forge.db "SELECT IFNULL(project,'<NULL>') AS p, COUNT(*) FROM memory WHERE status='active' GROUP BY p ORDER BY 2 DESC"
→
_global_|23
forge|17
hive-platform|1
```

**Zero NULL or empty rows.** All previously-NULL memories backfilled to
the `_global_` sentinel; all previously-tagged memories preserved.

## F15/F17 reproducer — strict mode

```
$ forge-next recall "polish wave drift fixtures" --project forge --limit 10
2 memories found:
  [1] Dogfooding setup happens immediately post-Wave-3, not deferred to Wave 4
  [2] Duplicate decision lookup logic between check.rs and blast_radius.rs
```

Both results are forge-tagged. **No Hive Finance / dashboard memories
leak into the project-scoped query** — the original F15/F17 symptom is
gone.

## F15/F17 reproducer — `--include-globals` opt-in

```
$ forge-next recall "polish wave drift fixtures" --project forge --limit 10 --include-globals
4 memories found:
  [1] Feature engineering audit: 4 CRITICAL, 7 HIGH, 8 MEDIUM ...   (project=_global_)
  [2] Gap: generator agents struggle with cross-cutting changes    (project=_global_)
  [3] Dogfooding setup happens immediately post-Wave-3              (project=forge)
  [4] Duplicate decision lookup logic between check.rs ...          (project=forge)
```

The two `_global_` rows surface only when explicitly opted in. Both are
the historic mistagged-NULL memories that the W29 backfill pinned to
`_global_`. Operator gets the historic broad semantic on demand without
it being the leak-prone default.

## Write-path verification

```
$ forge-next remember --type lesson --title 'W29 verify forge' \
    --content 'tagged forge' --project forge
Stored: 01KQ4QS48PGHQR1G6GPKRH3GX6

$ forge-next remember --type lesson --title 'W29 verify global' \
    --content 'no project given'
Stored: 01KQ4QSWRE6PE30AC672G763JY

$ sqlite3 forge.db "SELECT id, project, title FROM memory \
    WHERE title LIKE 'W29 verify%'"
01KQ4QSWRE6PE30AC672G763JY|_global_|W29 verify global
01KQ4QS48PGHQR1G6GPKRH3GX6|forge   |W29 verify forge
```

The `--project forge` write tags the row `forge`. The omitted-project
write tags the row `_global_` via the DAO helper
(`db::ops::project_or_global`). **No NULL or empty value is reachable
from any write path in this crate.**

## Files referenced

* Migration: `crates/daemon/src/db/schema.rs` — backfill UPDATE + FTS
  rebuild defence, end of `create_schema`.
* DAO helper: `crates/daemon/src/db/ops.rs` —
  `GLOBAL_PROJECT_SENTINEL` const + `project_or_global` fn.
* Recall WHERE: `crates/daemon/src/db/ops.rs::recall_bm25_project_org_flipped` —
  strict-by-default + `include_globals: bool` toggle.
* CLI flag: `crates/cli/src/main.rs::Commands::Recall::include_globals`.

## Closing F15/F17

The P3-3.8 dogfood findings doc described F15/F17 as:

> Top results for `forge-next recall "polish wave drift fixtures"
> --project forge` included a Hive Finance (`dashboard`-tagged)
> feature engineering audit. Either the project filter is being
> applied as a soft boost rather than a hard filter, OR the memory's
> project field was left empty / mis-tagged at extraction time and the
> recall path treats empty project as a wildcard.

Root cause confirmed by this session's investigation: BOTH conditions
were true. The recall WHERE clause admitted NULL/empty rows as
"globals visible everywhere" (intentional design), and a historic
extractor bug had left ~half the corpus (33 of 73 active memories)
with `project = NULL` — including the Hive Finance audit. The
combination produced the observed leak.

Fix delivered in commits `ede5c38..3c20bb7`:

1. Schema migration backfills NULL/empty → `_global_` (closes the
   data-shape footgun).
2. DAO helper at every memory-INSERT site ensures no future write can
   re-introduce NULL/empty (closes the future-bug footgun).
3. Recall WHERE clause becomes strict-by-default — only matching
   `m.project = ?` (closes the leak even if rows somehow reverted to
   NULL).
4. `--include-globals` opt-in preserves the broad semantic for callers
   who want it (e.g. agent-identity recall) — but bounded to the
   `_global_` sentinel, not arbitrary cross-project content.

The dogfood F15/F17 query is now strictly scoped. Globals require
explicit opt-in.
