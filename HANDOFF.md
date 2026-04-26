# Handoff — pre-compact, dogfood-fixes plan locked — 2026-04-26

**Public HEAD:** `14279c9` (P3-3.9/3.10/3.11 plan committed).
**Forge-app master:** unchanged.
**Version:** v0.6.0-rc.3.
**Plan A (closed sub-phases P3-1..P3-3, P3-4 queued):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5/3.6/3.7 closed).
**Plan C (active, NEXT):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md` (P3-3.9/3.10/3.11 — close all 23 dogfood findings before P3-4).
**Halt:** **PHASE-BOUNDARY HALT** — P3-3.8 closed, dogfood-fixes plan locked. Resume at **P3-3.9 W20** next session.

## State in one paragraph

**24 commits since the last pre-compact HANDOFF** (`a9fa9af` → `14279c9`):
22 polish-wave + dogfood commits (12 P3-3.5, 5 P3-3.6 otel cluster bump, 3 P3-3.7 drift fixtures, 1 P3-3.8 dogfood findings, 1 review-yaml + 1 fix-wave bundled within), plus the P3-3.8 close HANDOFF and this plan-doc commit. **23 forge-platform findings** captured during dogfood: 3 HIGH, 7 MEDIUM, 11 LOW, 2 OK. User has locked the decision to **fix every actionable finding** (21 of 23 — the 2 OK don't need fixes) before opening P3-4 release. Plan committed at `14279c9` schedules 15 fix-wave commits across 3 sub-phases (P3-3.9 / P3-3.10 / P3-3.11). All 11 CI gates green at HEAD. 22 review YAMLs valid. Working tree clean.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 14279c9
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 154 + 107
bash scripts/check-review-artifacts.sh             # 22 reviews valid
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh
cargo fmt --all --check                            # clean
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings  # 0 warnings

# Read the dogfood-fixes plan + findings doc, then begin P3-3.9 W20.
cat docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md
cat docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md

# W20 first action — locate daemon-spawn site for LD_LIBRARY_PATH fix:
grep -rn "Command::new.*forge-daemon\|forge-daemon.*spawn\|daemon_bin" crates/cli/src/ | head -10
```

## Wave roadmap (queued, unstarted)

### P3-3.9 — Pre-GA HIGH (4 commits, ~3h)

| Wave | Scope | Task ID | Source |
|------|-------|---------|--------|
| W20 | F4 LD_LIBRARY_PATH on daemon auto-spawn | #137 | dogfood F4 |
| W21 | F11+F13 forge-next send `--from <session_id>` flag | #138 | dogfood F11+F13 |
| W22 | F23 force-index async + IndexProgress endpoint | #139 | dogfood F23 |
| W23 | review + HANDOFF + halt for sign-off | #140 | per-wave-procedure |

**Halt at end of W23 for user sign-off** before opening P3-3.10.

### P3-3.10 — Quick MED/LOW (5 commits, ~3-4h)

| Wave | Scope | Task ID | Sources |
|------|-------|---------|---------|
| W24 | CLI cosmetics (`identity show` alias, `message-read <ID>` positional, `blast-radius <PATH>` positional) | #141 | F5, F10, F19 |
| W25 | Daemon-spawn polish (doctor version sanity, hooks-warning gate, socket-bind cold-start 10s) | #142 | F1, F2, F3 |
| W26 | Team primitives polish (idempotent run, role propagation, project flag, stop wording) | #143 | F6, F7, F8, F9 |
| W27 | message-read ULID lookup | #144 | F12, F14 |
| W28 | review + HANDOFF + halt | #145 | per-wave-procedure |

**Halt at end of W28** before P3-3.11.

### P3-3.11 — Investigation MED/LOW (6 commits, ~6-8h, halt-able)

