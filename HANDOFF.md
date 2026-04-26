# Handoff — pre-compact, polish-wave plan ready — 2026-04-26

**Public HEAD:** `da09c9c` (P3-3.5/3.6/3.7 plan committed). Working tree clean.
**Forge-app master:** unchanged.
**Version:** v0.6.0-rc.3 (will hold through polish wave; bumps to 0.6.0 only at P3-4 W3).
**Plan A (closed):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md` — P3-1 + P3-2 + P3-3 closed.
**Plan B (active):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` — P3-3.5 + P3-3.6 + P3-3.7 queued before P3-4.
**Halt:** **PHASE-BOUNDARY HALT** — P3-3 closed at HEAD `786ab64`. Polish-wave plan locked but unstarted; resume at P3-3.5 W1 next session.

## State in one paragraph

**P3-3 (new product phases) closed at HEAD `786ab64` in 39 commits across 6 stages.** Comprehensive bench + observability audits dispatched 2026-04-26 surfaced **9 bench gaps** (3 HIGH, 3 MED, 3 LOW) and **4 observability polish gaps** (2 MED, 2 LOW). User locked the full polish scope with 4 decisions: (1) polish wave: yes, (2) opentelemetry cluster bump: yes (separate halt-able session), (3) longmemeval/locomo re-run: deferred, (4) drift-fixture tests: yes. **23 commits queued across 3 sub-phases** (P3-3.5 polish wave + P3-3.6 otel bump + P3-3.7 drift fixtures) before P3-4 release. Plan committed at `da09c9c`.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD da09c9c
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 154 + 107
bash scripts/check-review-artifacts.sh             # 21 reviews valid
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh
cargo fmt --all --check                            # clean
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings  # 0 warnings

# Read the polish-wave plan + this HANDOFF, then begin P3-3.5 W1.
cat docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md
```

After verification, **resume at P3-3.5 W1 (re-run stale benches)**:

```bash
# W1 commit 1 — forge-consolidation re-run
export LD_LIBRARY_PATH="$PWD/.tools/onnxruntime-linux-x64-1.23.0/lib:$LD_LIBRARY_PATH"
./target/release/forge-bench forge-consolidation --seed 42 \
    --output /tmp/forge_cons_2026-04-26 --expected-recall-delta 0.0
# Capture composite + per-dim + wall-clock; write
# docs/benchmarks/results/2026-04-26-forge-consolidation-pre-release.md.

