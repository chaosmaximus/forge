# Handoff — P3-4 W1 iteration phase CLOSED — 2026-04-26

**Public HEAD:** `5d218ed` (W1.14+W1.15 backlog sweep + bench-test date freshness; HANDOFF rewrite to follow as the next commit).
**Working tree:** clean after this commit.
**Version:** `v0.6.0-rc.3` (release-stack DEFERRED per locked user direction 2026-04-26 — version bump happens AFTER #180 close + sign-off → release ramp).
**Plan A (P3-1..P3-3 closed; P3-4 iteration phase now closed too):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`.
**Plan C (closed):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md`.
**Halt:** end of iteration phase. **Awaiting user sign-off** to open the release stack (#101 + multi-OS + tag + gh release + marketplace + branch protection).

## Iteration phase summary

Per user direction 2026-04-26: **dogfood every Forge feature on Linux → identify every issue → resolve each one → re-dogfood → THEN open the release stack.** Phase covered tasks #163 through #180 (18 sub-tasks, all closed). Single-session continuous iteration, all on the `master` branch.

### What landed this session (16 commits)

| Wave | SHA | Scope |
|------|-----|-------|
| W1.3 review | `6ed8d09` | Adversarial review on W1.1+W1.2 — verdict `lockable-with-fixes`, 3 HIGH + 3 MED + 10 LOW. |
| W1.3 fw1 | `2f4ccda` | HIGH-1 (`run_clustering` accepts NAME via new `db::ops::get_reality_by_name`) + HIGH-2 (wire `derive_project_name` at `index_directory_sync` entry). |
| W1.3 fw2 | `a7cb1a0` | HIGH-3 (companion DELETE for foreign-root pollution across 13 FHS roots) + MED-1 (regression test mirroring W29/W30) + MED-3 (RTRIM trailing slashes). **CRITICAL latent bug surfaced & fixed: SQLite has no `REVERSE()` — original c1 SQL silently no-op'd on every legacy DB; replaced with standard `REPLACE/RTRIM/REPLACE` basename idiom.** |
| W1.3 fw3 | `848c164` | MED-2 — sweep harness layer (skills/forge-feature, forge-tdd, forge-debug, forge-verify, forge-think, agents/forge-planner) to thread `--project` flag with rationale. |
| W1.3 fmt | `13ed0c8` | rustfmt cleanup across fw1 + fw2. |
| W1.3 close | `6d64523` | HANDOFF rewrite (mid-iteration); review YAML updated with `fixed_by` SHAs. |
| W1.4-1.12 | `0beafac` | 9 dogfood surfaces closed (#164-#172): contradictions / teams / manas-health / observe / plugin / HUD / Grafana / Prometheus / bench harness. |
| W1.16-1.19 | `679efd9` | 4 dogfood surfaces closed (#176-#179): sync / healing / guardrails / config + scope. |
| W1.13 fw1 | `16f37a0` | W28 review HIGH-1 — `read_message_by_id_or_prefix` gains `caller_session: Option<&str>`; protocol field + CLI `--from` on `forge-next message-read`. |
| W1.13 fw2 | `11e09af` | W23 review HIGH-2 — `Request::SessionRespond` gains `from_session`; new `forge-next respond --message-id … --from <SESSION>` CLI. |
| W1.13 fw3 | `eb2ced3` | W23 review HIGH-1 — supervise dropped spawn_blocking JoinHandles at 3 sites (main.rs embedder init, writer.rs force-index dispatch, events.rs HUD-state writer). Panics + cancellations now emit structured tracing events. |
| W1.14-1.15 | `5d218ed` | LOW-3 dead `scope_msg` binding cleanup + LOW-7 ORT cross-ref comment + I-13 quote-strip in destructive matcher (`pre_bash_check`) + bench-test date freshness via `manas::now_offset`. |
| iteration close | (this commit) | HANDOFF rewrite + review YAML hygiene fix. |

### Issue ledger (final state for the iteration phase)

| ID | Sev | Title | Status |
|----|----:|-------|--------|
| I-1 | BLOCKER | fastembed → ort → ONNX RT API v24 mismatch | ✓ closed (`50ab231`) |
| I-2 | LOW | first force-index post-restart 5 s cold | observed; warm 9 ms; deferred |
| I-3 | LOW | "database is locked" warns during force-index dispatch | expected SQLite WAL contention; deferred |
| I-4 | LOW | `doctor` shows stale vergen git_sha after edits | resolved by rebuild; cosmetic |
| I-5 | LOW | mis-tagged hive-platform memory in earlier DB | irrelevant after wipe; closed |
| I-6 | LOW | `forge-next --help` lists 50+ commands flat | cosmetic; deferred |
| I-7 | HIGH | code-graph cross-project leakage | ✓ closed end-to-end (W1.2 c1+c2+c3 + W1.3 fw1+fw2+fw3) |
| I-8 | HIGH | c1 migration silently no-op'd (SQLite has no REVERSE) | ✓ closed (`a7cb1a0`) |
| I-9 | LOW | CLI `remember` lacks `--valence`/`--intensity` | cosmetic; deferred |
| I-10 | LOW | Phase 9b no dedicated INFO log | observability gap; deferred |
| I-11 | LOW | `forge-next observe` schema varies by shape | cosmetic; deferred |
| I-12 | LOW | `forge-bench` standalone telemetry warning | cosmetic; deferred |
| I-13 | LOW→ ✓ | forge-bash-check substring match catches argv content | ✓ closed (`5d218ed` quote-strip) |

### W23+W28 carried-forward HIGHs (closed in #173)

| | Site | Status |
|---|------|--------|
| W23 HIGH-1 | spawn_blocking JoinHandle dropped (3 sites) | ✓ supervised (`eb2ced3`) |
| W23 HIGH-2 | `Request::SessionRespond` no `from_session` + no CLI | ✓ field + `forge-next respond` CLI (`11e09af`) |
| W28 HIGH-1 | `read_message_by_id_or_prefix` unscoped | ✓ caller_session scoping (`16f37a0`) |

## Final test counts (HEAD `5d218ed`)

* `cargo fmt --all --check` — clean
* `cargo clippy -p forge-daemon -p forge-core -p forge-cli -p forge-hud -- -W clippy::all -D warnings` — 0 warnings
* `cargo test -p forge-daemon --lib` — **1535 passed** (was 1528 at start of session; +7 across W1.3 fw1+fw2 + W1.13 fw1 + W1.14)
* `cargo test -p forge-daemon --lib --features bench 'workers::disposition::tests::test_step_for_bench_parity'` — passes (was bit-rotted)
* `cargo test -p forge-core --lib` — clean
* `cargo test -p forge-cli` — clean
* `cargo test -p forge-hud` — clean
* `bash scripts/check-harness-sync.sh` — OK (155 + 108)
* `bash scripts/check-protocol-hash.sh` — OK (`1b3dec55ffa4…`)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-review-artifacts.sh` — OK (25 valid, no open blocking findings)
* `bash scripts/ci/check_spans.sh` — OK
* All 4 fast benches at seed=42: forge-consolidation 0.9667, forge-identity 0.9990, forge-isolation 1.0000, forge-coordination 1.0000 — PASS

## Live-dogfood evidence

* Daemon spawned at HEAD `13ed0c8` ran cleanly: fastembed pin loaded with no embedder panic, no-path force-index completed (188 files / 7781 symbols / 1454 import edges), `code_file.project` distribution `forge|188` only — no foreign leakage.
* `<clusters count="8">` in `forge-next compile-context` independently confirmed the W1.3 fw1 HIGH-1 fix in production: clustering runs end-to-end on no-path force-index via the new by-name fallback.
* W26 team primitives F6/F7/F8/F9 all verified end-to-end on a fresh `agent`-type team (idempotent run, no-op annotation, `--project` scope persisted, role names in `members`).
* W31 contradictions Phase 9a fired with `valence_distribution: "neutral=1"`, 0 false positives — F18 reproducer fixture (Session 17/16 boilerplate) does not trigger.

## Cumulative deferred backlog (re-promotion candidates → release-stack-tail or v0.6.1+)

* **W1.3 LOW-1** — depth-floor `≥4 slashes` heuristic in `find_project_dir` is host-shape-coupled. Fix: marker-file detection (Cargo.toml/.git/package.json). v0.6.1+.
* **W1.3 LOW-2** — FORGE_PROJECT env path skips depth-floor. v0.6.1+.
* **W1.3 LOW-4** — empty-string `--project ""` accepted by CLI silently fails-closed against the JOIN. v0.6.1+.
* **W1.3 LOW-5** — `code_search` 3 new `path:` JSON emit sites (CLI reads `file_path`); pre-existing key drift, c2 widened it. v0.6.1+.
* **W1.3 LOW-6** — single-column `idx_code_file_project` could be composite `(project, path)`. Performance-not-correctness. v0.6.1+.
* **W1.3 LOW-8** — `derive_project_name` hardcodes `"default"` org scope. Preventive only (single-org Forge today). v0.6.1+.
* **W1.3 LOW-9** — depth-floor regression test does NOT reproduce the underscore-in-component bug input. Strategic test extension. v0.6.1+.
* **W1.3 LOW-10** — BlastRadius cluster-expansion not project-scoped. HIGH-3 cleanup eliminates the surface on healthy DBs; v0.6.1+ if real-world surfaces it.
* **W28 MED-2** — git-sha drift detection for daemon-vs-CLI version match.
* **W28 LOW-2..LOW-10 + NIT-1..NIT-3** — cosmetic backlog.
* **W23 MED-3 + MED-4** — `(0,0)` background heuristic + PRAGMA/busy_timeout consistency.
* **W23 HIGH-1 strategic fix** — full SIGTERM-graceful coordination (JoinSet drained by shutdown handler) — fw3 ships tactical observability only.
* **W29/W30 nice-to-haves** — bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail; optional `memory.require_project = true` config gate; W30 extractor `tracing::warn!` on identity-project resolution falls-through. Folded into release-tail.
* **W31 nice-to-haves** — drift fixture for contradiction surface.
* **W32 nice-to-haves** — `notify::Watcher` event-driven detection (replace stat-walk on very large trees). v0.6.1+.
* **2A-4d.3 T17** — bench-fast required-gate flip (`continue-on-error: false`) after 14 consecutive green master runs accumulate. **Note: this is gated on green master CI, which depends on the GHA billing block being resolved — a pre-release-stack concern.**
* Earlier deferrals unchanged: longmemeval/locomo re-run, SIGTERM/SIGINT chaos drill modes, criterion latency benchmarks, Prometheus bench composite gauge, multi-window regression baseline, manual-override label, P3-2 W1 trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline panel.

## TaskList structure (post-iteration-phase)

**Iteration phase (all closed):**
| | | |
|---|---|---|
| #153 | iterative Linux dogfood umbrella | **completed** ✓ |
| #163 | W1.3 adversarial review | completed |
| #164-#172 | 9 dogfood surfaces | completed |
| #173 | re-promote 3 W23+W28 HIGHs | completed |
| #174 | actionable LOW sweep | completed |
| #175 | bench-test date fix | completed |
| #176-#179 | 4 dogfood surfaces | completed |
| #180 | close iteration phase | **in_progress** (this commit) |
| #181 | W1.3 fix-wave umbrella | completed |

**Release stack (NOT YET STARTED — awaiting user sign-off):**
| Task | Subject |
|------|---------|
| #101 | P3-4 release v0.6.0 — multi-OS verify + tag + GitHub release + marketplace bundle + branch protection |

## Halt-and-ask map for the post-iteration window

1. **NOW — halt for user sign-off.** The iteration phase is closed; every dogfood surface has been exercised and every HIGH+actionable MED has been resolved. Per Plan A §"Halt-and-ask points": "End of each phase: wait for user sign-off before opening the next." This is that point.
2. **On sign-off, the release stack opens:**
   * Re-promote #101 → multi-OS verify (Linux full sweep + macOS reproduction handoff per Plan A decision #2).
   * Bump version: `0.6.0-rc.3 → 0.6.0` across `Cargo.toml` (4 crates) + `plugin.json` + `Formula/forge.rb`.
   * `gh release create v0.6.0` with multi-arch binaries + release notes from CHANGELOG.
   * Marketplace submission bundle (manifest, listing copy, screenshots/demo GIF).
   * Branch protection rules (JSON config for required reviewers, required CI checks, no force-push).
3. **Pre-condition for release stack:** GHA billing block on chaosmaximus account must be resolved before bench-fast required-gate flip (T17) can fire on accumulated green runs. **Out of scope for the iteration phase per locked user direction; will surface as the first release-stack halt-and-brief.**

## One-line summary

**P3-4 W1 iteration phase CLOSED at HEAD `5d218ed` (16 commits this session): adversarial review + 3-commit fix-wave on W1.3 (3 HIGH + 3 MED resolved); 13 dogfood surfaces verified end-to-end; 3 carried HIGHs closed (W23+W28); 4 actionable LOWs swept; bench-test date bit-rot fixed. Bonus: surfaced + fixed CRITICAL c1 migration latent bug (SQLite has no REVERSE → original SQL silently no-op'd). 1535 daemon-lib tests pass; clippy 0 warnings; all 5 CI gate scripts green; 25 reviews valid. Halt for user sign-off before opening the release stack.**
