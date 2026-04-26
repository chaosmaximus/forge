# Handoff — P3-4 W1 iteration continues — 22-task drain queued — 2026-04-26

**Public HEAD:** `848f140` (W1.20 close commit; iteration phase originally signalled closed but the user re-opened the scope to tackle every deferred item before the release stack).
**Working tree:** clean.
**Version:** `v0.6.0-rc.3` (release-stack still DEFERRED — version bump happens after #182-#203 close, then #101 opens).
**Plan A:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Halt:** none. Resume directly at #182.

## Reframing of P3-4 W1 (locked 2026-04-26 — second pass)

User direction this session: **"tackle every one of them in the next session"** — the iteration phase does NOT close at #180. The 31 originally-deferred items have been broken out into 22 trackable tasks #182-#203 (#181 fix-wave already closed). Each is commit-able and individually reviewable per Plan A §6. After ALL of them close, then #101 release stack opens.

## State in one paragraph

**P3-4 W1 first pass (16 commits) closed at HEAD `848f140`.** Adversarial review on W1.1+W1.2 + 3-commit fix-wave (3 HIGH + 3 MED resolved); 13 dogfood surfaces verified end-to-end (#164-#172, #176-#179); 3 carried HIGHs closed (W23+W28 → #173); ad-hoc backlog sweep (#174+#175). Bonus: surfaced + fixed CRITICAL latent c1 migration bug (SQLite has no `REVERSE()` → original SUBSTR/REVERSE/INSTR silently no-op'd on every legacy DB; replaced with `REPLACE/RTRIM/REPLACE` basename idiom). All 1535 daemon-lib tests pass; clippy 0 warnings; all 5 CI gate scripts green; 25 reviews valid. **Second pass (22-task drain) queued at #182-#203 — all defensible-deferral items now first-class tasks instead of a backlog footnote.**

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -5                               # HEAD 848f140
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 155 + 108
bash scripts/check-protocol-hash.sh                # 1b3dec55ffa4…
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh             # 25 valid

# Daemon may still be running; respawn if needed for live dogfood
pgrep -af forge-daemon
sqlite3 ~/.forge/forge.db "SELECT project, COUNT(*) FROM code_file GROUP BY project"
forge-next health

# Resume at #182. Tasks #182-#203 are the queue.
```

## The 22-task drain queue (#182 → #203)

Prioritization rule of thumb: strategic refactors first (LOW-1 marker-file replaces an empirical heuristic), then surface-specific LOWs grouped by file, then carried-MEDs, then nice-to-haves, then pre-iteration backlog re-eval.

### Tier 1 — strategic refactors

| Task | Subject |
|------|---------|
| #182 | W1.21 — LOW-1 marker-file detection in `find_project_dir` (replaces depth-floor heuristic). Touches indexer.rs + new env override `FORGE_INDEXER_MIN_PATH_DEPTH`. Regression test pins both `/srv/foo` (admit on marker) and `/mnt` (reject — no marker). |
| #190 | W1.29 — W23 HIGH-1 strategic: SIGTERM-graceful `JoinSet` coord drained by shutdown handler. Touches main.rs + writer.rs + events.rs + chaos test. |

### Tier 2 — focused fixes (1 commit each, ~30-60min)

| Task | Subject |
|------|---------|
| #183 | W1.22 — LOW-2 FORGE_PROJECT env path also applies depth-floor / marker check. |
| #184 | W1.23 — LOW-4 CLI rejects empty-string `--project ""` across find-symbol/code-search/blast-radius. |
| #185 | W1.24 — LOW-5 `code_search` JSON key rename `path → file_path` + contract test (matches `feedback_json_macro_silent_drift` memory). |
| #186 | W1.25 — LOW-6 composite `idx_code_file_project_path` index (or `ANALYZE` post-migration). |
| #187 | W1.26 — LOW-8 `derive_project_name` accepts `org_id: &str` param (multi-org preventive). |
| #188 | W1.27 — LOW-9 regression test for the actual underscore decode-fallback bug input (`dhruvishah_finexos_io`). |
| #189 | W1.28 — LOW-10 BlastRadius cluster-expansion accepts `project_filter: Option<&str>`. |

### Tier 3 — observability / UX cosmetic batch

| Task | Subject |
|------|---------|
| #194 | W1.33 — I-2+I-3 force-index cold latency (5s → ~9ms) + WAL "database is locked" warns. |
| #195 | W1.34 — I-6 `forge-next --help` grouping via `clap::next_help_heading`. |
| #196 | W1.35 — I-9 CLI `remember` exposes `--valence`/`--intensity` flags. |
| #197 | W1.36 — I-10 Phase 9b dedicated INFO log. |
| #198 | W1.37 — I-11 `forge-next observe` shape schema uniformity (common envelope). |
| #199 | W1.38 — I-12 `forge-bench` standalone telemetry warn quiet (downgrade or `--telemetry` flag). |

### Tier 4 — carried-forward MEDs

| Task | Subject |
|------|---------|
| #191 | W1.30 — W23 MED-3+MED-4: disposition `(0,0)` heuristic + PRAGMA/busy_timeout consistency across all `Connection::open` sites (related to I-3). |
| #192 | W1.31 — W28 MED-2: daemon-vs-CLI git_sha drift detection (closes I-4 cosmetic). |
| #193 | W1.32 — W28 LOW/NIT cosmetic sweep (W28-LOW-2..LOW-10 + W28-NIT-1..NIT-3). |

### Tier 5 — nice-to-haves (release-tail / v0.6.1+ candidates)

| Task | Subject |
|------|---------|
| #200 | W1.39 — W29/W30 nice-to-haves: bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail (W29+W30); optional `memory.require_project = true` config gate. |
| #201 | W1.40 — W31 drift fixture for contradiction false-positive surface (matches `feedback_ci_drift_fixture_pattern` memory). |
| #202 | W1.41 — W32 `notify::Watcher` event-driven freshness gate (replaces stat-walk on big monorepos). |

### Tier 6 — pre-iteration backlog re-evaluation

| Task | Subject |
|------|---------|
| #203 | W1.42 — walk 9 pre-iteration deferrals: 2A-4d.3 T17 (BLOCKED on GHA billing); longmemeval/locomo re-run; SIGTERM/SIGINT chaos drill modes; criterion latency benchmarks; Prometheus bench composite gauge; multi-window regression baseline; manual-override label; P3-2 W1 trace-handler behavioral test gap; per-tenant Prometheus labels; OTLP timeline panel. Decide fix-or-permanently-defer per item with rationale. |

## TaskList structure (post-second-pass-queue)

| | | |
|---|---|---|
| #153 | iteration umbrella (1st pass closed) | completed |
| #163 .. #181 | first-pass tasks | all completed |
| #182 .. #203 | **second-pass drain (22 tasks)** | **all pending** ← next-session queue |
| #101 | P3-4 release v0.6.0 stack | DEFERRED — opens after #203 closes |

**Per-task standard procedure (unchanged from Plan A §6):** verify clean tree → TDD-first if behavior change → fmt+clippy+tests green → commit → adversarial review (per behavior-change wave) → fix-wave for HIGH+MED → LOWs to backlog → TaskUpdate → dogfood briefly when feasible.

## Issue ledger (cumulative, post-first-pass)

| ID | Sev | Title | Status |
|----|----:|-------|--------|
| I-1 | BLOCKER | fastembed → ort → ONNX RT API v24 mismatch | ✓ closed (`50ab231`) |
| I-2 | LOW | force-index cold 5s | open → tracked by #194 |
| I-3 | LOW | "database is locked" warns | open → tracked by #194 |
| I-4 | LOW | doctor stale vergen git_sha | open → tracked by #192 |
| I-5 | LOW | mis-tagged hive-platform memory | closed (irrelevant after wipe) |
| I-6 | LOW | forge-next --help flat | open → tracked by #195 |
| I-7 | HIGH | code-graph cross-project leakage | ✓ closed (W1.2 c1+c2+c3 + W1.3 fw1+fw2+fw3) |
| I-8 | HIGH | c1 migration silently no-op'd (SQLite no REVERSE) | ✓ closed (`a7cb1a0`) |
| I-9 | LOW | CLI remember lacks --valence/--intensity | open → tracked by #196 |
| I-10 | LOW | Phase 9b no INFO log | open → tracked by #197 |
| I-11 | LOW | observe shape schema varies | open → tracked by #198 |
| I-12 | LOW | forge-bench standalone telemetry warn | open → tracked by #199 |
| I-13 | LOW | forge-bash-check substring match in argv | ✓ closed (`5d218ed` quote-strip) |

## Auto-memory state (cross-session)

Saved this session: `feedback_sqlite_no_reverse_silent_migration_failure.md` (SQLite REVERSE trap + standard basename idiom). Already-saved relevant: `feedback_decode_fallback_depth_floor.md` (informs #182), `feedback_dual_helper_basename_vs_reality.md` (informs #187), `feedback_release_stack_deferred.md` (informs locked rules), `feedback_ci_drift_fixture_pattern.md` (informs #201), `feedback_json_macro_silent_drift.md` (informs #185), and the broader Forge corpus.

## Halt-and-ask map for the post-iteration window

1. **NO halt.** Per user direction this session, drain `#182 → #203` continuously. Per Plan A §6 still applies — adversarial review per behavior-change wave; fix-wave for HIGH+MED.
2. **Halt only on:** non-clean working tree across a wave boundary; review verdict `not-lockable`; surprise architectural blocker that needs user input (e.g. SIGTERM-graceful coord touches an external contract).
3. **AFTER #203 closes:** halt for sign-off → open `#101` release stack (multi-OS verify + version bump → `0.6.0` + `gh release create` + marketplace bundle + branch protection). GHA billing block on chaosmaximus is the first release-stack halt-and-brief.

## One-line summary

**HEAD `848f140`. P3-4 W1 first pass closed (16 commits, 18 tasks). Second pass drain queued at #182-#203 (22 tasks: 2 strategic + 7 focused LOW fixes + 6 cosmetic + 3 carried MEDs + 3 nice-to-haves + 1 pre-iteration re-eval umbrella). Resume at #182. Release stack #101 stays DEFERRED until #203 closes.**
