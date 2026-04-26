# Handoff — P3-3.10 closed (10 dogfood findings fixed) — 2026-04-26

**Public HEAD:** `7f8a694` (W28 review-status update closing MED-1+LOW-1).
**Forge-app master:** unchanged.
**Version:** v0.6.0-rc.3.
**Plan A (closed P3-1..P3-3, P3-4 queued):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5/3.6/3.7).
**Plan C (active):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md` (P3-3.9 closed; P3-3.10 closed; P3-3.11 next).
**Halt:** **PHASE-BOUNDARY HALT** — P3-3.10 closed; halt for sign-off before opening P3-3.11. Resume at **P3-3.11 W29**.

## State in one paragraph

**P3-3.10 closed at HEAD `7f8a694`** (7 commits since `46d525a`): W24 CLI cosmetics for F5/F10/F19, W25 daemon-spawn polish for F1/F2/F3, W26 team primitives for F6/F7/F8/F9, W27 single-message lookup endpoint for F12/F14, W28 adversarial review (verdict `lockable-with-fixes`; 0 BLOCKER, 1 deferred HIGH, 2 MED, 10 LOW, 3 NIT, 11 RESOLVED), W28 fix-wave closing MED-1 (rollback leak) + LOW-1 (CLI input trim), W28 review-YAML status update. **All 10 P3-3.10-targeted dogfood findings (F1, F2, F3, F5, F6, F7, F8, F9, F10, F12, F14, F19) end-to-end verified live.** All 11 CI gates green. 24 review YAMLs valid, 0 open blocking findings. Three deferred/open carry-forwards into P3-3.11: W23 HIGH-1 spawn_blocking supervisor, W23 HIGH-2 SessionRespond CLI surface, and W28 HIGH-1 SessionMessageRead caller-identity scope.

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 7f8a694
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 155 + 107
bash scripts/check-review-artifacts.sh             # 24 reviews valid
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh
cargo fmt --all --check                            # clean
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings  # 0 warnings

# Read the dogfood-fixes plan + W28 review for context, then begin P3-3.11 W29.
cat docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md
cat docs/superpowers/reviews/2026-04-26-p3-3-10-quick-fixes.yaml

# W29 first action — investigate cross-project recall scoping (F15/F17):
grep -rn "recall_bm25_project\|m\.project = ?" crates/daemon/src/db/ crates/daemon/src/server/ | head -20
```

## P3-3.10 close summary

### What landed (7 commits)

