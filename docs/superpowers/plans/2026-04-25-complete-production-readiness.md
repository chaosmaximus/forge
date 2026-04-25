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

---

## Phase P3-2 — 2A-4d follow-up drain (~25 commits)

**Source:** HANDOFF.md "Deferred backlog — what's still open" + `docs/superpowers/plans/2026-04-24-forge-identity-observability.md`.

| Wave | Scope | Acceptance |
|------|-------|------------|
| **W1** | Tier 3 review M3 — add `session_id` to `Request::CompileContextTrace`. Protocol change: forge-core enum + handler + CLI + harness propagation. | trace fn now sees per-scope overrides; harness-sync (from P3-1 W4) catches the change cleanly. |
| **W2** | Tier 3 review M2 — batch the 6 independent `resolve_scoped_config` calls per CompileContext via existing `resolve_effective_config`. | 6 → 1 call; tests prove no behavior change; latency benchmark shows reduction. |
| **W3** | Tier 1 #5 — T10 OTLP-path latency variant (Variant C harness with real `BatchSpanProcessor` + no-op span sink). | results doc updated with OTLP path numbers; budget within prior limits. |
| **W4** | Tier 1 #2 — `record()` span-scope refactor across remaining 22 phases (phase 19 = reference pattern). | every phase calls record() AFTER span scope drops; instrumentation-layer warns no longer attributed to phase span. |
| **W5** | Tier 3 #5 — `shape_bench_run_summary` percentile-cap CTE rewrite (`RANK() OVER (PARTITION BY group_key ORDER BY timestamp DESC)`). | per-group cap enforced in SQL; mirrors `shape_latency` pattern. |
| **W6** | Tier 3 #6 cosmetic batch — M1 `#[serial]` mark, M2 git-cluster, M3 chrono swap for `civil_from_days`, L1 `i64::from` cast, L2 `u32::try_from` cast. | last open items in 2A-4d.3.1 closed. |

**Phase P3-2 close:** version bump to `0.6.0-rc.2`. HANDOFF rewrite. Halt.

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
