# Handoff — P3-4 W1.3 closed: review + 3-commit fix-wave landed — 2026-04-26

**Public HEAD:** `13ed0c8` (fmt cleanup across fix-wave).
**Working tree:** clean.
**Version:** `v0.6.0-rc.3` (no bump until iteration phase closes — release-stack deferred per user direction 2026-04-26).
**Plan A (P3-1..P3-3 closed; P3-4 reframed):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`.
**Plan C (closed):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md`.
**Halt:** end-of-fix-wave HANDOFF, mid-stream of P3-4 W1 dogfood loop. Resume at #164 (W31 contradiction dogfood).

## Reframing of P3-4 (locked 2026-04-26)

User direction: **the release stack is deferred to project end.** Multi-OS verify, version bump, GitHub release, marketplace bundle, branch protection — none of that happens until every dogfood-identified issue is resolved on Linux. macOS, Docker for Linux, GitHub Actions billing, marketplace are explicitly OUT of scope until then.

P3-4 is a single iteration loop: **dogfood every Forge feature thoroughly on Linux → identify every issue → resolve each one → re-dogfood → THEN open the release stack.** Sub-tasks #163-#180 break the loop into 18 discrete surfaces. Task #101 is the release-stack umbrella, marked **DEFERRED** until #180 closes.

## State in one paragraph

**P3-4 W1.3 (review + fix-wave) closed at HEAD `13ed0c8` (5 commits this wave + 1 fmt cleanup).** The W1.3 adversarial review (`6ed8d09`, verdict `lockable-with-fixes`) flagged 3 HIGH + 3 MED + 10 LOW. The fix-wave addressed every HIGH and actionable MED across 3 commits: fw1 (`2f4ccda`) closed HIGH-1 (run_clustering by-name fallback) + HIGH-2 (wired `derive_project_name` at `index_directory_sync` entry); fw2 (`a7cb1a0`) closed HIGH-3 (companion DELETE for pre-W1.2 foreign-root pollution across 13 FHS-roots) + MED-1 (regression test mirroring W29/W30) + MED-3 (RTRIM trailing slashes); fw3 (`848c164`) closed MED-2 (swept skills/agents to thread `--project` flag with rationale across 5 SKILL.md files + 1 agent file). **The MED-1 test surfaced a CRITICAL latent bug**: SQLite has no built-in `REVERSE()`, so the original c1 migration's SUBSTR/REVERSE/INSTR SQL silently no-op'd on every legacy DB — the live `forge|188` baseline only looked clean because the DB had been wiped fresh. fw2 replaces with the standard SQLite basename idiom `REPLACE(p, RTRIM(p, REPLACE(p, '/', '')), '')` and retroactively unblocks the c1 promise. New auto-memory pinned (`feedback_sqlite_no_reverse_silent_migration_failure.md`). All 1533 daemon-lib tests pass; clippy 0 warnings; harness-sync OK; protocol-hash unchanged (`f8c1d4f04563…`); review YAML updated with `fixed_by` SHAs.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 13ed0c8 → 848c164 → a7cb1a0 → 2f4ccda → 6ed8d09 → af43257 → ef99156
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 155 + 107
bash scripts/check-protocol-hash.sh                # f8c1d4f04563…
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh             # 25 valid (was 24)
cargo fmt --all --check                            # clean
cargo clippy -p forge-daemon -p forge-core -p forge-cli -p forge-hud -- -W clippy::all -D warnings  # 0 warnings

# Verify daemon at fresh head; live DB still clean
pgrep -af forge-daemon
sqlite3 ~/.forge/forge.db "SELECT project, COUNT(*) FROM code_file GROUP BY project"
forge-next health
forge-next doctor

