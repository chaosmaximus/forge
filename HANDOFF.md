# Handoff — P3-4 W1 mid-iteration: I-1+I-7 closed; release stack deferred — 2026-04-26

**Public HEAD:** `ef99156` (W1.2 c3 depth guard + regression test).
**Working tree:** clean.
**Version:** `v0.6.0-rc.3` (no bump until iteration phase closes — release-stack deferred per user direction).
**Plan A (P3-1..P3-3 closed; P3-4 reframed):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`.
**Plan C (closed):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md`.
**Halt:** end-of-iteration HANDOFF, mid-stream of P3-4 W1 dogfood loop.

## Reframing of P3-4 (locked 2026-04-26)

User direction: **the release stack is deferred to project end.** Multi-OS verify, version bump, GitHub release, marketplace bundle, branch protection — none of that happens until every dogfood-identified issue is resolved on Linux. macOS, Docker for Linux, GitHub Actions billing, marketplace are explicitly OUT of scope until then.

P3-4 is now a single iteration loop:

1. **Dogfood every Forge feature thoroughly** on Linux.
2. **Identify every issue** (track in TaskList + dogfood-matrix doc).
3. **Resolve each one** (commit per fix, adversarial review per wave).
4. **Re-dogfood** until clean.
5. **Then** open the release stack (multi-OS / tag / gh release / marketplace / branch protection).

Sub-tasks #163-#180 break the loop into 18 discrete surfaces. Task #101 is the release-stack umbrella, marked **DEFERRED** until #180 closes.

## State in one paragraph

**P3-4 W0 + W1.1 + W1.2 closed at HEAD `ef99156` (5 commits this session).** W0 (`77b7ab2`) bumped the flaky timing test threshold (3000→10000 ms) — root-cause of 30 consecutive CI red runs since 2026-04-24 (CI billing block is unrelated and irrelevant to current scope). W1.1 (`50ab231`) pinned `fastembed = "=5.11.0"` to fix the silent ORT 2.0.0-rc.12 → ONNX Runtime API v24 mismatch panic on every fresh daemon spawn (I-1). W1.2 (`cbd043f`, `ea76e82`, `ef99156`) shipped the I-7 fix — code-graph per-project scoping mirroring the W29/W30 sentinel pattern: schema migration backfilling legacy PATH-tagged rows to NAME, indexer write-tagging via basename, protocol field on `FindSymbol`/`CodeSearch`/`BlastRadius`, CLI `--project` flag, **plus a depth-guard on `find_project_dir`'s decode-fallback** that prevents the indexer from rooting at `/mnt` when transcript path components contain underscores. Live-verified on a fresh wipe: only `forge|188` files indexed, zero foreign leakage (was `mnt|10005`). All 1528 daemon-lib tests pass; clippy clean; protocol-hash bumped (`d23de2ac97f3… → f8c1d4f04563…`).

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD ef99156
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 155 + 107
bash scripts/check-protocol-hash.sh                # f8c1d4f04563…
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh
cargo fmt --all --check                            # clean
cargo clippy -p forge-daemon -p forge-core -p forge-cli -p forge-hud -- -W clippy::all -D warnings  # 0 warnings

# Daemon at this HEAD; clean DB; only forge tagged
pgrep -af forge-daemon
sqlite3 ~/.forge/forge.db "SELECT project, COUNT(*) FROM code_file GROUP BY project"
forge-next health
forge-next doctor

# Read the dogfood matrix:
cat docs/benchmarks/results/2026-04-26-p3-4-w1-dogfood-matrix.md

