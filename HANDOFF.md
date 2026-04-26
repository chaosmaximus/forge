# Handoff — P3-3.11 W29 closed (F15/F17 cross-project leak fixed) — 2026-04-26

**Public HEAD:** `7523f54` (W29 c4 — live-verification doc).
**Forge-app master:** unchanged.
**Version:** v0.6.0-rc.3.
**Plan A (closed P3-1..P3-3, P3-4 queued):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5/3.6/3.7).
**Plan C (active):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md` (P3-3.9 + P3-3.10 + P3-3.11 W29 closed; P3-3.11 W30..W34 next).
**Halt:** **WAVE HALT** — P3-3.11 W29 closed end-to-end with live verification. Halt for sign-off before opening **P3-3.11 W30 (F16 identity per-(agent, project))**. The original W29 plan envisioned 1-2 commits; the realised W29 ran 4 commits because of the substantive sentinel-replacement architecture the user authorised (Path α). Worth a sign-off before continuing the wave train.

## State in one paragraph

**P3-3.11 W29 closed at HEAD `7523f54`** (4 commits since `e05e2c6`): c1 schema migration backfilling NULL/empty `memory.project` to the `_global_` sentinel + FTS-rebuild defence; c2 DAO helper `project_or_global()` + write-path enforcement at all 4 production memory-INSERT sites + comparison-site normalisation in 3 places (sync conflict-detect, ops::remember dedup, consolidator reweave); c3 `Recall.include_globals: Option<bool>` protocol field, strict-by-default WHERE clause, `--include-globals` CLI flag, +13 internal call-site updates for the new field; c4 live verification on the 218 MB `~/.forge/forge.db` confirming **zero NULL/empty rows post-migration**, F15/F17 reproducer query returning only forge memories under strict scope, opt-in surfacing the previously-leaking Hive Finance audit. All CI gates green: harness-sync 155+107, protocol-hash bumped `1dca2da7… → 5b9cada23419…`, fmt clean, clippy 0 warnings, 24 review YAMLs valid, license-manifest clean. Three deferred HIGHs from W23/W28 still open (carry to W34 close).

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 7523f54
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 155 + 107
bash scripts/check-review-artifacts.sh             # 24 reviews valid
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh                # 5b9cada23419…
cargo fmt --all --check                            # clean
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings  # 0 warnings

# Read the dogfood-fixes plan + W29 verification for context, then begin P3-3.11 W30.
cat docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md
cat docs/benchmarks/results/2026-04-26-w29-live-verification.md

# W30 first action — investigate F16 identity scoping (per-agent vs per-(agent, project)):
grep -rn "fn list_identity\|fn store_identity\|identity.*project\|identity SET" crates/daemon/src/db/ crates/daemon/src/server/ | head -20
sqlite3 -readonly file:~/.forge/forge.db?mode=ro "PRAGMA table_info(identity);"
```

## P3-3.11 W29 close summary

### What landed (4 commits)

| SHA | Commit | Scope |
|-----|--------|-------|
| `ede5c38` | W29 c1 | Schema migration: backfill `memory.project` NULL/empty → `_global_` sentinel; FTS-rebuild defence when backfill is going to run; idempotent thereafter; +1 unit test (`p3_3_11_w29_project_sentinel_backfill`). |
| `6efca61` | W29 c2 | `pub const GLOBAL_PROJECT_SENTINEL = "_global_"` + `pub fn project_or_global()` helper in `db::ops`; applied at 4 production INSERT sites (`remember`, `remember_raw`, `teams::decide_meeting`, `teams::synthesize_voting` with team-orchestrator JOIN to derive project); LHS `COALESCE(NULLIF(project,''), '_global_')` normalisation in dedup + sync + reweave; `_global` → `_global_` label normalisation in `health_by_project_org` and 3 test sites; rewrote 3 ops tests to assert strict semantics. |
| `3c20bb7` | W29 c3 | Tightened `recall_bm25_project_org_flipped` WHERE: `m.project = ?2` strict by default, `(m.project = ?2 OR m.project = '_global_')` when `include_globals=true`; removed dead NULL/empty branches; new `recall_bm25_project_with_globals` + `hybrid_recall_with_globals` opt-in entry points; `Request::Recall` gains `include_globals: Option<bool>` field; `forge-next recall --include-globals` flag; protocol-hash bumped to `5b9cada23419…`; 13 `Request::Recall { ... }` literal sites + handler destructure + CLI dispatcher updated; +2 regression tests (`recall_with_globals_admits_global_alongside_project_rows`, `recall_with_globals_does_not_leak_other_projects`). |
| `7523f54` | W29 c4 | Live verification on `~/.forge/forge.db` (218 MB): zero NULL/empty rows after migration; F15/F17 reproducer returns only forge memories under strict scope and 4 (forge + globals) under `--include-globals`; write-path emits `forge` for `--project forge` and `_global_` for omitted-project. Procedure documented in `docs/benchmarks/results/2026-04-26-w29-live-verification.md`. |

