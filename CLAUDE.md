# Forge — Cognitive Infrastructure for AI Agents

## SESSION START PROTOCOL (do this FIRST, every session)

```bash
# 1. Check health
forge-next health && forge-next manas-health

# 2. Recall relevant context BEFORE any work
forge-next recall "<topic of this session>" --limit 10

# 3. Check for team messages
forge-next messages --session claude-code --limit 5

# 4. Read handoff state
cat product/HANDOFF.md
```

**If `forge-next` fails:** The daemon may not be running. Start it: `cargo run --release -p forge-daemon &`

**If `forge:*` skills fail with "Unknown skill":** The plugin cache is stale. Sync it:
```bash
rsync -av --delete skills/ ~/.claude/plugins/marketplaces/forge-marketplace/skills/
rsync -av --delete agents/ ~/.claude/plugins/marketplaces/forge-marketplace/agents/
rsync -av .claude-plugin/ ~/.claude/plugins/marketplaces/forge-marketplace/.claude-plugin/
```
Then restart the Claude Code session.

---

## CRITICAL: Forge is the ONLY Entry Point for Development

**Before invoking ANY development-related skill, invoke `forge:forge` FIRST. This is non-negotiable.**

Forge orchestrates the full lifecycle and delegates to Superpowers, Codex, and other skills at the right phase. Calling them directly bypasses mode detection, prerequisite checks, agent team coordination, memory tracking, and security scanning.

### Routing Table — ALWAYS check this before invoking a skill

| User says | You invoke | NEVER invoke directly |
|-----------|-----------|----------------------|
| "build X" / "create X" / "add X" | `forge:forge` | ~~brainstorming, subagent-driven-development~~ |
| "fix this bug" / "debug this" / "tests failing" | `forge:forge` | ~~systematic-debugging, test-driven-development~~ |
| "plan the implementation" / "how should we build" | `forge:forge` | ~~writing-plans, brainstorming~~ |
| "review the code" / "check my changes" | `forge:forge-review` | ~~requesting-code-review, verification-before-completion~~ |
| "add a feature" / "refactor" / "improve" | `forge:forge` | ~~brainstorming, feature-dev~~ |
| "ship this" / "create a PR" / "merge" | `forge:forge-ship` | ~~finishing-a-development-branch~~ |
| "brainstorm" / "think through" / "requirements" | `forge:forge` | ~~brainstorming~~ |
| "let me start from scratch" / "new project" | `forge:forge` | ~~brainstorming, writing-plans~~ |
| "set up CI/CD" / "add tests" / "add e2e tests" | `forge:forge` | ~~test-driven-development~~ |
| "performance issue" / "optimize" / "N+1 queries" | `forge:forge` | ~~systematic-debugging~~ |

### Forge Workflow Skills

| Forge Skill | When to Use |
|-------------|------------|
| `forge:forge` | **Start here.** Auto-detects whether to use forge:new or forge:feature |
| `forge:forge-tdd` | Before implementing ANY feature or fix — test first, watch fail, implement, verify |
| `forge:forge-debug` | When encountering ANY bug or failure — root cause before fix |
| `forge:forge-verify` | Before claiming work is done — evidence before assertions |
| `forge:forge-think` | Requirements discovery — BDD specs, acceptance criteria |
| `forge:forge-feature` | Full build lifecycle — explore, plan, build (with two-stage review), ship |
| `forge:forge-review` | Code review — standard or adversarial |
| `forge:forge-ship` | Final verification + PR creation |
| `forge:forge-research` | Deep research with bounded exploration |
| `forge:forge-new` | Building a new project from scratch (PRD → design → agent team build) |
| `forge:forge-security` | Security scanning — `forge scan .` or always-on `--watch` mode |
| `forge:forge-handoff` | Pause/resume work across sessions |
| `forge:forge-setup` | First-time prerequisite checks |
| `forge:forge-agents` | View detailed status of running Forge agents |

### These plugins are SUPERSEDED by Forge — do NOT use them

| Superseded Plugin | Forge Replacement |
|-------------------|-------------------|
| `superpowers:brainstorming` | `forge:forge-think` |
| `superpowers:writing-plans` | `forge:forge-feature` (plan phase) |
| `superpowers:subagent-driven-development` | `forge:forge-feature` (build phase with two-stage review) |
| `superpowers:test-driven-development` | `forge:forge-tdd` |
| `superpowers:systematic-debugging` | `forge:forge-debug` |
| `superpowers:requesting-code-review` | `forge:forge-review` |
| `superpowers:verification-before-completion` | `forge:forge-verify` |
| `superpowers:finishing-a-development-branch` | `forge:forge-ship` |
| `superpowers:dispatching-parallel-agents` | Forge agent team (forge-planner/generator/evaluator) |
| `episodic-memory:*` | Forge 8-layer Manas memory (`forge-next recall/remember`) |
| `feature-dev:feature-dev` | `forge:forge-feature` |

