# Forge Public Re-Split + Plugin Re-Merge — Phase 2P-1 Design

**Phase:** 2P-1 (Packaging stream — sibling to 2A-4, not inside it)
**Date:** 2026-04-23
**Parent motivation:** conversation with user 2026-04-23 (see `HANDOFF.md` §Lifted constraints); the 2026-04-12 split (commits `ef14b60` / `2084c41` / `1a30550`) moved the plugin, hooks, skills, and agent teams to the private `forge-app` repo alongside the Tauri GUI and commercial bits. That is misaligned with the intended "open core daemon + commercial GUI" model: the plugin/hooks/skills are the agent-side SDK that makes the daemon usable, not part of the GUI.
**Prerequisite:** 2A-4c2 T1-T8 shipped (through commit `7d1ef2c`), `chore(dev-env)` wired (`729f2d4`), ban-lift + harness-philosophy docs landed (`5d25256`).
**Target milestone:** public `chaosmaximus/forge` repo exposes the full agent-facing Forge surface — daemon + CLI + HUD + plugin manifest + marketplace entry + hooks + skills + agent teams + Homebrew formula + install script — so a fresh `git clone` + two commands yields a working, agent-wired Forge.

## Summary

Undo the half of the 2026-04-12 split that should not have happened. Migrate the plugin (`plugin.json`, `marketplace.json`), hooks (`hooks/`, hook scripts), skills (15+ `forge-*` skills), agent team definitions (`forge-planner`, `forge-generator`, `forge-evaluator`), templates, and Homebrew formula from the private `forge-app` repo into public `chaosmaximus/forge`. Keep only the Tauri desktop app (`app/`), commercial licensing/Stripe code, and internal product/engineering docs private.

Each layer gets a **skill-creator-grade audit** (state-of-the-art skill and hook authoring) and an **adversarial subagent review pass** (Claude `code-reviewer` + Codex CLI) before merging to public master. A **harness-sync contract** (new, per CLAUDE.md): every daemon endpoint referenced by a plugin / hook / skill / agent MD must have either a contract test in `crates/daemon/tests/` or a doc cross-reference in `docs/api-reference.md`, enforced by a new CI check.