### Live DB verification (key surfaces)

* **Pre-W29 distribution**: 33 NULL + 31 forge + 5 hive-platform + 2 workspace + 2 production = 73 rows (33 leak vectors).
* **Post-W29 distribution**: 23 `_global_` + 17 forge + 1 hive-platform = 41 rows (0 leak vectors).
* **Strict recall**: `recall "polish wave drift fixtures" --project forge` → 2 forge memories. ✓
* **Opt-in recall**: same query `--include-globals` → 2 forge + 2 `_global_`-tagged (incl. the originally-leaking "Feature engineering audit"). ✓
* **Write tagged**: `remember --project forge ...` → row stored with `project = 'forge'`. ✓
* **Write untagged**: `remember ...` (no `--project`) → row stored with `project = '_global_'`. ✓

### Carry-forward findings → P3-3.11 W30..W34

* **W23 HIGH-1 (deferred)** — `tokio::task::spawn_blocking` for force-index drops its `JoinHandle`: panics swallowed, SIGTERM aborts mid-write split-brain risk, no concurrency guard. Reviewer-recommended fix: supervisor task + `AtomicBool` reject-overlap, mirroring `kpi_reaper::run_reap_blocking`. **Address in W34.**
* **W23 HIGH-2 (deferred)** — `Request::SessionRespond` still has no `from_session` field, AND there's no `forge-next respond` CLI surface at all. Decide between explicit descope OR adding the `respond` subcommand to close the F11/F13 round-trip. **Address in W34.**
* **W28 HIGH-1 (deferred)** — `read_message_by_id_or_prefix` is unscoped (no `to_session`/`from_session` filter). Single-tenant daemon means not a hard auth boundary today, but the architectural contract weakened from W27. Reviewer-recommended fix: optional `caller_session: Option<String>` on `Request::SessionMessageRead` that scopes the SQL when set. **Address in W34.**
* **W28 MED-2 (open)** — F1 stale-version detection only catches Cargo.toml version-string drift, not git-sha drift. Common dev workflow (commit, rebuild, daemon stays on prior commit) is silently reported as "matched". Fix path: also compare `option_env!("VERGEN_GIT_SHA")` against daemon-reported `git_sha`. **Address in W34.**
* **W28 LOW-2..LOW-10 + NIT-1..NIT-3 (open)** — cosmetic backlog. **Roll into W34 close.**
* **W29 nice-to-have backlog (deferred to v0.6.1+)**:
  * Bench D6 strict-project precision dimension (extend `forge_isolation` bench to gate strict + opt-in semantics).
  * Auto-extractor `tracing::warn!` audit trail when project resolution falls through to `_global_` (visibility into upstream tagging quality).
  * Optional config gate `memory.require_project = true` to hard-fail writes that would default to `_global_` (production strictness for telemetry).

## Wave roadmap (P3-3.11 W29 closed; remaining 5 commits to P3-4)

### P3-3.11 — Investigation MED/LOW (5 commits remaining, ~5-7h, halt-able)

| Wave | Scope | Task ID | Source |
|------|-------|---------|--------|
| W30 | F16 identity per-(agent, project) — decision + impl OR HALT-AND-BRIEF if schema change | #147 | F16 |
| W31 | F18 contradiction false-positives (Phase 9a/9b tightening) | #148 | F18 |
| W32 | F20+F22 indexer .rs file scope (watcher pattern) | #149 | F20, F22 |
| W33 | F21 force-index error UX (likely no-op after W22) | #150 | F21 |
| W34 | review + HANDOFF + halt + carry-forward W23/W28 deferred HIGHs + W29 nice-to-have | #151 | per-wave-procedure + W23/W28 deferrals + W29 backlog |

