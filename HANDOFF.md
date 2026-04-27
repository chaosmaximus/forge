# Handoff — P3-4 Wave X (cc-voice Round 3) + Wave A+B (#190-#191) closed — 2026-04-27

**Public HEAD:** `13a9618` (Wave A+B / fix-wave).
**Working tree:** clean.
**Version:** `v0.6.0-rc.3 (13a9618)` (release stack still DEFERRED — Plan A §6 backlog drain in progress).
**Plan A:** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Halt:** none. Next item: **#193 (W1.32 — W28 LOW/NIT cosmetic batch — Wave C/C1 start)**.

## This session's deltas (9 commits)

### Wave X — cc-voice Round 3 unblock (5 commits + response doc + 2 fix-waves)

| Task | Commit | What |
|------|--------|------|
| X1 | `97b6caf` | Fix auto-create write path under read-only routing — open ad-hoc writer connection from `state.db_path` when `Request::CompileContext` is in `is_read_only()`. New routing-aware regression test + 4 stale Wave Y/Z tests fixed. (MED-HIGH — closes Round 3 §B.) |
| X2 | `880ad1f` | DT_RUNPATH bake-in for Linux binaries via `.cargo/config.toml` rustflags. Closes deferred `#220`. (LOW — closes Round 3 §C.) |
| X3 | `c052a9b` | Architecture doc `docs/architecture/project-domain-lifecycle.md` — "domain is hint, not lock" design. v0.6.1 follow-up tracked as `#233`. (DOC — closes Round 3 §E.) |
| X4 | response doc | `feedback/2026-04-27-forge-team-round-3-response.md` — disposition matrix + post-write fw1+fw2 §G. |
| **fw1** | `cd6eb80` | **HIGH (dogfood-found)** — pre-existing data loss: `INSERT OR REPLACE` on `idx_reality_path_unique` collision wiped the user's `project init` setup. Fix: gate auto-create on path absence too; tracing::warn on alias mismatch. |
| **fw2** | `630e1c9` | Adversarial review (verdict `lockable-with-fixes` — 0 BLOCKER / 0 HIGH / 1 MED / 5 LOW). MED-1 concurrent-fresh-create race → switched auto-create to new `auto_create_reality_if_absent` (`INSERT OR IGNORE`). LOWs 2/3/4/5 included. LOW-1 (writer_tx routing) deferred to backlog `#238`. |
| HANDOFF | `1d5109b` | Wave X close HANDOFF rewrite. |

### Wave A+B — drain start (#190-#191, 3 commits + adversarial review)

