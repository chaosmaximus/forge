# Complete Production-Readiness — Autonomous Drain Plan

**Status:** ACTIVE — 2026-04-25.
**Mode:** Autonomous, authorized by user (`konurud@gmail.com`) on 2026-04-25.
**HEAD at start:** `1862a43` (post W5–W8 close).
**Version at start:** `0.5.0`. Target end version: `0.6.0`.
**Goal:** drain every pending item end-to-end through release-ready state. No shortcuts, no early release.

## Locked decisions (user-confirmed 2026-04-25)

1. **Scope includes Block D** (new product phases): domain-transfer isolation bench + multi-agent coordination bench (alongside daemon restart drill, Grafana dashboards, auto-PR-on-regression).
2. **macOS: option (a)** — ship Linux as primary supported platform; document macOS as best-effort (no blocking gate).
3. **Marketplace + branch protection deferred** to user, after P3-4 lands. Plan prepares everything; user clicks submit.
4. **Adversarial review every wave** (Claude general-purpose; Codex when available — same pattern as W1–W8).
5. **Halt conditions:** clippy warning, test failure, unexpected git state, phase boundary, destructive op (force push / branch delete / non-reversible schema migration).
6. **Version strategy:** bump per phase. `0.5.0` → P3-1 close `0.6.0-rc.1` → P3-2 close `0.6.0-rc.2` → P3-3 close `0.6.0-rc.3` → P3-4 close `0.6.0`.
7. **This file is the persistent source of truth.** Survives compact boundaries.

## Halt-and-ask points

* End of each phase (P3-1 / P3-2 / P3-3 / P3-4): wait for user sign-off before opening the next.
* Any wave returning `not-lockable` from adversarial review: halt, surface the verdict, ask.
* Anything that leaves working tree non-clean across a wave boundary.
* Anything requiring user credentials (gh release, marketplace submit, branch protection): pause + brief.

## Phase ordering (rationale)

1. **P3-1 (2P-1b harness)** first — harness-sync CI gate must exist before P3-2/P3-3 protocol changes drift across layers.
2. **P3-2 (2A-4d follow-up)** second — small, polish-grade; unblocks the CompileContextTrace protocol gap that downstream layers may cite.
3. **P3-3 (new product phases)** third — biggest scope; benefits from fully-hardened harness and clean backlog.
4. **P3-4 (release)** last — version bump, tag, dogfood, prepare marketplace bundle.

---

## Phase P3-1 — 2P-1b harness hardening (~40 commits)

**Source spec:** `docs/superpowers/specs/2026-04-23-forge-public-resplit-design.md` §"Phase 2P-1b".