**Halt-and-brief at W30** if F16 needs schema change (defer to v0.6.1).
**Halt at end of W34** for sign-off opening P3-4.

### P3-4 — Release & distribution (after P3-3.11 close, halted for sign-off)

7 waves per Plan A `2026-04-25-complete-production-readiness.md` §"Phase P3-4". Multi-OS dogfood → bench-fast gate flip → v0.6.0 bump → gh release → marketplace bundle (USER) → branch protection (USER) → final HANDOFF.

## Dogfood findings reference (23 findings, P3-3.8)

Source: `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md`

### HIGH (3) — closed in P3-3.9 ✓

* **F4** → `54aeecd`. **F11** → `6e27eb4`. **F13** → `6e27eb4`. **F23** → `611169b` + `39f84b2`.

### MEDIUM (7) — 4 closed in P3-3.10 ✓ + 2 closed in P3-3.11 W29 ✓ + 2 in W30..W32

* **F1** → `b965d0b` ✓. **F2** → `b965d0b` ✓. **F3** → `b965d0b` ✓. **F9** → `eb55a2d` ✓.
* **F15+F17** → `ede5c38..7523f54` ✓ (W29 — strict-by-default scoping + `--include-globals` opt-in).
* **F20** → W32. **F22** → W32.

### LOW (11) — 8 closed in P3-3.10 ✓ + 3 in W30..W33

* **F5** → `bd1bac6` ✓. **F6** → `eb55a2d` ✓. **F7** → `eb55a2d` ✓. **F8** → `eb55a2d` ✓.
* **F10** → `bd1bac6` ✓. **F12+F14** → `85712a8` ✓. **F19** → `bd1bac6` ✓.
* **F16** → W30 (decision needed). **F18** → W31. **F21** → W33 (likely no-op).

### WORKS-AS-EXPECTED (2) — no fix needed

* Identity (Ahankara) — 41 facets render cleanly in `compile-context` XML.
* Healing system — 8 layers all populate; manas-health surfaces them.

## Cumulative commit tally (P3-3.5..P3-3.11 W29)

| Range | Phase | Commits |
|-------|-------|---------|
| `3e86714..7091526` | P3-3.5 W1-W8 polish | 12 |
| `8e449a5..d7c5f73` | P3-3.5 polish-review fix-wave + YAML | 2 |
| `b80ae68..daf6491` | P3-3.6 W9-W13 otel cluster bump | 5 |
| `daa76ad..6118ec2` | P3-3.7 W14+W17+W19 drift fixtures | 3 |
| `0ba3f7b..14279c9` | P3-3.8 dogfood + plan-doc | 3 |
| `37c90b0` | pre-compact HANDOFF | 1 |
| `54aeecd..611169b` | P3-3.9 W20-W22 (3 HIGH dogfood fixes) | 3 |
| `2ef27e8..e190f70` | P3-3.9 W23 review + fix-wave + YAML status | 3 |
| `46d525a` | P3-3.9 close HANDOFF | 1 |
| `bd1bac6..85712a8` | P3-3.10 W24-W27 (10 dogfood fixes) | 4 |
| `cd2d733..7f8a694` | P3-3.10 W28 review + fix-wave + YAML status | 3 |
| `e05e2c6` | P3-3.10 close HANDOFF | 1 |
| `ede5c38..7523f54` | P3-3.11 W29 (F15+F17 sentinel + strict scope + live verify) | 4 |
| **Total since `a9fa9af`** | — | **45** |
| **Total this session (since `e05e2c6`)** | — | **4** |

## Tests + verification (final state at HEAD `7523f54`)

