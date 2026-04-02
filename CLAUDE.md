# Forge — Agentic OS for Claude Code

## CRITICAL: Forge is the Entry Point

**For ANY development task — building, fixing, planning, reviewing — invoke `forge:forge` FIRST.**

Forge orchestrates the full lifecycle and delegates to other skills (Superpowers, Codex) at the right phase. Do NOT invoke `superpowers:brainstorming`, `superpowers:writing-plans`, or `superpowers:subagent-driven-development` directly for development tasks. Forge calls them internally when needed.

| User says | You invoke | NOT |
|-----------|-----------|-----|
| "build X" | `forge:forge` | ~~`superpowers:brainstorming`~~ |
| "fix this bug" | `forge:forge` | ~~`superpowers:systematic-debugging`~~ |
| "plan the implementation" | `forge:forge` | ~~`superpowers:writing-plans`~~ |
| "review the code" | `forge:forge-review` | ~~`superpowers:requesting-code-review`~~ |
| "add a feature" | `forge:forge` | ~~`superpowers:brainstorming`~~ |

Superpowers skills are still useful — Forge invokes them at the right lifecycle phase:
- Think phase → Forge invokes brainstorming
- Plan phase → Forge invokes writing-plans
- Build phase → Forge invokes subagent-driven-development + TDD
- Debug → Forge invokes systematic-debugging

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

### CLI-First Commands (v0.3.0 — no MCP)

`forge` is a Rust binary with subcommands. **This is the only interface** — no MCP server.

```bash
# Memory (fast path — Rust cache, <5ms)
forge remember --type decision --title "..." --content "..."  # Store memory
forge recall "keyword"                    # Search memory cache
forge recall --list --type decision       # List all decisions
forge recall --graph "keyword"            # Search graph DB (slower, ~200ms)

# Memory (graph operations — Rust + Python, <200ms)
forge forget <node_id> --label Decision   # Soft-delete
forge sync                                # Sync pending → graph DB
forge health                              # Graph node/edge counts
forge query "MATCH (f:File) RETURN f.name LIMIT 10"  # Cypher query

# Code intelligence
forge index .                             # Parse Python/TS/JS → NDJSON
forge scan .                              # Detect exposed secrets
forge scan . --watch --interval 30        # Always-on security monitor

# Hooks (<5ms, called by Claude Code automatically)
forge hook session-start                  # Context injection
forge hook post-edit <file>               # Secret scan per file
forge hook session-end                    # Update HUD state
forge agent                               # Agent lifecycle tracking

# Research & Review
forge research "topic" --max-iterations 5 # AutoResearch loop
forge review . --base HEAD~3              # Diff context for council review

# System health
forge doctor --format text                # 13 health checks
```

### Storing Memory

**ALWAYS store important decisions.** When you make an architectural choice:
```bash
forge remember --type decision --title "..." --content "..." --sync
```
Use `--sync` to write immediately to graph DB. Without it, writes to cache only (fast, synced later).

---

## Architecture (v0.3.0)

**CLI-first. No MCP server.**

```
forge (Rust, 4.3MB)          — CLI: all operations, <5ms for cache, <200ms for graph
forge-graph (Python, 115 tests) — Graph library: LadybugDB 0.15.3, called by forge
forge-hud (Rust, 476KB)      — StatusLine: <2ms render, real-time stats
forge-channel (TS/Bun)       — Telegram + iMessage bridges
```

**Key architecture: No persistent Python process.**
- `forge` handles everything via CLI subcommands
- For graph operations, Rust shells out to `python3 -m forge_graph.cli`
- Python opens DB, operates, closes, exits — no lock contention
- Dual storage: `cache.json` (instant reads) + LadybugDB (durable graph)
- HUD reads `hud-state.json` written by forge (updated on remember/forget/agent events)

## Development

### Running Tests

```bash
# Python tests (ALWAYS use PYTHONPATH=src)
cd forge-graph && PYTHONPATH=src python3 -m pytest tests/ -v --tb=short

# Rust build + tests
cargo build --release
cargo test -p forge-agentic-os

# Test CLI
./target/release/forge index forge-graph/src/
./target/release/forge scan .
./target/release/forge recall --list
```

### Critical Rules

- **Python 3.10** — always `python3`, never `python`
- **No MCP** — removed in v0.3.0. All ops via `forge` CLI
- **LadybugDB** — use `current_timestamp()` not `timestamp()`. Secret table uses `status` column, not `invalid_at`.
- **Codex** — use `codex exec --model gpt-5.2` (default o4-mini broken on ChatGPT auth)
- **Plugin cache** — `~/.claude/plugins/cache/forge-marketplace/forge/0.3.0/`. After changes, sync with: `rsync -a forge-graph/src/ "$CACHE/forge-graph/src/" && cp target/release/forge "$CACHE/servers/forge"`
- **Circular imports** — `app.py` removed. `memory/tools.py` uses local stubs. `cli.py` is the Python entry point.

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
- 3 adversarial reviews completed (Forge evaluator + Codex gpt-5.2 x2)
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