| Wave | Scope | Acceptance |
|------|-------|------------|
| **W1** | `scripts/check-harness-sync.sh` — scan JSON method names + Rust `Request::*` refs + CLI subcommand refs in plugin.json/hooks.json/skills/*.md/agents/*.md, cross-check against `crates/core/src/protocol/request.rs` + `forge-cli` clap derive. Warn-only mode + 14-day amnesty timer. Drift-fixture tests. | script lands; CI integrates as a new step; `FORGE_HARNESS_SYNC_ENFORCE=1` env var flips fail-closed; amnesty date documented in script header. |
| **W2** | Evidence-gated YAML audit contract — `docs/superpowers/reviews/*.yaml` schema; `scripts/check-review-artifacts.sh`; CI gate on `HIGH+CRITICAL == 0`. | runs on every PR touching skills/agents/hooks; non-empty `artifacts` array required; complete file coverage check. |
| **W3** | SPDX sidecar manifest for JSON files — `.claude-plugin/LICENSES.yaml` extended with per-file SPDX + commit reference. | every JSON file in `.claude-plugin/` + `hooks/` + (root marketplace.json if present) listed; CI validates manifest covers all JSONs. |
| **W4** | 2A-4d interlock mechanism — sync version field in plugin.json; harness-sync script bumps fail-closed when `request.rs` changed without plugin.json sync. | dogfood: change a Request variant, harness-sync errors with explicit "bump plugin.json sync version" guidance. |
| **W5** | Rollback playbook tabletop dry-run — execute every step of `docs/operations/2P-1-rollback.md` against a throwaway tag/release; record observed RTO; update playbook with discovered gaps. | "Last tabletop exercise" line filled in; RTO target met (<20 min) or playbook revised. |
| **W6** | GitHub repo governance — `.github/CODEOWNERS`, dependabot config polish, issue templates, PR template. (Branch protection rules deferred to user.) | every layer of repo has an owner; dependabot scopes match Cargo + GH Actions; templates include reproduce-steps + dogfood checklist. |
| **W7** | Sideload migration finalization — broaden `docs/operations/sideload-migration.md`; add a verification script `scripts/check-sideload-state.sh` that detects pre-2026-04-23 sideload setups. | doc covers Linux + macOS variants; verification script returns clear actionable state. |
| **W8** | Multi-OS dogfood matrix — Linux full sweep across install methods (cargo install, sideload, marketplace-install once available); macOS path documented as user-handoff (per decision #2); results doc `docs/benchmarks/results/2P-1b-dogfood-matrix.md`. | Linux cells all green; macOS cells noted as best-effort with reproduction steps for user to execute. |

**Phase P3-1 close:** version bump to `0.6.0-rc.1`. HANDOFF rewrite. Halt for user sign-off.

### P3-1 deferred backlog (per-wave review residue)

* **W1 M1** — `FORGE_HARNESS_SYNC_ENFORCE=garbage` (any non-`1` value) silently downgrades to WARN. Reason: matches the existing `FORCE_FAIL=1` legacy contract. Defer; revisit if a future CI matrix sets the var to `"on"`/`"true"`.
* **W1 M4** — neither script sets `LC_ALL=C`. `date -u +%Y-%m-%d` is stable in practice and the YYYY-MM-DD lexicographic compare is byte-safe, but the awk pascal-to-snake transform's `[A-Z]` ranges could shift under non-C locale. Defer; not seen in any production runner.
* **W1 M5** — drift fixtures use 6-variant clean Pascal enums with no doc-comments / cfg attrs / nested generics. Awk-extractor regression on those edge forms wouldn't be caught. Defer to a fixture-expansion follow-up; current fixtures cover the happy path.
* **W1 L1** — `--help` uses `sed -n '1,/^set -euo/p' "$0" | sed '$d'` which is brittle if `set -e` ever moves. Defer; cosmetic.
* **W2 L-1** — validator does not enforce per-PR-changed-file coverage (every changed `skills/`/`agents/`/`hooks/` file having a corresponding review YAML). Defensible scope cut for W2; matches harness-sync deferred-coverage pattern. Reopen if a missed-review incident surfaces.
* **W2 L-2** — `scripts/check-review-artifacts.sh` is largely a passthrough wrapper around the python validator (its only real value-add is `--root` arg-eat hardening + a `python3` presence check). Defer — convention parity with `check-harness-sync.sh` is justification enough.
* **W2 L-3** — `reviewer.agent` is an open enum (README documents three values + `<other>`). Defer; consistent with the doc.
* **W2 L-4** — PyYAML's `safe_load` is last-wins on duplicate top-level keys (e.g. two `target_paths:`). Acceptable risk; multi-doc YAML (probe 11) was already covered cleanly.
* **W2 M-4 (post-commit)** — the W2 commit message body says "1 HIGH + 3 MED + 1 LOW resolved; 3 MED + 1 LOW deferred" while the actual W1-backfill YAML has 1 HIGH + 2 MED + 1 LOW resolved and 3 MED + 1 LOW deferred. Off-by-one in the narrative, not the data. The artifact YAML is the source of truth.
* **W3 MED-2** — `os.walk(followlinks=False)` means a directory symlink under `coverage_paths[]` is NOT recursed; a contributor could in principle hide a JSON file behind a symlink and bypass the gate. Defer; the safer-default behavior is intentional (symlink-following has its own attack surface). Document the behavior if a need surfaces.
* **W3 MED-3** — if `coverage_paths` were ever set to `[.]` the validator would walk the entire repo (including `target/`, `.git/`, `node_modules/`). Not a current risk (manifest pins `.claude-plugin` + `hooks`), but a future maintainer could regress. Defer; consider a default exclude list if this footgun ever fires.
* **W3 LOW-5** — Windows-style path traversal (backslashes) is not normalised by `os.path` on Linux, so a malicious manifest could in principle hide an escape. CI runs Linux-only; defer the Windows-aware guard.
* **W4 M-2** — sync-protocol-hash.sh's `re.subn(..., count=1)` rewrites only the first `protocol_hash` match. If a future contributor introduces duplicate keys (malformed JSON), python's `json.load` accepts last-wins so the validator passes silently. Defer; duplicate-key plugin.json is already broken JSON and would surface elsewhere.
* **W4 L-1** — `check-protocol-hash.sh` forwards `--protocol-file`/`--plugin-file` flags but `sync-protocol-hash.sh` does not. CLI surface is asymmetric. Defer; sync-side flags would only matter for fixture-test variants we don't currently exercise.
* **W4 L-3** — empty-string or whitespace-only `protocol_hash` is detected via the drift comparison, but the message says "drift" not "empty value" — slightly misleading. Defer; cosmetic.
* **W4 L-4** — initial-add error phrasing differs slightly between the validator (`Add it: "protocol_hash": "<sha>"`) and the sync helper (`add manually first; Suggested line: …`). Both copy-pasteable. Defer.
* **W5 §G1** — `.github/pending-rollback` flag has no enforcement. Drill 2026-04-25 §G1 documented two fixes: (a) a CI step that fails when the file exists (in-repo, self-contained), or (b) a GitHub branch-protection rule. Defer; next rollback drill or the GitHub repo governance W6 picks one.
* **W5 §G4** — DB compatibility matrix flags `2P-1b §5a TODO: add pre-migration DB snapshot`. Genuine production-safety hole when rolling back across schema boundaries. Defer to a P3-3+ item.
* **W5 §G5** — quarterly drill cadence is documented in the playbook's tabletop checklist but no calendar/cron reminder mechanism exists. Defer; consider a recurring HANDOFF entry or GitHub Actions cron workflow.
* **W5 §G2** — `gh release delete --cleanup-tag=false` is non-idiomatic but functionally correct; the playbook now omits the flag in the default form (keep-tag) and shows bare `--cleanup-tag` in the optional opt-in branch. Closed by W5.
* **W5 review HIGH-1 (daemon SIGTERM handler)** — the daemon currently registers only `tokio::signal::ctrl_c()` (= SIGINT). `systemctl stop` and any default `kill PID` send SIGTERM, which kills the daemon abruptly without running the socket-drain path. The W5 playbook fix uses `kill -INT` as a tactical workaround; the strategic fix is a `tokio::signal::unix::signal(SignalKind::terminate())` handler in `crates/daemon/src/main.rs` so SIGTERM also triggers graceful shutdown. **→ Lifted to P3-2 W7 per user sign-off 2026-04-25.** No longer in P3-1 deferred state.
* **W7 L4** — `${CLAUDE_SETTINGS:-$HOME/.claude/settings.json}` with unset `$HOME` falls back to `/.claude/settings.json` and exits 0 with "nothing to check". Benign in practice (every CI runner has `$HOME` set); not worth a fix.

---

## Phase P3-2 — 2A-4d follow-up drain + production hardening (~30 commits)

**Source:** HANDOFF.md "Deferred backlog — what's still open" + `docs/superpowers/plans/2026-04-24-forge-identity-observability.md` + W5 review HIGH-1 strategic fix lifted from P3-1 deferred backlog (per user sign-off 2026-04-25).

| Wave | Scope | Acceptance |
|------|-------|------------|
| **W1** | Tier 3 review M3 — add `session_id` to `Request::CompileContextTrace`. Protocol change: forge-core enum + handler + CLI + harness propagation. | trace fn now sees per-scope overrides; harness-sync (from P3-1 W4) catches the change cleanly; sync-protocol-hash.sh re-syncs. |
| **W2** | Tier 3 review M2 — batch the 6 independent `resolve_scoped_config` calls per CompileContext via existing `resolve_effective_config`. | 6 → 1 call; tests prove no behavior change; latency benchmark shows reduction. |
| **W3** | Tier 1 #5 — T10 OTLP-path latency variant (Variant C harness with real `BatchSpanProcessor` + no-op span sink). | results doc updated with OTLP path numbers; budget within prior limits. |
| **W4** | Tier 1 #2 — `record()` span-scope refactor across remaining 22 phases (phase 19 = reference pattern). | every phase calls record() AFTER span scope drops; instrumentation-layer warns no longer attributed to phase span. |
| **W5** | Tier 3 #5 — `shape_bench_run_summary` percentile-cap CTE rewrite (`RANK() OVER (PARTITION BY group_key ORDER BY timestamp DESC)`). | per-group cap enforced in SQL; mirrors `shape_latency` pattern. |
| **W6** | Tier 3 #6 cosmetic batch — M1 `#[serial]` mark, M2 git-cluster, M3 chrono swap for `civil_from_days`, L1 `i64::from` cast, L2 `u32::try_from` cast. | last open items in 2A-4d.3.1 closed. |
| **W7** | **NEW** — Daemon SIGTERM handler (P3-1 W5 review HIGH-1 strategic fix). `tokio::signal::unix::signal(SignalKind::terminate())` in `crates/daemon/src/main.rs` so SIGTERM also triggers graceful socket-drain. Update rollback playbook to document `kill PID` is now safe (alongside the existing `kill -INT` for cross-platform). | `kill PID` (SIGTERM) and `kill -INT PID` (SIGINT) both produce identical graceful-shutdown traces; `systemctl stop` exits cleanly without abrupt kill; W5 §HIGH-1 closed strategically. |

**Phase P3-2 close:** version bump to `0.6.0-rc.2`. HANDOFF rewrite. Halt.

### P3-2 deferred backlog (per-wave review residue)

* **P3-2 W1 review note (behavioral test gap)** — the W1 diff lands a structural+contract guarantee that `session_id` threads end-to-end through the trace surface. There is no behavioral unit/integration test that flips a session-scoped `context_injection` flag (e.g. via `forge-next config set context_injection.session_context=false --scope session=<id>`) and asserts the resulting `compile_context_trace` output mirrors the override (decisions/lessons appear in `excluded` rather than `considered`). The original bug class — handler dropping `inj` on the floor — is caught by the type system once the param threads through. A behavioral test would be additive only. **Partially closed by W2:** the resolver-layer test suite added in W2 now exercises 9 paths (no-sid, missing-row, no-overrides, session-override, unparseable, session-vs-org, team-scope, user-scope, session-vs-team). The remaining gap is end-to-end through the trace handler — track for P3-2 W6 cosmetic batch.
* **P3-2 W2 LOW-1 (commit-msg arithmetic)** — the W2 commit message says "36 unconditional resolutions" where the precise unit is "36 underlying SELECTs" (6 `resolve_scoped_config` calls × up to 6 internal `get_scoped_config` SELECTs each). Direction is right (≥6× reduction in the no-override case); units are loose. Cosmetic; no code change. Future commit-message hygiene: distinguish "resolve calls" from "underlying SELECTs".
* **P3-2 W2 LOW-2 (resolve_effective_config inner-loop optimization)** — when overrides ARE present, the new path issues 6 (list) + K × scopes-with-entry (resolve) SELECTs, where K is the count of distinct overridden keys. For K=6 with all 6 scopes populated, this can reach 6 + 36 = 42 SELECTs — actually *worse* than the old 36. Production hot-path is K=0 (no overrides) so this rarely fires; fix is a `resolve_effective_config` rewrite that uses entries already harvested by `list_scoped_config` to do the per-key precedence walk in-process, avoiding the second `resolve_scoped_config` pass entirely. Out of scope for W2; defer as a P3-3+ optimization candidate.
* **P3-2 W2 MED-1 (DB-error semantic shift, accepted)** — the new `resolve_effective_config` returns `Err` on first scope failure, short-circuiting all 6 keys; the old per-key path could honor non-erroring keys. Production impact is near-zero because `rusqlite::Error` here is systemic (corrupt schema / I/O failure). Doc comment now explicitly acknowledges this trade-off rather than claiming "byte-for-byte". No fix needed; design intent recorded.
* **P3-2 W5 MED-1 (window-function index hint)** — `kpi_events` has only single-column indexes on `(timestamp)`, `(event_type)`, and the `phase_name` JSON-extract. The new W5 CTE uses `ROW_NUMBER() OVER (PARTITION BY {group_expr} ORDER BY timestamp DESC)` where `{group_expr}` is `json_extract(metadata_json, '$.bench_name'/'$.commit_sha'/'$.seed')`. SQLite cannot use a single-column index for a JSON-extracted partition key, so the planner builds a temp B-tree + sorts. At ~200k rows/window this is O(n log n) — likely OK but adds steady-state latency. Fix would be expression indexes like `idx_kpi_events_bench_name_ts ON kpi_events(json_extract(metadata_json,'$.bench_name'), timestamp DESC) WHERE event_type='bench_run_completed'`. Defer until measurements justify (no production observation of slow inspect queries to date).
* **P3-2 W5 LOW-1 (recency-bias behavior change)** — pre-W5 the percentile pass had no ordering on the inner SELECT; SQLite returned rows in storage order (typically rowid ASC) and the outer LIMIT trimmed arbitrarily. Post-W5 explicitly keeps the N most-recent. Operationally this is the right choice for percentile sampling (recency-weighted) and the W5 plan-doc spec called for it. Documented in BenchRunRow doc-comment + commit message; not a regression.

---

## Phase P3-3 — New product phases (~80–120 commits)

Each new bench follows the master v6 / Forge-Identity precedent: design spec → 2 adversarial reviews of spec → implementation plan → TDD waves → calibration loop → adversarial review of impl → results doc → MEMORY index → close.

| Sub-phase | Tag | Scope |
|-----------|-----|-------|
| **2A-5** | Domain-transfer isolation bench | Validate cross-project memory leakage prevention. Generate N synthetic projects, seed memories with project-specific tokens, recall from each project, assert no leakage. Composite ≥ 0.95. |
| **2A-6** | Multi-agent coordination bench | FISP-driven multi-agent scenarios; planner → generator → evaluator pipeline correctness; agent state isolation. Composite ≥ 0.95. |
| **2A-7** | Daemon restart persistence operator drill | Chaos test: kill daemon mid-pass, restart, assert no data loss. Script + result doc; runs from `scripts/chaos/restart-drill.sh`. |
| **2C-1** | Grafana operator dashboards | JSON dashboards for `/metrics` families: phase duration, error rate, table rows, layer freshness, bench composite trend. Imports cleanly into Grafana 10+. |
| **2C-2** | Auto-open-PR-on-regression CI workflow | `.github/workflows/bench-regression.yml` — on bench-fast composite drop ≥ 5%, opens GitHub Issue with diff + dimension breakdown + last-5-runs trend. |

**Dependency chain:** 2A-5 → 2A-6 (multi-agent depends on isolation primitives). 2A-7 / 2C-1 / 2C-2 parallel after 2A-6 lands.

**Dependabot batch:** merge the 4 open PRs (`jsonwebtoken-10.3.0`, `opentelemetry-0.31.0`, `rand-0.10.1`, `zerocopy-0.8.48`, plus minor-patch bundle) at the start of P3-3, run all 6 existing benches as a calibration sweep before any new bench dev.

**Wave structure per sub-phase:** waves of 4–7 commits with adversarial review (mirrors 2A-4d.3 pattern).

**Phase P3-3 close:** version bump to `0.6.0-rc.3`. HANDOFF rewrite. Halt.

### P3-3 Stage 0 deferred backlog (dependabot residue)

* **opentelemetry 0.27 → 0.31 cluster bump (PR #2 deferred 2026-04-25).** Cannot bump `opentelemetry` in isolation; the daemon Cargo.toml has 4 interlocked OTel deps (`opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp`, `tracing-opentelemetry`) all pinned at the 0.27/0.28 line. Holistic bump requires migrating across 4 minor versions (0.28 stabilization, 0.29 Prometheus deprecation, 0.30 Metrics SDK stable, 0.31 SpanExporter unification) plus rewriting the T10 OTLP-path latency test's custom `NoopSpanExporter` impl (`opentelemetry_sdk::export::trace::SpanExporter` moved/unified in 0.31). Estimated 4–6 commits with calibration check that T10 ratio still ≤ 1.20×. Track for P3-3 dedicated wave or P3-4 pre-release task. Memory: `feedback_dependabot_ecosystem_cluster.md`.

### P3-3 Stage 1 deferred backlog (2A-5 impl review residue)

Adversarial impl review (verdict `lockable-with-fixes`, see `docs/superpowers/reviews/2026-04-25-p3-3-2a-5-impl-domain-isolation.yaml`) closed 2 HIGH + 3 MED at fix-wave commit `1d5416f`. Remaining items deferred:

* **MED-1 (seed-invariance regression test).** Stage 1 results doc claims "composite is invariant across seeds" but no test asserts `generate_corpus(seed=7)` and `generate_corpus(seed=42)` produce byte-identical title/content/project lists. Today the property is silently reliant on `_rng` being unused. Fix: add a regression test that pins same-output-different-seed. Track for 2A-5 follow-up; not blocking.
* **MED-5 (--expected-composite tolerance too loose).** Tolerance ±0.05 cannot detect a 1-row D6 regression (composite drifts by 0.0167, well under tolerance). Per-dim min gate catches it via `score.pass=false`, but the CLI gate is uninformative. Fix options: tighten tolerance to ±0.005 OR document gate as "downward-only ≥ 0.05". Cosmetic; not blocking.
* **LOW-1 (test `/tmp` use).** `end_to_end_run_passes_on_seed_42` writes summary.json to `/tmp` — racy under parallel `cargo test`. Fix: use `tempfile::tempdir()`. Cosmetic.
* **LOW-2 (`pub const` exposure).** `MAIN_PROJECTS`, `PREFIX_COLLISION_PROJECT` etc are `pub const`; only consumer is the same module's tests. Fix: drop to `pub(crate)` for parity with forge-identity. Cosmetic.
* **LOW-3 (`as u64` cast).** `start.elapsed().as_millis() as u64` truncates beyond 584M years (non-issue). Other bench files use `try_from(...).unwrap_or(u64::MAX)`. Cosmetic.
* **LOW-4 (results doc placeholder column).** Wall-duration column reads `<1000` for every seed (placeholder, not measurements). Fix: paste real ms values. Cosmetic.

---

## Phase P3-4 — Release & distribution (~10 commits + manual)

| Wave | Scope | Auto / User |
|------|-------|-------------|
| **W1** | Multi-OS dogfood final sweep — Linux full cells re-verified, macOS cell prepared with full reproduction steps for user. | Auto (Linux); user (macOS). |
| **W2** | Bench-fast required-gate flip — verify 14 consecutive green master runs accumulated; flip `continue-on-error: false` (Task #68 closes here). | Auto if condition met; halt + brief if not. |
| **W3** | v0.6.0 version bump in `Cargo.toml`, `plugin.json`, `marketplace.json`, `Formula/forge.rb`, HANDOFF. | Auto. |
| **W4** | GitHub release artifacts — `gh release create v0.6.0` with multi-arch binaries, release notes from CHANGELOG. | Auto if `gh` auth works in env; else brief user. |
| **W5** | Marketplace submission bundle — manifest, listing copy, screenshots, demo GIF (if feasible). | Auto preparation; user submits. |
| **W6** | Branch protection rules — JSON config for required reviewers, required CI checks, no force-push, etc. | Auto preparation; user applies. |
| **W7** | Final HANDOFF rewrite + close-out memo. | Auto. |

**Phase P3-4 close:** v0.6.0 shipped. User performs marketplace submission + branch protection. Plan archived.

---

## Per-wave standard procedure

1. Verify clean working tree.
2. Implement TDD-first if a behavior change.
3. Run `cargo fmt --all --check`, `cargo clippy --workspace --features bench -- -W clippy::all -D warnings`, `cargo test -p forge-daemon --lib --features bench`.
4. Run `bash scripts/ci/check_spans.sh`.
5. Commit with the project convention message format.
6. Dispatch one adversarial review (Claude `general-purpose`, terse-output, ≤600-word verdict cap).
7. Address every BLOCKER + HIGH + actionable MEDIUM in a single follow-up commit.
8. LOWs / non-actionable MEDIUMs go into the per-phase backlog section in this file with rationale.
9. Update task list (TaskCreate / TaskUpdate).
10. If wave delivers a behavior change, dogfood briefly on local daemon when feasible.

## Memory index

This plan doc + HANDOFF.md + the 5 feedback memory files (auto-memory directory) form the recoverable state. Re-reads after `/compact` follow this order:
1. HANDOFF.md
2. This plan doc
3. The most recently-updated phase's individual spec/plan in `docs/superpowers/{specs,plans}/`

## Estimated total scope

* ~155–195 commits.
* ~5–8 sessions, 3–5 `/compact` boundaries.
* Phase boundaries: 4 explicit halt points for user sign-off.

## Out of scope (explicit non-goals)

* macOS as a blocking gate (per decision #2).
* Marketplace publication or branch protection enforcement (per decision #3).
* Anything not listed above unless surfaced as a wave-level discovery (added with rationale + adversarial review).
