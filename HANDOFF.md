# Handoff — P3-3.11 W30..W34 closed (F16+F18+F20+F21+F22 fixed) — 2026-04-26

**Public HEAD:** `617e1c6` (W33 F21 verification doc).
**Forge-app master:** unchanged.
**Version:** v0.6.0-rc.3.
**Plan A (closed P3-1..P3-3, P3-4 queued):** `docs/superpowers/plans/2026-04-25-complete-production-readiness.md`.
**Plan B (closed):** `docs/superpowers/plans/2026-04-26-v0.6.0-polish-wave.md` (P3-3.5/3.6/3.7).
**Plan C (closed):** `docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md` — all 21 actionable dogfood findings closed across P3-3.9..P3-3.11.
**Halt:** **PHASE HALT** — P3-3.11 (and Plan C) closed. Halt for sign-off before opening **P3-4 (release v0.6.0)**.

## State in one paragraph

**P3-3.11 W30..W34 closed at HEAD `617e1c6`** (7 commits since `a6db621`):
W30 (4 commits, F16) applies the W29 sentinel-replacement architecture to the
identity table — schema migration, DAO helper, write-path enforcement at
every store_identity site, project-scoped readers, `Request::ListIdentity`
gains `project` + `include_global_identity` fields, `compile_context`
filters by project, CLI flags, protocol-hash bumped `5b9cada23419… →
d23de2ac97f3…`, live-verified on 219 MB DB. W31 (F18) tightens Phase 9b
content-contradiction detection — adds opposite-strong valence gate
(mirrors Phase 9a) + tighter Jaccard floors (title 0.5→0.7, content
0.3→0.20). W32 (F20+F22) adds a fresh-mtime gate inside the indexer —
heavy reindex runs every 60 s when files change, falls back to the
configured 300 s safety-net when quiet. W33 (F21) is a no-op verify of
the `force-index` UX closure shipped in W22. All CI gates green:
harness-sync 155+107, protocol-hash, fmt clean, clippy 0 warnings, 24
review YAMLs valid. Three deferred HIGHs from W23/W28 still open (carry
to v0.6.1+ unless the user wants them in P3-4 close).

## First actions after `/compact` or session resume

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
git log --oneline -10                              # HEAD 617e1c6
git status --short                                 # expect clean
bash scripts/check-harness-sync.sh                 # 155 + 107
bash scripts/check-review-artifacts.sh             # 24 reviews valid
bash scripts/check-license-manifest.sh
bash scripts/check-protocol-hash.sh                # d23de2ac97f3…
cargo fmt --all --check                            # clean
cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings  # 0 warnings

# Optional: re-read the dogfood-fixes plan + W30/W33 verifications.
cat docs/superpowers/plans/2026-04-26-dogfood-fixes-plan.md
cat docs/benchmarks/results/2026-04-26-w30-live-verification.md
cat docs/benchmarks/results/2026-04-26-w33-f21-verification.md

