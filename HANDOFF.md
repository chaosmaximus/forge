# Handoff — Phase 2A-4d backlog drained (2026-04-25, post-W8)

**Public HEAD:** `17f3436`.
**forge-app master:** `665c372c7c461016a8b5953d91e792b7b7221636` (unchanged).
**Current version:** **v0.5.0**.

## State in one paragraph

This session continued the autonomous backlog drain through Waves 5–8,
closing every actionable observability-backlog item that was still tagged
"deferred" after Phase B's W1–W4. **W5** closed all 5 #3-review HIGHs
(scoped-config wiring, compile_context_trace honoring inj, dynamic
layers_used, compose-direction doc, BlastRadius CLI message). **W6**
shipped the highest-risk single item — Tier 1 #1 — by giving the
consolidator its own SQLite connection so the shared
`Arc<Mutex<DaemonState>>` no longer gates worker progress during 2–30s
passes. **W7** added a real `kpi_events.run_id` column + index for the
HUD 24h rollup, with a backfill UPDATE for existing rows; a follow-up
fix disambiguated SQL alias shadowing in `shape_phase_run_summary` and
the HUD query via COALESCE. **W8** swept two items from the cosmetic
batch (compile-time-tautology markers, kpi_reaper log downgrade). Net
result: **6 commits** on top of the prior `d319e98` baseline; **1506
daemon-lib tests pass + 1 documented timing flake**; 0 clippy warnings
on the workspace `--features bench` gate; fmt clean. Adversarial reviews
on W5 and W6 returned `lockable-with-fixes` and `lockable-as-is`
respectively; both follow-ups landed.

The biggest live behavior changes since `d319e98`:

1. **Consolidator no longer holds the state mutex across passes** —
   workers (perception, indexer, diagnostics, writer) that share the
   mutex are no longer blocked during the multi-second consolidation
   pass. Major operator-visibility win when consolidator runs are
   frequent.
2. **`/inspect` and the HUD 24h rollup are JSON-parse-free for run_id**
   — the new `run_id TEXT` column with `idx_kpi_events_run_id_timestamp`
   lets the planner walk the index instead of re-extracting JSON for
   every 24h-window row.
3. **Per-organization context-injection toggles work** —
   `forge-next config set context_injection.<flag> true|false --scope
   organization=acme` now actually takes effect; the resolver walks the
   org → team → user → reality → agent → session chain on every
   CompileContext request, with the global config as the baseline.
4. **`compile_context_trace` honors `context_injection.session_context`**
   — operators using the trace shape no longer see misleading
   "considered/included" entries for sections that the actual compile
   would suppress.