**Scope cap:** this phase is about *the re-split and audit*, not about new product features. Any changes to daemon behavior surface in 2A-4 phases. Exception: restore the four things the original split deleted that were *not* proprietary (`scripts/install.sh`, `getting-started.md` Claude Code hook section, `doctor`'s Hook health check, `test_hook_e2e.rs`, CI plugin validation).

**Explicit non-goals:** no daemon feature work; no changes to 2A-4c2 / 2A-4d scope; no re-architecture of skills (audit may rewrite them for quality, but the agent-facing semantics stay stable); no opening the Tauri app source; no changes to pricing/licensing surface (the gate stays in the private app).

## 1. Architecture — after re-split

```
public chaosmaximus/forge (Apache-2.0)
├── crates/            — daemon, cli, core, hud       (unchanged)
├── .claude-plugin/    — plugin.json, marketplace.json (NEW public)
├── agents/            — forge-planner, -generator, -evaluator (NEW public)
├── hooks/             — hooks.json (NEW public)
├── scripts/           — install.sh, setup-dev-env.sh, with-ort.sh,
│                        post-edit-format.sh, session-{start,end}.sh,
│                        protect-sensitive-files.sh, task-completed-gate.sh
│                                                     (install.sh + hook scripts NEW public)
├── skills/            — forge-{new,feature,tdd,ship,review,security,
│                        debug,migrate,research,setup,think,verify,
│                        handoff,agents,build-workflow} (NEW public, 15+)
├── templates/         — greenfield/existing project scaffolds (NEW public)
├── Formula/           — Homebrew formula (NEW public)
├── mcp-servers/       — forge-graph MCP (moving public if still present
│                        in forge-app — TBD during T1 survey)
├── docs/              — daemon docs + restored Claude Code hook section,
│                        plus new docs/plugin/ covering skill/hook authoring
├── tests/             — test_hook_e2e.rs restored
└── .github/workflows/ — CI: cargo + plugin validation + marketplace lint

private chaosmaximus/forge-app (proprietary)
├── app/               — Tauri desktop (SolidJS, Cortex 3D, xterm)
├── licensing/         — Stripe, tier enforcement, license-check server
├── product/           — engineering handoffs, SESSION-GAPS, strategy docs
└── archive/           — historical / superseded material
```

The **harness** is the shaded rectangle in the public repo — daemon, plugin, hooks, skills, agents. All five layers live in one repo, version together, and ship together. The Tauri app becomes what it should have always been: a GUI client for the harness, not a wrapper that carries half of it.

## 2. Migration mechanics

### 2.1 History-preserving subtree split

For each layer being migrated, use `git subtree split` on the private `forge-app` repo to produce a commit stream isolated to that directory, then `git subtree add` (or `git fetch` + `git cherry-pick`) into public `forge`. This preserves per-file history (who changed the skill, why, when) — critical for the adversarial review pass that inspects rationale.

```bash
# inside forge-app
git subtree split --prefix=skills --branch=split-skills
git subtree split --prefix=agents --branch=split-agents
# …etc per layer

# inside forge (public)
git fetch ../forge-app split-skills
git merge --allow-unrelated-histories split-skills
```

Per-layer (one PR per layer, *not* one mega-PR) to keep diffs reviewable.

### 2.2 Proprietary scrub

Before each layer's merge, sweep for:

- **Brand**: "Bhairavi Tech", "forge.bhairavi.tech", `support@bhairavi.tech`
- **License tags**: `"license": "Proprietary"` in JSON → `"Apache-2.0"`
- **Author**: `Bhairavi Tech` → `Forge Contributors`
- **Paid-tier references**: any pricing / Stripe / tier-upgrade URLs (same class of leak caught by `1a30550`)
- **Internal paths**: references to `forge-app-private/product/...` rewritten or dropped

Enforced by a grep gate in the migration script (fail-closed if any match found). Same lexicon CI uses for release artifacts.

### 2.3 License retarget

Plugin manifest (`plugin.json`) and marketplace entry move from Proprietary → Apache-2.0. The underlying assets (hook shell scripts, skill MDs, agent MDs) are Apache-2.0 by inheritance. Headers added where absent.

### 2.4 Restoration of artifacts deleted in 2026-04-12 split

Per commit `1a30550` and `2084c41`, the split deleted four non-proprietary items that should come back:

1. **`scripts/install.sh`** — rewritten to point at public release artifacts (GitHub releases) instead of `forge.bhairavi.tech`.
2. **`docs/getting-started.md` Claude Code hook integration section** — describes how to install the plugin + wire hooks to the daemon.
3. **`doctor`'s Hook health check** in `crates/cli/src/commands/system.rs` — checks that `hooks.json` is present and hook scripts are executable.
4. **`crates/daemon/tests/test_hook_e2e.rs`** — end-to-end tests that invoke hook scripts against a running daemon.
5. **CI plugin validation step** — JSON-schema check on `plugin.json`, marketplace-schema check on `marketplace.json`, bats/shellcheck on hook scripts.

Each restoration is a separate commit within the corresponding layer's task.

## 3. Per-layer audit contract (state-of-the-art authoring)

Each layer gets two required passes before merging to public master:

### 3.1 skill-creator pass (for skills + agent MDs)

Invoke the `skill-creator` skill on every `forge-*/SKILL.md` and every `agents/forge-*.md`. The skill evaluates:

- Trigger clarity (description field surfaces the skill reliably for the intended user intent, doesn't over-trigger on adjacent intents)
- Prereq/precondition explicitness
- Examples that cover happy path + one failure mode
- Workflow commands are copy-pasteable and idempotent
- No stale references to proprietary paths, removed endpoints, or private-repo files
- Matches the style of official Claude Code skills (structure, headings, length)

Findings become fixup commits within the layer's task (not separate). A skill doesn't merge until skill-creator gives it a passing grade.

### 3.2 Adversarial subagent review pass

Per each layer, dispatch in parallel:

- **Claude `code-reviewer` subagent** — on the full layer diff, asked to find: proprietary leaks, security issues (unvetted `eval` in hooks, command injection, secret exposure), harness-sync gaps, staleness (references to daemon features that no longer exist at the noted versions).
- **Codex CLI rescue subagent** (`codex:rescue`) — same diff, independent model, same prompts.

Both subagents' findings become fixup commits. A layer doesn't merge until both reviews pass with no HIGH/CRITICAL outstanding.

### 3.3 Contract tests

Hook scripts and the plugin manifest reference daemon endpoints (e.g., `session-start.sh` calls `POST /api` with `{"method":"register_session",...}`). For each such call:

- Either add a test in `crates/daemon/tests/test_hook_e2e.rs` that exercises the hook against a live daemon instance
- Or add a doc cross-reference in `docs/api-reference.md` pointing from the endpoint to the hook/skill that calls it

CI enforces this via a new check (see §4).

## 4. Harness-sync contract (CI-enforced)

**Rule** (from CLAUDE.md philosophy section): daemon changes must propagate to the layers that reference them. To make this non-optional:

- Add `scripts/check-harness-sync.sh` that scans `hooks/`, `skills/`, `agents/`, `docs/` for references to daemon protocol methods (`"method":"<name>"`) and cross-checks against `crates/core/src/protocol/request.rs` variants. Fails if:
  - A method name in a hook/skill/doc doesn't match any `Request::` variant.
  - A `Request::` variant added in the last 10 commits has no corresponding hook/skill/doc reference (warning, not fail — not every endpoint needs a hook).
  - A `Request::` variant removed/renamed still has references.
- Wire the check into CI (`.github/workflows/ci.yml`) as a required step.

This is the mechanism that turns the philosophy statement into a testable invariant.

## 5. Task decomposition

| Task | Scope | Dep | Notes |
|------|-------|-----|-------|
| **T1** | **Survey + scope lock** — catalog every file in private `forge-app` that's a candidate for public migration; confirm with user which stay/go; produce `docs/superpowers/plans/2P-1-inventory.md` as the source of truth. | — | No code changes. |
| **T2** | **Migration tooling** — write `scripts/migrate-from-forge-app.sh` using `git subtree split`; includes the proprietary-scrub grep gate; dry-run mode prints diff stats without committing. | T1 | |
| **T3** | **Plugin manifest layer** — migrate `.claude-plugin/{plugin.json, marketplace.json}`, retarget license + owner + homepage, marketplace schema validation, proprietary scrub, adversarial review. | T2 | First real layer. Smallest. |
| **T4** | **Hooks layer** — migrate `hooks/hooks.json` + all hook shell scripts; shellcheck + bats tests; contract-test each daemon endpoint touched; adversarial review. | T3 | |
| **T5** | **Skills layer** — migrate `skills/forge-*` (15+ skills); run skill-creator pass on every SKILL.md; fixup commits per skill; adversarial review. | T3 | Largest layer by file count. Splits naturally into groups if review is too big for one PR (e.g. T5a = forge-new/feature/ship, T5b = forge-tdd/review/security, T5c = forge-debug/migrate/research, T5d = forge-setup/think/verify, T5e = forge-handoff/agents/build-workflow). |
| **T6** | **Agent teams layer** — migrate `agents/forge-{planner,generator,evaluator}.md`; skill-creator pass; adversarial review. | T3 | |
| **T7** | **Templates + Homebrew** — migrate `templates/` and `Formula/`; Formula rewrites download URL to public GitHub releases. | T3 | |
| **T8** | **Restore deleted artifacts** — `scripts/install.sh`, `docs/getting-started.md` Claude Code section, `doctor` Hook check in `crates/cli/src/commands/system.rs`, `crates/daemon/tests/test_hook_e2e.rs`. | T4 | `test_hook_e2e.rs` needs the hook scripts from T4 present. |
| **T9** | **forge-graph MCP** *(conditional)* — if still present in `forge-app`, migrate the MCP server into public `mcp-servers/`. If not present, skip and note in inventory. | T1 | Gated on T1 survey finding. |
| **T10** | **Harness-sync CI check** — add `scripts/check-harness-sync.sh`, wire into `.github/workflows/ci.yml`. | T3-T6 | Depends on the layers being in-repo to have anything to scan. |
| **T11** | **CI plugin validation** — `plugin.json` + `marketplace.json` schema checks, shellcheck on hook scripts, markdownlint on skill MDs. | T4, T5 | |
| **T12** | **Lift forge-app private references in daemon tree** — sweep `docs/superpowers/specs/` and `docs/superpowers/plans/` for `forge-app-private` mentions and rewrite to `forge/<path>` (public now) or drop if stale. | T5 | |
| **T13** | **Adversarial review on the full re-split diff** — Claude `code-reviewer` + Codex CLI on the entire accumulated T3-T12 diff, one more time, to catch cross-layer issues missed in per-layer reviews. | T3-T12 | Gate before T14. |
| **T14** | **Dogfood + results doc** — install the new public plugin on this cloud box, wire hooks to a running local daemon, run one agent session through it, verify session data lands in the daemon, write `docs/benchmarks/results/forge-public-resplit-2026-04-XX.md`. | T13 | |

## 6. Adversarial-review checkpoints (recap)

- **Before T2** (migration tooling): review the scrub gate logic and the subtree-split approach.
- **Per-layer** (T3-T12): Claude code-reviewer + Codex CLI, HIGH/CRITICAL must be zero.
- **Post-T12** (T13): holistic review across all layers + final proprietary-leak sweep.

## 7. Dogfood plan (T14)

1. Install the plugin locally (`.claude/plugins/forge/` or marketplace install, whichever is canonical).
2. Start `forge-daemon` in release mode.
3. Open a fresh Claude Code session in this repo — `session-start.sh` hook should register the session with the daemon.
4. Run one real task (e.g. "what's the status of phase 2P-1?") so skills surface in the agent's context.
5. Post-session, query daemon: `forge-next recall "2P-1"` — should return memories written during the session.
6. Capture traces into `docs/benchmarks/results/forge-public-resplit-2026-04-XX.md` alongside the other phase dogfood results.

## 8. Risks & mitigations

| Risk | Mitigation |
|------|-----------|
| Proprietary material leaks into public | §2.2 scrub gate fail-closed; §3.2 adversarial review; §6 post-T12 full-diff sweep. |
| Git-subtree preserves commits that reference private files | §2.2 grep gate scans commit messages too, not just file content; rewrite-on-merge where needed. |
| Plugin skills break existing user workflows | Skills are currently inaccessible (ban + private repo), so this migration is greenfield from the public-consumer POV. No backward-compat obligation. |
| CI harness-sync check is noisy and slows development | Scoped to *referenced* methods only; unreferenced new endpoints are warnings not failures; opt-in `force=false` style. |
| Dogfood finds the plugin/daemon are still out of sync | T14 is the proof point; any desync becomes T15+ follow-up tasks. |
| Migration takes longer than 2A-4d timeline allows | 2A-4d depends on 2A-4c2 + 2A-4d intrinsics, not on 2P-1. They can interleave. 2P-1 is scoped to packaging/plumbing only. |

## 9. Out of scope

- Public Tauri app. Stays private for commercial reasons.
- Licensing / Stripe / tier enforcement. Stays private.
- Any new daemon endpoints. Feature work is 2A-4 stream.
- Re-architecture of skills. Audit may rewrite for quality; agent-facing semantics preserved.
- iOS / Android / Windows plugin support. Linux + macOS only for T14.
- Automatic skill generation from daemon metadata (a sp2-era idea).
- A "forge app" as a Claude Code agent-orchestration surface beyond what's in `agents/forge-{planner,generator,evaluator}.md`.

## 10. Acceptance criteria

Phase 2P-1 is shipped when:

1. Public `chaosmaximus/forge` at some commit `X` has all layers migrated and passing T13 adversarial review.
2. `cargo test --workspace` is green at commit `X` (inherits from 2A-4c2 contract).
3. `scripts/check-harness-sync.sh` passes in CI at commit `X`.
4. `scripts/setup-dev-env.sh && cargo test --workspace` works from a fresh clone of public `forge` on Ubuntu 22.04 / glibc 2.35 (the 2026-04-23 cloud-session reproducer).
5. Dogfood results doc at `docs/benchmarks/results/forge-public-resplit-2026-04-XX.md` records: live daemon + plugin install + one full agent session + memories recalled successfully.
6. Private `forge-app` at commit `Y ≥ X` is reduced to just `app/` + `licensing/` + `product/` + `archive/`; no plugin-ish files remain.
7. HANDOFF.md gets a new "Lifted constraints" entry noting the completion.

## 11. Open questions (to resolve during T1 survey)

- **Q1**: Is `forge-graph` MCP server still in `forge-app`? (The v0.2.0 commit thread suggests yes, but it may have been moved or deprecated.) Answer determines T9 go/no-go.
- **Q2**: Are any of the 15+ `forge-*` skills commercially differentiated (e.g. contain proprietary benchmarks, enterprise-only workflows)? If yes, those stay private; revise T5 scope.
- **Q3**: Does `scripts/install.sh` need to be different on Linux vs. macOS (brew vs. curl | sh)? Resolve during T8.
- **Q4**: Does the harness-sync check (T10) need to also scan for references to CLI subcommands (not just daemon protocol methods)? Likely yes, but out of scope for 2P-1 unless T14 dogfood surfaces drift.

---

Locked when T1 survey completes. Ready for adversarial review on design before moving to the execution plan.