### When NOT to use Forge

Forge is for **development work**. These tasks should NOT go through forge:
- Explaining concepts, answering questions about code
- Reading/searching files without intent to modify
- Writing documentation, cover letters, presentations
- Configuring Claude Code settings, hooks, keybindings
- Creating or editing skills (use `skill-creator:skill-creator`)
- Data manipulation (spreadsheets, CSVs) without code changes

### Agent Team

For multi-file tasks, Forge dispatches an agent team:

| Agent | Role | Tools |
|-------|------|-------|
| **forge-planner** | Architecture, exploration, planning | Bash (forge recall/query) |
| **forge-generator** | Implementation in isolated worktrees | Full suite + Bash (forge) |
| **forge-evaluator** | Spec compliance + code quality review | Read-only + Bash (forge) |

**USE THESE AGENTS** for implementation work. Don't use raw subagents when Forge agents are available.

---

## Memory — ALWAYS Use It

**Every session must recall before planning and remember after deciding.**

```bash
# Recall before any planning
forge-next recall "<topic>" --limit 10

# Store every architectural/design decision
forge-next remember --type decision --title "..." --content "..."

# Store lessons learned from bugs/failures
forge-next remember --type lesson --title "..." --content "..."

# Store patterns discovered
forge-next remember --type pattern --title "..." --content "..."

# Check health
forge-next health
forge-next manas-health
forge-next doctor
forge-next healing-status
```

### Full CLI Reference

```bash
# Memory
forge-next remember --type decision --title "..." --content "..." [--metadata '{"key":"val"}']
forge-next recall "query" [--project P] [--type T] [--limit N] [--layer L] [--since 1h|7d]
forge-next forget <id>
forge-next supersede --old-id <old> --new-id <new>

# Session lifecycle
forge-next register-session --id <id> --agent <agent> [--project P] [--cwd D]
forge-next end-session --id <id>
forge-next sessions [--all]
forge-next set-task --session <id> --task "description"

# Context & health
forge-next compile-context --agent claude-code [--project P] [--focus <topic>]
forge-next health
forge-next health-by-project
forge-next doctor
forge-next manas-health

# Identity (Ahankara)
forge-next identity list [--agent A]
forge-next identity set --facet <facet> --description "..." [--agent A]
forge-next identity remove <id>

# Guardrails
forge-next check --file <path> [--action edit]
forge-next blast-radius --file <path>

# A2A messaging (FISP protocol)
forge-next send --to <session-id> --kind notification --topic <topic> --text "..."
forge-next send --to "*" --kind notification --topic schema_changed --text "..." --project P
forge-next messages --session <session-id> [--status pending] [--limit N]
forge-next ack <message-id-1> <message-id-2> ...
forge-next cleanup-sessions [--prefix hook-test] [--older-than 24h] [--prune]

# Teams & Orchestration
forge-next team run --name "Sprint" --templates tech-lead,frontend-dev,backend-dev
forge-next team stop --name "Sprint"
forge-next meeting vote --id <meeting-id> --session <session-id> --choice "yes"
forge-next meeting result --id <meeting-id>

# Workspace
forge-next org-init --name "MyOrg" [--template startup]
forge-next workspace-status

# License
forge-next license-status
forge-next license-set --tier pro --key <license-key>

# Healing
forge-next healing-status
forge-next healing-run
forge-next healing-log [--limit N]

# Extraction & quality
forge-next backfill-project
forge-next consolidate
forge-next extract

# Import/export & sync
forge-next export [--format json]
forge-next import [--file F]
forge-next ingest-claude
forge-next sync-export [--project P] [--since S]
forge-next sync-import
forge-next sync-pull <host> [--project P]
forge-next sync-push <host> [--project P]
forge-next sync-conflicts
forge-next sync-resolve <id>

# Code intelligence
forge scan .
forge scan . --watch --interval 30

# Other
forge-next platform
forge-next tools
forge-next perceptions [--project P] [--limit N] [--offset N]
forge-next lsp-status
forge-next config get-effective

# Hooks (<5ms, called by Claude Code automatically)
forge hook session-start
forge hook post-edit <file>
forge hook session-end
forge agent
```

---

## Dogfooding Protocol

**Forge builds itself.** Every session follows this loop:
1. **Pull & check** — `git pull`, `forge-next health`, `forge-next doctor`
2. **Recall** — `forge-next recall "<topic>"` before ANY planning
3. **Build** — use Forge agents (planner/generator/evaluator), store decisions with `forge-next remember`
4. **Test** — TDD, adversarial review, UAT with live daemon, run E2E tests
5. **Track** — store issues in Forge memory, write artifacts to workspace
6. **Evaluate** — `forge-next doctor`, `forge-next healing-status`, review perceptions
7. **Push & handoff** — update HANDOFF.md, push, start fresh session if needed

