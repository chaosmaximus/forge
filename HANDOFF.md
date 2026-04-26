# Handoff — P3-3.9 closed (3 HIGH dogfood findings fixed) — 2026-04-26

**Public HEAD:** `e190f70` (W23 review-status update closing MED-1+MED-2).
**Forge-app master:** unchanged.
**Version:** v0.6.0-rc.3.
**Plan A (closed sub-phases P3-1..P3-3, P3-4 queued):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5/3.6/3.7).
**Plan C (active):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md` (P3-3.9 closed; P3-3.10 next).
**Halt:** **PHASE-BOUNDARY HALT** — P3-3.9 closed; halt for sign-off before opening P3-3.10. Resume at **P3-3.10 W24**.

## State in one paragraph

**P3-3.9 closed at HEAD `e190f70`** (6 commits since `37c90b0`): W20 LD_LIBRARY_PATH propagation, W21 `--from`/`FORGE_SESSION_ID` for `send`/`team-send`, W22 force-index async-via-spawn_blocking, W23 adversarial review (lockable-with-fixes verdict; 0 BLOCKER, 2 deferred HIGH, 4 MED, 5 LOW, 2 NIT, 10 RESOLVED), W23 fix-wave closing MED-1 (TOCTOU) and MED-2 (doc drift), W23 YAML status update. **All 3 dogfood HIGH findings (F4, F11+F13, F23) end-to-end verified live.** All 11 CI gates green. 23 review YAMLs valid, 0 open blocking findings. The 2 deferred HIGHs (spawn_blocking supervision + `SessionRespond` CLI surface) and remaining LOW/NIT findings carry forward to P3-3.10 follow-up waves per review YAML.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD e190f70
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 154 + 107
bash scripts/check-review-artifacts.sh             # 23 reviews valid
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh
cargo fmt --all --check                            # clean
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings  # 0 warnings

# Read the dogfood-fixes plan + W23 review for context, then begin P3-3.10 W24.
cat docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md
cat docs/superpowers/reviews/2026-04-26-p3-3-9-pre-ga-high.yaml

# W24 first action — locate clap definitions for F5/F10/F19 cosmetics:
grep -n "identity\|message-read\|blast-radius\|MessageRead\|BlastRadius" crates/cli/src/main.rs | head -20
```

## P3-3.9 close summary

### What landed (6 commits)

| SHA | Wave | Scope | Files |
|-----|------|-------|-------|
| `54aeecd` | W20 | F4: forward `LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH`/`DYLD_FALLBACK_LIBRARY_PATH` to auto-spawned daemon. Const-extracted `FORWARDED_ENV_KEYS` + 2 unit tests. | `crates/cli/src/client.rs` |
| `6e27eb4` | W21 | F11+F13: `--from <SESSION_ID>` clap flag on `Send`. `resolve_from_session(explicit)` helper with order `flag > FORGE_SESSION_ID > None+stderr-warn`. Wire `team_send` through same helper. 4 unit tests. | `crates/cli/src/main.rs`, `commands/system.rs`, `commands/teams.rs` |
| `611169b` | W22 | F23: `Request::ForceIndex` intercepted in `WriterActor::run` and dispatched to `tokio::task::spawn_blocking` with its own SQLite write connection. Adds `db_path: String` to `DaemonState`. CLI prints "started in background" when daemon returns `IndexComplete{0,0}`. | `crates/daemon/src/server/handler.rs`, `server/writer.rs`, `crates/cli/src/commands/system.rs` |
| `2ef27e8` | W23 review | Adversarial review on W20-W22. Verdict: `lockable-with-fixes`. 0 BLOCKER / 2 HIGH (deferred) / 4 MED / 5 LOW / 2 NIT / 10 RESOLVED. | `docs/superpowers/reviews/2026-04-26-p3-3-9-pre-ga-high.{yaml,transcript.md}` |
| `39f84b2` | W23 fix | Address MED-1 (TOCTOU: canonical path resolved once on actor thread, no recanonicalize in spawn closure) + MED-2 (doc-comment honesty: "stderr warning per CLI invocation" not "one-time"). | `crates/daemon/src/server/writer.rs`, `crates/cli/src/commands/system.rs` |
| `e190f70` | W23 status | Update review YAML statuses for MED-1 and MED-2 from `open` → `resolved`. | `docs/superpowers/reviews/2026-04-26-p3-3-9-pre-ga-high.yaml` |

