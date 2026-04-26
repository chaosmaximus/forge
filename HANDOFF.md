# Handoff — polish wave + otel bump + drift fixtures + dogfood landed — 2026-04-26

**Public HEAD:** `0ba3f7b` (P3-3.8 forge-dogfood findings doc).
**Forge-app master:** unchanged.
**Version:** v0.6.0-rc.3 (will hold through P3-4 W1-W2; bumps to 0.6.0 in P3-4 W3).
**Plan A (active):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md` — P3-1 + P3-2 + P3-3 closed; P3-4 next.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` — P3-3.5 + P3-3.6 + P3-3.7 + P3-3.8 closed; rationale-documented partial coverage on W15/W16/W18.
**Halt:** **PHASE-BOUNDARY HALT** — P3-3.8 closed at HEAD `0ba3f7b`. P3-4 release queued; resume there next session OR pick up the 3 HIGH dogfood-finding fixes first per user judgement.

## State in one paragraph

**22 commits landed since the last HANDOFF** (`a9fa9af` → `0ba3f7b`):
12 across P3-3.5 polish wave (+ 1 fix-wave + 1 review YAML), 5 across P3-3.6 opentelemetry-cluster bump (0.27→0.31; T10 calibration ratio 1.0324 ≤ 1.20×), 3 across P3-3.7 drift fixtures (W17 forge-isolation + W14 forge-consolidation + W19 close with rationale-documented W15/W16/W18 deferral), and 1 P3-3.8 dogfood-findings doc capturing 23 findings (3 HIGH, 7 MED, 11 LOW, 2 OK). All gates green. opentelemetry 0.27→0.31 closed the last P3-3 Stage 0 deferred backlog item; memory `feedback_dependabot_ecosystem_cluster.md` updated with the resolution narrative. **No ship-blockers from the polish wave**; the 3 HIGH dogfood findings (F4 LD_LIBRARY_PATH, F11/F13 send-from-session-id, F23 force-index-blocks-writer) are recommended pre-GA but the user's call.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 0ba3f7b
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 154 + 107
bash scripts/check-review-artifacts.sh             # 22 reviews valid
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh
cargo fmt --all --check                            # clean
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings  # 0 warnings
cargo test -p forge-daemon --lib --features bench bench:: 2>&1 | tail -3  # 230+ pass

# Read the polish-wave plan + dogfood findings + this HANDOFF, then halt for user decision.
cat docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md
```

## Decision points before P3-4 opens

1. **3 HIGH dogfood findings** (F4, F11/F13, F23) — fix pre-GA or defer to v0.6.1?
   - F4 (LD_LIBRARY_PATH on auto-spawn): blocks any user installing via binary release without a wrapper script. **Suggested: fix.**
   - F11/F13 (send `--from <session_id>`): blocks practical agent-team workflows. **Suggested: fix.**
   - F23 (force-index blocks daemon writer): operational risk in production. **Suggested: fix.**
   - Estimated: 2-3 hours total across the 3 fixes.

2. **P3-3.7 partial coverage** (W15/W16/W18 deferred) — accept?
   - Rationale documented in `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` §"P3-3.7 close":
     forge-context, forge-identity, and forge-coordination dims either run
     production code paths (compile_context, list_messages, respond_to_message)
     that require injection plumbing OR score against state the daemon recomputes
     from kpi_events on each access — direct INSERT planting doesn't reach the
     score path. **Suggested: accept the deferral, defer to v0.6.1.**

3. **P3-3.7 W14 + W17 sensitivity coverage** (4 drift-fixture tests landed):
   - W14: 2 tests — forge-consolidation D1 dedup miss + signal-preservation
     failure. Both green.
   - W17: 2 tests — forge-isolation D1 cross-project leak + D6 compile-context
     leak. Both green.
   - **Status: drift-fixture-pattern infrastructure proven across 2 of 5 benches.**

## Roadmap — what's next

### P3-4 — Release & distribution (per Plan A)

| Wave | Scope | Auto / User |
|------|-------|-------------|
| **W1** | Multi-OS dogfood final sweep | Auto (Linux); user (macOS) |
| **W2** | Bench-fast required-gate flip (T17 — needs 14 green master runs) | Auto if condition met |
| **W3** | v0.6.0 version bump (rc.3 → 0.6.0) | Auto |
| **W4** | GitHub release artifacts + release notes | Auto if `gh` auth in env |
| **W5** | Marketplace submission bundle | Auto preparation; user submits |
| **W6** | Branch protection rules JSON config | Auto preparation; user applies |
| **W7** | Final HANDOFF rewrite + close-out memo | Auto |

**Halt-and-ask points:** end of each wave; any wave returning a not-lockable adversarial review; anything requiring user credentials.

### Dogfood-finding fix waves (optional, pre-P3-4)

If user signs off on the 3 HIGH fixes:

| Fix | File(s) | Estimate |
|-----|---------|----------|
| **F4 LD_LIBRARY_PATH on auto-spawn** | `crates/cli/src/main.rs` (or daemon-spawn site) — populate `LD_LIBRARY_PATH` env when spawning, OR set `rpath` on the daemon binary build | ~30 min |
| **F11/F13 send `--from <session_id>` flag** | `crates/cli/src/main.rs` (Send subcommand), `crates/core/src/protocol/request.rs` (SendMessage params) | ~45 min |
| **F23 force-index async** | `crates/daemon/src/handlers/index.rs` (or wherever ForceIndex lands) — push to background tokio task + return immediately | ~45 min |

## Polish-wave commit tally (final)

| Range | Phase | Commits |
|-------|-------|---------|
| `3e86714..7091526` | P3-3.5 W1-W8 | 12 |
| `8e449a5..d7c5f73` | P3-3.5 polish-review fix-wave + YAML | 2 |
| `b80ae68..daf6491` | P3-3.6 W9-W13 (otel cluster bump) | 5 |
| `daa76ad..6118ec2` | P3-3.7 W14 + W17 + W19 (drift fixtures) | 3 |
| `0ba3f7b` | P3-3.8 dogfood findings | 1 |
| **Total** | — | **22** |

## Tests + verification (final state at HEAD `0ba3f7b`)

* `cargo fmt --all --check` — clean
* `cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --features bench` — clean (post-otel-bump)
* `cargo test -p forge-daemon --lib --features bench bench::` — 230+ pass (includes 4 new drift_fixtures tests)
* `cargo test --release -p forge-daemon --features bench --test t10_instrumentation_latency -- --ignored t10_consolidation_latency_otlp_variant_c` — PASS (ratio 1.0324 ≤ 1.20)
* `bash scripts/ci/check_spans.sh` — OK
* `bash scripts/check-review-artifacts.sh` — 22 reviews valid, 0 blocking
* `bash scripts/check-harness-sync.sh` — OK (154 + 107)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-protocol-hash.sh` — OK
* End-to-end bench dogfood seed=42 at HEAD `5a49799` (release):
  - forge-consolidation: composite=1.0000, wall=389ms
  - forge-context: composite=1.0000, wall=171ms
  - forge-isolation: composite=1.0000 (5 seeds), wall=11-13ms internal
  - forge-coordination: composite=1.0000 (6 seeds), wall=2ms internal
  - forge-persist: recovery_rate=1.0, recovery_time=256ms