Track all gaps in `product/engineering/daemon-team/SESSION-GAPS.md`.

---

## Architecture (v0.7.0)

**Daemon-first. CLI-first. 8-layer memory. Actor model. 990+ tests. 0 clippy.**

```
forge-daemon (Rust, single binary)
  ├── Actor architecture (hot/cold path separation)
  │   ├── Socket handler: per-connection read-only SQLite (NEVER blocks)
  │   ├── HTTP server: Axum, POST /api (port 8430)
  │   ├── Writer actor: serializes writes via mpsc channel
  │   └── Workers: extraction, embedding, consolidation, healing, perception, reaper
  ├── SQLite FTS5 + sqlite-vec (single file, WAL mode)
  ├── 8-layer Manas memory
  ├── Enterprise: JWT/OIDC, RBAC, audit log, multi-tenant
  ├── A2A/FISP protocol (messaging, broadcast, meetings)
  ├── Guardrails (check + blast_radius)
  ├── Multi-agent adapters (Claude Code + Cline + Codex CLI)
  ├── Auto-extraction (Gemini 2.5 Flash / Claude Haiku / Ollama)
  └── Event bus (SSE + tokio::broadcast)

forge-next (Rust CLI)  — client for daemon, auto-starts daemon
forge-hud (Rust)       — StatusLine rendering

App (SolidJS + Tauri + PixiJS)
  ├── 48 components, 140 daemon endpoints available
  ├── Canvas engine (PixiJS infinite canvas, agent cards, team frames)
  ├── Bridge: HTTP POST to daemon :8430, WebSocket for terminals, SSE for events
  ├── 4-tier feature gating (Free / Pro $12 / Team $19 / Enterprise)
  ├── 28 Playwright E2E tests (100% P0 passing)
  └── Tauri: 90+ IPC commands, PTY terminal, deep linking

Deploy: Docker (41.7MB distroless) · Helm · docker-compose
```

## Development

### Running Tests

```bash
# Daemon tests (990+)
cargo test -p forge-daemon

# Core protocol tests (56)
cargo test -p forge-core --lib

# Socket E2E (requires release binary)
cargo build --release -p forge-daemon
cargo test -p forge-daemon --test test_socket_e2e -- --test-threads=1

# Clippy (zero warnings required)
cargo clippy -p forge-daemon -p forge-core -p forge-cli -- -W clippy::all

# App E2E tests (requires daemon running on :8430)
cd app/forge && npx playwright test --config e2e/playwright/playwright.config.ts --project=all

# App unit tests
cd app/forge && npx vitest run
```

### Plugin Sync

After changing skills, agents, or plugin config, sync to the marketplace cache:
```bash
rsync -av --delete skills/ ~/.claude/plugins/marketplaces/forge-marketplace/skills/
rsync -av --delete agents/ ~/.claude/plugins/marketplaces/forge-marketplace/agents/
rsync -av .claude-plugin/ ~/.claude/plugins/marketplaces/forge-marketplace/.claude-plugin/
```
Then restart the Claude Code session for skills to take effect.

### Critical Rules

- **Codex** — use `codex exec --model gpt-5.2` (default o4-mini broken on ChatGPT auth)
- **Plugin cache** — `~/.claude/plugins/marketplaces/forge-marketplace/`. After skill changes, run the sync above.
- **Daemon port** — HTTP API on port 8430 (`POST /api` with `{method, params}` JSON)
- **Unit variants** — Some daemon endpoints (health, healing_status, healing_run, doctor, license_status, sync_conflicts, list_team_templates) are unit variants — do NOT send `params: {}`, omit params entirely
- **Parameterized variants** — Most other endpoints require `params: {}` even if empty (manas_health, sessions, budget_status, etc.)

## Security

- Parameterized SQL (never string interpolation)
- Property key validation regex `^[A-Za-z_][A-Za-z0-9_]{0,63}$`
- Per-agent ACL enforcement via `agent_id`
- Hook scripts derive paths from script location (not env vars)
- Secret scanner: SHA256 fingerprint only, never stores actual values
- Symlink defense, umask 0177, 50MB file limit, UTF-8 safe truncation

## Remaining Work

Track all items in `product/engineering/daemon-team/SESSION-GAPS.md`.

### Code gaps (P2, not launch-blocking)
- WASM Task Runner (wasmtime dep)
- Raft leader election for leaderless teams

### Founder tasks
- Dodo Payments KYC + products
- Terms of Service / Privacy Policy
- Apple Developer ID (binary signing)
- Firebase docs site deploy (`firebase deploy` — 21 pages built)
- Replace license public key placeholder in `app/forge/src/lib/licensePublicKey.ts`