| Wave | Scope | Task ID | Source |
|------|-------|---------|--------|
| W29 | F15+F17 cross-project recall scoping investigation + fix | #146 | F15, F17 |
| W30 | F16 identity per-(agent, project) — decision + impl OR HALT-AND-BRIEF if schema change | #147 | F16 |
| W31 | F18 contradiction false-positives (Phase 9a/9b tightening) | #148 | F18 |
| W32 | F20+F22 indexer .rs file scope (watcher pattern) | #149 | F20, F22 |
| W33 | F21 force-index error UX (likely no-op after W22) | #150 | F21 |
| W34 | review + HANDOFF + halt for P3-4 | #151 | per-wave-procedure |

**Halt-and-brief at W30** if F16 needs a schema change (defer to v0.6.1).
**Halt-and-brief at W29** if F15/F17 reveals architectural drift wider than scope.
**Halt at end of W34** for sign-off opening P3-4.

### P3-4 — Release & distribution (after P3-3.11 close, halted for sign-off)

7 waves per Plan A `2026-04-25-complete-production-readiness.md` §"Phase P3-4". Multi-OS dogfood → bench-fast gate flip → v0.6.0 bump → gh release → marketplace bundle (USER) → branch protection (USER) → final HANDOFF.

## Dogfood findings reference (23 findings, P3-3.8)

Source: `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md`

### HIGH (3) — close in P3-3.9

* **F4** — daemon auto-spawn doesn't propagate `LD_LIBRARY_PATH` for ONNX. Daemon dies with `error while loading shared libraries: libonnxruntime.so.1`. → W20.
* **F11** — `forge-next send` defaults `from_session` to `"api"` regardless of any context. Agent-team intercommunication broken at CLI surface. → W21.
* **F13** — `FORGE_SESSION_ID` env var doesn't propagate to message `from_session`. Companion to F11. → W21.
* **F23** — synchronous `force-index` blocks daemon writer loop for 30s+. Subsequent writes timeout. → W22.

### MEDIUM (7) — close in P3-3.10 (F1-F3, F9) + P3-3.11 (F15/F17, F20, F22)

* **F1** — `doctor` reports stale daemon-version after rebuild until restart. → W25.
* **F2** — `[WARN] hook: plugin hooks.json not found` warning even running in-tree. → W25.
* **F3** — daemon socket-bind cold-start timeout 3s too tight for ONNX init. → W25.
* **F9** — `team members` shows role=`?` instead of spawning template name. → W26.
* **F15+F17** — even with `--project forge`, recall returns hive/dashboard memories. → W29.
* **F20** — indexer lacks recently-modified files in code graph (`find-symbol`/`code-search` empty). → W32.
* **F22** — `blast-radius --file <path>` reports file not in code graph. → W32.

### LOW (11) — close in P3-3.10 (F5/F10/F12/F14/F19, F6/F7/F8) + P3-3.11 (F16, F18, F21)

* **F5** — `identity show` doesn't exist (only `list`). → W24.
* **F6** — `team create` + `team run` aren't compositional (UNIQUE constraint). → W26.
* **F7** — `team stop` reports "0 agent(s) retired" without context. → W26.
* **F8** — Templates spawn agents with `project: (none)`. → W26.
* **F10** — `message-read` requires `--id` flag, not positional. → W24.
* **F12+F14** — `message-read --id <full-ULID>` returns "message not found" while `messages` lists it. → W27.
* **F16** — Identity facets cross-pollinate across projects (41 facets mixed). → W30 (decision needed).
* **F18** — Contradiction detector false-positives on chronological session summaries. → W31.
* **F19** — `blast-radius <path>` (positional) errors. → W24.
* **F21** — `force-index` timeout error UX unclear. → W33 (likely no-op after W22).

### WORKS-AS-EXPECTED (2) — no fix needed

* Identity (Ahankara) — 41 facets render cleanly in `compile-context` XML.
* Healing system — 8 layers all populate; manas-health surfaces them.

## Cumulative commit tally (P3-3.5 + P3-3.6 + P3-3.7 + P3-3.8 + plan)

