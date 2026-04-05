# Forge — Cognitive Infrastructure for AI Agents

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

### Skills that Forge calls internally — do NOT call directly

| Superpowers Skill | Forge Phase |
|-------------------|-------------|
| `superpowers:brainstorming` | Think phase (via forge-think) |
| `superpowers:writing-plans` | Plan phase (via forge-feature/forge-new) |
| `superpowers:subagent-driven-development` | Build phase (via forge agents) |
| `superpowers:test-driven-development` | Build phase (generators follow TDD) |
| `superpowers:systematic-debugging` | Debug phase (when tests/verification fail) |
| `superpowers:requesting-code-review` | Review phase (via forge-review) |
| `superpowers:verification-before-completion` | Ship phase (via forge-ship) |
| `superpowers:finishing-a-development-branch` | Ship phase (via forge-ship) |
| `superpowers:dispatching-parallel-agents` | Build phase (forge dispatches its own agent team) |
| `superpowers:using-git-worktrees` | Build phase (forge-generator uses worktrees) |
| `feature-dev:feature-dev` | Entire lifecycle (forge-feature supersedes this) |

### When NOT to use Forge

Forge is for **development work**. These tasks should NOT go through forge:
- Explaining concepts, answering questions about code
- Reading/searching files without intent to modify
- Writing documentation, cover letters, presentations
- Configuring Claude Code settings, hooks, keybindings
- Creating or editing skills (use `skill-creator:skill-creator`)
- Searching conversation history (use `episodic-memory`)
- Data manipulation (spreadsheets, CSVs) without code changes

## How to Use Forge

### Skills (Applications)

| Skill | When to Use |
|-------|------------|
| `forge:forge` | **Start here.** Auto-detects whether to use forge:new or forge:feature |
| `forge:forge-new` | Building a new project from scratch (PRD → design → agent team build) |
| `forge:forge-feature` | Modifying existing code (explore → plan → agent team build) |
| `forge:forge-review` | Code review — standard or council mode (multi-reviewer with Codex) |
| `forge:forge-ship` | Final verification, PR creation |
| `forge:forge-research` | Autonomous research loop — bounded exploration with git checkpoints |
| `forge:forge-security` | Security scanning — `forge scan .` or always-on `--watch` mode |
| `forge:forge-handoff` | Pause/resume work across sessions |
| `forge:forge-setup` | First-time prerequisite checks |
| `forge:forge-think` | Product discovery — BDD requirements, feature specs, acceptance criteria |
| `forge:forge-agents` | View detailed status of running Forge agents |

### Skill Development

Use `skill-creator:skill-creator` for creating and improving skills within Forge. This skill provides:
- Structured skill creation workflow (intent → interview → draft → test → iterate)
- Automated evaluation with test cases and benchmark comparison
- Description optimization for better skill triggering
- Blind A/B comparison between skill versions

**When to use:** Creating new `forge:*` skills, improving existing skill descriptions for better triggering, evaluating skill quality with quantitative benchmarks.

### Agent Team

For multi-file tasks, Forge dispatches an agent team:

| Agent | Role | Tools |
|-------|------|-------|
| **forge-planner** | Architecture, exploration, planning | Bash (forge recall/query) |
| **forge-generator** | Implementation in isolated worktrees | Full suite + Bash (forge) |
| **forge-evaluator** | Spec compliance + code quality review | Read-only + Bash (forge) |

**USE THESE AGENTS** for implementation work. Don't use raw subagents when Forge agents are available.

### CLI-First Commands (v0.6.0 — Manas)

`forge-next` is the Rust CLI client for the forge-daemon. **This is the only interface** — no MCP server.

```bash
# Memory
forge-next remember --type decision --title "..." --content "..."
forge-next recall "query" [--project P] [--type T] [--limit N] [--layer L]
forge-next forget <id>

# Session lifecycle
forge-next register-session --id <id> --agent <agent> [--project P] [--cwd D]
forge-next end-session --id <id>
forge-next sessions [--all]

# Context & health
forge-next compile-context --agent claude-code [--project P]
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

# A2A inter-session messaging (FISP protocol)
forge-next send --to <session-id> --kind notification --topic <topic> --text "..."
forge-next send --to "*" --kind notification --topic schema_changed --text "..." --project P
forge-next messages --session <session-id> [--status pending] [--limit N]
forge-next ack <message-id-1> <message-id-2> ...
forge-next cleanup-sessions [--prefix hook-test]

# Import/export
forge-next export [--format json]
forge-next import [--file F]
forge-next ingest-claude

# Memory sync
forge-next sync-export [--project P] [--since S]
forge-next sync-import                    # reads NDJSON from stdin
forge-next sync-pull <host> [--project P] # pull remote memories via SSH
forge-next sync-push <host> [--project P] # push local memories via SSH
forge-next sync-conflicts                 # list unresolved conflicts
forge-next sync-resolve <id>              # resolve a conflict

# Code intelligence
forge scan .                              # Detect exposed secrets
forge scan . --watch --interval 30        # Always-on security monitor

# Other
forge-next platform
forge-next tools
forge-next perceptions [--project P] [--limit N]
forge-next lsp-status

# Hooks (<5ms, called by Claude Code automatically)
forge hook session-start                  # Context injection
forge hook post-edit <file>               # Secret scan per file
forge hook session-end                    # Update HUD state
forge agent                               # Agent lifecycle tracking
```

