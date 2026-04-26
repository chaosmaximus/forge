# Handoff — P3-4 Wave Z (CC voice unblock) closed — 22 deferred items still queued — 2026-04-26

**Public HEAD:** `b02bfcd` (review YAML + cc-voice onboarding doc).
**Working tree:** clean.
**Version:** `v0.6.0-rc.3` (release-stack still DEFERRED — Wave Z addressed external-user-blocker first).
**Plan A:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Halt:** none. Resume directly at #182 (W1.21 marker-file detection in `find_project_dir`).

## Wave Z closed (CC voice unblock — 8 commits + 1 fix-wave + 1 docs commit)

User-direction this session: external user (cc-voice team) filed
`feedback/2026-04-26-setup-and-isolation.md` describing 3 blockers
(plugin hooks duplicate, cross-project leak, unshipped binary) plus
~10 polish items. User locked **"project everywhere — no
inconsistencies"** as the vocabulary direction, then authorized
full-autonomous implementation.

11 tasks (#204-#214) created and closed. cc-voice can use Forge
end-to-end at HEAD `b02bfcd`. Onboarding doc at
`docs/onboarding/local-plugin-cc-voice.md`.

| Task | Commit | What |
|------|--------|------|
| Z1 #204 | `3af9303` | plugin.json — drop duplicate hooks ref (CC §1.1) |
| Z6 #209 | `77ee831` | detect-reality positional path (CC §2.3) |
| Z2 #205 | `929220d` | compile-context honors --project; XML `reality=` → `project=` + `resolution=` (CC §1.2 — the cross-project leak fix) |
| Z3 #206 | `f07936b` | `forge-next project init/list/show/detect` subcommand tree; renamed `Request::DetectReality` → `ProjectDetect`, `ListRealities` → `ProjectList` (CC §2.4) |
| Z4 #207 | `f07936b` | forge-setup skill rewrite — drops nonexistent `forge` CLI refs, adds project init + ingest-claude (CC §2.1+2.2+2.8) |
| Z5 #208 | `23cc4b6` | compile-context --dry-run (CC §2.9) |
| Z7 #210 | `23cc4b6` | compile-context --cwd auto-create on first contact (CC §1.2 fix #2) |
| Z9 #212 | `23cc4b6` | FORGE_HOOK_VERBOSE opt-in for hook stderr (CC §2.10) |
| Z8 #211 | `de10b9a` | update-session for misregistered project label (CC §2.6) |
| Z10 #213 | `420c6e2` | doctor backup_hygiene check (CC §2.7) |
| Z11 #214 | `420c6e2` | doctor git_sha drift warning + crates/cli/build.rs (CC §1.3 fix #2) |
| fw1 | `3fcc1eb` | review HIGH-1+HIGH-2+HIGH-3 + MED-4+MED-5 fixes — auto-create error logging, race documented, cluster-drift regression test, CHANGELOG.md created |
| docs | `b02bfcd` | review YAML/transcript + onboarding doc |

**Adversarial review verdict:** `lockable-with-fixes`. 3 HIGH (all
resolved by fw1) + 5 MED (3 resolved, 2 deferred to backlog) + 3 LOW
(deferred). See `docs/superpowers/reviews/2026-04-26-p3-4-wave-z-cc-voice.yaml`.

**Protocol break, signposted in CHANGELOG.md:**
* `Request::DetectReality` → `ProjectDetect`
* `Request::ListRealities` → `ProjectList`
* `ResponseData::RealityDetected.reality_id` → `id`, `reality_type` → `engine`
* `ResponseData::RealitiesList.realities` → `ResponseData::ProjectList.projects`
* CLI `detect-reality` and `realities` removed (hard cut, no aliases)
* protocol_hash bumped `1b3dec55ffa4…` → `68432a815353…`

## State in one paragraph

**Wave Z (CC voice unblock) closed at HEAD `b02bfcd`.** 11 tasks +
fix-wave + docs. cc-voice can use Forge cleanly: project-only
vocabulary everywhere, cross-project leak fixed, auto-create from
CWD on first SessionStart, --dry-run audit, update-session recovery,
doctor warns on stale daemon + backup pile-up. New auto-memory:
`feedback_project_everywhere_vocabulary.md` and
`feedback_xml_attribute_resolution_pattern.md`. 1608 daemon-lib
tests pass; clippy 0 warnings; all 4 main CI gates green
(harness-sync 158+108, protocol-hash `68432a815353…`,
license-manifest, review-artifacts 26 valid). **22-task drain
queue at #182-#203 still queued** — Wave Z pre-empted them; resume
at #182 (W1.21 marker-file detection) when next session opens.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD b02bfcd
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 158 + 108
bash scripts/check-protocol-hash.sh                # 68432a815353…
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh             # 26 valid

# Daemon may still be at old binary; rebuild + respawn for live dogfood
pgrep -af forge-daemon
# (if cc-voice user is hitting their daemon, leave theirs alone)

# Resume at #182. Tasks #182-#203 are the existing queue (CC voice didn't
# add new tasks; Wave Z was orthogonal to the W1.3-LOW drain).
```

## Cumulative pending work

### Wave Z deferred (3 items — backlog candidates)

* **MED-1** (Wave Z review): Z8 SessionUpdate TOCTOU error-message hygiene. Track for Z12+ session-handler hardening.
* **MED-3** (Wave Z review): `forge-next project rename / delete / relocate` not in CC voice scope. Track for v0.6.1.
* **LOW-1** (Wave Z review): `code_engine.rs::context_section` dead code → ZR scope.
* **LOW-2** (Wave Z review): Z10 backup hygiene XDG_DATA_HOME / Docker paths → v0.6.1+ ops.
* **LOW-3** (Wave Z review): cc-voice §1.2 end-to-end integration test → reactive.

### 22-task drain queue (#182 → #203) — UNCHANGED from prior session

Prioritization rule of thumb: strategic refactors first, then
surface-specific LOWs grouped by file, then carried-MEDs, then
nice-to-haves, then pre-iteration backlog re-eval.

#### Tier 1 — strategic refactors

| Task | Subject |
|------|---------|
| #182 | W1.21 — LOW-1 marker-file detection in `find_project_dir` (replaces depth-floor heuristic). Touches indexer.rs + new env override `FORGE_INDEXER_MIN_PATH_DEPTH`. Regression test pins both `/srv/foo` (admit on marker) and `/mnt` (reject — no marker). |
| #190 | W1.29 — W23 HIGH-1 strategic: SIGTERM-graceful `JoinSet` coord drained by shutdown handler. Touches main.rs + writer.rs + events.rs + chaos test. |

#### Tier 2 — focused fixes (1 commit each, ~30-60min)

| Task | Subject |
|------|---------|
| #183 | W1.22 — LOW-2 FORGE_PROJECT env path also applies depth-floor / marker check. |
| #184 | W1.23 — LOW-4 CLI rejects empty-string `--project ""` across find-symbol/code-search/blast-radius. |
| #185 | W1.24 — LOW-5 `code_search` JSON key rename `path → file_path` + contract test. |
| #186 | W1.25 — LOW-6 composite `idx_code_file_project_path` index (or `ANALYZE` post-migration). |
| #187 | W1.26 — LOW-8 `derive_project_name` accepts `org_id: &str` param (multi-org preventive). |
| #188 | W1.27 — LOW-9 regression test for the actual underscore decode-fallback bug input. |
| #189 | W1.28 — LOW-10 BlastRadius cluster-expansion accepts `project_filter: Option<&str>`. |

#### Tier 3 — observability / UX cosmetic batch

| Task | Subject |
|------|---------|
| #194 | W1.33 — I-2+I-3 force-index cold latency + WAL "database is locked" warns. |
| #195 | W1.34 — I-6 `forge-next --help` grouping via `clap::next_help_heading`. |
| #196 | W1.35 — I-9 CLI `remember` exposes `--valence`/`--intensity` flags. |
| #197 | W1.36 — I-10 Phase 9b dedicated INFO log. |
| #198 | W1.37 — I-11 `forge-next observe` shape schema uniformity (common envelope). |
| #199 | W1.38 — I-12 `forge-bench` standalone telemetry warn quiet (downgrade or `--telemetry` flag). |

#### Tier 4 — carried-forward MEDs

| Task | Subject |
|------|---------|
| #191 | W1.30 — W23 MED-3+MED-4: disposition `(0,0)` heuristic + PRAGMA/busy_timeout consistency across all `Connection::open` sites. |
| #193 | W1.32 — W28 LOW/NIT cosmetic sweep (W28-LOW-2..LOW-10 + W28-NIT-1..NIT-3). |

#### Tier 5 — nice-to-haves (release-tail / v0.6.1+ candidates)

| Task | Subject |
|------|---------|
| #200 | W1.39 — W29/W30 nice-to-haves: bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail; optional `memory.require_project = true` config gate. |
| #201 | W1.40 — W31 drift fixture for contradiction false-positive surface. |
| #202 | W1.41 — W32 `notify::Watcher` event-driven freshness gate. |

#### Tier 6 — pre-iteration backlog re-evaluation

| Task | Subject |
|------|---------|
| #203 | W1.42 — walk 9 pre-iteration deferrals: 2A-4d.3 T17 (BLOCKED on GHA billing); longmemeval/locomo re-run; SIGTERM/SIGINT chaos drill modes; criterion latency benchmarks; Prometheus bench composite gauge; multi-window regression baseline; manual-override label; P3-2 W1 trace-handler behavioral test gap; per-tenant Prometheus labels; OTLP timeline panel. Decide fix-or-permanently-defer per item with rationale. |

### Deferred internal cleanup (queued — not in #182-#203 numbering)

* **ZR** — internal rename pass. `Reality` Rust struct → `Project`,
  `mod reality` → `mod project`, SQL `reality` table → `project` (with
  migration + regression test per the SQLite-no-REVERSE memory). Delete
  dead `code_engine.rs::context_section`. Open after #203 closes.

## TaskList structure (post-Wave Z)

| | | |
|---|---|---|
| #153 | iteration umbrella (1st pass closed) | completed |
| #163 .. #181 | first-pass tasks | all completed |
| #182 .. #203 | second-pass drain (22 tasks) | **all pending** ← next-session queue |
| #192 | (W28 MED-2 — closed early by Wave Z Z11) | **completed** |
| #204 .. #214 | **Wave Z (CC voice unblock)** | **all completed** |
| #101 | P3-4 release v0.6.0 stack | DEFERRED — opens after #203 closes |

**Per-task standard procedure (unchanged from Plan A §6):** verify
clean tree → TDD-first if behavior change → fmt+clippy+tests green →
commit → adversarial review (per behavior-change wave) → fix-wave for
HIGH+MED → LOWs to backlog → TaskUpdate → dogfood briefly when
feasible.

## Issue ledger (cumulative)

| ID | Sev | Title | Status |
|----|----:|-------|--------|
| I-1 | BLOCKER | fastembed → ort → ONNX RT API v24 mismatch | ✓ closed (`50ab231`) |
| I-2 | LOW | force-index cold 5s | open → tracked by #194 |
| I-3 | LOW | "database is locked" warns | open → tracked by #194 |
| I-4 | LOW | doctor stale vergen git_sha | ✓ closed by Wave Z Z11 (`420c6e2`) |
| I-5 | LOW | mis-tagged hive-platform memory | closed (irrelevant after wipe) |
| I-6 | LOW | forge-next --help flat | open → tracked by #195 |
| I-7 | HIGH | code-graph cross-project leakage | ✓ closed (W1.2 c1+c2+c3 + W1.3 fw1+fw2+fw3) |
| I-8 | HIGH | c1 migration silently no-op'd (SQLite no REVERSE) | ✓ closed (`a7cb1a0`) |
| I-9 | LOW | CLI remember lacks --valence/--intensity | open → tracked by #196 |
| I-10 | LOW | Phase 9b no INFO log | open → tracked by #197 |
| I-11 | LOW | observe shape schema varies | open → tracked by #198 |
| I-12 | LOW | forge-bench standalone telemetry warn | open → tracked by #199 |
| I-13 | LOW | forge-bash-check substring match in argv | ✓ closed (`5d218ed` quote-strip) |
| I-14 | HIGH | compile-context cross-project leak (CC voice §1.2) | ✓ closed by Wave Z Z2 (`929220d`) + Z7 (`23cc4b6`) |
| I-15 | LOW | plugin.json hooks duplicate (CC voice §1.1) | ✓ closed by Wave Z Z1 (`3af9303`) |

## Auto-memory state (cross-session)

Saved this session: `feedback_project_everywhere_vocabulary.md`
(user's locked vocabulary direction) +
`feedback_xml_attribute_resolution_pattern.md` (the `resolution=`
attribute pattern from Z2/Z7). Already-saved relevant:
`feedback_sqlite_no_reverse_silent_migration_failure.md` (informs ZR
schema rename caution), `feedback_decode_fallback_depth_floor.md`
(informs #182), `feedback_dual_helper_basename_vs_reality.md`
(informs #187), `feedback_release_stack_deferred.md` (informs locked
rules), `feedback_json_macro_silent_drift.md` (informs #185).

## Halt-and-ask map for the post-Wave-Z window

1. **NO halt.** Per user direction (Wave Z planning session), drain
   `#182 → #203` continuously. Per Plan A §6 still applies —
   adversarial review per behavior-change wave; fix-wave for HIGH+MED.
2. **Halt only on:** non-clean working tree across a wave boundary;
   review verdict `not-lockable`; surprise architectural blocker that
   needs user input (e.g. SIGTERM-graceful coord touches an external
   contract); cc-voice filing follow-up feedback that supersedes the
   queued work.
3. **AFTER #203 closes:** halt for sign-off → open ZR (internal
   reality→project rename) → halt for sign-off → open `#101` release
   stack (multi-OS verify + version bump → `0.6.0` + `gh release
   create` + marketplace bundle + branch protection).

## One-line summary

**HEAD `b02bfcd`. P3-4 Wave Z (CC voice unblock) CLOSED — 11 tasks +
fix-wave + docs (10 commits). cc-voice can use Forge cleanly:
project vocabulary everywhere, cross-project leak fixed, auto-create
on first contact, --dry-run audit, update-session recovery, doctor
git_sha + backup hygiene warnings. 22-task drain queue at #182-#203
unchanged; resume at #182. ZR (internal rename) and #101 (release
stack) stay DEFERRED until #203 closes.**
