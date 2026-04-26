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

## §7 W31 F18 contradiction false-positives — ⏳

Will exercise the consolidator after I-1 fix (rebuild) by:
1. Inspecting `phase_9b_detect_content_contradictions` log line in daemon log.
2. Verifying `valence_distribution` field in phase_9a output (saw `neutral=41` already — meaning Phase 9a correctly fired 0 valence-based contradictions on the all-neutral live corpus).
3. Probing `recall --type decision` for any flagged contradiction edges.

## §8 W26 F6/F7/F8/F9 team primitives — ⏳ (queued for after I-1 fix)

## §9 W27 F12+F14 message-read ULID lookup — ⏳

## §10 W21 F11+F13 send/respond — ⏳

## §11 W24 F5/F10/F19 CLI cosmetics — ⏳

## §12 W25 F1/F2/F3 daemon-spawn polish — ⏳

## §13 Healing system + manas-health — ⏳

## §14 Observability (`forge-next observe`, /metrics) — ⏳

## §15 Plugin surface — ⏳

## §16 HUD statusline — ⏳

## §17 Grafana dashboards (panel-by-panel) — ⏳

## §18 Prometheus families — ⏳

## §19 Bench harness end-to-end — ⏳

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
