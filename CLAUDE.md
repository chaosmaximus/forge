# Forge — Agentic OS for Claude Code

## How to Use Forge

**Forge is the master orchestrator.** When working in this repo or any project with Forge installed, USE Forge's skills and agents — don't do raw work.

### Skills (Applications)

| Skill | When to Use |
|-------|------------|
| `forge:forge` | **Start here.** Auto-detects whether to use forge:new or forge:feature |
| `forge:forge-new` | Building a new project from scratch (PRD → design → agent team build) |
| `forge:forge-feature` | Modifying existing code (explore → plan → agent team build) |
| `forge:forge-review` | Code review — standard or council mode (multi-reviewer with Codex) |
| `forge:forge-ship` | Final verification, PR creation |
| `forge:forge-research` | Autonomous research loop — bounded exploration with git checkpoints |
| `forge:forge-security` | Security scanning — `forge-core scan .` or always-on `--watch` mode |
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
| **forge-planner** | Architecture, exploration, planning | forge_recall, forge_decisions, forge_patterns, forge_cypher |
| **forge-generator** | Implementation in isolated worktrees | forge_recall, forge_decisions, forge_index |
| **forge-evaluator** | Spec compliance + code quality review | forge_recall, forge_cypher, forge_decisions |

**USE THESE AGENTS** for implementation work. Don't use raw subagents when Forge agents are available.

### CLI-First Commands

`forge-core` is a Rust binary with subcommands. Use these directly when appropriate:

```bash
forge-core index .                    # Parse Python/TS/JS → code graph (NDJSON)
forge-core scan .                     # Detect exposed secrets (regex + entropy)
forge-core scan . --watch --interval 30  # Always-on security monitor
forge-core research "topic" --max-iterations 5  # AutoResearch loop
forge-core review . --base HEAD~3     # Generate diff context for council review
forge-core hook session-start         # Hook handler (<5ms)
forge-core hook post-edit <file>      # Secret scan single file (<5ms)
forge-core hook session-end           # Update HUD state (<5ms)
```

### MCP Tools (12)

These are available via the forge-graph MCP server:

| Tool | Purpose | Path |
|------|---------|------|
| `forge_remember` | Store decisions, patterns, lessons, preferences | Deterministic (structured) or Agent (NL) |
| `forge_recall` | Search memory by keyword | Deterministic (FTS) |
| `forge_link` | Create edges between any nodes | Deterministic |
| `forge_decisions` | Query active decisions, filter by code path | Deterministic |
| `forge_patterns` | Query learned patterns | Deterministic |
| `forge_timeline` | Follow SUPERSEDES/EVOLVED_FROM chains | Deterministic |
| `forge_forget` | Soft-delete a memory node | Deterministic |
| `forge_usage` | Token usage statistics | Deterministic |
| `forge_scan` | Scan for secrets (delegates to forge-core) | Deterministic |
| `forge_index` | Index codebase (delegates to forge-core) | Deterministic |
| `forge_cypher` | Sandboxed read-only Cypher queries on code nodes | Deterministic |
| `forge_health` | Graph health check | Deterministic |

### Storing Memory

**ALWAYS store important decisions in the graph.** When you make an architectural choice, run:
```
forge_remember type=decision structured={"title": "...", "rationale": "...", "status": "active", "confidence": 0.9}
```

This makes the HUD show real memory counts and enables context injection in future sessions.

---

## Architecture

**Hybrid Rust + Python + TypeScript:**

```
forge-core (Rust, 4.3MB)     — CLI: index, scan, hook, research, review
forge-graph (Python, 117 tests) — MCP server: 12 tools backed by LadybugDB 0.15.3
forge-hud (Rust, 476KB)      — StatusLine: <2ms render, real-time stats
forge-channel (TS/Bun)       — Telegram + iMessage bridges via MCP channels API
```

**Key architecture: `app.py` breaks circular imports.**
- `forge_graph/app.py` owns the `mcp` FastMCP instance and `get_db()`/`set_db()` functions
- `server.py`, `memory/tools.py`, `security/tools.py` all import from `app.py`
- Tool modules register at module import time (not in `main()`) so MCP `tools/list` returns all 12 tools

**Data flow:**
- `forge-core` handles hot paths (called every edit, every session start/end)
- `forge-graph` MCP server handles graph operations (long-running, C++ LadybugDB does the work)
- MCP tools delegate non-graph work to `forge-core` CLI via subprocess
- HUD reads `hud-state.json` written by the MCP server (updates on every `forge_remember`/`forge_forget`)

## Development

### Running Tests

```bash
# Python tests (ALWAYS use PYTHONPATH=src)
cd forge-graph && PYTHONPATH=src python3 -m pytest tests/ -v --tb=short

# Rust build (full workspace)
cargo build --release

# Test CLI
./target/release/forge-core index forge-graph/src/
./target/release/forge-core scan .
./target/release/forge-core review . --base HEAD~3
```

### Critical Rules

- **Python 3.10** — always `python3`, never `python`
- **MCP tool type hints** — use `Optional[str]`, `Dict[str, Any]` from typing (not `str | None`, `dict[str, Any]`) in `@mcp.tool()` signatures
- **LadybugDB** — use `current_timestamp()` not `timestamp()`. Secret table uses `status` column, not `invalid_at`.
- **Codex** — use `codex exec --model gpt-5.2` (default o4-mini broken on ChatGPT auth)
- **Plugin cache** — stale copy at `~/.claude/plugins/cache/forge-marketplace/forge/0.2.0/`. After changes, sync with: `rsync -a forge-graph/src/ "$CACHE/forge-graph/src/"`
- **Circular imports** — `mcp` instance lives in `app.py`, NOT `server.py`. All tool modules import from `app.py`.

### CI Pipeline (6 jobs, all green)

- static-validation (shellcheck, plugin/hooks/skills/agents validation)
- unit-tests (BATS)
- integration-tests (hook behavior)
- python-tests (117 tests + adversarial suite)
- rust-build (forge-core + forge-hud)
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
- `forge-core doctor` — system health checks wired to HUD
- Shannon integration — `forge:forge-pentest` security pentesting skill
- CLI-Anything patterns — agent-native CLI wrapper generation
- XML context injection — structured context for agent spawn (decisions, architecture, task)
- Agent team overhaul — wave-to-wave handoff, context passing, AgentRun population