### Live verification

* **F4 (LD_LIBRARY_PATH):** `/proc/<daemon>/environ` contains `LD_LIBRARY_PATH=…/onnxruntime…/lib`; `forge-next health` succeeds without manual env export. ✓
* **F11+F13 (`--from`):**
  * `--from session-1777180469 …` → `from_session=session-1777180469` ✓
  * `FORGE_SESSION_ID=… forge-next send …` → `from_session=session-1777180469` ✓ (closes F13)
  * Neither set → stderr warning + `from_session=api` ✓
* **F23 (force-index async):** `forge-next force-index` returns 0.008 s (was 30 s timeout). Concurrent `health` 0.007 s, `sessions` 0.006 s, `send` 9.4 s (was full 30 s wedge). Read paths unaffected.

### Carry-forward findings → P3-3.10

* **HIGH-1 (deferred):** W22 `spawn_blocking` is fire-and-forget — JoinHandle dropped, panics swallowed silently, SIGTERM aborts mid-write. Reviewer recommends supervisor task + `AtomicBool` reject-overlap, mirroring `kpi_reaper::run_reap_blocking` pattern. → P3-3.10 follow-up wave.
* **HIGH-2 (deferred):** W21 plan said "Same for `respond`" but `Request::SessionRespond` still has no `from_session` field, AND there is no `forge-next respond` CLI surface at all. Either descope explicitly OR add the `respond` subcommand. → P3-3.10 follow-up wave.
* **4 LOW + 2 NIT (open):** env-mutex test fragility, `LD_PRELOAD`-exclusion documentation, tautological shape-tests, env-capture documentation, protocol-meaning shift without wire bump, tracing format niceties, dead `path_for_task` rebinding (already removed in `39f84b2` MED-1 fix). Roll into the P3-3.10 backlog and address opportunistically.
* **MED-3 / MED-4 (open):** "0,0 background" heuristic false-positives on legitimately empty projects; redundant PRAGMA + busy_timeout drift from `kpi_reaper` precedent (5000 vs 10000). Acknowledge but defer.

## Wave roadmap (P3-3.9 closed; remaining 11 commits to GA)

### P3-3.10 — Quick MED/LOW + W23 carry-forwards (5 commits, ~3-4h)

| Wave | Scope | Task ID | Sources |
|------|-------|---------|---------|
| W24 | CLI cosmetics: `identity show` alias, `message-read <ID>` positional, `blast-radius <PATH>` positional | #141 | F5, F10, F19 |
| W25 | Daemon-spawn polish: doctor version sanity, hooks-warning gate, socket-bind cold-start 10s | #142 | F1, F2, F3 |
| W26 | Team primitives: idempotent `team run`, role propagation on spawn, `--project` flag, `team stop` wording | #143 | F6, F7, F8, F9 |
| W27 | `message-read` ULID lookup (full-ID vs displayed-ID inconsistency) | #144 | F12, F14 |
| W28 | review + HANDOFF + halt + carry-forward W23 HIGHs (spawn_blocking supervisor, respond CLI) | #145 | per-wave-procedure + W23 deferrals |

**Halt at end of W28** for sign-off before P3-3.11.

### P3-3.11 — Investigation MED/LOW (6 commits, ~6-8h, halt-able)

| Wave | Scope | Task ID | Source |
|------|-------|---------|--------|
| W29 | F15+F17 cross-project recall scoping investigation + fix | #146 | F15, F17 |
| W30 | F16 identity per-(agent, project) — decision + impl OR HALT-AND-BRIEF if schema change | #147 | F16 |
| W31 | F18 contradiction false-positives (Phase 9a/9b tightening) | #148 | F18 |
| W32 | F20+F22 indexer .rs file scope (watcher pattern) | #149 | F20, F22 |
| W33 | F21 force-index error UX (likely no-op after W22) | #150 | F21 |
| W34 | review + HANDOFF + halt for P3-4 | #151 | per-wave-procedure |