# Resume at task #163 (adversarial review on W1.1+W1.2) — mandatory per Plan A §6.
```

## P3-4 W0 + W1.1 + W1.2 close summary

### What landed (5 commits)

| SHA | Wave | Scope |
|-----|------|-------|
| `77b7ab2` | P3-4 W0 | `test_daemon_state_new_is_fast` threshold 3000→10000 ms — fixes 30 consecutive CI red runs from cargo's parallel-test scheduler contention. |
| `50ab231` | P3-4 W1.1 | `fastembed = "=5.11.0"` pin — closes I-1 BLOCKER. ort 2.0.0-rc.12 (pulled by fastembed 5.12+) wants ONNX Runtime API v24 but `.tools/onnxruntime-linux-x64-1.23.0/` only ships v23; fresh daemon spawns panicked in tokio-rt-worker on every embedder init (silent — daemon survived but new memories never got embeddings). v0.6.1+ unpins when ONNX Runtime 1.24+ ships in `.tools/setup-dev-env.sh`. |
| `cbd043f` | P3-4 W1.2 c1 | Code-graph schema + indexer write-tagging. `code_file.project` default `''`→`'_global_'`; legacy PATH-tagged rows backfilled to basename via SQL; `idx_code_file_project` index added; `db::ops::derive_project_name` + `indexer::project_name_from_dir` helpers (basename-fast for hot paths, reality-table-aware for handler entry-points); `store_file` normalizes via `project_or_global`. |
| `ea76e82` | P3-4 W1.2 c2 | Protocol field + read-path filter + CLI flag. `Request::FindSymbol`/`CodeSearch`/`BlastRadius` gain `project: Option<String>` (`#[serde(default)]`); handler routes `JOIN code_file ... AND f.project = ?` when set; CLI gains `--project` flag on `find-symbol`, `code-search`, `blast-radius`. Protocol-hash bumped to `f8c1d4f04563…`. ~10 literal sites updated across tests + writer + handler + CLI. |
| `ef99156` | P3-4 W1.2 c3 | Depth-guard fix on `find_project_dir`'s decode-fallback. Pre-W1.2 the loop returned `/mnt` when underscore-bearing path components broke the dash↔slash decode; indexer then walked /mnt's whole subtree and pulled 10,005 foreign-user files into the code graph. Now requires ≥4 path-segment slashes before accepting a candidate. Regression test pinned. |

### Live-verification evidence (post-W1.2 c3)

```
$ sqlite3 ~/.forge/forge.db "SELECT project, COUNT(*) FROM code_file GROUP BY project"
forge|188

$ forge-next find-symbol audit_dedup --project forge
2 symbol(s) found:
  audit_dedup [function] .../forge_consolidation.rs:2063
  test_audit_dedup_smoke [function] .../forge_consolidation.rs:3781

$ forge-next find-symbol main --project hive-finance
No symbols found.

$ forge-next blast-radius crates/daemon/src/workers/indexer.rs --project forge
Callers: 6 (all forge paths, no foreign leakage)
```

W1.2 closes I-7 across all three surfaces (find-symbol, code-search, blast-radius) and prevents future leakage at the indexer source via the depth guard.

## Issue ledger (running, scoped to W1 dogfood)

| ID | Sev | Title | Status |
|----|----:|-------|--------|
| I-1 | BLOCKER | fastembed 5.13.3 → ort rc.12 → ONNX RT API v24 mismatch | ✓ closed (`50ab231`) |
| I-2 | LOW | first force-index post-restart 5 s cold | observed; warm 9 ms |
| I-3 | LOW | "database is locked" warns during force-index dispatch | expected SQLite WAL contention |
| I-4 | LOW | `doctor` shows stale vergen git_sha after edits | resolved by rebuild |
| I-5 | LOW | mis-tagged hive-platform memory in earlier DB | irrelevant after wipe |
| I-6 | LOW | `forge-next --help` lists 50+ commands flat (no grouping) | cosmetic; defer |
| I-7 | HIGH | code-graph cross-project leakage | ✓ closed (W1.2 c1+c2+c3) |

Future issues: append to this ledger and to `docs/benchmarks/results/2026-04-26-p3-4-w1-dogfood-matrix.md`.

## TaskList structure (post-restructure)

**Active iteration loop:**

| Task | Subject | Status |
|------|---------|--------|
| #153 | P3-4 W1: iterative Linux dogfood — identify + resolve every issue (umbrella) | in_progress |
| #163 | W1.3 — adversarial review on W1.1 + W1.2 (mandatory per Plan A §6) | pending |
| #164 | W1.4 §7 — W31 contradiction detection dogfood | pending |
| #165 | W1.5 §8 — W26 team primitives dogfood | pending |
| #166 | W1.6 §13 — Manas 8-layer verification | pending |
| #167 | W1.7 §14 — observability (observe + /metrics + /inspect) | pending |
| #168 | W1.8 §15 — plugin surface (hooks + skills + agents) | pending |
| #169 | W1.9 §16 — HUD statusline render | pending |
| #170 | W1.10 §17 — Grafana dashboards (critical lens) | pending |
| #171 | W1.11 §18 — Prometheus families + value sanity | pending |
| #172 | W1.12 §19 — bench harness 4-bench end-to-end | pending |
| #173 | W1.13 — re-promote W23+W28 deferred HIGHs (3 items) | pending |
| #174 | W1.14 — backlog sweep (W28 LOWs/NITs + earlier) | pending |
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