| Range | Phase | Commits |
|-------|-------|---------|
| `3e86714..7091526` | P3-3.5 W1-W8 polish | 12 |
| `8e449a5..d7c5f73` | P3-3.5 polish-review fix-wave + YAML | 2 |
| `b80ae68..daf6491` | P3-3.6 W9-W13 otel cluster bump | 5 |
| `daa76ad..6118ec2` | P3-3.7 W14+W17+W19 drift fixtures | 3 |
| `0ba3f7b` | P3-3.8 dogfood findings | 1 |
| `44c9094` | P3-3.8 close HANDOFF | 1 |
| `14279c9` | P3-3.9/3.10/3.11 plan | 1 |
| **Total** | — | **25** |

## Tests + verification (final state at HEAD `14279c9`)

* `cargo fmt --all --check` — clean
* `cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --features bench` — clean (post-otel-bump)
* `cargo test -p forge-daemon --lib --features bench bench::` — 230+ pass (incl. 4 drift_fixtures)
* T10 OTLP latency calibration — ratio 1.0324 ≤ 1.20× ceiling (PASS)
* `bash scripts/ci/check_spans.sh` — OK
* `bash scripts/check-review-artifacts.sh` — 22 reviews valid, 0 blocking
* `bash scripts/check-harness-sync.sh` — OK (154 + 107)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-protocol-hash.sh` — OK
* End-to-end bench dogfood seed=42 (release): forge-consolidation 1.0/389ms, forge-context 1.0/171ms, forge-isolation 1.0/12ms, forge-coordination 1.0/2ms, forge-persist 1.0/256ms.

## Cumulative deferred backlog (post P3-3.8 close, unchanged through this HANDOFF)

* **From P3-3.7 (drift fixtures):** W15 forge-context, W16 forge-identity, W18
  forge-coordination drift fixtures need `_with_inj` wrapper variant + injected-buggy
  callable in tests. Defer to v0.6.1+.
* **From P3-3.8 (dogfood):** the 21 actionable findings now scheduled in P3-3.9/3.10/3.11. **No defer.**
* **Earlier deferrals unchanged:** longmemeval / locomo re-run (datasets unavailable), SIGTERM / SIGINT chaos drill modes, criterion latency benchmarks, Prometheus bench composite gauge, multi-window regression baseline, manual-override label, P3-2 W1 trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline panel.

## Tasks (next session)

15 individual tasks created (#137-#151) for per-wave tracking:

| Task ID | Wave | Status |
|---------|------|--------|
| #137 | P3-3.9 W20 (F4) | pending |
| #138 | P3-3.9 W21 (F11+F13) | pending |
| #139 | P3-3.9 W22 (F23) | pending |
| #140 | P3-3.9 W23 close | pending |
| #141 | P3-3.10 W24 | pending |
| #142 | P3-3.10 W25 | pending |
| #143 | P3-3.10 W26 | pending |
| #144 | P3-3.10 W27 | pending |
| #145 | P3-3.10 W28 close | pending |
| #146 | P3-3.11 W29 | pending |
| #147 | P3-3.11 W30 | pending |
| #148 | P3-3.11 W31 | pending |
| #149 | P3-3.11 W32 | pending |
| #150 | P3-3.11 W33 | pending |
| #151 | P3-3.11 W34 close | pending |

## Halt-and-ask map (3 sub-phase halts + 2 conditional)

1. **End of P3-3.9 W23** (3 HIGH fixes): halt for sign-off.
2. **P3-3.11 W29** if recall scoping reveals wider architectural drift: halt + brief.
3. **P3-3.11 W30** if identity scope needs schema change: halt + brief.
4. **End of P3-3.10 W28**: halt for sign-off.
5. **End of P3-3.11 W34**: halt for sign-off, opens P3-4.

## One-line summary

**P3-3.5/3.6/3.7/3.8 closed (24 commits, v0.6.0-rc.3).** Dogfood surfaced 23 findings; user-locked decision to fix all 21 actionable ones before P3-4. Plan committed at `14279c9` schedules 15 fix-wave commits across P3-3.9 (3 HIGH, ~3h), P3-3.10 (4 quick fixes, ~3-4h), P3-3.11 (5 investigation fixes, ~6-8h). Resume at **W20 (F4 LD_LIBRARY_PATH)** next session. After P3-3.11 closes, P3-4 release halts for user sign-off.