## Cumulative deferred backlog (post P3-3.8 close)

* **From P3-3.7 (drift fixtures):** W15 forge-context, W16 forge-identity, W18
  forge-coordination drift fixtures need `_with_inj` wrapper variant + injected-buggy
  callable in tests. Defer to v0.6.1+.
* **From P3-3.8 (dogfood):** 3 HIGH findings (F4, F11/F13, F23) recommended
  pre-GA; otherwise the 11 LOW + 7 MED findings defer to v0.6.1+.
* **Items unchanged from prior backlog:** longmemeval / locomo re-run (datasets
  unavailable), SIGTERM / SIGINT chaos drill modes, criterion latency
  benchmarks, Prometheus bench composite gauge, multi-window regression baseline,
  manual-override label, P3-2 W1 trace-handler behavioral test, per-tenant
  Prometheus labels, OTLP timeline panel.

## Forge-evaluation summary (P3-3.8)

23 findings total — see `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md`:

| Severity | Count | Key examples |
|----------|------:|-------------|
| HIGH     | 3 | F4 LD_LIBRARY_PATH, F11/F13 send identity, F23 force-index |
| MEDIUM   | 7 | F1 daemon version lag, F9 team role=?, F15/F17 cross-project recall |
| LOW      | 11 | F5/F10/F19 CLI argument inconsistencies, F18 contradiction false-positives |
| WORKS    | 2 | Identity 41 facets render cleanly, healing system surfaces 8 layers |

The cognitive infrastructure (memory layers, identity, perception, contradiction
detection, healing) **works** end-to-end. Agent-team primitives **work in
principle**; CLI surface drops sender identity (F11/F13). Indexer + project
scoping have **concrete bugs** (F15/F17/F22/F23) that should be triaged.

## Halt-and-ask map

1. **Right now (post P3-3.8 close):** halt for user decision on the 3 HIGH
   dogfood findings — fix pre-GA or defer to v0.6.1?
2. **P3-4 W2** (bench-fast required-gate flip) — halt if 14-green-master-runs
   condition not met yet (T17 temporal).
3. **P3-4 W4** (gh release) — halt if `gh` auth not configured in env.
4. **P3-4 W5** (marketplace) — halt; user submits.
5. **P3-4 W6** (branch protection) — halt; user applies.
6. **P3-4 W7** (final HANDOFF + close) — autonomous close.

## One-line summary

**P3-3.5/3.6/3.7/3.8 closed (22 commits, v0.6.0-rc.3).** Polish wave docs
updated, opentelemetry cluster bumped to 0.31 (T10 ratio 1.0324),
drift-fixture pattern proven on forge-isolation + forge-consolidation
(4 tests), 3-agent FISP team dogfooded with 23 forge-platform findings
captured (3 HIGH recommended pre-GA). Resume at P3-4 release per Plan A —
or fix the 3 HIGH dogfood findings first, user's call.