## Tests + verification (final state at HEAD `ef99156`)

* `cargo fmt --all --check` — clean
* `cargo clippy -p forge-daemon -p forge-core -p forge-cli -p forge-hud -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --tests` — clean
* `cargo test -p forge-daemon --lib` — **1528 passed**, 0 failed, 1 ignored (incl. updated `test_index_directory_sync_python` + new `p3_4_w1_2_find_project_dir_rejects_shallow_filesystem_roots`)
* `cargo test -p forge-core --lib` — 109 passed
* `cargo test -p forge-cli` — 92 passed
* `cargo test -p forge-hud` — 3 passed
* `bash scripts/check-harness-sync.sh` — OK (155 + 107)
* `bash scripts/check-review-artifacts.sh` — OK (24 valid, 0 blocking)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-protocol-hash.sh` — OK (`f8c1d4f04563…`)
* `bash scripts/ci/check_spans.sh` — OK

Pre-existing test failure (unchanged, tracked by #175):

* `workers::disposition::tests::test_step_for_bench_parity_with_tick_for_agent` — date-sensitive bench-only test, hardcoded fixture date bit-rots; only fires under `--features bench`, not in CI's main test job.

## Daemon state at handoff

* Live daemon running at PID `805759` (release build at HEAD `ef99156` via `with-ort.sh`)
* `~/.forge/forge.db` — 175.9 MB; 188 forge-tagged code files; embedder ready (fastembed pin verified clean)
* Backups: `forge.db.pre-W1.2-wipe-20260426-145000.bak` (219 MB pre-wipe baseline) plus older P3-3.11-era backups
* Daemon log at `~/.forge/daemon.log` and `/tmp/dogfood-daemon-5.log`

It is fine to leave the daemon running across the compact boundary. Next session's first actions can either keep it (and just register a new session) or stop+respawn for a clean slate.

## Cumulative deferred backlog (re-promotion candidates per "future ready")

* **W23 HIGH-1** — `tokio::task::spawn_blocking` JoinHandle dropped (panics swallowed, SIGTERM split-brain risk). Tracked by #173.
* **W23 HIGH-2** — `Request::SessionRespond` no `from_session`; no `forge-next respond` CLI surface. Tracked by #173.
* **W28 HIGH-1** — `read_message_by_id_or_prefix` unscoped (no caller_session filter). Tracked by #173.
* **W28 MED-2** — git-sha drift detection. Tracked by #174.
* **W28 LOW-2..LOW-10 + NIT-1..NIT-3** — cosmetic backlog. Tracked by #174.
* **W23 MED-3 + MED-4** — `(0,0)` background heuristic + PRAGMA/busy_timeout consistency. Tracked by #174.
* **W29 nice-to-haves** — bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail; optional config gate `memory.require_project = true`. Audit during #167/#171/#172.
* **W30 nice-to-haves** — extractor `tracing::warn!` when identity-project resolution falls through. Audit during #167.
* **W31 nice-to-haves** — drift fixture for contradiction surface. Audit during #164.
* **W32 nice-to-haves** — notify::Watcher event-driven detection (replace stat-walk on very large trees). v0.6.1+.
* **Earlier deferrals unchanged**: longmemeval / locomo re-run, SIGTERM/SIGINT chaos drill modes, criterion latency benchmarks, Prometheus bench composite gauge, multi-window regression baseline, manual-override label, P3-2 W1 trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline panel.

## Halt-and-ask map

1. **NEXT SESSION** — start at #163 (adversarial review on W1.1 + W1.2). Per per-wave §6, this is mandatory before opening #164+. Spawn `general-purpose` agent with terse-output, ≤600-word verdict cap. Capture review YAML at `docs/superpowers/reviews/2026-04-26-p3-4-w1-2-i7-impl.yaml`.
2. **Per Plan A §"Halt-and-ask points"**: anything returning `not-lockable` from adversarial review halts; any non-clean working-tree across a wave boundary halts.

## One-line summary

**P3-4 W0 + W1.1 + W1.2 closed at HEAD `ef99156` (5 commits): I-1 (ORT API mismatch BLOCKER) closed via fastembed pin; I-7 (code-graph cross-project leakage HIGH) closed end-to-end with sentinel + DAO + protocol field + CLI flag + depth guard; live-verified `forge|188` only on fresh DB.** Release stack DEFERRED to project end. Resume at #163 (adversarial review) before opening #164+ dogfood surfaces.