# Repeat for forge-context (W1 commit 2) + forge-persist (W1 commit 3).
```

Then proceed through W2 → W8 of P3-3.5. Halt at end of W8 for user sign-off before opening P3-3.6.

## Wave roadmap (queued, unstarted)

### P3-3.5 — Polish wave (12 commits, ~3-4h)

| Wave | Scope | Commits |
|------|-------|---------|
| W1 | Re-run forge-consolidation, forge-context, forge-persist | 3 |
| W2 | Reconcile recall_delta → recall_improvement spec drift | 1 |
| W3 | baselines/composites.json + protocol README | 2 |
| W4 | Wall-clock assertions in isolation/coordination results docs | 1 |
| W5 | 3 phase-error alerts in forge-alerts.yml | 1 |
| W6 | 9 runbook stubs (existing 6 alerts + W5 3 new) | 2 |
| W7 | docs/operations/observability-slos.md SLO registry | 1 |
| W8 | OTLP-validation + recall-probe-bench docs | 1 |

### P3-3.6 — opentelemetry 0.27 → 0.31 cluster bump (5 commits, ~2h, halt-able)

| Wave | Scope | Commits |
|------|-------|---------|
| W9 | Cargo.toml bump (4 sibling deps) | 1 |
| W10 | Compile-error fixes (API renames, import paths) | 1 |
| W11 | NoopSpanExporter rewrite for unified SpanExporter trait | 1 |
| W12 | T10 OTLP latency calibration (≤ 1.20× gate) — HALT IF EXCEEDED | 1 |
| W13 | Backlog removal + memory update + HANDOFF | 1 |

### P3-3.7 — Drift-fixture tests (6 commits, ~2-3h)

| Wave | Scope | Commits |
|------|-------|---------|
| W14 | drift fixtures — forge-consolidation | 1 |
| W15 | drift fixtures — forge-context | 1 |
| W16 | drift fixtures — forge-identity | 1 |
| W17 | drift fixtures — forge-isolation | 1 |
| W18 | drift fixtures — forge-coordination | 1 |
| W19 | Integrate into cargo test --features bench + sensitivity sections | 1 |

### P3-4 — Release (after P3-3.7 close, halted for user sign-off)

7 waves per `docs/superpowers/plans/2026-04-25-complete-production-readiness.md` §"Phase P3-4".

## Audit findings (source of polish-wave scope)

### Bench audit gaps (9 total)

**HIGH (3):**
1. Stale results docs (forge-consolidation 9d, forge-context 10d, forge-persist 11d). Numbers may not reflect current code. **Closes in W1.**
2. No locked-baseline catalog at `docs/benchmarks/baselines/composites.json`. **Closes in W3.**
3. Spec ↔ registry name drift: forge-consolidation D5 = `recall_improvement` in code but `recall_delta` in spec. **Closes in W2.**

**MEDIUM (3):**
4. longmemeval/locomo excluded from CI. Documented as on-demand pattern. **Closes in W8 (doc).**
5. Missing wall-clock assertions in isolation/coordination results docs. **Closes in W4.**
6. No drift-fixture tests — benches infrastructure-check themselves but never adversarially test sensitivity. **Closes in P3-3.7 W14-W19.**

**LOW (3):**
7. Chaos drill SIGKILL-only. Documented backlog; defer to v0.6.1.
8. No criterion latency benches. Defer to v0.6.1.
9. forge-identity master v6 fix-regression test consolidation. Cosmetic; defer.

### Observability polish gaps (4 total)

**MEDIUM (2):**
10. No alerts on new metric families (`forge_phase_persistence_errors_total`, `forge_layer_freshness_seconds`). **Closes in W5.**
11. Runbook URLs broken in `forge-alerts.yml`. **Closes in W6.**

**LOW (2):**
12. SLO numbers scattered. **Closes in W7.**
13. OTLP not validated in CI; manual procedure undocumented. **Closes in W8.**

### Strengths confirmed

- All 23 consolidator phases wrapped with info_span! + PhaseOutcome.
- 5 Prometheus metric families registered + tested.
- kpi_events namespace fully versioned (7 event types).
- All Tier 1 + Tier 2 + Tier 3 spec acceptance criteria met.
- HUD consolidation segment rendering live.
- GaugeSnapshot atomic torn-read prevention via parking_lot::RwLock.
- Zero `eprintln!`/`println!` in production workers.
- Two Grafana dashboards (15-panel user/dev + 6-panel operator).
- Health endpoints `/healthz`/`/readyz`/`/startupz`.
- forge-next observe CLI all 6 inspect shapes.
- Retention reaper wired with per-event-type override.
- OTLP wired with env-var control + trace_id wiring.
- All 8 bench impls deterministic (ChaCha20-seeded, no rand_range hot-path).
- forge-identity master v6 spec compliance (0.999 composite).
- Forge-coordination 5/5 calibration seeds composite=1.0.
- Forge-isolation 5/5 calibration seeds composite=1.0.

## P3-3 commits (final tally — closed)

| Range | Stage | Commits |
|-------|-------|---------|
| `ea75081..479126e` | Stage 0 | 6 |
| `aa14763..1377ee1` | Stage 1 (2A-5) | 16 |
| `d64fe83..d60c7b2` | Stage 2 (2A-6) | 12 |
| `2c8fcfb` | Stage 3 (2A-7) | 1 |
| `d36f1f6` | Stage 4 (2C-1) | 1 |
| `2a9fc52` | Stage 5 (2C-2) | 1 |
| `572d545` `786ab64` | Stage 6 close | 2 |
| `da09c9c` | Polish-wave plan | 1 |
| **Total P3-3 + 3.5/6/7 plan** | — | **40** |

## Tests + verification (final state at HEAD `da09c9c`)

* `cargo fmt --all --check` — clean
* `cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --features bench` — clean
* `cargo test -p forge-daemon --lib --features bench bench::forge_isolation` — 17/17 pass
* `cargo test -p forge-daemon --lib --features bench bench::forge_coordination` — 15/15 pass
* `bash scripts/ci/check_spans.sh` — OK
* `bash tests/static/run-shellcheck.sh` — all PASS
* `bash scripts/check-review-artifacts.sh` — 21 reviews valid, 0 blocking
* End-to-end dogfoods: forge-isolation seed=42 PASS; forge-coordination seed=42 PASS (5ms); restart-drill PASS (256ms recovery).

## Cumulative deferred backlog (post P3-3.7 close)

Items below stay open through v0.6.0 release and tracked for v0.6.1+:

* longmemeval / locomo re-run (datasets unavailable as of 2026-04-26).
* SIGTERM / SIGINT chaos drill modes (forge-persist harness only does SIGKILL).
* Criterion latency benchmarks (`benches/` dir).
* Bench composite as Prometheus gauge (would obviate SQLite plugin dep in 2C-1 panel 6).
* Multi-window regression check (currently pairwise current-vs-prior).
* Manual-override label to skip 2C-2 regression gate during planned recalibration.
* P3-2 W1 trace-handler behavioral test.
* Per-tenant label dimensions in Prometheus metrics.
* OTLP timeline panel in operator dashboard.

## Known quirks (P3-3 carryover, unchanged)

* `test_daemon_state_new_is_fast` — pre-existing timing flake (since 2P-1a). Passes in isolation.
* Harness-sync amnesty auto-flips to fail-closed on 2026-05-09.
* Stderr `[a2a] WARN: session ... tried to respond to message ...` lines from `sessions::respond_to_message:455` during forge-coordination D4 trials are EXPECTED.

## Halt-and-ask map (3 sub-phase boundaries before P3-4)

1. **P3-3.5 close** (end of W8) → halt for user sign-off.
2. **P3-3.6 W12** (T10 OTLP latency calibration) → halt-and-brief if ratio > 1.20×.
3. **P3-3.6 close** (end of W13) → halt for user sign-off.
4. **P3-3.7 close** (end of W19) → halt for user sign-off.
5. **P3-4** opens on (4) sign-off.

## One-line summary

**P3-3 closed (40 commits, v0.6.0-rc.3).** Pre-release polish wave queued: 12-commit P3-3.5 (doc/spec polish), 5-commit P3-3.6 (opentelemetry 0.27→0.31 cluster bump, halt-able), 6-commit P3-3.7 (drift-fixture adversarial tests). Plan locked at `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`. Resume at P3-3.5 W1 next session — re-run forge-consolidation/context/persist + post fresh results docs. After all 23 polish commits land, P3-4 release halts for user sign-off.