# When ready to open P3-4 (release v0.6.0):
# Plan A §"Phase P3-4" has the 7-wave release sequence.
cat docs/superpowers/plans/2026-04-25-complete-production-readiness.md
```

## P3-3.11 W30..W34 close summary

### What landed (7 commits)

| SHA | Commit | Scope |
|-----|--------|-------|
| `ec81e6d` | W30 c1 | Schema migration: `ALTER TABLE identity ADD COLUMN project TEXT NOT NULL DEFAULT '_global_'` + idx_identity_project + idx_identity_agent_project + defensive UPDATE for legacy NULL/empty rows; +1 unit test. |
| `4cbe030` | W30 c2 | `IdentityFacet.project: Option<String>` field; `store_identity` normalises via `project_or_global` and dedup keys on (agent, description, project); 4 production write sites tag with project (extractor → session.project, teams → spawn session, sync_import → remote pass-through, CLI defers to flag); 2 new readers `list_identity_for_project` + `list_identity_for_user_project`; W29-latent bench fix (D3 + end_to_end re-routed via `_with_globals` + bench seed normalises via `project_or_global`); 9 new unit tests. |
| `b958808` | W30 c3 | `Request::ListIdentity` gains `project` + `include_global_identity` (`#[serde(default)]`); `compile_static_prefix(_with_inj)` gains `project: Option<&str>`; `compile_context` threads through; handler routes through project-scoped variant; `forge-next identity list/set --project P --include-global-identity`; protocol-hash bumped to `d23de2ac97f3…`; 5 sweep sites + 13 internal call-sites updated. |
| `782636c` | W30 c4 | Live verification on `~/.forge/forge.db` (219 MB): zero NULL/empty identity rows after migration; `identity list --project forge` returns 0 rows (Hive Finance pollution closed); `--include-global-identity` admits the 43 globals on demand; write paths emit forge / `_global_` correctly. Procedure documented in `docs/benchmarks/results/2026-04-26-w30-live-verification.md`. |
| `9ca7e2b` | W31 | Phase 9b content-contradiction detection now requires opposite-strong valence (mirrors Phase 9a's `valence IN ('positive','negative') AND intensity > 0.5` gate) + same-valence pair skip; Jaccard floors tightened (title 0.5→0.7, content 0.3→0.20). 4 existing tests updated for the new contract; 3 new W31 regression tests (chronological neutral-valence pair, same-valence pair, low-intensity pair). |
| `fa19a54` | W32 | Indexer fresh-mtime gate: `CODE_FILE_EXTENSIONS` + `code_files_max_mtime` helper; loop wakes every `FAST_TICK = 60s`, runs heavy LSP reindex only when (a) safety-net interval (default 300 s) elapsed OR (b) at least one tracked code file's mtime is newer than `last_completed_at`. Worst-case responsiveness 60 s on save; CPU on quiet projects bounded by stat-walk only. +4 unit tests (incl. extension-coverage parity contract). |
| `617e1c6` | W33 | F21 verification doc — `force-index` returns in 9 ms (was 30 s+ pre-W22) with clear background-dispatch message. F21 closed end-to-end by W22 + W32. |

### Live DB verification (key surfaces — W30)

* **Pre-W30 distribution**: identity table has no `project` column at all; 43 active facets (42 claude-code + 1 codex) all per-agent.
* **Post-W30 distribution**: identity table has `project TEXT NOT NULL DEFAULT '_global_'`; 43 rows all migrated to `_global_` (legacy rows have no source data to recover original project); future writes tag explicitly.
* **Strict identity list**: `identity list --project forge` → 0 rows (Hive Finance / dashboard topics no longer pollute the forge agent's compile_context). ✓
* **Opt-in identity list**: `identity list --project forge --include-global-identity` → 42 rows (broad agent-wide identity preserved on demand). ✓
* **Write tagged**: `identity set --facet role --project forge ...` → row stored with `project = 'forge'`. ✓
* **Write untagged**: `identity set --facet expertise ...` → row stored with `project = '_global_'`. ✓
* **Strict view after tagged write**: only the explicitly forge-tagged row appears; globals do NOT leak. ✓

### Self-review (W34 close)

Manual adversarial pass (general-purpose Explore agent crashed mid-run; manual sweep covered the same surface). No BLOCKERs, no HIGHs.

* **MED-1 (deferred)** — `code_files_max_mtime` walks the project once per 60 s fast tick. Bounded by `MAX_FILES_PER_PROJECT = 5000`, so worst-case ~5000 `metadata` syscalls per minute. Fine for local SSD; on slow / NFS-mounted projects this could become noticeable. Acceptable for v0.6.0; v0.6.1+ could keep a running max in `WatchedProject` state and only stat the deltas reported by a notify-style watcher. **No regression vs pre-W32 (which ran the LSP indexer every 5 minutes — far heavier).**
* **MED-2 (deferred)** — W30 schema migration is a no-op retag for legacy rows (all 43 land as `_global_`; the original project tagging information is lost forever). Operators who want forge-only identity can `identity remove <id>` + `identity set --project forge ...`. **Document in v0.6.0 release notes.**
* **LOW-1 (cosmetic)** — `Memory::new(...).with_valence(...)` boilerplate in W31 tests could move to a helper. Cosmetic; v0.6.1+.
* **LOW-2 (cosmetic)** — `bench/forge_isolation::drift_fixtures` only exercise D1 + D6. Adding a planted-neutral-pair drift fixture for the W31 contradiction surface would tighten the bench. Defer to v0.6.1+ as an optional new bench dim.

### Carry-forward findings → P3-4 W7 OR v0.6.1+

* **W23 HIGH-1 (deferred)** — `tokio::task::spawn_blocking` for force-index drops its `JoinHandle`: panics swallowed, SIGTERM aborts mid-write split-brain risk, no concurrency guard. Reviewer-recommended fix: supervisor task + `AtomicBool` reject-overlap, mirroring `kpi_reaper::run_reap_blocking`. **Carry to v0.6.1+ unless P3-4 W7 explicitly addresses.**
* **W23 HIGH-2 (deferred)** — `Request::SessionRespond` still has no `from_session` field, AND there's no `forge-next respond` CLI surface at all. Decide between explicit descope OR adding the `respond` subcommand to close the F11/F13 round-trip. **Carry to v0.6.1+.**
* **W28 HIGH-1 (deferred)** — `read_message_by_id_or_prefix` is unscoped (no `to_session`/`from_session` filter). Single-tenant daemon means not a hard auth boundary today, but the architectural contract weakened from W27. Reviewer-recommended fix: optional `caller_session: Option<String>` on `Request::SessionMessageRead` that scopes the SQL when set. **Carry to v0.6.1+.**
* **W28 MED-2 (open)** — F1 stale-version detection only catches Cargo.toml version-string drift, not git-sha drift. Common dev workflow (commit, rebuild, daemon stays on prior commit) is silently reported as "matched". Fix path: also compare `option_env!("VERGEN_GIT_SHA")` against daemon-reported `git_sha`. **Carry to v0.6.1+.**
* **W28 LOW-2..LOW-10 + NIT-1..NIT-3 (open)** — cosmetic backlog. **Carry to v0.6.1+.**
* **W29 nice-to-have backlog (deferred to v0.6.1+)**: bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail when project resolution falls through to `_global_`; optional config gate `memory.require_project = true`.
* **W30 nice-to-have**: extractor `tracing::warn!` when identity facet resolution falls through to `_global_` (parallel to the W29 nice-to-have). v0.6.1+.

## Wave roadmap (P3-3.11 closed; next is P3-4)

### P3-4 — Release & distribution (after sign-off here)

7 waves per Plan A `2026-04-25-complete-production-readiness.md` §"Phase P3-4". Multi-OS dogfood → bench-fast gate flip → v0.6.0 bump → gh release → marketplace bundle (USER) → branch protection (USER) → final HANDOFF.

| Wave | Scope | Auto / User |
|------|-------|-------------|
| W1 | Multi-OS dogfood final sweep | Auto (Linux); user (macOS) |
| W2 | Bench-fast required-gate flip (T17) | Auto if 14 green master runs |
| W3 | v0.6.0 version bump (rc.3 → 0.6.0) | Auto |
| W4 | GitHub release artifacts + notes | Auto if `gh` auth |
| W5 | Marketplace submission bundle | Auto preparation; user submits |
| W6 | Branch protection rules | Auto preparation; user applies |
| W7 | Final HANDOFF + close-out | Auto |

## Dogfood findings reference (23 findings, P3-3.8) — all closed

Source: `docs/benchmarks/results/2026-04-26-forge-dogfood-findings.md`

### HIGH (3) — closed in P3-3.9 ✓

* **F4** → `54aeecd`. **F11** → `6e27eb4`. **F13** → `6e27eb4`. **F23** → `611169b` + `39f84b2`.

### MEDIUM (7) — all closed

* **F1** → `b965d0b` ✓. **F2** → `b965d0b` ✓. **F3** → `b965d0b` ✓. **F9** → `eb55a2d` ✓.
* **F15+F17** → `ede5c38..7523f54` ✓ (P3-3.11 W29).
* **F20+F22** → `fa19a54` ✓ (P3-3.11 W32).

### LOW (11) — all closed

* **F5** → `bd1bac6` ✓. **F6** → `eb55a2d` ✓. **F7** → `eb55a2d` ✓. **F8** → `eb55a2d` ✓.
* **F10** → `bd1bac6` ✓. **F12+F14** → `85712a8` ✓. **F19** → `bd1bac6` ✓.
* **F16** → `ec81e6d..782636c` ✓ (P3-3.11 W30).
* **F18** → `9ca7e2b` ✓ (P3-3.11 W31).
* **F21** → verified closed by W22 (`611169b`); doc at `617e1c6` (P3-3.11 W33).

### WORKS-AS-EXPECTED (2) — no fix needed

* Identity (Ahankara) — 41 facets render cleanly in `compile-context` XML.
* Healing system — 8 layers all populate; manas-health surfaces them.

## Cumulative commit tally (P3-3.5..P3-3.11)

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
| `e05e2c6` | P3-3.10 close HANDOFF | 1 |
| `ede5c38..7523f54` | P3-3.11 W29 (F15+F17 sentinel + strict scope + live verify) | 4 |
| `a6db621` | P3-3.11 W29 close HANDOFF | 1 |
| `ec81e6d..782636c` | P3-3.11 W30 (F16 identity per-(agent, project)) | 4 |
| `9ca7e2b` | P3-3.11 W31 (F18 contradiction tightening) | 1 |
| `fa19a54` | P3-3.11 W32 (F20+F22 indexer fresh-mtime gate) | 1 |
| `617e1c6` | P3-3.11 W33 (F21 verify) + W34 close (this HANDOFF) | 2 |
| **Total since `a9fa9af`** | — | **54** |
| **Total this session (since `e05e2c6`)** | — | **12** |

## Tests + verification (final state at HEAD `617e1c6`)

* `cargo fmt --all --check` — clean
* `cargo clippy --workspace --tests --features bench -- -W clippy::all -D warnings` — 0 warnings
* `cargo build --workspace --features bench` — clean
* `cargo test -p forge-daemon --lib --features bench db::` — 239 passed (incl. W29 + W30 schema migration tests + W30 ops tests)
* `cargo test -p forge-daemon --lib --features bench bench::forge_isolation` — 19 passed (W29 latent bench fix integrated)
* `cargo test -p forge-daemon --lib --features bench workers::consolidator` — 44 passed (incl. 3 new W31 regression tests)
* `cargo test -p forge-daemon --lib --features bench workers::indexer` — 23 passed (incl. 4 new W32 tests)
* `cargo test -p forge-core --lib` — 109 passed (incl. updated test_identity_facet_serde)
* `bash scripts/check-harness-sync.sh` — OK (155 + 107)
* `bash scripts/check-review-artifacts.sh` — OK (24 review(s) valid, 0 open blocking)
* `bash scripts/check-license-manifest.sh` — OK
* `bash scripts/check-protocol-hash.sh` — OK (`d23de2ac97f3…`)

Pre-existing test failure in the full bench suite (unchanged from W29):

* `workers::disposition::tests::test_step_for_bench_parity_with_tick_for_agent` — date-sensitive: hardcoded fixture date `2026-04-25 10:00:00` falls outside the 24h `query_recent_sessions_for_agent` window now that today is `2026-04-26`. Pre-existing test bug, unrelated to W30..W34.

## Cumulative deferred backlog

* **From P3-3.7 (drift fixtures):** W15 forge-context, W16 forge-identity, W18 forge-coordination drift fixtures need `_with_inj` wrapper variant + injected-buggy callable in tests. Defer to v0.6.1+.
* **From P3-3.9 W23 review:** HIGH-1 spawn_blocking supervisor + concurrency-guard; HIGH-2 `SessionRespond` CLI surface (descope or add `forge-next respond`); 4 LOW + 2 NIT cosmetics; MED-3 `(0,0)` background heuristic; MED-4 PRAGMA + busy_timeout consistency. **Carry into P3-4 W7 OR v0.6.1+**.
* **From P3-3.10 W28 review:** HIGH-1 SessionMessageRead caller-identity scope; MED-2 git-sha drift detection; LOW-2..LOW-10 (LIKE escape, error-wrapping wording, partial-retire visibility, JSON-shape contract test, env-var boot timeout, project validation, broken-symlink detection, missing helper unit tests, retired-row filter on team_member); NIT-1..NIT-3 (clap message wording, terminal-width decoration, ID truncation length). **Carry into P3-4 W7 OR v0.6.1+**.
* **From P3-3.11 W29 nice-to-haves**: bench D6 strict-project precision dim; auto-extractor `tracing::warn!` audit trail; optional config gate `memory.require_project = true`. **v0.6.1+**.
* **From P3-3.11 W30..W34 nice-to-haves**: extractor warn on identity-project fall-through; W31 drift fixture for contradiction surface; W32 stat-walk optimization for very large trees on slow storage; helper to compress `Memory::new(...).with_valence(...)` test boilerplate. **v0.6.1+**.
* **Earlier deferrals unchanged:** longmemeval / locomo re-run, SIGTERM/SIGINT chaos drill modes, criterion latency benchmarks, Prometheus bench composite gauge, multi-window regression baseline, manual-override label, P3-2 W1 trace-handler behavioral test, per-tenant Prometheus labels, OTLP timeline panel.

## Tasks (next session)

| Task ID | Wave | Status |
|---------|------|--------|
| #101 | P3-4 release v0.6.0 | pending (halt for sign-off) |
| #147..#150 | P3-3.11 W30..W33 | completed ✓ |
| #151 | P3-3.11 W34 close | completed ✓ (this HANDOFF) |

## Halt-and-ask map

1. **HALT NOW** — P3-3.11 closed. Plan C closed. All 21 actionable dogfood findings closed. Confirm direction before opening **P3-4 (release v0.6.0)**.
2. **End of P3-4 W7**: final close-out HANDOFF; v0.6.0 ships.

## One-line summary

**P3-3.11 W30..W34 closed at HEAD `617e1c6` (7 commits): F16 identity per-(agent, project) scoped end-to-end with sentinel + DAO + protocol field + CLI flag (live-verified on 219 MB DB); F18 contradiction false-positives gated on opposite-strong valence + tighter Jaccard; F20+F22 indexer fresh-mtime gate (60 s responsiveness); F21 verified closed by W22.** All 21 actionable dogfood findings closed across P3-3.9..P3-3.11. CI gates green, 24 review YAMLs valid, working tree clean. **Resume at P3-4 (release v0.6.0)** next session — halt for sign-off before opening.
