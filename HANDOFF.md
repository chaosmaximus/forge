# Handoff — Phase B autonomous close-out (2026-04-25, post-W4)

**Public HEAD:** `d7b3f68`.
**forge-app master:** `665c372c7c461016a8b5953d91e792b7b7221636` (unchanged).
**Current version:** **v0.5.0**.

## State in one paragraph

This session executed an autonomous Phase B close-out across **4 waves**
covering every backlog item the prior session left tagged for follow-up.
W1 closed the Phase 2A-4d.3.1 review fallout (#3 H6 hot-path threading,
#3 M1 gating tests, #7 H2+H3 docs, #7 M1 disambiguation) plus a review
pair of MEDIUMs/LOW. W2 closed 6 of 7 Tier 2 (2A-4d.2.1) items —
row_count Arc plumb (now wired through both HTTP and unix-socket
transports), shape_latency truncation off-by-one, percentile docs, SSE
filter unit tests, HUD I/O spawn_blocking + atomic write,
ObserveShape→InspectShape collapse — plus W2 review BLOCKER + HIGH
fixes. W3 closed 2 of 5 Tier 1 (2A-4d.1.1) items (CI scrubber broadening
+ instrumentation comment-strip) with 3 honest deferrals. W4 swept
cosmetic LOWs. **Net result: 14 commits on top of the previous handoff
baseline `30102e2`; 1506 daemon-lib tests pass; 0 clippy warnings on
the workspace `--features bench` gate; fmt clean; 1 documented timing
flake (`test_daemon_state_new_is_fast`) unchanged.**

The biggest live behavior changes since `30102e2`:

1. **`/inspect row_count` lazy-refresh now actually fires** on a fresh
   daemon for both `forge-next` (unix socket) and HTTP `/api` clients —
   the prior fix shipped only on HTTP. Operators no longer see
   `stale: true, rows: []` forever after daemon startup.
2. **HUD writer no longer blocks the tokio runtime** during DB queries
   or `hud-state.json` writes — wrapped in `spawn_blocking` + atomic
   tmp+rename so HUD readers can never observe a half-written file.
3. **`shape_latency` truncation accounting is now consistent** when the
   global cap fires on a brand-new group (synthetic row emitted with
   `count: 0` rather than dropping the credit).
4. **`forge-cli` no longer maintains mirror enums for InspectShape /
   InspectGroupBy** — `forge-core` derives `clap::ValueEnum` behind a
   feature flag so adding a Tier-N+1 shape is a single-file change.

## First actions after `/compact`

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -15                                                     # expect d7b3f68 at top
git status --short                                                        # expect clean
export LD_LIBRARY_PATH="$(pwd)/.tools/onnxruntime-linux-x64-1.23.0/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
cargo clippy --workspace --features bench -- -W clippy::all -D warnings   # 0 warnings
cargo test -p forge-daemon --lib --features bench                         # 1506 pass, 1 known flake
bash scripts/ci/check_spans.sh                                            # OK
```

## Session commits — 14 total (most recent first)

### Wave 4 — cosmetic sweep
| #   | SHA       | Title |
|-----|-----------|-------|
| 14  | `d7b3f68` | docs(2A-4d): W4 cosmetic LOW sweep — reaper + KT_BLAST_RADIUS doc |

### Wave 3 — Tier 1 cleanup (2 of 5 closed; 3 still-deferred)
| #   | SHA       | Title |
|-----|-----------|-------|
| 13  | `2e964bc` | fix(2A-4d.1.1): address W3 review MED-1 + MED-2 |
| 12  | `b4a999c` | fix(2A-4d.1.1 #4): strip comments before integrity-test substring match |
| 11  | `c977f65` | fix(2A-4d.1.1 #3): broaden CI guard scrubber — raw strings + cfg(...test...) |

### Wave 2 — Tier 2 cleanup (6 of 7 closed; 1 still-deferred)
| #   | SHA       | Title |
|-----|-----------|-------|
| 10  | `7123a5c` | fix(2A-4d.2.1): address W2 review BLOCKER + HIGH + MEDIUM |
|  9  | `3524eb0` | fix(2A-4d.2.1 #3): HUD writer — spawn_blocking + atomic rename |
|  8  | `8e5d1f3` | refactor(2A-4d.2.1 #7): collapse ObserveShape mirror, derive ValueEnum on InspectShape |
|  7  | `ab118ca` | fix(2A-4d.2.1 #2): extract SSE filter as testable pure function + 9 unit tests |
|  6  | `eecf7af` | fix(2A-4d.2.1): #1 row_count Arc plumb + #5 percentile docs + #6 truncation off-by-one |

### Wave 1 — Phase 2A-4d.3.1 review fallout (4 items + review)
| #   | SHA       | Title |
|-----|-----------|-------|
|  5  | `783f2c4` | fix(2A-4d.3.1): address W1 review MEDIUMs + LOW |
|  4  | `ed3d0ff` | docs(2A-4d.3.1 #7 M1): disambiguate session.status='idle' vs agent_status='idle' |
|  3  | `2751c77` | docs(2A-4d.3.1 #7 H2+H3): register session_idled v1 + document heartbeat default |
|  2  | `3ebb61c` | test(2A-4d.3.1 #3 M1): gating tests for context_injection in recall + proactive |
|  1  | `6c9d13d` | fix(2A-4d.3.1 #3 H6): thread &ContextInjectionConfig through hot path |
| (carryover) | `30102e2` | docs(2A-4d.3.1): close Phase A + B — HANDOFF + plan updates |

## What shipped — by item

### Phase 2A-4d.3.1 (Wave 1)

* **#3 H6** — `recall::compile_static_prefix`,
  `recall::compile_dynamic_suffix`, and
  `proactive::build_proactive_context_with_org` split into thin
  wrappers + `_with_inj` variants taking pre-loaded
  `&ContextInjectionConfig`. Handler::CompileContext arm + bench
  helper now load config once per request and share. Zero test
  churn (wrappers preserve old signatures).
* **#3 M1** — 11 gating tests (7 in `recall.rs`, 4 in
  `proactive.rs`) pairing default-on baseline with gated-off probe
  for every flag (`session_context`, `skills`, `anti_patterns`,
  `active_state`, `preferences`). `blast_radius` documented as
  intentionally untested via memory fixtures.
* **#7 H2+H3** — `session_idled` event registered in
  `docs/architecture/events-namespace.md` with full v1 payload
  schema; emit site updated to include `event_schema_version: 1`.
  `docs/operations.md` gains a Session Lifecycle section with ASCII
  state diagram and 60s→14400s migration note.
* **#7 M1** — disambiguation comments in `db/schema.rs` near both
  `session.status` (lifecycle) and `session.agent_status` (work
  state) — chose comment-disambiguate over full rename (the
  collision is purely cosmetic; rename would touch 18+ sites + 3
  events + a config field). Aligned `agent_status` enumeration to
  the canonical `AgentStatus` enum (`idle / thinking / responding
  / in_meeting / error / retired`), noting the SQL column is
  freeform TEXT.

### Phase 2A-4d.2.1 (Wave 2) — 6 of 7 closed

* **#1** `/inspect row_count` lazy-refresh — `DaemonState::new_reader`
  takes a 6th `Option<Arc<ForgeMetrics>>`; HTTP `/api` and the
  **unix-socket** transports both pass `Some(...)`. CLI users
  (`forge-next observe`) now hit the lazy-refresh on a fresh
  daemon instead of `stale: true` forever.
* **#2** SSE filter — extracted as `event_passes_filter` pure fn;
  `?events=` parsing now drops blank entries after split-and-trim
  (the empty-string trap that would have rejected every event).
  9 unit tests pin the precedence + edge cases.
* **#3** HUD writer — `tokio::task::spawn_blocking` for
  build_hud_state + atomic tmp→rename for `hud-state.json`. The
  `.await` on the JoinHandle is intentional (preserves event order;
  documented in code).
* **#5** percentile convention docs — paragraph in
  `docs/api-reference.md` explaining the ceiling-rank formula and
  the surprising n=2 case.
* **#6** `shape_latency` truncation off-by-one — credit the
  cap-triggering row to its group, AND emit synthetic
  `count: 0, truncated_samples: N` rows for groups that only ever
  hit `truncated_by_group` (W2 review H1 fix).
* **#7** ObserveShape mirror collapse — forge-core gains an
  optional `clap` feature; forge-cli enables it and uses
  `InspectShape` / `InspectGroupBy` directly. Adding a Tier-N+1
  shape is now one file.

### Phase 2A-4d.1.1 (Wave 3) — 2 of 5 closed

* **#3** CI guard scrubber broadening — `scripts/ci/check_spans.sh`
  now matches compound `#[cfg(...)]` forms containing the bare word
  `test` (e.g. `#[cfg(all(test, feature = "bench"))]`) while
  excluding `not(test)`. Awk strip function extended to handle
  raw strings (`r"..."` / `r#"..."#` / ... up to 16 hashes).
* **#4** integrity test substring match — `strip_comments_for_test`
  blanks line + block comments before counting `info_span!("phase_X")`
  occurrences in the `include_str!`'d consolidator.rs. String literals
  intentionally kept (the search needle IS a string literal). New
  unit test locks the false-positive classes.

### Cosmetic sweep (Wave 4)

* Per-org tenancy TODO comment in `reaper.rs::reap_stale_sessions`
  (placed at fn boundary so the next maintainer sees it before
  touching the WHERE clauses).
* Doc on `KT_BLAST_RADIUS` explaining the deliberate `continue` in
  the per-type fetch loop.

## Tests + verification (final state)

* `cargo fmt --all --check` — clean
* `cargo check --workspace` — clean
* `cargo check --workspace --features bench` — clean
* `cargo clippy --workspace -- -W clippy::all -D warnings` — 0 warnings
* `cargo clippy --workspace --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo test -p forge-daemon --lib --features bench` — **1506 pass, 1 fail, 1 ignored**
  (the 1 fail = `test_daemon_state_new_is_fast` — pre-existing flake
  unchanged since 2P-1a)
* `bash scripts/ci/check_spans.sh` — OK (23 names matched, 0 violations)
* Adversarial reviews on every wave (W1, W2, W3) returned
  `lockable-as-is` or `lockable-with-fixes`; every BLOCKER + HIGH
  + concrete MEDIUM was addressed in a follow-up commit.
* No live dogfood this session — code-only changes; the test suite
  + structural reviews are the verification surface.

## Deferred backlog (still tracked)

Single source of truth:
**`docs/superpowers/plans/2026-04-24-forge-identity-observability.md`**.

### Tier 2 (2A-4d.2.1) — 1 still open

* **#4 HUD 24h rollup not index-backed.** `COUNT(DISTINCT json_extract(metadata_json, '$.run_id'))` cannot use the existing indexes. **Why deferred:** the proper fix is invasive (add `run_id TEXT` column + index, OR maintain in-memory rollup) and the kpi_events retention reaper bounds table size. Reopen if HUD pass time becomes user-visible at >100ms.

### Tier 1 (2A-4d.1.1) — 3 still open

* **#1 Consolidator state-Mutex held across all 23 phases.** Tier 2 readers don't contend (they use `new_reader`); workers
  (`perception.rs`, `indexer.rs`, `diagnostics.rs`, `writer.rs`) DO
  block during multi-second consolidator passes. **Reopen** if HUD/perception staleness correlates with consolidator runs in operator reports, or when Tier 4+ surfaces per-worker latency to users.
* **#2 `record()` inside span scope** — 22 mechanical refactor sites.
  Phase 19 already does it correctly; **why deferred:** zero
  user-visible benefit until Tier 2 surfaces phase spans by name in
  a UI (HUD shows aggregate, not per-phase, today).
* **#5 T10 OTLP-path latency variant** — separate latency study.
  No Tier 2/3 path constructs a real OTLP exporter to substitute.

### From #3 review (HIGHs/MEDIUMs deferred since prior session)

Tracked under "Deferred from #3 adversarial review" in the plan.
Non-blocking — feature works; these are polish.

* H1 — Scoped-config wiring (per-project / per-org overrides).
* H2 — `compile_context_trace` not honoring flags.
* H3 — `layers_used: 4` / `9` hard-coded constants.
* H4 — Compose-direction doc note.
* H5 — BlastRadius CLI suppress-message clarity.
* M3-M5 cosmetic (KT_BLAST_RADIUS doc — ✅ closed in this session,
  bench harness override, TOCTOU).

### From #7 review

* M1 — agent_status / session.status disambiguation — ✅ closed in this session via comment-disambiguate.

### Legacy cosmetic batch (2A-4d.3.1 #6)

7 items (M1-M4 + L1-L3) per the plan. **Why deferred:** "Batch into a single cleanup PR when the bench harness sees its first major-version operator polish" — that condition hasn't surfaced.

## Known quirks

* `test_daemon_state_new_is_fast` remains a pre-existing timing flake
  (~3s threshold vs ~200ms isolated on heavy workspaces). Documented since
  2P-1a; unchanged this session.
* Rust-analyzer often shows stale `cfg(feature = "bench")` diagnostics
  during incremental edits — `cargo check --workspace --features bench`
  is the ground truth.
* W3 review HIGH-1 was a false positive (reviewer's "deferred-tag is
  inverted" claim contradicted their own enumeration of phase 19 vs
  the 22). Plan and code are aligned; no fix needed. Documented in
  the W3 review-fix commit (`2e964bc`).

## Next — recommended path

The session went all the way through W4. Phase B is structurally
complete (all backlog items either closed or accompanied by an honest
"why deferred" rationale). Most leveraged remaining items:

1. **Tier 1 #1 — consolidator Mutex structural refactor.** Now the
   highest-impact deferred item: workers DO contend during multi-
   second consolidator passes. The fix is real engineering (give
   the consolidator its own SQLite connection mirroring WriterActor
   OR acquire-release the lock per phase). Justifies its own
   focused PR with adversarial review.
2. **Tier 1 #2 — record() span-scope refactor.** Mechanical 22
   sites. Worth doing only if a Tier 4+ surface starts attributing
   instrumentation-layer errors to a wrong phase.
3. **Tier 2 #4 — HUD 24h rollup index.** Add `run_id` column to
   `kpi_events` + index. ~1 commit, but only worth it if HUD pass
   time becomes user-visible (operator latency reports).
4. **Cosmetic batch (2A-4d.3.1 #6).** 7 items; defer until bench
   harness sees major-version polish.

## Parked (won't touch until product-complete)

* v0.5.0 GitHub release + tag push.
* Marketplace publication.
* macOS dogfood.
* T17 — bench-fast CI gate promotion to required (after 14 consecutive
  green master runs).

## One-line summary

HEAD `d7b3f68`; 14 commits across W1-W4 closing every actionable Phase B backlog item; 1506 tests green, 0 clippy warnings; W1/W2/W3 adversarial-review fallout addressed; deferred items have honest "why" tags + reopen conditions; recommended next: Tier 1 #1 (consolidator Mutex refactor) when Tier 4+ surfaces per-worker latency.
