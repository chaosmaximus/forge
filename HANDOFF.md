# Handoff — Tier 3 Bench Harness complete (2026-04-25, pre-compact)

**Public HEAD (chaosmaximus/forge):** `8dccf58`.
**forge-app master:** `665c372c7c461016a8b5953d91e792b7b7221636` (unchanged this session).
**Current version:** **v0.5.0** — not tagged on GitHub (parked until product complete).

## State in one paragraph

Phase **2A-4d.3 (Bench harness)** is **COMPLETE except for T12 calibration**, which is intentionally deferred per user directive (path B in the close-out discussion). 19 commits this session (`250cb1a..8dccf58`), all on master. Ships: `forge-bench forge-identity` CLI subcommand emitting per-dim scores + composite + pass/fail with non-zero exit on FAIL; 6-dimension scorer in `crates/daemon/src/bench/forge_identity.rs` with 5 of 6 dimensions calibrated to ≥0.996 (Dim 2 disposition_drift is a stub blocked on backlog #2 missing `Request::StepDispositionOnce`); 14 infrastructure assertions per master v6 §6 (run BEFORE dimensions, fail-fast); per-dim DaemonState isolation (T15.3 master v6 §13 D7 fix); `bench_run_completed` v1 kpi_events event written by `crates/daemon/src/bench/telemetry.rs::emit_bench_run_completed` (WAL+busy_timeout connection, idempotent schema init, no-op when FORGE_DIR unset); `Request::Recall.query_embedding` bench-gated extension; `InspectShape::BenchRunSummary` + `BenchRunRow` with `shape_bench_run_summary` two-pass aggregation; `window_cap_secs_for_shape` per-shape window-cap helper (D8 amendment to Tier 2 — 7d for existing shapes, 180d for BenchRunSummary); `kpi_events_retention_days_by_type` per-event-type retention (D9 — `bench_run_completed`=180d default, others=30d global); `.github/workflows/ci.yml` `bench-fast` job with matrix [forge-consolidation, forge-identity], continue-on-error: true, ORT cache, summary-only artifact upload (no forge.db); adversarial review pair on T1-T13 (Claude verdict needs-section-rewrite — 3 BLOCKERs + 5 HIGHs); T15 closed all BLOCKERs + HIGHs via 3 commits (T15.1 docs alignment, T15.2 small fixes, T15.3 per-dim isolation refactor); end-to-end dogfood verified composite=0.849, exit=1 on FAIL, kpi_events row written with v1 schema. **1,470 daemon-lib tests pass; 0 clippy warnings both profiles; cargo fmt clean.** Live dogfood doc at `docs/benchmarks/results/2026-04-25-forge-identity-observability-T3.md`.

T12 calibration deferred to 2A-4d.3.1; the bench's 0.849 ceiling is honest (Dim 2 stub) and the leaderboard infrastructure is fully usable today.

## First actions after `/compact`

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -1                                                # expect 8dccf58
git status --short                                                  # expect clean
export LD_LIBRARY_PATH="$(pwd)/.tools/onnxruntime-linux-x64-1.23.0/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
cargo clippy --workspace --features bench -- -W clippy::all -D warnings    # 0 warnings
cargo test -p forge-daemon --lib --features bench bench::forge_identity   # 11 pass
cargo test -p forge-daemon --lib --features bench bench::telemetry        # 5 pass
cargo test -p forge-daemon --lib workers::kpi_reaper                      # 8 pass
cargo test -p forge-daemon --lib --features bench server::inspect         # 36 pass
```

Live dogfood (optional; requires fresh FORGE_DIR):
```bash
mkdir -p /tmp/fi-resume && rm -rf /tmp/fi-resume/* && \
  FORGE_DIR=/tmp/fi-resume FORGE_HARDWARE_PROFILE=local \
  ./target/release/forge-bench forge-identity --seed 42 --output /tmp/fi-resume/out
# expect: composite=0.849, exit 1, kpi_events has 1 bench_run_completed row.
```

If green, resume with **2A-4d.3.1 backlog item #2** (`Request::StepDispositionOnce` — unblocks Dim 2 → unblocks T12 calibration → unblocks composite ≥ 0.95). Or: pick a different backlog item.

## Session commits (most recent first)

| #  | SHA       | Title |
|----|-----------|-------|
| 19 | `8dccf58` | fix(2A-4d.3 T15.3): per-dim DaemonState isolation (B1 + B2) |
| 18 | `a436781` | fix(2A-4d.3 T15.2): T14 H1 + H3 + H5 — small fixes |
| 17 | `fea74fd` | docs(2A-4d.3 T15.1): register bench_run_completed in kpi_events-namespace.md (B3 + H4) |
| 16 | `f429694` | feat(2A-4d.3 T13): CI bench-fast job |
| 15 | `0c18fa7` | feat(2A-4d.3 T11): per-event-type kpi_events retention (D9) |
| 14 | `7df0727` | docs(2A-4d.3 T9): register bench_run_completed v1 in events-namespace.md |
| 13 | `e93d84b` | feat(2A-4d.3 T8): bench telemetry — emit_bench_run_completed + 6 wiring sites |
| 12 | `5ede10b` | fix(2A-4d.3 T7): forge-identity exits non-zero on FAIL |
| 11 | `eba3169` | feat(2A-4d.3 T7): forge-bench forge-identity subcommand |
| 10 | `d6399ab` | fix(2A-4d.3.1 #1): always-emit `<preferences>` in CompileContext XML |
|  9 | `20864c8` | docs(2A-4d.3.1): add backlog — infra #10, Dim 2 blocker, harness ergonomics |
|  8 | `5d9e28c` | feat(2A-4d.3 T5): Dim 5 behavioral skill inference |
|  7 | `a565fa9` | feat(2A-4d.3 T4): Dim 4 valence flipping correctness |
|  6 | `40d5214` | feat(2A-4d.3 T10): InspectShape::BenchRunSummary + per-shape window cap (D8) |
|  5 | `09a37a7` | feat(2A-4d.3 T3+T6): Request::Recall query_embedding + Dim 1/3/6 + infra |
|  4 | `7ed232e` | feat(2A-4d.3 T2): forge_identity.rs skeleton + 6 dim stubs + composite scorer |
|  3 | `53a88ed` | docs(2A-4d.3 T1): recon addendum + wall-clock measurement |
|  2 | `adbecf2` | docs(2A-4d.3 T1): Tier 3 bench harness v2 design spec locked |
|  1 | (carryover) `250cb1a` chore: handoff — 2A-4d.2 Observability API complete |

19 commits this session; one carryover from prior 2A-4d.2 close-out.

## What shipped in Tier 3 (2A-4d.3)

### New files

- `crates/daemon/src/bench/forge_identity.rs` — 1818 LoC. 6 dim scorers, 14 infra assertions, run_bench orchestrator with per-dim DaemonState isolation, composite scorer. Gated `#[cfg(feature = "bench")]`.
- `crates/daemon/src/bench/telemetry.rs` — ~340 LoC. `emit_bench_run_completed` helper + payload structs + commit-metadata + hardware-profile detection. Gated `#[cfg(feature = "bench")]`.
- `crates/daemon/tests/forge_identity_harness.rs` — integration test stub.
- `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier3-design.md` — Tier 3 v2 LOCKED design.
- `docs/superpowers/specs/2026-04-24-forge-identity-observability-tier3-design-recon.md` — T1 recon addendum.
- `docs/benchmarks/results/2026-04-25-forge-identity-observability-T3.md` — dogfood results.

### Modified files

- `crates/core/src/protocol/inspect.rs` — `InspectShape::BenchRunSummary` variant; `InspectGroupBy::{BenchName, CommitSha, Seed}`; `InspectFilter` gains `bench_name` + `commit_sha` (#[serde(default)]); `InspectData::BenchRunSummary { rows: Vec<BenchRunRow> }`; new `BenchRunRow` 9-column struct.
- `crates/core/src/protocol/request.rs` — `Request::Recall` gains unconditional `query_embedding: Option<Vec<f32>>` field with `#[serde(default)]`.
- `crates/core/src/protocol/contract_tests.rs` — Recall variants updated for query_embedding; bench_run_summary added to parameterized + raw-JSON catalogs.
- `crates/daemon/src/server/inspect.rs` — `window_cap_secs_for_shape(shape)` helper (D8); `parse_window_secs` takes shape param + parameterized error message; `resolve_group_by` matrix accepts BenchName/CommitSha/Seed only for BenchRunSummary; `effective_filter` honors bench_name + commit_sha; `shape_bench_run_summary` SQL aggregation with two-pass percentile; `run_inspect` dispatch.
- `crates/daemon/src/server/handler.rs` — Recall arm threads query_embedding only under `#[cfg(any(test, feature = "bench"))]`; `let _ = query_embedding;` discard on production path.
- `crates/daemon/src/server/tier.rs`, `writer.rs`, `system.rs`, `rbac.rs`, `memory.rs` (CLI), and ~12 test files — query_embedding initializer added to all `Request::Recall` call sites.
- `crates/daemon/src/bench/mod.rs` — gates `forge_identity` and `telemetry` modules on `feature = "bench"`.
- `crates/daemon/src/bench/forge_consolidation.rs`, `forge_context.rs` — telemetry emit at completion.
- `crates/daemon/src/bin/forge-bench.rs` — `Commands::ForgeIdentity` variant; `run_forge_identity` dispatcher; telemetry emit in all 6 run_* fns; non-zero exit on FAIL.
- `crates/daemon/Cargo.toml` — `[[bin]] forge-bench` gains `required-features = ["bench"]` so default builds skip it; deps added: `ulid = "1"`.
- `crates/daemon/src/config.rs` — `WorkerConfig.kpi_events_retention_days_by_type: HashMap<String, u32>` with default `{"bench_run_completed": 180}`; `validated()` clamps 1..=365 and drops 0-valued entries (T15.2 H5).
- `crates/daemon/src/workers/kpi_reaper.rs` — `reap_once` takes 3-arg signature; two-pass DELETE: per-event-type pass + global pass for unkeyed event_types.
- `crates/daemon/src/workers/mod.rs` — spawn site updated to thread the retention map through.
- `crates/daemon/src/recall.rs` — backlog #1 fix: always-emit `<preferences>` element (self-closing `<preferences/>` when empty).
- `crates/cli/src/commands/observe.rs` — ObserveShape + ObserveGroupBy mirror BenchName/CommitSha/Seed; shape-aware validate_window; BenchRunRow ASCII table renderer; new test `bench_run_summary_accepts_180d_rejects_200d`.
- `crates/cli/src/commands/memory.rs`, `system.rs` — Recall query_embedding init.
- `docs/architecture/events-namespace.md` — bench_run_completed v1 broadcast registration + per-bench `dimensions[].name` registry.
- `docs/architecture/kpi_events-namespace.md` — bench_run_completed v1 SQL-table registration; `event_schema_version` canonicalized; legacy `metadata_schema_version` compat note for `phase_completed`.
- `docs/superpowers/plans/2026-04-24-forge-identity-observability.md` — 2A-4d.3.1 backlog grew from 4 to 6 items (added T14 H2 + cosmetic batch).
- `.github/workflows/ci.yml` — `bench-fast` job (matrix forge-consolidation + forge-identity, continue-on-error:true, retention-days:14, no forge.db).
- `docs/cli-reference.md`, `docs/api-reference.md`, `skills/forge-observe.md` — D8 per-shape ceiling notes.

## Tests + verification (final state)

- `cargo fmt --all --check` — clean
- `cargo check --workspace` — clean (forge-bench skipped via required-features)
- `cargo check --workspace --features bench` — clean
- `cargo clippy --workspace -- -W clippy::all -D warnings` — 0 warnings
- `cargo clippy --workspace --features bench -- -W clippy::all -D warnings` — 0 warnings
- `cargo test -p forge-daemon --lib --features bench` — **1470 pass, 0 fail, 1 ignored** (pre-existing `test_daemon_state_new_is_fast` flake)
- `cargo test -p forge-daemon --test forge_identity_harness --features bench` — 1 pass
- Live dogfood — composite=0.849, exit=1 on FAIL, 1 kpi_events row written.

## Deferred backlog (tracked)

Single source of truth: **`docs/superpowers/plans/2026-04-24-forge-identity-observability.md`** — three backlog sections.

### 2A-4d.1.1 — 5 items from Tier 1 (untouched this session)
1. Codex MEDIUM — consolidator state Mutex across 23 phases.
2. Claude HIGH-4 T8 — `record()` inside span scope.
3. Claude HIGH-5/6 T12 — CI guard raw strings + cfg(all(test, …)).
4. Claude MEDIUM-10 T12 — integrity test substring match.
5. Claude MEDIUM-9 T12 — T10 OTLP exporter exercise.

### 2A-4d.2.1 — 7 items from Tier 2 (untouched this session)
1. `/inspect row_count` lazy-refresh Arc plumb.
2. SSE filter `?events=consolidate_pass_completed` returned 0 events bug.
3. HUD I/O refactor (spawn_blocking + atomic write).
4. HUD 24h rollup not index-backed.
5. Percentile convention surfaced in API docs.
6. `shape_latency` truncation off-by-one.
7. CLI ObserveShape mirror vs forge-core ValueEnum feature flag.

### 2A-4d.3.1 — 6 items from Tier 3 (this session)
1. ✅ **CLOSED** in `d6399ab` — recall.rs always-emit `<preferences>`.
2. **Dim 2 disposition drift — `Request::StepDispositionOnce` variant missing.** Composite caps at 0.849 until landed. **This unblocks T12 calibration.**
3. **forge-next config knob to suppress context-injection per-session/project.** Operator-ergonomics raised by user.
4. **Sub-agent commit-discipline failures.** Investigate forge-generator harness; require explicit `git log -1 --format=%H` verification turn.
5. **shape_bench_run_summary percentile cap pulls all rows into Rust** (T14 H2). Reopen at >10k rows per window.
6. **Cosmetic batch** — 4 MEDIUM (M1-M4) + 3 LOW (L1-L3) from T14 review. Single cleanup PR when convenient.

## Next — 2A-4d.3.1 (path of least friction)

Two paths:

1. **Land item #2 (StepDispositionOnce) → unblocks T12 calibration → composite ≥ 0.95.** Single sequential agent. ~600 LoC: new bench-gated Request variant + handler arm + disposition step-fn exposure + Dim 2 body. Then T12 5-seed calibration.
2. **Land item #4 (sub-agent commit-discipline)** — investigate the forge-generator harness. Smaller. No code change unless we add a verification turn to the agent prompt template.

Recommend item #2 (StepDispositionOnce + T12 calibration) for the next big push — it's the only thing standing between Tier 3 + the master v6 1.0 composite gate.

## Known quirks / state

- `test_daemon_state_new_is_fast` remains a pre-existing timing flake on heavy workspaces (~3s threshold vs ~200ms isolated). Not related to any session change; documented since 2P-1a.
- Rust-analyzer frequently emits stale diagnostics (especially around `cfg(feature = "bench")` arms in forge-bench.rs and after fresh module additions). `cargo check --workspace` is the ground truth.
- `humantime`, `parking_lot`, `ulid` are all direct forge-daemon deps now; all were already transitive prior.
- forge-bench binary requires `--features bench` to build; default `cargo build --workspace` skips it via `required-features = ["bench"]` (added T11).
- T14 Codex review terminated mid-investigation (Codex bug, same pattern as Tier 2). Claude T14 review returned full verdict; folded into T15.
- Sub-agent commit-discipline: 3 of 5 forge-generator dispatches this session (T4, T8, T10, T11) claimed "Committing" then exited without running `git commit`. Orchestrator finished commits manually each time. Logged as backlog #4.
- Per-dim DaemonState isolation increased forge-identity wall-clock from 146ms → 909ms; well within CI budget.

## Parked (won't touch until product-complete)

- v0.5.0 GitHub release + tag push.
- Marketplace publication.
- macOS dogfood.
- T17 — bench-fast CI gate promotion to required (after 14 consecutive green master runs).

## One-line summary

HEAD `8dccf58`; 2A-4d.3 Bench Harness shipped with composite=0.849 honest ceiling; 1,470 tests pass; 18 deferred items tracked across 2A-4d.{1.1, 2.1, 3.1}; T12 calibration deferred to 2A-4d.3.1 item #2 (StepDispositionOnce) before composite can reach 1.0.