| SHA | Wave | Scope |
|-----|------|-------|
| `bd1bac6` | W24 | F5/F10/F19: positional `message-read <ID>`/`blast-radius <PATH>` (+ legacy `--id`/`--file` aliases), `identity show` clap alias for `list`. 5 unit tests. |
| `b965d0b` | W25 | F1: CLI doctor flags stale daemon when `cli_version != daemon.version`. F2: hooks-warning gated on plugin-install root presence (no warn when standalone). F3: socket-bind cold-start 3s→10s. |
| `eb55a2d` | W26 | F6: `crate::teams::run_team` upserts team by name (idempotent). F7: `team stop` annotates `(team had no spawned agents)` when retired==0. F8: `Request::RunTeam.project: Option<String>` plumbed through to `spawn_agent` → session.project. F9: CLI reads correct JSON keys `role`/`agent_status` (was `template_name`/`status`). 2 new tests + protocol-hash bump. |
| `85712a8` | W27 | F12+F14: new `Request::SessionMessageRead { id }` + `ResponseData::SessionMessageItem`. Daemon `read_message_by_id_or_prefix` (exact → unambiguous prefix). CLI rewrite to dedicated endpoint. Variant census 124 + harness-sync 155 + protocol-hash bump. |
| `cd2d733` | W28 review | Adversarial review on W24-W27. Verdict `lockable-with-fixes`. 0 BLOCKER / 1 HIGH (deferred) / 2 MED / 10 LOW / 3 NIT / 11 RESOLVED. |
| `118f0db` | W28 fix | Address MED-1 (rollback arm now DELETEs partial team_member rows so pre-existing teams don't leak) + LOW-1 (CLI message_read trims input). |
| `7f8a694` | W28 status | Update review YAML statuses for MED-1 + LOW-1 from `open` → `resolved`. |

### Live verification (key surfaces)

* **F5**: `forge-next identity show` → 41 facets render ✓
* **F10**: `forge-next message-read <FULL-ULID>` parses; `--id <FULL>` legacy form parses ✓
* **F19**: `forge-next blast-radius <PATH>` parses; `--file <PATH>` legacy parses ✓
* **F1**: doctor renders `Version: 0.6.0-rc.3 (bd1bac6)` with no stale-decoration when CLI/daemon match ✓
* **F2**: doctor shows `[OK] hook: running outside a Claude Code plugin install (no hooks expected)` (was `[WARN]`) ✓
* **F3**: source confirms 100×100ms = 10s poll ceiling ✓
* **F6**: `team run --name X` then `team run --name X` again — second succeeds with `1 agent(s) spawned` (was UNIQUE failure) ✓
* **F8**: `team run --project forge` → all spawned agents `project: forge` (was `(none)`) ✓
* **F9**: `team members` renders `01KQ…: product-manager [idle]` (was `01KQ…: ? [idle]`) ✓
* **F12+F14**: `message-read <FULL-ULID>` and `message-read <8-CHAR-PREFIX>` both resolve; nonexistent ID errors cleanly with `message not found: <ID>` ✓

### Carry-forward findings → P3-3.11

* **W23 HIGH-1 (deferred)** — `tokio::task::spawn_blocking` for force-index drops its `JoinHandle`: panics swallowed, SIGTERM aborts mid-write split-brain risk, no concurrency guard. Reviewer-recommended fix: supervisor task + `AtomicBool` reject-overlap, mirroring `kpi_reaper::run_reap_blocking`.
* **W23 HIGH-2 (deferred)** — `Request::SessionRespond` still has no `from_session` field, AND there's no `forge-next respond` CLI surface at all. Decide between explicit descope OR adding the `respond` subcommand to close the F11/F13 round-trip.
* **W28 HIGH-1 (deferred)** — `read_message_by_id_or_prefix` is unscoped (no `to_session`/`from_session` filter). Single-tenant daemon means not a hard auth boundary today, but the architectural contract weakened from W27. Reviewer-recommended fix: optional `caller_session: Option<String>` on `Request::SessionMessageRead` that scopes the SQL when set.
* **W28 MED-2 (open)** — F1 stale-version detection only catches Cargo.toml version-string drift, not git-sha drift. Common dev workflow (commit, rebuild, daemon stays on prior commit) is silently reported as "matched". Fix path: also compare `option_env!("VERGEN_GIT_SHA")` against daemon-reported `git_sha`.
* **W28 LOW-2..LOW-10 + NIT-1..NIT-3 (open)** — cosmetic (LIKE wildcards, error wording, partial-retire visibility, contract-test pinning of JSON shape, env-var override for boot timeout, project validation, broken-symlink detection, missing read_message tests, `team_member` retired-row filter, dispatcher message wording, terminal width, ID truncation length). Roll into P3-3.11 W34 close.

## Wave roadmap (P3-3.10 closed; remaining 6 commits to P3-4)

### P3-3.11 — Investigation MED/LOW (6 commits, ~6-8h, halt-able)

| Wave | Scope | Task ID | Source |
|------|-------|---------|--------|
| W29 | F15+F17 cross-project recall scoping investigation + fix | #146 | F15, F17 |
| W30 | F16 identity per-(agent, project) — decision + impl OR HALT-AND-BRIEF if schema change | #147 | F16 |
| W31 | F18 contradiction false-positives (Phase 9a/9b tightening) | #148 | F18 |
| W32 | F20+F22 indexer .rs file scope (watcher pattern) | #149 | F20, F22 |
| W33 | F21 force-index error UX (likely no-op after W22) | #150 | F21 |
| W34 | review + HANDOFF + halt + carry-forward W23/W28 deferred HIGHs | #151 | per-wave-procedure + W23/W28 deferrals |

**Halt-and-brief at W30** if F16 needs schema change (defer to v0.6.1).
**Halt-and-brief at W29** if F15/F17 reveals architectural drift wider than scope.
**Halt at end of W34** for sign-off opening P3-4.

### P3-4 — Release & distribution (after P3-3.11 close, halted for sign-off)

7 waves per Plan A `2026-04-25-complete-production-readiness.md` §"Phase P3-4". Multi-OS dogfood → bench-fast gate flip → v0.6.0 bump → gh release → marketplace bundle (USER) → branch protection (USER) → final HANDOFF.

## Dogfood findings reference (23 findings, P3-3.8)

Source: `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md`

### HIGH (3) — closed in P3-3.9 ✓

* **F4** → `54aeecd`. **F11** → `6e27eb4`. **F13** → `6e27eb4`. **F23** → `611169b` + `39f84b2`.

### MEDIUM (7) — 4 closed in P3-3.10 ✓ + 3 in P3-3.11

* **F1** → `b965d0b` ✓. **F2** → `b965d0b` ✓. **F3** → `b965d0b` ✓. **F9** → `eb55a2d` ✓.
* **F15+F17** → W29.  **F20** → W32.  **F22** → W32.

### LOW (11) — 8 closed in P3-3.10 ✓ + 3 in P3-3.11

* **F5** → `bd1bac6` ✓. **F6** → `eb55a2d` ✓. **F7** → `eb55a2d` ✓. **F8** → `eb55a2d` ✓.
* **F10** → `bd1bac6` ✓. **F12+F14** → `85712a8` ✓. **F19** → `bd1bac6` ✓.
* **F16** → W30 (decision needed).  **F18** → W31.  **F21** → W33 (likely no-op).

### WORKS-AS-EXPECTED (2) — no fix needed

* Identity (Ahankara) — 41 facets render cleanly in `compile-context` XML.
* Healing system — 8 layers all populate; manas-health surfaces them.

## Cumulative commit tally (P3-3.5..P3-3.10)

| Range | Phase | Commits |
|-------|-------|---------|
| `3e86714..7091526` | P3-3.5 W1-W8 polish | 12 |
| `8e449a5..d7c5f73` | P3-3.5 polish-review fix-wave + YAML | 2 |
| `b80ae68..daf6491` | P3-3.6 W9-W13 otel cluster bump | 5 |
| `daa76ad..6118ec2` | P3-3.7 W14+W17+W19 drift fixtures | 3 |
| `0ba3f7b..14279c9` | P3-3.8 dogfood + plan-doc | 3 |
| `37c90b0` | pre-compact HANDOFF | 1 |
| `54aeecd..611169b` | P3-3.9 W20-W22 (3 HIGH dogfood fixes) | 3 |
| `2ef27e8..e190f70` | P3-3.9 W23 review + fix-wave + YAML status | 3 |
| `46d525a` | P3-3.9 close HANDOFF | 1 |
| `bd1bac6..85712a8` | P3-3.10 W24-W27 (10 dogfood fixes) | 4 |
| `cd2d733..7f8a694` | P3-3.10 W28 review + fix-wave + YAML status | 3 |
| **Total since `a9fa9af`** | — | **40** |
| **Total this session (since `37c90b0`)** | — | **14** |

## Tests + verification (final state at HEAD `7f8a694`)

* `cargo fmt --all --check` — clean
* `cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --features bench` — clean
* `cargo test -p forge-cli --bin forge-next` — 92 passed (incl. W24's 5 new clap-parse tests)
* `cargo test -p forge-daemon teams::tests` — 38 passed (incl. W26's 2 new run_team tests)
* `cargo test -p forge-core --lib protocol::contract_tests` — 37 passed (incl. W27's variant census 124)
* `cargo test -p forge-daemon test_force_index_produces_edges` — 1 passed (W22 sync path bypass)
* `bash scripts/check-harness-sync.sh` — OK (**155** + 107)
* `bash scripts/check-review-artifacts.sh` — OK (**24** review(s) valid, 0 open blocking)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-protocol-hash.sh` — OK (`1dca2da7…`)

## Cumulative deferred backlog

* **From P3-3.7 (drift fixtures):** W15 forge-context, W16 forge-identity, W18
  forge-coordination drift fixtures need `_with_inj` wrapper variant + injected-buggy
  callable in tests. Defer to v0.6.1+.
* **From P3-3.9 W23 review:** HIGH-1 spawn_blocking supervisor + concurrency-guard;
  HIGH-2 `SessionRespond` CLI surface (descope or add `forge-next respond`);
  4 LOW + 2 NIT cosmetics; MED-3 `(0,0)` background heuristic; MED-4 PRAGMA
  + busy_timeout consistency. **Carry into P3-3.11 W34**.
* **From P3-3.10 W28 review:** HIGH-1 SessionMessageRead caller-identity scope;
  MED-2 git-sha drift detection; LOW-2..LOW-10 (LIKE escape, error-wrapping
  wording, partial-retire visibility, JSON-shape contract test, env-var boot
  timeout, project validation, broken-symlink detection, missing helper unit
  tests, retired-row filter on team_member); NIT-1..NIT-3 (clap message
  wording, terminal-width decoration, ID truncation length). **Carry into
  P3-3.11 W34**.
* **Earlier deferrals unchanged:** longmemeval / locomo re-run, SIGTERM/SIGINT
  chaos drill modes, criterion latency benchmarks, Prometheus bench composite
  gauge, multi-window regression baseline, manual-override label, P3-2 W1
  trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline
  panel.

## Tasks (next session)

6 individual tasks remaining (#146-#151) for P3-3.11:

| Task ID | Wave | Status |
|---------|------|--------|
| #146 | P3-3.11 W29 (F15+F17) | pending |
| #147 | P3-3.11 W30 (F16) | pending (halt-and-brief if schema) |
| #148 | P3-3.11 W31 (F18) | pending |
| #149 | P3-3.11 W32 (F20+F22) | pending |
| #150 | P3-3.11 W33 (F21) | pending |
| #151 | P3-3.11 W34 close | pending |

## Halt-and-ask map (3 sub-phase halts + 2 conditional)

1. **End of P3-3.10 W28**: **HALT NOW** for sign-off before P3-3.11.
2. **P3-3.11 W29** if recall scoping reveals wider architectural drift: halt + brief.
3. **P3-3.11 W30** if identity scope needs schema change: halt + brief.
4. **End of P3-3.11 W34**: halt for sign-off, opens P3-4.

## One-line summary

**P3-3.10 closed at HEAD `7f8a694` (7 commits): 10 dogfood findings (F1/F2/F3/F5/F6/F7/F8/F9/F10/F12/F14/F19) end-to-end verified live; W28 adversarial review lockable-with-fixes; MED-1 + LOW-1 closed by fix-wave `118f0db`; 1 deferred HIGH carries forward.** All 11 CI gates green, 24 review YAMLs valid, working tree clean. Resume at **W29 (cross-project recall scoping investigation)** next session. After P3-3.11 closes, P3-4 release halts for user sign-off.