### Storing Memory

**ALWAYS store important decisions.** When you make an architectural choice:
```bash
forge-next remember --type decision --title "..." --content "..."
```

---

## Architecture (v0.7.0 — FISP)

**Daemon-first. CLI-first. No MCP server. 8-layer memory. Actor model. Tunable. 488+ tests.**

```
forge-daemon (Rust, single binary) — always-on daemon, Unix socket API
  ├── Actor architecture (hot/cold path separation, like Docker)
  │   ├── Socket handler: per-connection read-only SQLite (NEVER blocks)
  │   ├── Writer actor: serializes writes via mpsc channel
  │   └── Workers: background tasks (extraction, embedding, consolidation, etc.)
  ├── SQLite FTS5 + sqlite-vec (memory + vectors + edges, single file, WAL mode)
  ├── 8-layer Manas memory (platform, tools, skills, domain DNA, experience, perception, declared, latent)
  ├── A2A/FISP protocol (inter-session messaging, broadcast, delegation)
  ├── Context intelligence (excluded_layers, domain DNA boost, project-scoped prefetch)
  ├── Guardrails engine (check + blast_radius)
  ├── Multi-agent adapters (Claude Code + Cline + Codex CLI)
  ├── Auto-extraction (Gemini 2.5 Flash / Claude Haiku / Ollama)
  ├── Session tracking + session cards (capabilities, current_task)
  └── Event bus (tokio::broadcast for Mac app + A2A notifications)

forge-next (Rust CLI)  — client for daemon, auto-starts daemon
forge-hud (Rust)       — StatusLine rendering
```

**Key architecture:**
- `forge-daemon` handles everything via Unix domain socket (NDJSON protocol)
- Adapters watch transcript directories for Claude Code, Cline, and Codex CLI
- sqlite-vec stores persistent embeddings (768-dim, cosine distance)
- Graph traversal via SQL recursive CTEs on edge table
- Guardrails query the knowledge graph before agent actions
- Identity system (Ahankara) for per-agent personality facets
- Proactive context compiler assembles from all 8 layers + identity + disposition
- Security: umask 0177, 50MB file limit, symlink defense, parameterized SQL, UTF-8 safe truncation

## Development

### Running Tests

```bash
# Full workspace (560+ tests)
cargo test --workspace

# Daemon only
cargo test -p forge-daemon

# Socket E2E (requires release binary built)
cargo build --release -p forge-daemon
cargo test -p forge-daemon --test test_socket_e2e -- --test-threads=1

# Clippy (zero warnings required)
cargo clippy -p forge-daemon -p forge-core -p forge-cli -- -W clippy::all
```

### Critical Rules

- **Codex** — use `codex exec --model gpt-5.2` (default o4-mini broken on ChatGPT auth)
- **Plugin cache** — `~/.claude/plugins/cache/forge-marketplace/forge/0.3.0/`. After changes, sync with: `cp target/release/forge "$CACHE/servers/forge"`

### CI Pipeline (6 jobs, all green)

- static-validation (shellcheck, plugin/hooks/skills/agents validation)
- unit-tests (BATS)
- integration-tests (hook behavior)
- python-tests (115 tests + adversarial suite)
- rust-build (forge + forge-hud)
- security-scan (hardcoded secrets, dangerous patterns)

## Security

- Parameterized Cypher queries (`$param` syntax, never string interpolation)
- Property key validation regex `^[A-Za-z_][A-Za-z0-9_]{0,63}$`
- axon_cypher sandbox blocks memory node labels + write keywords
- Per-agent ACL enforcement via `agent_id`
- Hook scripts derive paths from script location (not env vars)
- Trust-level filtering on session context injection (`trust_level = 'user'`)
- Symlink defense in scanner and workspace boundary checks
- Secret scanner NEVER stores actual values — SHA256 fingerprint only
- 6 adversarial reviews completed (Forge evaluator + Codex gpt-5.2)
- 15 adversarial tests in CI

## Remaining Work

- Use Forge agents (planner/generator/evaluator) for building features — dogfood
- Live Telegram channel test with real bot
- AutoResearch: flesh out the explore/measure/keep/discard loop with Claude driving
- Council review: wire multi-model dispatch in the skill
- HUD: update on ALL tool calls (currently only remember/forget)
- Full Rust MCP server when kuzu crate reaches v0.15+ compatibility
- `forge doctor` — system health checks wired to HUD
- Shannon integration — `forge:forge-pentest` security pentesting skill
- CLI-Anything patterns — agent-native CLI wrapper generation
- XML context injection — structured context for agent spawn (decisions, architecture, task)
- Agent team overhaul — wave-to-wave handoff, context passing, AgentRun population
