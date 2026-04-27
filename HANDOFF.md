# Handoff — P3-4 Wave X (CC voice Round 3) closed — 2026-04-27

**Public HEAD:** `630e1c9` (Wave X / X1.fw2).
**Working tree:** clean (response doc lives outside repo at `/mnt/colab-disk/DurgaSaiK/forge/feedback/2026-04-27-forge-team-round-3-response.md`).
**Version:** `v0.6.0-rc.3 (630e1c9)` (release stack still DEFERRED — Plan A §6 backlog drain in progress).
**Daemon respawned:** binary at `target/release/forge-daemon`, doctor green except expected `backup_hygiene` WARN.
**Plan A:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Halt:** none. Next item: #190 (W1.29 SIGTERM-graceful JoinSet, Tier 1 strategic) — paused at start of Wave X.

## This session's deltas

### Wave X closed (CC voice Round 3 unblock — 5 commits + response doc)

User-direction this session: external user (cc-voice) filed
`feedback/2026-04-27-round-3-post-wave-y.md` after Round 2 verification.
TL;DR: 7/8 of Wave Y verified live; 1 partial (§B Y2 auto-create
write path) + 1 caveat (§C `forge-daemon --version` on glibc<2.38) +
1 design question (§E sticky vs upgrade for `domain="unknown"`).

User authorized full-autonomous Wave X; mirror of Wave Y / Wave Z
pattern. Each commit plus dogfood plus adversarial review per
Plan A §6.

| Task | Commit | What |
|------|--------|------|
| X1 #229 | `97b6caf` | Fix auto-create write path under read-only routing. `Request::CompileContext` is in `is_read_only()` so `state.conn` was a per-request read-only SQLite handle; INSERT errored silently. Switched the Z7 auto-create site to open a fresh writer connection from `state.db_path` (mirrors `kpi_reaper` precedent). New routing-aware regression test. Also fixed 4 stale tests left behind by Wave Y / Wave Z (`test_detect_reality_empty_dir_*`, `test_compile_dynamic_suffix_includes_agents`, 2× `protocol::contract_tests`). (MED-HIGH — closes Round 3 §B.) |
| X2 #230 | `880ad1f` | Bake DT_RUNPATH into Linux binaries via three `$ORIGIN`-relative entries in `.cargo/config.toml`'s `rustflags`. Closes deferred task #220. `forge-daemon --version` now works without `LD_LIBRARY_PATH` on glibc<2.38. (LOW — closes Round 3 §C.) |
| X3 #231 | `c052a9b` | Architecture doc `docs/architecture/project-domain-lifecycle.md` locking the "domain is a HINT, not a lock" design — first successful detection upgrades `domain="unknown"` → real domain in place. v0.6.0 ships bind-time logic; v0.6.1 ships indexer upgrade per the contract (tracked as #233). (DOC — closes Round 3 §E.) |
| X4 #232 | response doc | `feedback/2026-04-27-forge-team-round-3-response.md` — disposition matrix, X1 root-cause writeup (read-only routing trap), X2 RPATH delivery plan, X3 design answer with rationale, post-wave fw1+fw2 delta in §G. |
| **fw1** #236 | `cd6eb80` | **HIGH (dogfood-found)**: pre-existing data loss. Schema carries `UNIQUE INDEX idx_reality_path_unique ON reality(project_path) WHERE project_path IS NOT NULL`; auto-create's `INSERT OR REPLACE` REPLACED conflicting rows on path collision, silently wiping the user's `project init` setup. Live dogfood after X1 surfaced this. Fix: gate auto-create on path absence too; emit `tracing::warn!` on alias mismatch. New regression test seeds a pre-existing row + asserts it survives a colliding alias call. |
| **fw2** #237 | `630e1c9` | Adversarial-review fixes (verdict: `lockable-with-fixes` — 0 BLOCKER / 0 HIGH / 1 MED / 5 LOW). MED-1: concurrent-fresh-create race — switched auto-create to `ops::auto_create_reality_if_absent` (`INSERT OR IGNORE`) so the second writer is a no-op instead of triggering REPLACE. LOWs: `cargo install` caveat in onboarding doc (LOW-2), comment fix (LOW-3), real ULID in fw1 test seed (LOW-4), symmetric warn for name-bound-different-path (LOW-5). LOW-1 (route through `writer_tx`) deferred — backlog #238. |