# Resume at task #164 (W31 contradiction dogfood) — #163 is closed.
cat docs/benchmarks/results/2026-04-26-p3-4-w1-dogfood-matrix.md
```

## P3-4 W1.3 review + fix-wave close summary

### What landed (6 commits)

| SHA | Wave | Scope |
|-----|------|-------|
| `6ed8d09` | W1.3 review | Adversarial review YAML + transcript at `docs/superpowers/reviews/2026-04-26-p3-4-w1-2-i7-impl.{yaml,transcript.md}`. Verdict: `lockable-with-fixes`, 3 HIGH + 3 MED + 10 LOW. |
| `2f4ccda` | W1.3 fw1 | HIGH-1 (`run_clustering` accepts NAME via new `db::ops::get_reality_by_name` with by-path-then-by-name fallback) + HIGH-2 (`db::ops::derive_project_name` wired at `index_directory_sync` entry — monorepo subdir invocation now inherits registered ancestor reality NAME). 2 regression tests pinned. |
| `a7cb1a0` | W1.3 fw2 | HIGH-3 (companion DELETE for pre-W1.2 foreign-root pollution: 13 FHS roots × code_file/code_symbol/edge cascades, doubly-anchored on project=basename(root) AND path LIKE root-prefix) + MED-1 (regression test mirroring W29/W30) + MED-3 (RTRIM trailing slashes before basename). **Replaces the original c1 SUBSTR/REVERSE/INSTR with the standard SQLite basename idiom — original SQL was silently no-op'ing because SQLite has no `REVERSE()`.** 2 new schema tests. |
| `848c164` | W1.3 fw3 | MED-2 — sweep harness layer to thread `--project` flag with rationale: skills/forge-feature, forge-tdd, forge-debug, forge-verify, forge-think, agents/forge-planner. |
| `13ed0c8` | W1.3 fw chore | rustfmt cleanup across fw1 + fw2 (mechanical, no behavior change). |

### Review findings — final state

| ID | Sev | Status | fixed_by |
|----|-----|--------|----------|
| HIGH-1 | run_clustering NAME mismatch | resolved | 2f4ccda |
| HIGH-2 | derive_project_name dead code | resolved | 2f4ccda |
| HIGH-3 | foreign-root pollution carryover | resolved | a7cb1a0 |
| MED-1 | no migration regression test | resolved | a7cb1a0 (caught REVERSE() bug as bonus) |
| MED-2 | harness layer un-propagated --project | resolved | 848c164 |
| MED-3 | trailing-slash basename SUBSTR corner | resolved | a7cb1a0 |
| LOW-1..LOW-10 | various cosmetic / strategic followups | open | (deferred to backlog, see below) |

### Test counts (final state at HEAD `13ed0c8`)

* `cargo fmt --all --check` — clean
* `cargo clippy -p forge-daemon -p forge-core -p forge-cli -p forge-hud -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --tests` — clean
* `cargo test -p forge-daemon --lib` — **1533 passed** (was 1528 pre-fix-wave; +5 tests across fw1+fw2)
* `cargo test -p forge-core --lib` — 109 passed
* `cargo test -p forge-cli` — 92 passed
* `cargo test -p forge-hud` — 3 passed
* All 5 `bash scripts/check-*.sh` gates green

## Issue ledger (running, scoped to W1 dogfood)

| ID | Sev | Title | Status |
|----|----:|-------|--------|
| I-1 | BLOCKER | fastembed 5.13.3 → ort rc.12 → ONNX RT API v24 mismatch | ✓ closed (`50ab231`) |
| I-2 | LOW | first force-index post-restart 5 s cold | observed; warm 9 ms |
| I-3 | LOW | "database is locked" warns during force-index dispatch | expected SQLite WAL contention |
| I-4 | LOW | `doctor` shows stale vergen git_sha after edits | resolved by rebuild |
| I-5 | LOW | mis-tagged hive-platform memory in earlier DB | irrelevant after wipe |
| I-6 | LOW | `forge-next --help` lists 50+ commands flat (no grouping) | cosmetic; defer |
| I-7 | HIGH | code-graph cross-project leakage | ✓ closed end-to-end (W1.2 c1+c2+c3 + W1.3 fw1+fw2+fw3) |
| I-8 | HIGH | c1 migration silently no-op'd (SQLite has no REVERSE()) | ✓ closed (W1.3 fw2 — `a7cb1a0`) |

Future issues: append to this ledger and to `docs/benchmarks/results/2026-04-26-p3-4-w1-dogfood-matrix.md`.

## Deferred backlog from W1.3 review (LOW findings)

All 10 LOW findings are deferred per Plan A §6 (LOWs / non-actionable MEDs go to backlog with rationale). They're listed in the review YAML at `docs/superpowers/reviews/2026-04-26-p3-4-w1-2-i7-impl.yaml` with full rationale; carry-forward summary:

* **LOW-1** Depth-floor `≥4 slashes` heuristic in `find_project_dir` is host-shape coupled — works for `/mnt/colab-disk/DurgaSaiK/forge` (4 slashes) but rejects `/srv/foo` (2 slashes). Strategic fix: marker-file detection (Cargo.toml/.git/package.json). Track for v0.6.1+ via an env var override `FORGE_INDEXER_MIN_PATH_DEPTH`.
* **LOW-2** FORGE_PROJECT env path skips depth-floor — `FORGE_PROJECT=/mnt` bypasses the c3 guard. User-explicit opt-in lowers severity but the same shape is exposed.
* **LOW-3** BlastRadius handler has a dead `(bool, &str)` tuple `scope_msg` populated but never read; explicit `let _ = scope_msg;` is anti-signal. Cosmetic.
* **LOW-4** Empty-string `--project ""` accepted by CLI silently fails-closed against `cf.project = ''` post-c1. Same shape as W29 review MED-2; defer (consistent with that pattern).
* **LOW-5** `code_search` 3 new emit sites in c2 use `"path":` JSON key while CLI reads `"file_path"` — pre-existing key drift, widened by c2 not introduced. Defer (cosmetic; the typed-roundtrip find-symbol path still works).
* **LOW-6** New `idx_code_file_project` is single-column; composite `(project, path)` would be more JOIN-friendly. Performance-not-correctness; ANALYZE remediates.
* **LOW-7** Cargo.toml fastembed-pin comment names `scripts/setup-dev-env.sh`, but the reverse coupling has no cross-reference. Mechanical doc fix.
* **LOW-8** `derive_project_name` hardcodes `"default"` org scope — multi-org deployments would silently miss. Preventive only (single-org Forge today).
* **LOW-9** Regression test `p3_4_w1_2_find_project_dir_rejects_shallow_filesystem_roots` exercises depth-floor math but does NOT reproduce the actual bug input (transcript dir name with un-decodable underscores). Strategic test extension.
* **LOW-10** BlastRadius cluster-expansion path explicitly NOT scoped — `--project forge` blast-radius can still emit foreign callers via `cluster_files` since edges aren't project-tagged. HIGH-3 cleanup makes this surface auto-close on a polluted-then-renamed DB; v0.6.1+ if real-world surfaces it.

These join the existing P3-4 deferred set (W23 carry-forwards #173, W28 LOW/NIT sweep #174, etc.).

## TaskList structure (post-W1.3-fix-wave)

**Active iteration loop:**

| Task | Subject | Status |
|------|---------|--------|
| #153 | P3-4 W1: iterative Linux dogfood — identify + resolve every issue (umbrella) | in_progress |
| #163 | W1.3 — adversarial review on W1.1 + W1.2 (mandatory per Plan A §6) | **completed** ✓ |
| #181 | W1.3 fix-wave — address 3 HIGH + 3 MED from review | **completed** ✓ |
| #164 | W1.4 §7 — W31 contradiction detection dogfood | **next** |
| #165 | W1.5 §8 — W26 team primitives dogfood | pending |
| #166 | W1.6 §13 — Manas 8-layer verification | pending |
| #167 | W1.7 §14 — observability (observe + /metrics + /inspect) | pending |
| #168 | W1.8 §15 — plugin surface (hooks + skills + agents) | pending |
| #169 | W1.9 §16 — HUD statusline render | pending |
| #170 | W1.10 §17 — Grafana dashboards (critical lens) | pending |
| #171 | W1.11 §18 — Prometheus families + value sanity | pending |
| #172 | W1.12 §19 — bench harness 4-bench end-to-end | pending |
| #173 | W1.13 — re-promote W23+W28 deferred HIGHs (3 items) | pending |
| #174 | W1.14 — backlog sweep (W28 LOWs/NITs + W1.3 LOWs + earlier) | pending |
| #175 | W1.15 — date-sensitive bench test fix | pending |
| #176 | W1.16 — sync surfaces dogfood | pending |
| #177 | W1.17 — healing system end-to-end | pending |
| #178 | W1.18 — guardrails (check / post-edit / pre-bash / post-bash) | pending |
| #179 | W1.19 — config + scope resolver (precedence) | pending |
| #180 | W1.20 — close iteration phase + carry-forward to release stack | pending |

**Deferred (do not touch until #180 closes):**

| Task | Subject | Reason |
|------|---------|--------|
| #101 | P3-4 release v0.6.0 — multi-OS + tag + gh release + marketplace + branch protection | DEFERRED to project end per user direction 2026-04-26 |

**Per-wave standard procedure (unchanged from Plan A):**

1. Verify clean working tree.
2. TDD-first if behavior change.
3. fmt + clippy + tests + spans gates green.
4. Commit with project-conventions message.
5. Adversarial review (general-purpose agent, terse, ≤600-word verdict).
6. Address every BLOCKER + HIGH + actionable MED in fix-wave.
7. LOWs / non-actionable MEDs → backlog with rationale.
8. TaskUpdate.
9. Dogfood briefly when behavior-change.

## Daemon state at handoff

* Live daemon running at PID `805762` (release build at HEAD `ef99156` via `with-ort.sh`) — **PRE-fix-wave** code; new release build at HEAD `13ed0c8` is fresh on disk after the build that ran in this session. Next session may opt to keep the running daemon (the fix-wave changes are migration-safe + idempotent — re-running create_schema is a no-op on the wiped DB) or stop+respawn for a clean test.
* `~/.forge/forge.db` — 12 MB; 188 forge-tagged code files; embedder ready (fastembed pin verified clean)
* Backups: `forge.db.pre-W1.2-wipe-20260426-145000.bak` (219 MB pre-wipe baseline) plus older P3-3.11-era backups
* Daemon log at `~/.forge/daemon.log` and `/tmp/dogfood-daemon-5.log`

It is fine to leave the daemon running across the compact boundary. Next session's first actions can either keep it or stop+respawn.

## Cumulative deferred backlog (re-promotion candidates per "future ready")

* **W23 HIGH-1** — `tokio::task::spawn_blocking` JoinHandle dropped (panics swallowed, SIGTERM split-brain risk). Tracked by #173.
* **W23 HIGH-2** — `Request::SessionRespond` no `from_session`; no `forge-next respond` CLI surface. Tracked by #173.
* **W28 HIGH-1** — `read_message_by_id_or_prefix` unscoped (no caller_session filter). Tracked by #173.
* **W28 MED-2** — git-sha drift detection. Tracked by #174.
* **W28 LOW-2..LOW-10 + NIT-1..NIT-3** — cosmetic backlog. Tracked by #174.
* **W23 MED-3 + MED-4** — `(0,0)` background heuristic + PRAGMA/busy_timeout consistency. Tracked by #174.
* **W1.3 LOW-1..LOW-10** — per W1.3 review backlog rationale above. Tracked by #174.
* **W29 nice-to-haves** — bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail; optional config gate `memory.require_project = true`. Audit during #167/#171/#172.
* **W30 nice-to-haves** — extractor `tracing::warn!` when identity-project resolution falls through. Audit during #167.
* **W31 nice-to-haves** — drift fixture for contradiction surface. Audit during #164.
* **W32 nice-to-haves** — notify::Watcher event-driven detection (replace stat-walk on very large trees). v0.6.1+.
* **Earlier deferrals unchanged**: longmemeval / locomo re-run, SIGTERM/SIGINT chaos drill modes, criterion latency benchmarks, Prometheus bench composite gauge, multi-window regression baseline, manual-override label, P3-2 W1 trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline panel.

## Halt-and-ask map

1. **NEXT SESSION** — start at #164 (W31 contradiction dogfood). The W1.3 review + fix-wave is closed. Continue the dogfood loop sequentially (#164 → #165 → ... → #180).
2. **Per Plan A §"Halt-and-ask points"**: anything returning `not-lockable` from adversarial review halts; any non-clean working-tree across a wave boundary halts.

## One-line summary

**P3-4 W1.3 closed at HEAD `13ed0c8` (6 commits): adversarial review (verdict `lockable-with-fixes`) + 3-commit fix-wave (HIGH-1 + HIGH-2 → fw1; HIGH-3 + MED-1 + MED-3 → fw2; MED-2 → fw3) + fmt cleanup. Bonus: fw2's regression test caught a CRITICAL latent bug — the c1 migration's REVERSE() SQL silently no-op'd on every legacy DB because SQLite has no REVERSE function. Fixed via standard SQLite basename idiom. Resume at #164 (W31 contradiction dogfood).**