* `cargo fmt --all --check` — clean
* `cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --features bench` — clean
* `cargo test -p forge-daemon --lib db::` — 229 passed (incl. 5 new W29 tests)
* `cargo test -p forge-daemon --lib teams::tests` — 38 passed
* `cargo test -p forge-daemon --test test_e2e_lifecycle` — 6 passed
* `cargo test -p forge-core --lib protocol::contract_tests::tests::test_variant_count_completeness` — 1 passed (124 variants, 1 new field)
* `bash scripts/check-harness-sync.sh` — OK (**155** + 107)
* `bash scripts/check-review-artifacts.sh` — OK (**24** review(s) valid, 0 open blocking)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-protocol-hash.sh` — OK (`5b9cada23419…`)

Two pre-existing test failures observed in the full `--features bench` suite, both unrelated to W29 and confirmed to fail identically on master without my changes (`git stash && cargo test ...`):

* `server::handler::tests::test_daemon_state_new_is_fast` — flaky timing test (passes individually).
* `workers::disposition::tests::test_step_for_bench_parity_with_tick_for_agent` — date-sensitive: hardcoded fixture date `2026-04-25 10:00:00` falls outside the 24h `query_recent_sessions_for_agent` window now that today is `2026-04-26`. Pre-existing test bug, unrelated to W29.

## Cumulative deferred backlog

* **From P3-3.7 (drift fixtures):** W15 forge-context, W16 forge-identity, W18
  forge-coordination drift fixtures need `_with_inj` wrapper variant + injected-buggy
  callable in tests. Defer to v0.6.1+.
* **From P3-3.9 W23 review:** HIGH-1 spawn_blocking supervisor + concurrency-guard;
  HIGH-2 `SessionRespond` CLI surface (descope or add `forge-next respond`);
  4 LOW + 2 NIT cosmetics; MED-3 `(0,0)` background heuristic; MED-4 PRAGMA
  + busy_timeout consistency. **Carry into W34**.
* **From P3-3.10 W28 review:** HIGH-1 SessionMessageRead caller-identity scope;
  MED-2 git-sha drift detection; LOW-2..LOW-10 (LIKE escape, error-wrapping
  wording, partial-retire visibility, JSON-shape contract test, env-var boot
  timeout, project validation, broken-symlink detection, missing helper unit
  tests, retired-row filter on team_member); NIT-1..NIT-3 (clap message
  wording, terminal-width decoration, ID truncation length). **Carry into W34**.
* **From P3-3.11 W29 nice-to-haves**: bench D6 strict-project precision
  dim; auto-extractor `tracing::warn!` audit trail; optional config gate
  `memory.require_project = true`. **v0.6.1+** unless surfaced by W34
  review.
* **Earlier deferrals unchanged:** longmemeval / locomo re-run, SIGTERM/SIGINT
  chaos drill modes, criterion latency benchmarks, Prometheus bench composite
  gauge, multi-window regression baseline, manual-override label, P3-2 W1
  trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline
  panel.

## Tasks (next session)

5 individual tasks remaining (#147-#151) for P3-3.11:

| Task ID | Wave | Status |
|---------|------|--------|
| #147 | P3-3.11 W30 (F16) | pending (halt-and-brief if schema) |
| #148 | P3-3.11 W31 (F18) | pending |
| #149 | P3-3.11 W32 (F20+F22) | pending |
| #150 | P3-3.11 W33 (F21) | pending |
| #151 | P3-3.11 W34 close | pending |

## Halt-and-ask map (1 active + 1 conditional + 1 final)

1. **End of P3-3.11 W29**: **HALT NOW** for sign-off before W30. Original W29 plan envisioned 1-2 commits; the realised W29 ran 4 substantive commits (sentinel-replacement architecture). Confirm direction before continuing wave train.
2. **P3-3.11 W30** if identity scope needs schema change: halt + brief.
3. **End of P3-3.11 W34**: halt for sign-off, opens P3-4.

## One-line summary

**P3-3.11 W29 closed at HEAD `7523f54` (4 commits): F15/F17 cross-project recall leak fixed end-to-end with `_global_` sentinel + strict-by-default WHERE + `--include-globals` opt-in; live-verified on the 218 MB working DB (zero NULL/empty rows post-migration; reproducer returns only forge memories under strict scope).** All CI gates green, 24 review YAMLs valid, working tree clean. Resume at **W30 (F16 identity per-(agent, project) scoping)** next session. After P3-3.11 closes, P3-4 release halts for user sign-off.