### Open questions §F — answered in response doc

1. cc-voice Round 4 question (sticky vs upgrade for `domain="unknown"`):
   answered hint+upgrade; doc `docs/architecture/project-domain-lifecycle.md`
   pins the v0.6.1 implementer's contract (SQL guard, tracing, test cases).
2. The §B regression that escaped Wave Y / Y2: postmortem in response
   doc §C — Y2's tests built `DaemonState::new(":memory:")` directly,
   bypassing the read-only routing layer. Future review checklists
   should require routing-aware test setup for any handler arm whose
   `Request` is in `is_read_only()`.

## State in one paragraph

**HEAD `630e1c9`. Wave X (#229–#232 + #236 + #237) closed (5 commits + response doc + 2 fix-waves + adversarial review).** cc-voice's Round 3 3 items (1 MED-HIGH, 1 LOW, 1 doc) all resolved; plus a HIGH-severity pre-existing data-loss path closed via fw1; plus 6 review findings closed via fw2. Doctor green at HEAD `630e1c9`. clippy 0 warnings; full test suite green (1557 daemon, 109 core, all CLI); harness-sync + protocol-hash + license-manifest + review-artifacts all OK. **15 drain items still pending (#190-#191, #193-#203, #215, #216-#219, #233, #238).** Resume at #190 (Tier 1 strategic SIGTERM-graceful JoinSet) when next session opens.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 630e1c9
git status --short                                 # expect clean
forge-next doctor                                  # version + git_sha sanity
bash scripts/check-harness-sync.sh                 # sanity gates
bash scripts/check-protocol-hash.sh
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh

# Resume at #190 (W1.29 — SIGTERM-graceful JoinSet, Tier 1 strategic).
# Touches main.rs (shutdown path) + writer.rs (force-index dispatch) +
# possibly events.rs (HUD writer). Add chaos test: mid-run SIGTERM,
# assert no partial DB state.
```

## Cumulative pending work

### Tier 1 — strategic (1 item)

| Task | Subject |
|------|---------|
| #190 | W1.29 — W23 HIGH-1 strategic: SIGTERM-graceful `JoinSet` coord drained by shutdown handler. Touches main.rs + writer.rs + events.rs + chaos test. |

### Tier 4 — carried-forward MEDs (1 item)

| Task | Subject |
|------|---------|
| #191 | W1.30 — W23 MED-3+MED-4: disposition `(0,0)` heuristic + PRAGMA/busy_timeout consistency across all `Connection::open` sites. |

### Tier 3 — observability / UX cosmetic batch (7 items)

| Task | Subject |
|------|---------|
| #193 | W1.32 — W28 LOW/NIT cosmetic batch (W28-LOW-2..LOW-10 + W28-NIT-1..NIT-3). |
| #194 | W1.33 — I-2+I-3 force-index cold latency + WAL "database is locked" warns. |
| #195 | W1.34 — I-6 `forge-next --help` grouping via `clap::next_help_heading`. |
| #196 | W1.35 — I-9 CLI `remember` exposes `--valence`/`--intensity` flags. |
| #197 | W1.36 — I-10 Phase 9b dedicated INFO log. |
| #198 | W1.37 — I-11 `forge-next observe` shape schema uniformity (common envelope). |
| #199 | W1.38 — I-12 `forge-bench` standalone telemetry warn quiet (downgrade or `--telemetry` flag). |

### Tier 5 — nice-to-haves (release-tail / v0.6.1+ candidates) (3 items)

| Task | Subject |
|------|---------|
| #200 | W1.39 — W29/W30 nice-to-haves: bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail; optional `memory.require_project = true` config gate. |
| #201 | W1.40 — W31 drift fixture for contradiction false-positive surface. |
| #202 | W1.41 — W32 `notify::Watcher` event-driven freshness gate. |

### Tier 6 — pre-iteration backlog re-evaluation (1 item)

| Task | Subject |
|------|---------|
| #203 | W1.42 — walk 9 pre-iteration deferrals: 2A-4d.3 T17 (BLOCKED on GHA billing); longmemeval/locomo re-run; SIGTERM/SIGINT chaos drill modes; criterion latency benchmarks; Prometheus bench composite gauge; multi-window regression baseline; manual-override label; P3-2 W1 trace-handler behavioral test gap; per-tenant Prometheus labels; OTLP timeline panel. Decide fix-or-permanently-defer per item with rationale. |

### Wave Z + Y + X deferred (review residue) (5 items)

* **#216** — Wave Z MED-1: SessionUpdate TOCTOU error-message hygiene.
* **#217** — Wave Z MED-3: `forge-next project rename / delete / relocate` (cc-voice §C question 3 — walking-up TOML discovery lands here; also covers fw1 alias-mismatch escape valve).
* **#218** — Wave Z LOW-2: doctor backup hygiene XDG_DATA_HOME / Docker paths.
* **#219** — Wave Z LOW-3: cc-voice §1.2 end-to-end integration test.
* **#238** — Wave X LOW-1 (deferred): route compile-context auto-create through `writer_tx` (architectural; v0.6.1+).

### v0.6.1 follow-ups (from Wave X)

* **#233** — domain="unknown" → real-domain upgrade in indexer per `docs/architecture/project-domain-lifecycle.md`. Small UPDATE in `workers/perception.rs` or `workers/indexer.rs` with SQL guard `WHERE domain = 'unknown'`. Test contract pinned in the doc.

### Deferred internal cleanup (queued — not in #182-#203 numbering)

* **#215 — ZR — internal rename pass.** `Reality` Rust struct → `Project`,
  `mod reality` → `mod project`, SQL `reality` table → `project` (with
  migration + regression test per the SQLite-no-REVERSE memory). Delete
  dead `code_engine.rs::context_section`. **Open after #203 closes.**

## TaskList structure (post-Wave X)

| Range | Subject | Status |
|---|---|---|
| #153 | iteration umbrella (1st pass closed) | completed |
| #163 .. #181 | first-pass tasks | all completed |
| #182 .. #189 | W1.3 LOW drain Tier 1+2 (prior session) | all completed |
| #190, #191, #193-#203 | second-pass drain (14 pending) | ← next-session queue |
| #192 | (W28 MED-2 — closed early by Wave Z Z11) | completed |
| #204 .. #214 | Wave Z (CC voice Round 1 unblock) | all completed |
| #215 | **ZR — internal rename + dead-code cleanup** | **pending — opens after #203 closes** |
| #216 .. #219 | Wave Z deferred (review residue) | all pending — v0.6.1+ |
| #220 | Wave Z dogfood (RPATH bake) | **completed (lifted into Wave X / X2)** |
| #221 .. #228 | Wave Y (CC voice Round 2 unblock — prior session) | all completed |
| #229 .. #232 | **Wave X (CC voice Round 3 unblock — this session)** | **all completed** |
| #233 | v0.6.1 follow-up — domain="unknown" upgrade in indexer | pending — v0.6.1 |
| #234 .. #237 | **Wave X review + fix-waves (this session)** | **all completed** |
| #238 | Wave X review LOW-1 backlog (writer_tx routing) | pending — v0.6.1+ |
| #101 | P3-4 release v0.6.0 stack | DEFERRED — opens after #215 closes |

**Per-task standard procedure (unchanged from Plan A §6):** verify
clean tree → TDD-first if behavior change → fmt+clippy+tests green →
commit → adversarial review (per behavior-change wave) → fix-wave for
HIGH+MED → LOWs to backlog → TaskUpdate → dogfood briefly when
feasible.

**Wave-pattern lesson surfaced this session:** for any handler arm
whose `Request` is in `is_read_only()`, future tests MUST build a
`DaemonState::new_reader(...)` (not `DaemonState::new(":memory:")`)
to exercise the routing-aware path. The X1 bug (Y2 silently failed
in production) only surfaced because the routing layer was bypassed
in unit tests. The new test `p3_4_x1_compile_context_cwd_auto_creates_under_readonly_routing`
is the template.

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
| I-16 | HIGH | hook discards CC stdin JSON (CC voice Round 2 §C) | ✓ closed by Wave Y Y1 (`a03365b`) |
| I-17 | MED | auto-create rejects code-less dirs / no auto-created render (CC voice Round 2 §B) | ✓ closed by Wave Y Y2 (`b4078eb`) |
| I-18 | LOW | --version doesn't work (CC voice Round 2 §D) | ✓ closed by Wave Y Y4 (`3f7e7ff`) |
| I-19 | LOW-MED | project init silently overwrites (CC voice Round 2 §E) | ✓ closed by Wave Y Y5 (`d76b4c1`) |
| **I-20** | **MED-HIGH** | **auto-create write fails under read-only routing (CC voice Round 3 §B)** | **✓ closed by Wave X X1 (`97b6caf`)** |
| **I-21** | **HIGH** | **auto-create wipes existing row on path collision (Wave X dogfood)** | **✓ closed by Wave X fw1 (`cd6eb80`)** |
| **I-22** | **MED** | **concurrent-fresh-create race (Wave X review)** | **✓ closed by Wave X fw2 (`630e1c9`)** |
| **I-23** | **LOW** | **forge-daemon --version dynamic-linker fail on glibc<2.38 (CC voice Round 3 §C)** | **✓ closed by Wave X X2 (`880ad1f`)** |

## Auto-memory state (cross-session)

Saved across recent sessions (relevant to drain):
* `feedback_project_everywhere_vocabulary.md` — locked vocabulary direction (Wave Z)
* `feedback_xml_attribute_resolution_pattern.md` — `resolution=` attribute pattern (Wave Z; extended to `auto-created` by Y2)
* `feedback_decode_fallback_depth_floor.md` — informs #182 (now strategic-fixed)
* `feedback_dual_helper_basename_vs_reality.md` — informs #187
* `feedback_release_stack_deferred.md` — informs `#101` deferral
* `feedback_json_macro_silent_drift.md` — informs #185
* `feedback_sqlite_no_reverse_silent_migration_failure.md` — informs ZR
* `feedback_lazy_count_with_expand_call.md` — Y7 / Z static-prefix lazy load pattern
* `feedback_clap_conflicts_with_stack_overflow.md` — Y6 clap 4.x bug

**Memory candidates from Wave X (to add when current session closes):**

* **Read-only routing trap for handler arms with side effects.** Pattern:
  if `Request::Foo` is in `is_read_only()` (`crates/daemon/src/server/writer.rs:75`)
  AND the `Foo` handler does any `INSERT`/`UPDATE`/`DELETE`, the SQL silently
  fails because `state.conn` is opened with `SQLITE_OPEN_READ_ONLY`. Fix patterns:
  open ad-hoc writer from `state.db_path` (cheap one-shot) OR send a
  `WriteCommand::Raw` through `state.writer_tx` (proper serialization). Tests must
  build `DaemonState::new_reader(...)` not `DaemonState::new(":memory:")` to
  exercise the routing path.
* **`INSERT OR REPLACE` on tables with non-PK unique indexes is data-loss.** When
  an INSERT collides with a unique index that's NOT the PK, SQLite REPLACE
  removes the conflicting row (different PK value) before inserting the new one.
  For idempotent-create paths use `INSERT OR IGNORE` instead. The X1.fw2 split
  between `store_reality` (REPLACE — for explicit upserts by id) and
  `auto_create_reality_if_absent` (IGNORE — for race-safe creates) is the
  right pattern.

## Halt-and-ask map for the post-Wave-X window

1. **NO halt.** Per user direction, drain `#190 → #203` continuously. Per Plan A §6 still applies — adversarial review per behavior-change wave; fix-wave for HIGH+MED.
2. **Halt only on:** non-clean working tree across a wave boundary; review verdict `not-lockable`; surprise architectural blocker that needs user input (e.g. SIGTERM-graceful coord touches an external contract); cc-voice filing follow-up Round 4 feedback that supersedes the queued work.
3. **AFTER #203 closes:** halt for sign-off → open ZR (#215, internal reality→project rename) → halt for sign-off → open `#101` release stack.

## Daemon-binary state (end of session)

Released binary at `target/release/forge-daemon` rebuilt at HEAD `630e1c9`. Daemon respawned mid-session for live dogfood (PID confirmed at `c052a9b`); user can re-respawn with the standard drill in `feedback/2026-04-27-forge-team-round-3-response.md` §G.

## One-line summary

**HEAD `630e1c9`. This session: Wave X (cc-voice Round 3 unblock — #229-#232 + #236 + #237, 5 commits + response doc + 2 fix-waves + adversarial review). cc-voice Round 3 3/3 items + 1 dogfood-found HIGH (data loss) + 6 review findings all resolved. 15 drain items still queued (#190-#191, #193-#203, #215, #216-#219, #233, #238). Resume at #190 (Tier 1 SIGTERM-graceful JoinSet) when next session opens. Daemon live at git_sha `630e1c9`.**
