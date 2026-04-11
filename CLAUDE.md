# Forge — Cognitive Infrastructure for AI Agents

## Project

Forge gives AI agents persistent memory, proactive context, and self-healing intelligence. One Rust daemon, one SQLite file, zero cloud dependency.

**Stack:** Rust daemon (`crates/`) + SolidJS app (`app/forge/`) + Tauri desktop shell
**Port:** Daemon HTTP API on `8430` — `POST /api` with `{method, params}` JSON
**Tests:** `cargo test -p forge-daemon` (990+) · `cd app/forge && npx playwright test --config e2e/playwright/playwright.config.ts` (28 files)
**Lint:** `cargo clippy -p forge-daemon -p forge-core -p forge-cli -- -W clippy::all` (0 warnings required)

## Forge Powers This Project

Forge runs as an always-on daemon alongside Claude. It handles context automatically — you don't explain the project from scratch each session.

### What happens automatically
- **Memory injection**: Hooks inject relevant decisions, lessons, and patterns at session start
- **Blast radius**: Before editing files, Forge shows what depends on them
- **Extraction**: Transcripts are auto-extracted into structured memories after each session
- **Healing**: Stale, contradictory, or low-quality memories are auto-repaired

### What you do
- **Recall before planning**: `forge-next recall "<topic>" --limit 10`
- **Remember after deciding**: `forge-next remember --type decision --title "..." --content "..."`
- **Use forge:forge for all dev work**: It routes to the right workflow and tracks context
- **Check health**: `forge-next health` · `forge-next doctor` · `forge-next manas-health`

### Forge skills (invoke via `forge:forge` — it auto-routes)

| Skill | When |
|-------|------|
| `forge:forge` | **Start here** — auto-detects task type |
| `forge:forge-feature` | Modify existing code (explore → plan → build → review) |
| `forge:forge-think` | Requirements discovery, BDD specs |
| `forge:forge-tdd` | Test-first development |
| `forge:forge-debug` | Root cause analysis before fix |
| `forge:forge-review` | Code review (standard or adversarial) |
| `forge:forge-verify` | Evidence before assertions |
| `forge:forge-ship` | Final verification + PR |

### Agent team (for multi-file tasks)

| Agent | Role |
|-------|------|
| **forge-planner** | Architecture, exploration, planning — never writes code |
| **forge-generator** | Implementation in isolated worktrees — atomic commits |
| **forge-evaluator** | Spec compliance + code quality — adversarial skeptic |

### Do NOT use these directly — Forge supersedes them

`superpowers:brainstorming`, `superpowers:writing-plans`, `superpowers:subagent-driven-development`, `superpowers:test-driven-development`, `superpowers:systematic-debugging`, `superpowers:requesting-code-review`, `superpowers:verification-before-completion`, `superpowers:finishing-a-development-branch`, `superpowers:dispatching-parallel-agents`, `episodic-memory:*`, `feature-dev:feature-dev`

## Architecture

```
crates/daemon/     — Rust daemon (actor model, SQLite FTS5 + sqlite-vec, 8-layer Manas memory)
crates/cli/        — forge-next CLI client
crates/core/       — Protocol types (Request/Response enums)
crates/hud/        — StatusLine rendering
app/forge/src/     — SolidJS app (48 components, PixiJS canvas, 4-tier feature gating)
app/forge/e2e/     — Playwright E2E tests (28 files, live daemon, no mocks)
skills/            — Forge skill definitions (15 skills)
agents/            — Agent definitions (planner, generator, evaluator)
product/           — Product org (engineering, business, marketing, operations)
```

## Conventions

- **Daemon protocol**: Some endpoints are unit variants (no params: `health`, `healing_status`, `healing_run`, `doctor`, `license_status`, `sync_conflicts`, `list_team_templates`). Others require `params: {}` even if empty.
- **App**: SolidJS (not React) — `createSignal`, `Show`, `For`, `createResource`. BEM CSS naming.
- **Feature gating**: 4 tiers (free/pro/team/enterprise) via `canAccess(feature, tier)` in `lib/featureGate.ts`.
- **Plugin sync**: After changing skills/agents, run: `rsync -av --delete skills/ ~/.claude/plugins/marketplaces/forge-marketplace/skills/ && rsync -av --delete agents/ ~/.claude/plugins/marketplaces/forge-marketplace/agents/` — then restart session.

## Remaining Work

Track in `product/engineering/daemon-team/SESSION-GAPS.md`. Current state in `product/cross-team/HANDOFF.md`.

**Code (P2):** WASM Task Runner, Raft leader election
**Founder:** Dodo Payments KYC, ToS, Privacy Policy, Apple Developer ID, Firebase docs deploy, license key replacement (`app/forge/src/lib/licensePublicKey.ts`)