**Halt-and-brief at W30** if F16 needs schema change (defer to v0.6.1).
**Halt-and-brief at W29** if F15/F17 reveals architectural drift wider than scope.
**Halt at end of W34** for sign-off opening P3-4.

### P3-4 — Release & distribution (after P3-3.11 close, halted for sign-off)

7 waves per Plan A `2026-04-25-complete-production-readiness.md` §"Phase P3-4". Multi-OS dogfood → bench-fast gate flip → v0.6.0 bump → gh release → marketplace bundle (USER) → branch protection (USER) → final HANDOFF.

## Dogfood findings reference (23 findings, P3-3.8)

Source: `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md`

### HIGH (3) — closed in P3-3.9 ✓

* **F4** — daemon auto-spawn doesn't propagate `LD_LIBRARY_PATH` for ONNX → `54aeecd`.
* **F11** — `forge-next send` defaults `from_session` to `"api"` regardless of any context → `6e27eb4`.
* **F13** — `FORGE_SESSION_ID` env var doesn't propagate to message `from_session` → `6e27eb4`.
* **F23** — synchronous `force-index` blocks daemon writer loop for 30s+ → `611169b` + `39f84b2`.

### MEDIUM (7) — open in P3-3.10 (F1-F3, F9) + P3-3.11 (F15/F17, F20, F22)

* **F1** — `doctor` reports stale daemon-version after rebuild until restart. → W25.
* **F2** — `[WARN] hook: plugin hooks.json not found` warning even running in-tree. → W25.
* **F3** — daemon socket-bind cold-start timeout 3s too tight for ONNX init. → W25.
* **F9** — `team members` shows role=`?` instead of spawning template name. → W26.
* **F15+F17** — even with `--project forge`, recall returns hive/dashboard memories. → W29.
* **F20** — indexer lacks recently-modified files in code graph. → W32.
* **F22** — `blast-radius --file <path>` reports file not in code graph. → W32.

### LOW (11) — open in P3-3.10 (F5/F10/F12/F14/F19, F6/F7/F8) + P3-3.11 (F16, F18, F21)

* **F5** — `identity show` doesn't exist. → W24.
* **F6** — `team create` + `team run` aren't compositional (UNIQUE constraint). → W26.
* **F7** — `team stop` reports "0 agent(s) retired" without context. → W26.
* **F8** — Templates spawn agents with `project: (none)`. → W26.
* **F10** — `message-read` requires `--id` flag, not positional. → W24.
* **F12+F14** — `message-read --id <full-ULID>` returns "message not found" while `messages` lists it. → W27.
* **F16** — Identity facets cross-pollinate across projects. → W30 (decision needed).
* **F18** — Contradiction detector false-positives on chronological session summaries. → W31.
* **F19** — `blast-radius <path>` (positional) errors. → W24.
* **F21** — `force-index` timeout error UX unclear. → W33 (likely no-op after W22).

### WORKS-AS-EXPECTED (2) — no fix needed

* Identity (Ahankara) — 41 facets render cleanly in `compile-context` XML.
* Healing system — 8 layers all populate; manas-health surfaces them.

## Cumulative commit tally (P3-3.5..P3-3.9)

| Range | Phase | Commits |
|-------|-------|---------|
| `3e86714..7091526` | P3-3.5 W1-W8 polish | 12 |
| `8e449a5..d7c5f73` | P3-3.5 polish-review fix-wave + YAML | 2 |
| `b80ae68..daf6491` | P3-3.6 W9-W13 otel cluster bump | 5 |
| `daa76ad..6118ec2` | P3-3.7 W14+W17+W19 drift fixtures | 3 |
| `0ba3f7b` | P3-3.8 dogfood findings | 1 |
| `44c9094` | P3-3.8 close HANDOFF | 1 |
| `14279c9` | P3-3.9/3.10/3.11 plan | 1 |
| `37c90b0` | pre-compact HANDOFF | 1 |
| `54aeecd..611169b` | P3-3.9 W20-W22 (3 HIGH dogfood fixes) | 3 |
| `2ef27e8` | P3-3.9 W23 review | 1 |
| `39f84b2..e190f70` | P3-3.9 W23 fix-wave + YAML status | 2 |
| **Total since prior pre-compact** | — | **6** |
| **Total since `a9fa9af`** | — | **32** |