## First actions after `/compact`

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                                                     # expect 17f3436 at top
git status --short                                                        # expect clean
export LD_LIBRARY_PATH="$(pwd)/.tools/onnxruntime-linux-x64-1.23.0/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
cargo clippy --workspace --features bench -- -W clippy::all -D warnings   # 0 warnings
cargo test -p forge-daemon --lib --features bench                         # 1506 pass + 1 known flake
bash scripts/ci/check_spans.sh                                            # OK
```

## Session commits — 6 total (most recent first)

| #   | SHA       | Wave | Title |
|-----|-----------|------|-------|
|  6  | `17f3436` | W7-fix | fix(2A-4d.2.1 #4 W7 follow-up): disambiguate run_id column shadowing |
|  5  | `11aa2a1` | W8 | docs(2A-4d.3.1 #6): W8 cosmetic batch — M4 + L3 |
|  4  | `f160bc4` | W7 | feat(2A-4d.2.1 #4): add kpi_events.run_id column + index + HUD query |
|  3  | `3c979ee` | W6 | refactor(2A-4d.1.1 #1): consolidator owns its own SQLite connection |
|  2  | `59a84df` | W5-fix | fix(2A-4d.3.1 #3 W5 review): pass global config to resolver + clarify trace scope |
|  1  | `3ee65f8` | W5 | fix(2A-4d.3.1 #3): close W5 — H1+H2+H3+H4+H5 review HIGHs |
| (carryover) | `d319e98` | — | docs(2A-4d): close Phase B autonomous run — HANDOFF rewrite |

## What shipped — by item

### Wave 5 — close 5 of the 5 deferred #3-review HIGHs

* **H1 — scoped-config wiring.** New
  `config::resolve_context_injection_for_session(conn, session_id,
  agent, &global)` walks the org → team → user → reality → agent →
  session chain via `db::ops::resolve_scoped_config` for each of the
  6 toggles. Boolean values parsed `eq_ignore_ascii_case("true"|
  "false")` with global-fallback on parse failure (debug-logged).
  CompileContext arm uses this instead of `config.context_injection
  .clone()`. With no session anchor, collapses to the global config
  (prior behavior). The W5 review spotted that the original draft
  re-loaded config inside the resolver, defeating H6's "load once"
  invariant — fix passes the global as a parameter.

* **H2 — compile_context_trace honors `inj` flags.** Loads
  `ContextInjectionConfig` at the top, gates the decisions and
  lessons blocks on `inj.session_context`. When gated off, surfaces
  a single synthetic `<gated>` excluded entry per layer so operators
  see the suppression in the trace without leaking the gated
  contents. Docstring caveats that the trace fn sees only the GLOBAL
  config (CompileContextTrace has no session_id) — closing that gap
  needs a protocol change tracked as follow-up.

* **H3 — `layers_used: 4`/`9` hard-coded constants** replaced with
  `recall::count_layers_used(inj, static_only)`. Static-only count
  reflects platform+tools (always) plus identity+disposition (gated
  on `session_context`). Full count adds decisions+lessons
  (`session_context`), skills (`skills`), perceptions+working-set
  (`active_state`). The `context_compiled` event payload and
  `CompiledContext.layers_used` response field now match what the
  consumer actually sees.

* **H4 — compose-direction doc note.** New comment block on
  `section_disabled` documents the OR semantics — operator-disable
  wins via either `excluded_layers` membership OR a falsy `inj
  .<flag>`. There is NO force-include semantic; per-request the
  only verb is "disable further". Operators set coarse policy via
  `context_injection`; per-request `excluded_layers` refines down.

* **H5 — BlastRadius CLI suppress message** replaced the bare
  "blast_radius injection suppressed" notice with an actionable form
  that tells the operator how to re-enable
  (`forge-next config set` or `FORGE_CONTEXT_INJECTION_BLAST_RADIUS`),
  flags that the analysis was not run, and points at the daemon-side
  gate so they don't grep the source.

### Wave 6 — Tier 1 #1 consolidator owns its own connection

* `crates/daemon/src/workers/consolidator.rs::run_consolidator` —
  new signature `(db_path, events, metrics, shutdown_rx,
  interval_secs)`. Initializes sqlite-vec, opens its own
  `Connection`, applies `WAL` + `busy_timeout=5000` pragmas, then
  loops on the consolidation interval. Connection held for the
  worker's lifetime — no re-open per pass.
* `crates/daemon/src/workers/mod.rs::spawn_workers` gains a
  `metrics: Option<Arc<ForgeMetrics>>` parameter and clones it +
  `events` + `db_path` into the consolidator handle.
* `crates/daemon/src/main.rs` passes
  `Some(Arc::clone(&shared_metrics))` to spawn_workers.
* SQLite WAL allows multiple readers + a single writer — the
  consolidator's owned writer connection contends with at most the
  writer actor at a time, and `busy_timeout=5000` absorbs that
  contention. Startup-consolidation in `main.rs` still uses the
  brief-lock pattern (one phase at a time) — intentionally untouched.

### Wave 7 — Tier 2 #4 HUD 24h rollup index

* `crates/daemon/src/db/schema.rs` migration:
  `ALTER TABLE kpi_events ADD COLUMN run_id TEXT;`
  one-time `UPDATE` to backfill from `json_extract(metadata_json,
  '$.run_id')` on existing rows;
  `CREATE INDEX idx_kpi_events_run_id_timestamp ON kpi_events(run_id,
  timestamp)`.
* `workers/instrumentation.rs::record` writes `outcome.run_id` to
  the column (and keeps it in metadata_json for v1-schema
  compatibility).
* `bench/telemetry.rs::emit_bench_run_completed` populates `run_id`
  with the same Ulid that doubles as the kpi_events PK.
* `events.rs` HUD 24h rollup query reads
  `COUNT(DISTINCT COALESCE(run_id, json_extract(metadata_json,
  '$.run_id')))` so the indexed column wins for production rows and
  the JSON value remains a fallback for legacy or test-fixture rows.
* W7 follow-up: `shape_phase_run_summary` had its
  `json_extract(...) AS run_id` alias shadowed by the new column,
  causing `GROUP BY run_id` / `HAVING run_id IS NOT NULL` to evaluate
  against the column value (NULL for test fixtures) and drop every
  group. Renamed the alias to `effective_run_id` and sourced via
  COALESCE.

### Wave 8 — cosmetic batch (#6 M4 + L3)

* M4 — `compile-time-tautology:` prefix on `detail` strings for the
  bench infra checks #3 (preference_table_schema) and #4
  (skill_table_schema) so operators reading the dashboard see
  "passed: true" tagged with honest semantics.
* L3 — `workers/kpi_reaper.rs` per-type pass-start log downgraded
  from `info!` to `debug!` (it fires once per configured override
  per pass regardless of whether any rows are reapable, flooding
  info-level logs).
* The other 5 cosmetic items (M1 #[serial], M2 git-cluster, M3
  chrono swap, L1/L2 cast nits) and Tier 1 #2 record() span scope
  remain deferred per the original "batch when bench harness sees
  major-version polish" / "wait for phase-span UI" rationales.

## Tests + verification (final state)

* `cargo fmt --all --check` — clean
* `cargo check --workspace --features bench` — clean
* `cargo clippy --workspace --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo test -p forge-daemon --lib --features bench` — **1506 pass, 1 fail, 1 ignored**
  (the 1 fail = `test_daemon_state_new_is_fast` — pre-existing flake unchanged)
* `bash scripts/ci/check_spans.sh` — OK (23 names matched, 0 violations)
* W5 adversarial review: `lockable-with-fixes` → M1 (resolver re-loading config) +
  M3 (trace docstring scope) addressed in `59a84df`.
* W6 adversarial review: `lockable-as-is` (verified WAL semantics + busy_timeout
  symmetry + sqlite-vec global init + no lock-ordering regression).
* No live dogfood this session — code-only changes; the test suite + structural
  reviews + integrity tests are the verification surface.

## Deferred backlog — what's still open

Single source of truth:
**`docs/superpowers/plans/2026-04-24-forge-identity-observability.md`**.

### Tier 1 (2A-4d.1.1) — 3 still open

* **#2 record() inside span scope** (22 sites). Phase 19 already
  refactored to call record() AFTER the span scope drops. The
  remaining 22 phases still call record() inside the span. **Why
  deferred:** zero user-visible benefit until a Tier 4+ surface
  attributes instrumentation-layer errors per phase by name. HUD
  shows aggregate, not per-phase, today.
* **#5 T10 OTLP-path latency variant** — separate latency study
  with its own numbers. No Tier 2/3 path constructs a real OTLP
  exporter to substitute.

(Tier 1 #1 closed in W6; #3 + #4 closed in W3.)

### Tier 2 (2A-4d.2.1) — 0 still open

All 7 items closed across W2 + W7.

### Tier 3 (2A-4d.3.1) — partially still open

* **#3 review M2** — six independent `resolve_scoped_config` calls
  per request could batch via `resolve_effective_config` (already
  exists in db/ops.rs). Defer until profiling shows it matters.
* **#3 review M3 protocol gap** — `Request::CompileContextTrace`
  has no `session_id`, so per-scope overrides don't reach the
  trace fn. Closing requires a protocol change.
* **#6 cosmetic batch** — 5 of 7 items still open: M1 #[serial]
  test mark, M2 git-cluster, M3 chrono swap for civil_from_days,
  L1 i64::from cast style, L2 u32::try_from cast style. **Why
  deferred:** "batch when bench harness sees major-version
  operator polish" — that condition still hasn't surfaced.

## Known quirks

* `test_daemon_state_new_is_fast` remains the documented timing
  flake (pre-existing since 2P-1a). Unchanged.
* The W7 column addition created a SQL alias-shadowing trap that
  cost one follow-up commit (`17f3436`). Future schema changes that
  introduce columns matching JSON-extract alias names should audit
  every shape SQL for the same pattern (`AS <colname>` collisions).
* Rust-analyzer often shows stale `cfg(feature = "bench")` diagnostics
  during incremental edits — `cargo check --workspace --features
  bench` is the ground truth.

## Next — recommended path

The 2A-4d daemon-side observability backlog is now structurally
empty modulo the explicitly-deferred items (Tier 1 #2/#5, Tier 3
review M2/M3, cosmetic #6). Recommended next directions:

1. **2P-1b harness hardening** — the public-resplit spec
   (`docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md`)
   carved out a 2P-1b phase covering harness-sync CI checks (daemon ↔
   plugin ↔ hooks ↔ skills ↔ agents in lockstep), multi-OS dogfood
   matrix (incl. macOS), marketplace republication ownership,
   2A-4d interlock (link bench-fast CI to daemon changes),
   rollback playbook, and SPDX header backfill for JSON files. None
   started; whole new domain.
2. **Tier 1 #2 record() span-scope refactor** if/when Tier 4+
   surfaces phase-span-by-name UI. 22 mechanical sites; phase 19 is
   the reference pattern.
3. **Tier 3 review M2 / M3 protocol surface** — batch
   `resolve_effective_config` and add `session_id` to
   `Request::CompileContextTrace` if operator usage of trace
   warrants it.

## Parked (manual / temporal)

* v0.5.0 GitHub release + tag push.
* Marketplace publication.
* macOS dogfood.
* T17 — bench-fast CI gate promotion to required (after 14
  consecutive green master CI runs).

## One-line summary

HEAD `17f3436`; W5–W8 close every actionable Phase 2A-4d backlog
item; consolidator no longer locks state mutex across passes;
`/inspect` and HUD rollup index-backed via `kpi_events.run_id`;
scoped-config overrides take effect end-to-end; 1506 tests green,
0 clippy warnings; recommended next: 2P-1b harness hardening or
the smaller Tier 1 #2 / Tier 3 M2-M3 follow-ups.