| Task | Commit | What |
|------|--------|------|
| **W1.29** (#190) | `bd0b3ca` | **SIGTERM-graceful JoinSet drain** for force-index (W23 HIGH-1 strategic close). New `crates/daemon/src/server/supervisor.rs` with `BackgroundTaskSupervisor` (CAS-based AtomicBool reject-overlap + `Mutex<JoinSet>` for in-flight tasks + `drain(deadline)`). `WriterActor` gains `bg: Arc<...>`. `process_force_index_async` claim-or-reject; `main.rs` shutdown drains before socket teardown. **5 unit tests.** |
| **W1.30** (#191) | `55edb92` | **Typed `dispatched: bool` flag** on `ResponseData::IndexComplete` (W23 MED-3) + canonical `crate::db::pragma::apply_runtime_pragmas` helper (W23 MED-4). 9 production sites swept; CLI keys off the typed flag instead of `(0,0)` heuristic. **2 unit tests + serde-default wire compat.** |
| **fw1** | `13a9618` | Adversarial review fixes (verdict `lockable-with-fixes` — 0 BLOCKER / 0 HIGH / 3 MED / 6 LOW). MED-1 missed PRAGMA sweep at `workers/mod.rs::open_read_conn` (5 worker callers); MED-2 `signal_shutdown` gate so late-arriving force-index requests are rejected before drain; MED-3 real-panic test replacing the prior fake `catch_unwind`; LOW-2 env-overridable drain timeout (`FORGE_DRAIN_TIMEOUT_SECS`, clamped [1,300]); LOW-3 doc clarification; LOW-4 `:memory:` warn suppression; LOW-6 perf-claim cleanup. **+3 new unit tests.** LOW-1 (CAS ordering cosmetic) and LOW-5 (read-only inline-vs-helper split) deferred. |

### Issue ledger updates

* **I-20** MED-HIGH (cc-voice Round 3 §B): auto-create write fails under read-only routing → ✓ closed by X1.
* **I-21** HIGH (Wave X dogfood): auto-create wipes existing row on path collision → ✓ closed by fw1.
* **I-22** MED (Wave X review): concurrent-fresh-create race → ✓ closed by fw2.
* **I-23** LOW (cc-voice Round 3 §C): `forge-daemon --version` dynamic-linker fail on glibc<2.38 → ✓ closed by X2.
* **I-24** HIGH (W23 carry-forward): SIGTERM mid-pass split-brain on force-index → ✓ closed by W1.29.
* **I-25** MED (W23 carry-forward): force-index dispatch ambiguity / PRAGMA drift → ✓ closed by W1.30.

## State in one paragraph

**HEAD `13a9618`. Wave X (#229-#232 + #236-#237) and Wave A+B (#190 + #191 + #239-#240) closed (9 commits + 2 adversarial reviews + response doc).** cc-voice Round 3 3/3 + 1 dogfood HIGH (data loss) + 6 review findings + W23 carry-forward HIGH-1 + 2 W23 MEDs all resolved. Doctor green. clippy 0 warnings; full daemon test suite at 1566/1566 (+10 new); harness-sync + protocol-hash + license-manifest + review-artifacts all OK. **11 drain items still pending (#193-#203, #215, #216-#219, #233, #238).** Resume at **#193 (W1.32 — W28 LOW/NIT cosmetic batch)** — first of Wave C/C1.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 13a9618
git status --short                                 # expect clean
forge-next doctor                                  # version + git_sha sanity
bash scripts/check-harness-sync.sh                 # all 4 sanity gates
bash scripts/check-protocol-hash.sh
bash scripts/check-license-manifest.sh
bash scripts/check-review-artifacts.sh

# Resume at #193 (W1.32 — W28 LOW/NIT cosmetic batch).
# Wave C plan (split for review-friendliness):
#   C1: #193 (W28 LOW/NIT cosmetics) + #198 (observe shape envelope)
#   C2: #195 (--help grouping) + #196 (CLI remember --valence/--intensity)
#   C3: #194 (force-index cold + WAL warns) + #197 (Phase 9b INFO log)
#         + #199 (forge-bench telemetry quiet)
# One adversarial review covers C1+C2+C3.
```

## Cumulative pending work

### Tier 3 — observability / UX cosmetic batch (Wave C — 7 items)

| Task | Subject |
|------|---------|
| **#193** | W1.32 — W28 LOW/NIT cosmetic batch (W28-LOW-2..LOW-10 + W28-NIT-1..NIT-3). |
| #194 | W1.33 — I-2+I-3 force-index cold latency + WAL "database is locked" warns. |
| #195 | W1.34 — I-6 `forge-next --help` grouping via `clap::next_help_heading`. |
| #196 | W1.35 — I-9 CLI `remember` exposes `--valence`/`--intensity` flags. |
| #197 | W1.36 — I-10 Phase 9b dedicated INFO log. |
| #198 | W1.37 — I-11 `forge-next observe` shape schema uniformity (common envelope). |
| #199 | W1.38 — I-12 `forge-bench` standalone telemetry warn quiet (downgrade or `--telemetry` flag). |

### Tier 5 — nice-to-haves (Wave D — 3 items)

| Task | Subject |
|------|---------|
| #200 | W1.39 — W29/W30 nice-to-haves: bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail; optional `memory.require_project = true` config gate. |
| #201 | W1.40 — W31 drift fixture for contradiction false-positive surface. |
| #202 | W1.41 — W32 `notify::Watcher` event-driven freshness gate. |

### Tier 6 — pre-iteration backlog re-evaluation (Wave D umbrella — 1 item)

| Task | Subject |
|------|---------|
| #203 | W1.42 — walk 9 pre-iteration deferrals: 2A-4d.3 T17 (BLOCKED on GHA billing); longmemeval/locomo re-run; SIGTERM/SIGINT chaos drill modes; criterion latency benchmarks; Prometheus bench composite gauge; multi-window regression baseline; manual-override label; P3-2 W1 trace-handler behavioral test gap; per-tenant Prometheus labels; OTLP timeline panel. Decide fix-or-permanently-defer per item with rationale. |

### Wave Z + Y + X deferred (review residue) (5 items)

* **#216** — Wave Z MED-1: SessionUpdate TOCTOU error-message hygiene.
* **#217** — Wave Z MED-3: `forge-next project rename / delete / relocate` (cc-voice Round 3 §C-3 walking-up TOML lands here; covers fw1 alias-mismatch escape valve too).
* **#218** — Wave Z LOW-2: doctor backup hygiene XDG_DATA_HOME / Docker paths.
* **#219** — Wave Z LOW-3: cc-voice §1.2 end-to-end integration test.
* **#238** — Wave X LOW-1 (deferred): route compile-context auto-create through `writer_tx` (architectural; v0.6.1+).

### v0.6.1 follow-ups

* **#233** — domain="unknown" → real-domain upgrade in indexer per `docs/architecture/project-domain-lifecycle.md`. Small UPDATE in `workers/perception.rs` or `workers/indexer.rs` with SQL guard `WHERE domain = 'unknown'`. Test contract pinned in the doc.

### Deferred internal cleanup

* **#215 — ZR — internal rename pass.** `Reality` Rust struct → `Project`,
  `mod reality` → `mod project`, SQL `reality` table → `project` (with
  migration + regression test per the SQLite-no-REVERSE memory). Delete
  dead `code_engine.rs::context_section`. **Open after #203 closes.**

### Final wave

* **#101 — P3-4 release v0.6.0 stack.** DEFERRED — opens after `#215` closes.

## TaskList structure (post-Wave-A+B)

| Range | Subject | Status |
|---|---|---|
| #190 | W1.29 SIGTERM-graceful JoinSet | **completed (this session)** |
| #191 | W1.30 typed dispatched + PRAGMA helper | **completed (this session)** |
| #229 .. #232 | Wave X (cc-voice Round 3) | all completed (this session) |
| #234 .. #237 | Wave X review + fix-waves | all completed (this session) |
| #239 .. #240 | Wave A+B review + fix-wave | all completed (this session) |
| **#193** | **W28 LOW/NIT cosmetic batch** | **← next session resume** |
| #194 .. #199 | Tier 3 cosmetic continuation | pending |
| #200 .. #203 | Tier 5 + umbrella | pending |
| #215 | ZR internal rename | pending — opens after #203 |
| #216-#219, #238 | deferred review residue | pending — v0.6.1+ |
| #233 | v0.6.1 indexer domain upgrade | pending — v0.6.1 |
| #101 | release v0.6.0 stack | DEFERRED — opens after #215 |

## Halt-and-ask map for the post-Wave-A+B window

1. **NO halt.** Per user direction, drain `#193 → #203` continuously. Per Plan A §6 still applies — adversarial review per behavior-change wave; fix-wave for HIGH+MED.
2. **Halt only on:** non-clean working tree across a wave boundary; review verdict `not-lockable`; surprise architectural blocker that needs user input; cc-voice filing follow-up Round 4 feedback that supersedes the queued work.
3. **AFTER #203 closes:** halt for sign-off → open ZR (`#215`, internal reality→project rename) → halt for sign-off → open `#101` release stack.

## Auto-memory state (cross-session)

Saved across recent sessions:
* `feedback_project_everywhere_vocabulary.md` — Wave Z user-facing vocabulary lock
* `feedback_xml_attribute_resolution_pattern.md` — `resolution=` attribute pattern (extended to `auto-created` by Y2)
* `feedback_decode_fallback_depth_floor.md` — informs W1.21 (now strategic-fixed)
* `feedback_dual_helper_basename_vs_reality.md` — informs W1.26
* `feedback_release_stack_deferred.md` — informs `#101` deferral
* `feedback_json_macro_silent_drift.md` — informs W1.24
* `feedback_sqlite_no_reverse_silent_migration_failure.md` — informs ZR
* `feedback_lazy_count_with_expand_call.md` — Y7 / Z static-prefix lazy load
* `feedback_clap_conflicts_with_stack_overflow.md` — Y6 clap 4.x bug
* `feedback_readonly_routing_trap_for_side_effecting_handlers.md` — X1 root cause + test fixture rule
* `feedback_insert_or_replace_data_loss_on_unique_index.md` — X1.fw1 + fw2 split

**Memory candidates from this session (W1.29 + W1.30 + their fix-wave):**

* **`BackgroundTaskSupervisor` pattern for fire-and-forget blocking writes.** When adding a future heavy-write feature (analytical batch, multi-table backfill, bench-corpus rebuild), use the supervisor: per-resource `AtomicBool` reject-overlap + `signal_shutdown` gate + `spawn_supervised` into the JoinSet. Drain on shutdown via `bg_supervisor.drain(timeout)`. The release MUST fire in ALL completion paths (Ok/Err/panic) — production supervisor closure structure mirrors `process_force_index_async` at `crates/daemon/src/server/writer.rs`.
* **Adding a field to a tagged `ResponseData` enum is wire-back-compatible via `#[serde(default)]`.** Old daemon → new CLI: serde defaults the missing field (false for bool, 0 for usize, etc.). New daemon → old CLI: serde silently ignores unknown fields. So adding `dispatched: bool` to `IndexComplete` doesn't bump protocol_hash (Request hash, not Response). The CLI heuristic that the field replaces (e.g. `(0,0)` → "dispatched") should be dropped at the same time on the new-CLI side.
* **PRAGMA helper as drift gate.** Sweeping `PRAGMA journal_mode=WAL; PRAGMA busy_timeout={varies}` literals into `crate::db::apply_runtime_pragmas(&conn)` enforces a single source of truth for `BUSY_TIMEOUT_MS` (10s canonical). Read-only handles can't engage WAL — keep them inline using the same const.

The `feedback_writer_actor_spawn_blocking.md` memory's W23 HIGH-1 carry-forward is now resolved by W1.29; should be updated next session (low priority).

## Daemon-binary state (end of session)

Daemon was respawned mid-session at `c052a9b` for live dogfood (Wave X §B reproduction); next session should rebuild release at HEAD `13a9618` and respawn for Wave C dogfood when feasible.

## One-line summary

**HEAD `13a9618`. This session: Wave X (cc-voice Round 3 unblock, 5 commits + response doc + 2 fix-waves) + Wave A+B (#190 SIGTERM-graceful supervisor + #191 typed dispatched + PRAGMA helper, 2 strategic commits + 1 fix-wave + 2 adversarial reviews). 9 commits total. 11 issue-ledger items resolved (I-20..I-25 + W23 HIGH-1 carry-forward). 11 drain items still queued (#193-#203 + #215 + deferred). Resume at #193 (Wave C/C1) when next session opens.**
