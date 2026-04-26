# Dogfood-fixes plan — close all 23 findings before P3-4 release

**Status:** ACTIVE — 2026-04-26.
**Mode:** Autonomous, authorized by user 2026-04-26.
**Predecessor plan:** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (closed at HEAD `44c9094`).
**Findings doc:** `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md` (23 findings, 3 HIGH / 7 MED / 11 LOW / 2 OK).
**Goal:** close every actionable finding (21 of 23 — the 2 WORKS-AS-EXPECTED don't need fixes) before opening P3-4 v0.6.0 release.

## Locked decisions (user-confirmed 2026-04-26)

1. **Fix all findings** — not just the 3 HIGH. User wants every actionable finding closed.
2. **Plan and track first** — discrete tasks per fix, then implement next session.
3. **Compact-survivable** — this plan + HANDOFF + auto-memory entries are the resumable state.

## Phase ordering rationale

1. **P3-3.9 Pre-GA HIGH** first — true ship-blockers per dogfood report; estimated 2-3h.
2. **P3-3.10 Quick MED/LOW** second — bundles cheap wins; ~3-4h.
3. **P3-3.11 Investigation MED/LOW** third — deeper, may surface bigger issues; ~6-8h.
4. **P3-4 release** last — once all findings closed.

If P3-3.11 surfaces an architectural issue that needs deferral to v0.6.1+, halt
and brief; otherwise plow through.

## Halt-and-ask triggers

* End of P3-3.9 (3 HIGH fixes): halt for user sign-off before opening P3-3.10.
* End of P3-3.10 (quick fixes): halt for user sign-off before opening P3-3.11.
* P3-3.11 W29 investigation: if F15/F17 (cross-project recall) reveals a wider
  architectural drift, halt + brief.
* P3-3.11 W30 design decision: F16 identity-per-(agent, project) is potentially
  a schema change — halt + brief before implementing.
* End of P3-3.11: halt for sign-off before P3-4.

---

## P3-3.9 — Pre-GA HIGH fixes (4 commits, ~2-3 hours)

### W20 — F4: LD_LIBRARY_PATH propagation on daemon auto-spawn (1 commit)

**Source:** dogfood F4. Daemon spawned by CLI dies on
`error while loading shared libraries: libonnxruntime.so.1: cannot open shared object file`
because the spawn site doesn't propagate the env from `.cargo/config.toml`.

**Files to touch:**
- `crates/cli/src/daemon_spawn.rs` (or wherever the spawn site lives) — populate
  `LD_LIBRARY_PATH` for the spawned process. Locate via:
  `grep -rn "spawn\|Command::new.*forge-daemon" crates/cli/src/`.
- Optionally `build.rs` of `crates/daemon` — add `rpath` linker flag so the
  binary finds the lib via embedded RUNPATH. (More elegant; eliminates the
  env dependency entirely.)

**Acceptance:**
- Reproducer: `forge-next restart` → `forge-next health` works without
  manual `LD_LIBRARY_PATH` export.
- Daemon log shows no libonnxruntime error.
- macOS path: ditto for `DYLD_LIBRARY_PATH` (per existing CLAUDE.md
  multi-platform setup).

### W21 — F11+F13: `forge-next send --from <session_id>` flag (1 commit)

**Source:** dogfood F11+F13. `forge-next send` defaults `from_session` to
`"api"` regardless of `FORGE_SESSION_ID` env. Agent-team intercommunication
needs caller identity to round-trip correctly.

**Files to touch:**
- `crates/cli/src/main.rs` — add `--from <SESSION_ID>` flag to the `Send`
  subcommand. Same for `respond`.
- `crates/cli/src/commands/send.rs` (if exists) — pipe through.
- `crates/core/src/protocol/request.rs` — verify `SendMessage` already
  accepts `from_session` (it does — see `sessions.rs:363`).
- Default behavior: if `--from` not passed, fall back to
  `env::var("FORGE_SESSION_ID")`. If neither, fall back to `"api"` with a
  warn log so the existing CLI scripts don't silently break.

**Acceptance:**
- `forge-next send --from <ses> --to <ses2> ...` produces a row with
  `from_session=<ses>`.
- `FORGE_SESSION_ID=<ses> forge-next send --to <ses2> ...` produces a row
  with `from_session=<ses>`.
- Without either: `from_session=api` + a one-time warning to stderr.

### W22 — F23: `force-index` runs async (1 commit)

**Source:** dogfood F23. Synchronous force-index blocks the daemon writer
loop for 30s+ — observed `team stop` and `cleanup-sessions` timing out
right after.

**Files to touch:**
- `crates/daemon/src/handlers/index.rs` (or wherever `Request::ForceIndex`
  routes) — push the actual indexing work to a `tokio::spawn` task and
  return immediately with a `started_at` ULID + `progress_endpoint`.
- `crates/cli/src/main.rs` — `force-index` accepts `--wait` flag for
  legacy-blocking behavior; default is `--background`.
- New `Request::IndexProgress { started_at_ulid }` for polling.

**Acceptance:**
- `forge-next force-index` returns within 1s with a started_at id.
- Subsequent `forge-next health` / writes are not blocked.
- `forge-next force-index --wait` retains the prior synchronous behavior
  for scripts that depend on it.

### W23 — P3-3.9 close (review + HANDOFF)

Adversarial review on the 3 fix commits + HANDOFF rewrite + halt for
user sign-off before P3-3.10.

---

## P3-3.10 — Quick MED/LOW fixes (5 commits, ~3-4 hours)

### W24 — CLI argument-style cosmetics (F5, F10, F19) (1 commit)

**Source:** F5 (`identity show` doesn't exist), F10 (`message-read --id`
should accept positional), F19 (`blast-radius <path>` should accept
positional).

**Approach:** add positional-or-flag clap variants. Keep `--id` / `--file`
as compatibility aliases.

**Acceptance:** all 3 commands accept both forms.

### W25 — Daemon-spawn polish (F1, F2, F3) (1 commit)

- F1: `doctor` reports daemon version from binary on disk if running
  daemon's reported version differs (with a `(stale daemon — restart for
  updates)` note).
- F2: hooks.json warning downgraded to info-level when running outside
  a plugin install (detect via absence of `~/.claude/plugins/forge/`).
- F3: socket-bind cold-start timeout 3s → 10s with exponential backoff.

**Files to touch:**
- `crates/cli/src/commands/doctor.rs` (F1)
- `crates/cli/src/commands/doctor.rs` or daemon health-check (F2)
- `crates/cli/src/daemon_spawn.rs` or socket-connect (F3)

**Acceptance:** doctor prints clear version comparison; no hooks.json
warning when running in-tree without a plugin install; cold-start succeeds
within the new 10s window across 5 consecutive restart-then-call attempts.

### W26 — Team primitives polish (F6, F7, F8, F9) (1 commit)

- F6: `team run --name X` reuses an existing team named X instead of
  failing UNIQUE constraint.
- F7: `team stop --name X` reports `(team had no spawned agents)` when
  retired count == 0.
- F8: `team run --templates ... --project <P>` flag — propagate to
  spawned agents' session.project.
- F9: agent template name written to agent.role on spawn so `team
  members` shows `<role>` not `?`.

**Files to touch:**
- `crates/daemon/src/handlers/team.rs` (or sessions.rs team-run path)
- `crates/cli/src/commands/team.rs`

**Acceptance:** create-then-run flow works idempotently; `team members`
shows roles; spawned agents pick up project scope from the run command.

### W27 — Message-read ULID lookup (F12, F14) (1 commit)

**Source:** F12+F14. `message-read --id <full-ULID>` returns
`message not found` even when `messages --session ...` lists the same
message. Likely lookup uses different ID column or there's a truncation
path bug.

**Files to touch:**
- `crates/daemon/src/sessions.rs` (read_message function — `WHERE id = ?1`
  vs `WHERE id LIKE ?1 || '%'`).
- `crates/cli/src/commands/message_read.rs`.

**Investigation:** why does the listed ID prefix-match but full-ID lookup
fail? Could be ID column has whitespace or extra hyphens; could be the
`messages` listing displays a different ID than the storage column.

**Acceptance:** full-ULID and prefix lookups both work; column inconsistency
fixed at the source.

### W28 — P3-3.10 close (review + HANDOFF)

Adversarial review on the 4 fix commits + HANDOFF rewrite + halt for
user sign-off before P3-3.11.

---

## P3-3.11 — Investigation MED/LOW fixes (6 commits, ~6-8 hours)

### W29 — F15/F17: cross-project recall scoping investigation + fix (1 commit)

**Source:** dogfood F15/F17. Even with `--project forge`, recall returned
a `dashboard`-tagged feature engineering audit. Either:
1. Project filter is applied as a soft boost rather than hard filter.
2. Memory was extracted with wrong project field.
3. Cross-project recall via `null` project as wildcard.

**Investigation steps:**
1. `EXPLAIN QUERY PLAN` for the recall SQL with project filter.
2. Inspect `recall_bm25_project` in `crates/daemon/src/server/handler.rs`
   or wherever recall lives — confirm WHERE clause shape.
3. Sample 10 memories with `project='forge'` vs the suspect dashboard
   memory — what is its actual project field?
4. Dogfood the forge-isolation bench (which has 1.0 D1 cross_project_precision)
   — confirm the bench-path SQL matches the production-path SQL.

**Likely fix:** tighten the production recall WHERE clause to match the
bench's; add a regression test asserting cross-project precision in
production code path.

**Acceptance:** `forge-next recall <q> --project forge --limit 10`
returns ZERO `dashboard`-project memories.

### W30 — F16: identity per-(agent, project) decision + implementation (1 commit OR halt-and-brief)

**Source:** dogfood F16. 41 facets visible in `identity list` — many are
hive-finance / dashboard topics, not relevant to the forge repo.

**Investigation:**
1. Read the identity facet schema (Ahankara). Is project field present?
2. If present but unused: just add a project filter to `identity list`
   + `compile_context`.
3. If absent: schema change needed → halt-and-brief, defer to v0.6.1.

**Acceptance:** `identity list --project forge` returns only
forge-relevant facets; `compile_context` injects only project-scoped
identity.

### W31 — F18: contradiction false-positives (1 commit)

**Source:** F18. Contradiction detector fires on chronological session-
summary memories ("Session 17" vs "Session 16" both "complete").

**Investigation:**
1. Read Phase 9b `detect_content_contradictions` in
   `crates/daemon/src/workers/consolidator.rs`.
2. The Jaccard threshold is too loose for boilerplate session summaries.
3. Also session summaries probably have valence=neutral so Phase 9a
   shouldn't flag them — but the example shows neutral pairs flagged.

**Likely fix:** add a content-similarity floor (e.g., title Jaccard ≥ 0.7
+ content Jaccard < 0.3 — current values may be too loose) AND filter out
neutral-valence memories from Phase 9a (only opposite-valence pairs are
real contradictions).

**Acceptance:** consolidator pass produces 0 contradictions on a corpus
of 5 chronological session summaries with shared "complete" boilerplate.

### W32 — F20/F22: indexer lag investigation + fix (1 commit)

**Source:** F20+F22. `find-symbol audit_dedup` returned no symbols even
though the function has been at HEAD for hours; `blast-radius --file
crates/daemon/src/bench/forge_consolidation.rs` says file not in code
graph.

**Investigation:**
1. Daemon log: any indexer errors related to `forge_consolidation.rs`?
2. Indexer subscription path — does it pick up file changes via watcher
   or only on `force-index`?
3. The `[watcher] detected: ...` lines in the daemon log show many
   `*.jsonl` files but no `*.rs` files — indexer scope may be limited
   to JSONL transcripts.

**Likely fix:** add `*.rs` (and other code globs) to the watcher's pattern
list; ensure the indexer's queue isn't restricted by file extension.

**Acceptance:** `find-symbol audit_dedup` returns the file:line; a
freshly-edited file appears in the code graph within 30 seconds.

### W33 — F21: force-index error UX (1 commit)

**Source:** F21. `force-index` returning `daemon response timed out (30s)`
is unclear about whether the indexer is still running.

**Approach:** with W22's async refactor, `force-index` returns immediately
with a started_at id. The error UX problem dissolves — the timeout
shouldn't fire at all because the call returns in <1s.

**This wave converges with W22**; after W22 lands, verify F21 is closed
and remove if redundant.

### W34 — P3-3.11 close (review + HANDOFF + halt for P3-4)

Adversarial review on the 4-5 fix commits + HANDOFF rewrite + halt for
user sign-off before P3-4 release.

---

## P3-4 — Release & distribution (after all dogfood fixes close)

Per Plan A `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`
§"Phase P3-4". 7 waves:

| Wave | Scope | Auto / User |
|------|-------|-------------|
| W1 | Multi-OS dogfood final sweep | Auto (Linux); user (macOS) |
| W2 | Bench-fast required-gate flip (T17) | Auto if 14 green master runs |
| W3 | v0.6.0 version bump (rc.3 → 0.6.0) | Auto |
| W4 | GitHub release artifacts + notes | Auto if `gh` auth |
| W5 | Marketplace submission bundle | Auto preparation; user submits |
| W6 | Branch protection rules | Auto preparation; user applies |
| W7 | Final HANDOFF + close-out | Auto |

---

## Total scope

| Phase | Commits | Time est. | Halt point |
|-------|---------|-----------|------------|
| P3-3.9 (Pre-GA HIGH) | 4 | ~3h | end of W23 |
| P3-3.10 (Quick MED/LOW) | 5 | ~3-4h | end of W28 |
| P3-3.11 (Investigation) | 6 | ~6-8h | end of W34 + per-wave halts |
| **Subtotal dogfood-fixes** | **15** | **~12-15h** | 4-5 halt points |
| P3-4 release | 7 | varies | per-wave |

Total post-P3-3.8: ~22 commits + P3-4 to GA.

## Per-wave standard procedure (unchanged)

1. Verify clean working tree.
2. TDD-first if behavior change.
3. fmt + clippy + test + spans.
4. Commit with project conventions.
5. Adversarial review (one general-purpose agent per fix-cluster).
6. Address BLOCKER+HIGH+actionable-MED in fix-wave.
7. LOWs/non-actionable into per-phase backlog with rationale.
8. TaskUpdate.
9. Dogfood briefly when behavior-change.

## Out of scope (explicit non-goals)

- Anything beyond the 23 dogfood findings + Plan A's P3-4 (no scope creep).
- Per-tenant Prometheus labels (already deferred).
- longmemeval / locomo re-run (already deferred).
- New benches (forge-coding, forge-perception, etc.) — defer to v0.7.

## Memory index

This plan + HANDOFF + auto-memory entries form recoverable state.
Re-reads after `/compact` follow:
1. HANDOFF.md
2. This plan-doc
3. Findings doc: `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md`
4. Predecessor plans (Plan A + polish-wave plan) as reference
