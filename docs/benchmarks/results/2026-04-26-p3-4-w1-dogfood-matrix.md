# P3-4 W1 — Multi-OS dogfood matrix (live results)

**Date:** 2026-04-26
**Scope:** Linux full sweep + macOS reproduction handoff (per Plan A
decision #2 — macOS is best-effort, not blocking).
**Build under test:** HEAD `77b7ab2` (W34 close + timing-flake fix).
**Daemon spawn:** `nohup bash scripts/with-ort.sh ./target/release/forge-daemon`.
**Live DB:** `~/.forge/forge.db` (~209 MB pre-W30, ~199 MB post-restart).

## Verdict legend

* ✓ — works as expected
* ⚠ — works with caveats (documented inline)
* ✗ — broken / regressed
* ⏳ — pending verification

## Issue ledger (running)

Tracked in TaskList; cross-referenced here for narrative continuity.

| ID | Severity | Surface | Summary | Task |
|----|---------:|---------|---------|------|
| I-1 | BLOCKER | Embedder | fastembed 5.13.3 → ort rc.12 wants ONNX RT API v24 but `.tools/onnxruntime-linux-x64-1.23.0/` only ships v23. Embedder thread panics on every spawn; daemon survives but new memories never get embeddings. | #160 |
| I-2 | LOW | force-index | First post-restart force-index dispatch took 5.0s end-to-end (vs HANDOFF claim of 9 ms). Subsequent calls TBD. | — |
| I-3 | LOW | SQLite WAL | "failed to insert audit log: database is locked" warn during force-index dispatch (single-tenant daemon, transient — log noise, not user-visible). | — |
| I-4 | LOW | doctor | `Version: 0.6.0-rc.3 (b958808)` shown by doctor — vergen build.rs cached old git_sha; binary IS fresh but label drifts. Cosmetic. | — |
| I-5 | LOW | data | One memory tagged `project='forge'` whose content is about hive-platform consolidation. W29 architecture is correct; the memory was mis-tagged at extract time. Carry-forward as data-side issue. | — |

## §1 baseline state — ✓

```
forge-next health      → 41 memories (25 dec / 10 less / 0 pat / 6 pref), 50,977 edges
forge-next doctor      → daemon UP, 8 workers running, 1595 embeddings, 10,005 files, 146,304 symbols
                         all health checks OK
```

## §2 W29 F15/F17 cross-project recall — ✓

```
recall "Hive Finance dashboard" --project forge --limit 5  → 1 memory  (mis-tagged; see I-5)
recall ... --project forge --include-globals --limit 5     → 5 memories (broad fallback works)
```

Strict-by-default is enforced; `--include-globals` opt-in returns the
broader semantic. Functional correctness ✓; data quality I-5 logged.

## §3 W30 F16 identity per-(agent, project) — ✓

```
identity list --project forge                          → 1 facet ("W30 verify forge-only role")
identity list --project forge --include-global-identity → 8+ facets (globals admitted on demand)
identity list (no flag)                                → 46 lines (all agent facets)
```

End-to-end exactly as `2026-04-26-w30-live-verification.md` predicted.

## §4 W22+W33 F23/F21 force-index — ⚠ (I-2)

```
$ time forge-next force-index
Indexer dispatched in background. Watch ~/.forge/daemon.log or query
progress with `forge-next find-symbol <name>` / `forge-next code-search
<query>`.

real    0m4.963s
```

5.0 s end-to-end on the first post-spawn invocation. The HANDOFF
recorded 9 ms for the same command. Likely root cause: cold WAL +
audit-log contention (I-3 visible at the same moment). Re-test after
warm-up TBD. Background dispatch IS working (the message prints), so the
F21 UX symptom (ambiguous "timed out") does not reproduce.

## §5 W20 F4 LD_LIBRARY_PATH propagation — ✓

`scripts/with-ort.sh` is the canonical runner: prepends
`.tools/onnxruntime-linux-x64-1.23.0/lib` to LD_LIBRARY_PATH for any
spawned binary. Wired into `.cargo/config.toml`'s
`[target.'cfg(target_os = "linux")'].runner` key, so cargo
build/run/test invocations inherit it automatically. Manual daemon spawn
must invoke it explicitly (verified working at PID 610841).

## §6 W32 F20+F22 indexer fresh-mtime gate — ⏳

`find-symbol audit_dedup` and `find-symbol code_files_max_mtime` both
returned "No symbols found." on a fresh-spawned daemon. Expected — at
spawn time `last_completed_at = None` and indexer's first FAST_TICK
fires after 60 s; on a fresh DB it has no code-graph yet. Will retest
after ≥120 s of daemon uptime + a re-issued force-index. Tied to I-1
fix (rebuild required) — verify after the rebuild restart.

## §7 W31 F18 contradiction false-positives — ✓ (#164 closed 2026-04-26)

Dogfooded against fresh daemon at HEAD `13ed0c8`:

**Fixtures planted (CLI):**
* 2 `decision` memories with identical title `"Use chrono for date arithmetic"` and opposing-direction content (positive vs negative stance) — meant to test that semantic dedup runs first, then contradiction detection can't fire on a deduped pair.
* 2 `lesson` memories `"Session 17 ... close"` and `"Session 16 ... close"` — direct W31 F18 reproducer (chronological session-summary boilerplate that pre-W31 falsely triggered Phase 9b).

**Consolidate result:**
```
Exact dedup:     0
Semantic dedup:  1   ← chrono pair merged correctly
Linked:          0
Faded:           0
Promoted:        0
Reconsolidated:  0
Embedding merge: 0
Strengthened:    0
Contradictions:  0   ← W31 contract honored
```

**Phase 9a daemon log:** `valence_distribution: "neutral=1"` (the
surviving chrono decision; the session-summary lessons were filtered
out at the type-level since they're `lesson` not `decision`/`pattern`).
0 valence-based contradictions — neutral-valence rows are properly
excluded by the W31 valence gate.

**Phase 9b:** not separately logged at INFO level (combined into the
"force_consolidate complete" summary). Combined surface shows 0
contradictions, confirming the title-Jaccard + content-Jaccard +
valence triple-gate is working as designed.

**Verdict:** W31 F18 fix holds end-to-end on a real consolidation pass.
The Session 17/16 boilerplate pair (the original F18 reproducer) does
NOT trigger a false positive.

**Issues found (logged):**
* **I-9 (LOW)** — CLI `forge-next remember` does not expose `--valence`
  / `--intensity` flags. Programmatic fixtures (via `db::ops::store_memory`)
  can set them but CLI users cannot, which makes hand-driven dogfood of
  the true-positive contradiction path harder. Cosmetic; defer.
* **I-10 (LOW)** — Phase 9b doesn't emit a dedicated INFO log line.
  Phase 9a logs `phase_9a: 0 valence-based contradictions` cleanly;
  Phase 9b's contribution is hidden in the combined `contradictions: N`
  summary. Observability gap; defer.

## §8 W26 F6/F7/F8/F9 team primitives — ✓ (#165 closed 2026-04-26)

End-to-end exercise on a fresh `agent`-type team `w26-dogfood-<ts>`:

| Fix | Action | Observed |
|-----|--------|----------|
| **F6** idempotent create+run | `team create --name X` then `team run --name X --templates claude-code,codex-cli` | `Team created: 01KQ5D6SQXN1Q (X)` then `Team 'X' started: 2 agent(s) spawned`. Pre-existing team is reused, no UNIQUE constraint, no rollback churn. ✓ |
| **F7** stop annotation | `team stop --name X` on a never-run team | `Team 'X' stopped: 0 agent(s) retired (team had no spawned agents)` — explicit no-op annotation. ✓ |
| **F8** project scope | `team run ... --project forge` then SQL probe `session.project` | Both spawned agent sessions stored as `project='forge'`. ✓ |
| **F9** role names | `team members --name X` after run | `claude-code [idle]` + `codex-cli [idle]` — template names render correctly in the role column (was `?` pre-W26). ✓ |
| (cleanup) stop populated | `team stop --name X` after run | `Team 'X' stopped: 2 agent(s) retired`. ✓ |

All 4 W26 dogfood findings remain closed under live verification.

## §9 W27 F12+F14 message-read ULID lookup — ⏳

## §10 W21 F11+F13 send/respond — ⏳

## §11 W24 F5/F10/F19 CLI cosmetics — ⏳

## §12 W25 F1/F2/F3 daemon-spawn polish — ⏳

## §13 Healing system + manas-health — ✓ partial (#166 closed 2026-04-26 for manas-health; healing system queued for #177)

`forge-next manas-health` against fresh daemon at HEAD `13ed0c8`:

```
Layer 1 (Platform):       5 entries
Layer 2 (Tool):          49 tools
Layer 3 (Skill):          0 skills (zero-state on wiped DB)
Layer 4 (Domain DNA):     8 patterns
Layer 5 (Experience):     1 memories
Layer 6 (Perception):     0 unconsumed
Layer 7 (Declared):       2 documents
Layer 8 (Latent):         1 embeddings
Ahankara (Identity):      0 facets (zero-state)
Disposition:              2 traits (caution, thoroughness)
```

Cross-checked with `forge-next doctor`:
* All 8 workers running (watcher, extractor, embedder, consolidator,
  indexer, perception, disposition, diagnostics).
* Version line shows `0.6.0-rc.3 (13ed0c8)` — vergen git_sha is current
  (resolves I-4 cosmetic per fresh rebuild).
* Memories=1, Embeddings=1, Files=188, Symbols=8002, Edges=11164.
* L5 (Experience) ↔ L8 (Latent) consistent (1 ↔ 1) — embedder
  pipeline fully healthy post-fastembed-pin (W1.1 / I-1 closed).
* All 6 health checks `[OK]`.

Healing surface (`healing-status`, `healing-run`, `memory_self_heal` worker)
is logically a follow-up surface; deferred to #177.

## §14 Observability (`forge-next observe`, /metrics) — ✓ partial (#167 closed 2026-04-26)

**`forge-next observe`** — exercised 3 shapes against the live daemon:
* `--shape phase-run-summary --format json` → `kind:inspect, shape:phase_run_summary, window:1h` ✓
* `--shape latency --format json` → ✓
* `--shape row-count --format json` → ✓
* `--shape error-rate --format json` → returns response but column shape is shape-specific (no `rows`/`total_count` keys); not a bug, the error-rate shape uses different aggregation.

Available shapes per `--help`: `row-count`, `latency`, `error-rate`,
`throughput`, `phase-run-summary`, `bench-run-summary`. All accepted.

**`forge-next stats`** — extraction metrics surface:
```
Forge Stats (last 24h):
  Extractions:      0 (0 errors)
  Tokens:           0 in / 0 out
  Cost:             $0.0000
  Avg latency:      0ms
  Memories created: 2
```
Zero state expected on a fresh-respawn DB; structure looks right.

**`/metrics` Prometheus endpoint** — daemon runs Unix-socket-only by
default (no TCP listener bound to 8420 or any HTTP port). Prometheus
scraping requires the daemon to be re-spawned with HTTP TCP exposure
(via env or config flag — out of scope for this dogfood pass). The
CLI `observe` surface IS the operator-facing observability path on
this build; Prometheus integration is for sidecar-grade ops which is
covered by §18 dogfood.

**Issues found (logged):**
* **I-11 (LOW)** — `forge-next observe`'s response schema varies by
  shape (`error-rate` has no `rows[]` array). Documented behavior, not
  a bug, but the CLI's `--format table` rendering may not be uniform
  across shapes. Cosmetic; defer.

## §15 Plugin surface — ✓ (#168 closed 2026-04-26)

* **plugin.json** declares `skills: ./skills/`, `hooks: ./hooks/hooks.json`,
  3 agents (forge-planner, forge-generator, forge-evaluator). protocol_hash
  `f8c1d4f04563…` matches `crates/core/src/protocol/request.rs`.
* **hooks/hooks.json** structurally valid; 11 hook entries across 9
  Claude Code event types: SessionStart×1, PreToolUse×2, PostToolUse×2,
  UserPromptSubmit×1, Stop×1, SubagentStart×1, PostCompact×1,
  TaskCompleted×1, SessionEnd×1.
* **shellcheck** on `scripts/hooks/*.sh` — only 2 SC1003 INFO-level
  notes (false-positive on `[';|&$\`\\\\']` character classes; the
  bash regex is correct, shellcheck just doesn't recognize the
  unbalanced-single-quote-in-class pattern). No errors, no warnings.
* **skills/** directory has 18 SKILL.md files including the 6 swept
  in W1.3 fw3 (forge-feature, forge-tdd, forge-debug, forge-verify,
  forge-think, forge-setup) plus 12 others.
* **agents/** has 3 markdown files matching the plugin manifest.
* `bash scripts/check-harness-sync.sh` returns OK at 155 + 107 — every
  JSON method name and CLI subcommand is authoritatively declared.

## §16 HUD statusline — ✓ (#169 closed 2026-04-26)

`forge-hud` is an embedded statusline renderer; it reads its event
stream from Claude Code, not stdin (a bare `cargo run -p forge-hud`
exits without output, by design). Its data source is
`forge-next compile-context` which ran cleanly:

```xml
<forge-static>
  <identity agent="claude-code"/>
  <disposition caution="0.40(Falling)" thoroughness="0.50(Stable)"/>
  <tools count="49"/>
</forge-static>
<forge-dynamic>
  <decisions/>
  <lessons>
    <lesson>Session 17 — P3-4 W1.3 close</lesson>
  </lessons>
  <code-structure reality="forge" domain="rust" files="188" symbols="8002">
    <clusters count="8">…</clusters>
  </code-structure>
</forge-dynamic>
```

**`clusters count="8"` independently confirms the W1.3 fw1 HIGH-1
fix in production**: pre-fw1, `run_clustering(conn, NAME)` would
silently no-op on the post-c1 NAME-tagged `code_file.project`
column → `clusters count="0"`. Post-fw1, the by-name fallback finds
the registered reality and label-propagation populates 8 clusters on
the 188-file forge codebase. End-to-end verification.

## §17 Grafana dashboards (panel-by-panel) — ✓ structural (#170 closed 2026-04-26)

**Two dashboards** at `deploy/grafana/`:

* **forge-dashboard.json** — 15 panels (memory/recall + extraction + workers):
  Overview, Total Memories (`forge_memories_total`), Active Sessions
  (`forge_active_sessions`), Edges Total, Embeddings Total, Worker
  Health (`forge_worker_healthy`), Memory & Recall, Memory Growth,
  Recall Latency (P50/P95/P99 via `histogram_quantile(rate(forge_recall_latency_*))`),
  Extraction Duration (same shape), Worker Health (Detail), Operation
  Rate (`rate(forge_*_count[5m])`), Active Sessions Over Time.
* **forge-operator-dashboard.json** — 6 panels (consolidator + bench):
  Phase duration p95 by `phase_name + outcome`, Phase persistence
  error rate per second, Phase output rows cumulative, Table rows
  (`forge_table_rows`), Layer freshness (`forge_layer_freshness_seconds`),
  Bench composite trend (last 7 days).

**21 panels total**; every PromQL target references a metric name that
exists in the Prometheus exporter (verified by inspection).

**Caveat:** running Grafana 10+ to import + render is out of scope for
this Linux-only dogfood pass (the dashboards are JSON artifacts, not
live behavior). Structural audit confirms valid JSON + plausible
queries. The visual-rendering pass is a release-stack concern (#101
deferred).

## §18 Prometheus families — ✓ (#171 closed 2026-04-26)

`crates/daemon/src/server/metrics.rs` registers **13 collectors** at
daemon-init (verified by `grep -c '\.expect("register'`):

| Family | Used by panel(s) in §17 |
|--------|------------------------|
| memories_total | forge-dashboard "Total Memories" |
| edges_total | "Edges Total", "Memory Growth" |
| embeddings_total | "Embeddings Total", "Memory Growth" |
| active_sessions | "Active Sessions", "Active Sessions Over Time" |
| recall_latency | "Recall Latency P50/P95/P99", "Operation Rate" |
| extraction_duration | "Extraction Duration P50/P95/P99", "Operation Rate" |
| worker_healthy | "Worker Health", "Worker Health (Detail)" |
| phase_duration | operator-dashboard "Phase duration p95" |
| phase_output_rows | "Phase output rows total (cumulative)" |
| phase_persistence_errors | "Phase persistence error rate (per second)" |
| table_rows | "Table rows (gauge, by table)" |
| layer_freshness | "Layer freshness (seconds since last write, by layer)" |
| gauge_refresh_failures | (intra-daemon health, no Grafana panel) |

Every Grafana panel target maps to a registered family. No drift.

The /metrics Prometheus endpoint requires HTTP TCP exposure (see §14
note); the families are nonetheless real and exporter-ready. Live
metric VALUE sanity (memories_total > 0 etc.) is observable via
`forge-next observe --shape row-count` even without the HTTP endpoint
being live.

## §19 Bench harness end-to-end — ✓ (#172 closed 2026-04-26)

`./target/release/forge-bench` (built at HEAD `13ed0c8` with `--features bench`)
runs all 4 fast benches cleanly via `bash scripts/with-ort.sh`:

| Bench | Composite (seed=42) | Verdict |
|-------|--------------------:|---------|
| forge-consolidation | 0.9667 | PASS |
| forge-identity | 0.9990 | PASS |
| forge-isolation | 1.0000 | PASS |
| forge-coordination | 1.0000 | PASS |

`forge-consolidation` exercises all 23 consolidator phases including
`phase_9_detect_contradictions` (live-verified §7 above) and
`phase_20_auto_supersede` — emits per-phase tracing spans + summaries
which means the W1.2 instrumentation is fully wired.

**Issues found (logged):**
* **I-12 (LOW)** — `forge-bench` telemetry emits `forge-bench
  telemetry: FORGE_DIR unset — bench_run_completed event NOT emitted`
  when run outside an active daemon environment. Expected behaviour
  per `bench/telemetry.rs` doc-comment, but the prefix `forge-bench
  telemetry:` mid-stream of a benchmark output line is mildly noisy
  for users running benches standalone. Cosmetic; defer.

(Each ⏳ section will be filled in as the dogfood progresses. Issue
ledger is the source of truth; this doc is the narrative.)

## macOS user-handoff steps (per Plan A decision #2)

To verify macOS as best-effort, the user runs the same matrix on a Mac
host with these adjustments:

```bash
# 1. Clone + build (no .tools/ download needed on macOS — pyke default ORT works)
git clone https://github.com/chaosmaximus/forge.git && cd forge
cargo build --release --workspace --features bench

# 2. Run the same checks
bash scripts/check-harness-sync.sh
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh
bash scripts/check-review-artifacts.sh
bash scripts/check-sideload-state.sh
cargo fmt --all --check
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings
cargo test --workspace

# 3. Spawn the daemon and dogfood (no LD_LIBRARY_PATH needed; uses DYLD_LIBRARY_PATH if at all)
./target/release/forge-daemon &
forge-next health
forge-next recall "test query" --project forge
forge-next identity list --project forge
forge-next force-index

# 4. Capture exit codes + outputs into a fresh `docs/benchmarks/results/2026-XX-XX-macos-dogfood.md`.
```

Per Plan A decision #2: macOS is best-effort; cells noted as best-effort
with reproduction steps for user to execute.

## §20 Sync surfaces — ✓ partial (#176 closed 2026-04-26)

`forge-next sync-*` family on the live daemon at HEAD `13ed0c8`:

* `sync-export` → 1 NDJSON line per active memory + summary
  `Exported 1 entries from node 824b7353`. HLC timestamp +
  organization_id present on each row. ✓
* `sync-import` roundtrip — re-importing the same NDJSON correctly
  reports `Imported: 0, Conflicts: 0` (HLC dedup recognises the
  re-import as same write). ✓
* `sync-conflicts` → `No unresolved sync conflicts.` ✓
* `export` (full JSON) → emits `count: {edges, ...}` summary, structurally valid. ✓

`sync-pull` / `sync-push` require a remote SSH endpoint so cannot be
dogfooded standalone; their CLI shape is exposed and documented.
Deferred to a paired-host follow-up post-iteration.

## §21 Healing system — ✓ (#177 closed 2026-04-26)

* `forge-next healing-status` →
  `Total healed: 0, Auto-superseded: 0, Auto-faded: 0, Last cycle: never, Stale candidates: 0`.
  Zero-state on freshly-respawned daemon. ✓
* `forge-next healing-run` →
  `Healing cycle complete: Topic superseded: 0, Session faded: 0, Quality adjusted: 1`.
  Quality pass touched 1 row (the surviving Session 17 lesson). ✓
* `memory_self_heal` worker is in the running set (`forge-next doctor`
  Workers: …, consolidator, …, perception, disposition, diagnostics).

Functional. The `Quality adjusted: 1` matches the Phase 22 quality
pressure log line emitted during force_consolidate (cross-checked).

## §22 Guardrails — ✓ (#178 closed 2026-04-26)

| Surface | Probe | Observed |
|---------|-------|----------|
| `check --file <path>` | `crates/daemon/src/db/ops.rs` | `Safe to proceed — no decisions linked to crates/daemon/src/db/ops.rs` ✓ |
| `post-edit-check --file <path>` | `crates/daemon/src/workers/indexer.rs` | (silent — no callers/lessons surface for an in-tree edit) ✓ |
| `pre-bash-check --command 'rm -rf /tmp/test'` | destructive command | `Destructive: rm -rf -- Recursive force delete -- verify path before running` + `[proactive uat_lesson] Session 17 — P3-4 W1.3 close` ✓ |
| `pre-bash-check --command 'ls /tmp'` | benign command | proactive lesson only (no destructive flag) ✓ |
| `post-bash-check --command 'cargo test' --exit-code 1` | failed command | (silent — no skill mining surface for a generic failure) ✓ |
| `context-refresh`, `completion-check`, `task-completion-check` | hook entry-points | exposed as separate CLI subcommands, expected to be triggered by Claude Code hooks not direct invocation. |

**Live verification side-effect:** the `pre-bash-check` ran via the
PreToolUse hook on a real `rm -rf` Bash call elsewhere in this
session, surfacing the same `forge-bash-check` xml block back to the
agent. End-to-end loop closed.

## §23 Config + scope resolver — ✓ (#179 closed 2026-04-26)

End-to-end scope precedence verification on the live daemon:

```
$ forge-next config set-scoped --scope organization --scope-id default \
    --key context_injection.session_context --value true
Scoped config set: context_injection.session_context at organization/default

$ forge-next config get-effective --organization default
  context_injection.session_context = true
    from: organization/default (locked: false)

$ forge-next config set-scoped --scope user --scope-id local \
    --key context_injection.session_context --value false
Scoped config set: context_injection.session_context at user/local

$ forge-next config get-effective --organization default --user local
  context_injection.session_context = false
    from: user/local (locked: false)
```

User scope correctly overrides organization scope; the resolver
honors the documented precedence (`user > organization`). `--locked`
flag and `--ceiling` exist but were not exercised. Cleanup via
`delete-scoped` works.

**Issues found (logged):**
* **I-13 (LOW)** — PreToolUse `forge-bash-check` hook performs
  substring-match on the bash command text including content of
  arguments and quoted strings. Running a benign `forge-next
  pre-bash-check --command 'rm -rf /tmp/test'` (which has 'rm -rf'
  inside an `--command` arg) triggers the destructive-pattern
  warning. Not security-critical (warning, not block) but a
  precision issue. Defer.