## Tests + verification (final state at HEAD `e190f70`)

* `cargo fmt --all --check` — clean
* `cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --features bench` — clean
* `cargo test -p forge-cli --bin forge-next commands::system::tests` — 4 passed (W21 resolver matrix)
* `cargo test -p forge-cli --bin forge-next client::tests` — 2 passed (W20 const)
* `cargo test -p forge-daemon test_force_index_produces_edges` — 1 passed (W22 sync path bypass)
* `bash scripts/check-harness-sync.sh` — OK (154 + 107)
* `bash scripts/check-review-artifacts.sh` — OK (**23** review(s) valid, 0 open blocking)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-protocol-hash.sh` — OK
* End-to-end live force-index timing: dispatch=0.008s, concurrent health=0.007s, concurrent send=9.4s (was 30s timeout for force-index alone)

## Cumulative deferred backlog

* **From P3-3.7 (drift fixtures):** W15 forge-context, W16 forge-identity, W18
  forge-coordination drift fixtures need `_with_inj` wrapper variant + injected-buggy
  callable in tests. Defer to v0.6.1+.
* **From P3-3.9 W23 review:** HIGH-1 spawn_blocking supervisor + concurrency-guard;
  HIGH-2 `SessionRespond` CLI surface (descope or add `forge-next respond`);
  4 LOW + 2 NIT cosmetics; MED-3 `(0,0)` background heuristic; MED-4 PRAGMA
  + busy_timeout consistency. Carried forward into P3-3.10 W28's review/close
  pass.
* **Earlier deferrals unchanged:** longmemeval / locomo re-run (datasets unavailable),
  SIGTERM / SIGINT chaos drill modes, criterion latency benchmarks, Prometheus bench
  composite gauge, multi-window regression baseline, manual-override label, P3-2 W1
  trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline panel.

## Tasks (next session)

11 individual tasks remaining (#141-#151) for P3-3.10 + P3-3.11:

| Task ID | Wave | Status |
|---------|------|--------|
| #141 | P3-3.10 W24 | pending |
| #142 | P3-3.10 W25 | pending |
| #143 | P3-3.10 W26 | pending |
| #144 | P3-3.10 W27 | pending |
| #145 | P3-3.10 W28 close (+ W23 HIGH-1/HIGH-2 carry-forwards) | pending |
| #146 | P3-3.11 W29 | pending |
| #147 | P3-3.11 W30 | pending |
| #148 | P3-3.11 W31 | pending |
| #149 | P3-3.11 W32 | pending |
| #150 | P3-3.11 W33 | pending |
| #151 | P3-3.11 W34 close | pending |

## Halt-and-ask map (4 sub-phase halts + 2 conditional)

1. **End of P3-3.9 W23** (3 HIGH fixes + W23 review-fix-wave): **HALT NOW** for sign-off.
2. **End of P3-3.10 W28**: halt for sign-off before P3-3.11.
3. **P3-3.11 W29** if recall scoping reveals wider architectural drift: halt + brief.
4. **P3-3.11 W30** if identity scope needs schema change: halt + brief.
5. **End of P3-3.11 W34**: halt for sign-off, opens P3-4.

## One-line summary

**P3-3.9 closed at HEAD `e190f70` (6 commits): all 3 HIGH dogfood findings (F4, F11+F13, F23) end-to-end verified live; W23 adversarial review lockable-with-fixes; MED-1 + MED-2 closed by fix-wave `39f84b2`; 2 deferred HIGHs + cosmetics carry forward to P3-3.10.** All 11 CI gates green, 23 review YAMLs valid, working tree clean. Resume at **W24 (CLI cosmetics: F5/F10/F19)** next session. After P3-3.10 + P3-3.11 close, P3-4 release halts for user sign-off.
