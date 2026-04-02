# Forge

<p align="center">
  <img src="docs/images/creation-of-adam.jpg" alt="The Creation of Adam — Michelangelo, Sistine Chapel, c. 1512" width="720" />
</p>

The gap between the fingers — that's where Forge lives. Human intent meets machine execution. You decide what's worth building. Agent teams execute in parallel, review across model boundaries, remember what they learned. The spark happens in the space between.

<sub>Yes, Forge uses OpenAI to grade Anthropic's homework. The future is weird and I love it.</sub>

---

**The missing operating system for AI-powered development.**

One plugin. Think → Ship. Agent teams with memory that compounds.

[![Version](https://img.shields.io/badge/version-0.3.0-blue)](https://github.com/chaosmaximus/forge/releases)
[![CI](https://github.com/chaosmaximus/forge/actions/workflows/ci.yml/badge.svg)](https://github.com/chaosmaximus/forge/actions)
[![License: MIT](https://img.shields.io/badge/license-MIT-green)](LICENSE)
[![Tests](https://img.shields.io/badge/tests-161%20passing-brightgreen)]()

## Quick Start

```bash
# Install
claude plugin install forge@forge-marketplace

# Verify
forge doctor --format text

# Start building
forge             # Auto-detects: new project or existing codebase
forge think       # Product discovery (BDD-style)
forge plan        # Explore + plan with blast radius
forge build       # Agent team execution
forge test        # Verify with Playwright + security scan
forge ship        # PR + changelog + release
```

## The Lifecycle

```
DISCOVER  →  SPECIFY  →  PLAN  →  BUILD  →  VERIFY  →  SHIP
forge think   (specs)   forge plan  forge build  forge test  forge ship
    │                       │           │            │           │
    └── BDD questions       │           │            │           └── PR + changelog
        PRD + feature specs │           │            └── Playwright CLI
                            │           │                Security scan
                            │           └── 3 agents    Property tests
                            │               in parallel
                            └── Graph queries
                                Memory recall
                                Blast radius
```

Every phase stores decisions in a knowledge graph. Session 50 knows what session 1 decided.

## Security — Five Layers Deep

Security isn't a feature. It's the foundation.

### Layer 1: Every Edit (<5ms, Rust)
```
PostToolUse hook → forge hook post-edit → regex + entropy secret scan
PreToolUse hook → forge protect → blocks .env, .pem, credentials
```
Every file edit is scanned. Every sensitive file is protected. Rust binary, <5ms.

### Layer 2: Every Agent
- **Worktree isolation** — generators work in git copies, can't corrupt main branch
- **Agent ACLs** — evaluator cannot Edit/Write, planner cannot Bash
- **Input validation** — all hook payloads: regex-validated IDs, no path traversal, no shell metacharacters
- **Bounded stdin** — agent hooks reject payloads >64KB

### Layer 3: Every Memory
- **Trust-level filtering** — only user-trust decisions injected into agent context
- **Parameterized Cypher** — `$param` syntax, never string interpolation
- **Property key validation** — `^[A-Za-z_][A-Za-z0-9_]{0,63}$`
- **Secret fingerprinting** — SHA256 hashes only, NEVER stores actual values
- **Cypher sandbox** — `forge query` blocks memory node access, code nodes only

### Layer 4: Every Review
- **Two-stage evaluation** — spec compliance THEN code quality (mandatory order)
- **Cross-model adversarial review** — Claude writes, Codex reviews. Different model = different blind spots.
- **Auto-fail rules** — security rubric auto-fails if Input Validation or Auth ≤ 2
- **Hard gate** — changes to `infrastructure/**`, `terraform/**`, `k8s/**` MUST pass Codex review

### Layer 5: Every Session
- **`forge doctor`** — 13 health checks verify the entire installation
- **No persistent process** — no MCP server, no DB lock, no attack surface
- **Symlink defense** — all file operations verify `symlink_metadata()` before read/write
- **Atomic writes** — tmp file + rename pattern on all state files (0o600 perms)

## Agent Team

```
Planner (Opus, read-only)  →  Generator(s) (worktree-isolated)  →  Evaluator (Opus, read-only)  →  Codex (adversarial)
```

| Agent | Role | Security |
|-------|------|----------|
| **Planner** | Architecture + wave planning. Never specifies implementation. | Read-only. No Write/Edit/Bash. |
| **Generator** | Implements tasks in isolated worktrees. Deviation rules: auto-fix bugs, STOP on architecture changes. | Worktree-isolated. Cannot touch main branch. |
| **Evaluator** | Two-stage graded review. Distrusts claims — runs tests, verifies on disk. | Read-only. Cannot modify code. |
| **Codex** | Cross-model adversarial review. Different model catches different bugs. | External. No repo write access. |

Agents receive structured XML context at spawn:
```xml
<forge-agent-context>
  <task>
    <description>Implement user authentication</description>
    <acceptance-criteria>JWT tokens, bcrypt passwords, 24h expiry</acceptance-criteria>
  </task>
  <decisions>
    <decision title="REST API pattern" confidence="0.95">Express + middleware chain</decision>
  </decisions>
  <prior-wave-summary>Wave 1 built: database schema, 12 tests passing</prior-wave-summary>
</forge-agent-context>
```

Wave 2 agents know what Wave 1 built. Session 10 agents know what Session 1 decided.

## CLI Reference

### Memory
```bash
forge remember --type decision --title "..." --content "..."   # Store
forge remember --type lesson --title "..." --content "..."     # Learn
forge remember --type pattern --title "..." --content "..."    # Pattern
forge recall "keyword"                                          # Search
forge recall --list --type decision                            # List all
forge recall --graph "keyword"                                  # Deep graph search
forge forget <id> --label Decision                             # Soft-delete
forge sync                                                      # Flush pending → graph
```

### Code Intelligence
```bash
forge index .                    # Parse Python/TS/JS → symbol graph
forge scan .                     # Detect exposed secrets
forge scan . --watch             # Always-on security monitor
forge query "MATCH (f:File) RETURN f.name LIMIT 10"  # Cypher graph query
```

### System
```bash
forge doctor --format text       # 13 health checks
forge health                     # Graph node/edge counts
forge --version                  # forge 0.3.0
```

### Hooks (automatic — called by Claude Code)
```bash
forge hook session-start         # XML context injection (<5ms)
forge hook post-edit <file>      # Secret scan per file (<5ms)
forge hook session-end           # HUD update + memory sync
forge agent                      # Agent lifecycle tracking
```

## Memory System

Forge remembers across sessions. Not flat files — a knowledge graph.

```
Session 1: forge remember --type decision --title "Use PostgreSQL" --content "..."
Session 2: XML context injection → agent receives: <decision title="Use PostgreSQL">...</decision>
Session 5: forge recall "database" → returns decision with confidence decay
```

**Dual storage:** Cache (Rust, <5ms reads) + LadybugDB graph (Python, <200ms, durable).

**Confidence decay:** `effective = confidence × e^(-0.03 × days)` — ~23-day half-life. Recent decisions are stronger.

**XML injection at session start:**
```xml
<forge-context version="0.3.0">
  <decisions count="6">
    <decision title="CLI-first architecture" confidence="0.95">...</decision>
  </decisions>
  <lessons count="2">
    <lesson>Always test end-to-end, not just unit tests</lesson>
  </lessons>
</forge-context>
```

## Skills

| Skill | When to Use |
|-------|------------|
| `forge` | **Start here.** Auto-detects greenfield vs existing codebase |
| `forge:forge-new` | New project from scratch (PRD → design → build) |
| `forge:forge-feature` | Existing code (explore → plan → build → review) |
| `forge:forge-review` | Code review — evaluator + Codex adversarial gate |
| `forge:forge-ship` | PR creation + final verification |
| `forge:forge-research` | Autonomous research with git checkpoints |
| `forge:forge-security` | Security scanning — `forge scan .` or `--watch` mode |
| `forge:forge-agents` | View agent status, tool calls, activity timeline |
| `forge:forge-handoff` | Pause/resume sessions with state preservation |
| `forge:forge-setup` | First-time prerequisite checks |

## Agent Team: Known Limitations

Agent teams are experimental. Forge handles their limitations, but you should understand them.

<details>
<summary><strong>Click to expand: 10 known limitations and mitigations</strong></summary>

### 1. Teammate context exhaustion (CRITICAL)
**Problem:** When a teammate's context fills up, it becomes unresponsive or loops.
**Forge handles it:** Isolated worktrees with focused single-task prompts, `maxTurns` caps (50 generators, 30 evaluator), session guard at 90/120 minutes, `TeammateIdle` hook detection.
**You should:** Monitor progress. If quality degrades, tell the lead: "Shut down [teammate] and spawn a replacement."

### 2. No session resumption for teammates
**Problem:** `/resume` doesn't restore in-process teammates.
**Forge handles it:** `forge-handoff` saves state to STATE.md. On resume, spawns fresh teammates.
**You should:** Always use `forge:forge-handoff` before ending a session with active teammates.

### 3. Task status can lag
**Problem:** Teammates sometimes fail to mark tasks complete, blocking dependent waves.
**Forge handles it:** `TaskCompleted` hook validates completion. Wave-based execution limits cross-dependencies.

### 4. Lead does work instead of delegating
**Problem:** The lead agent sometimes implements tasks itself.
**Forge handles it:** Explicit "You NEVER write code" instruction. Use delegate mode (Shift+Tab).

### 5. File conflicts between teammates
**Problem:** Two teammates editing the same file causes corruption.
**Forge handles it:** Every generator runs in `isolation: worktree`. Conflicts caught at merge time.

### 6. Graceful shutdown can be slow
**Problem:** Teammates finish current operations before shutting down.
**Forge handles it:** `forge-handoff` shuts down sequentially. Work committed before shutdown.

### 7. Orphaned processes
**Problem:** Unexpected crashes leave agent processes running (150-800 MB each).
**You should:** `tmux ls && tmux kill-session -t <name>` — do NOT use killall.

### 8. Token costs scale with team size
**Forge handles it:** Team size guidelines (1-2 tasks → subagents, 3-4 → 2-3 generators + evaluator, max 5 teammates). Focused single-task prompts minimize per-agent tokens.

### 9. One team per session
**Forge handles it:** Teams created and destroyed per wave group, not persistent.

### 10. No nested teams
**Forge handles it:** Generators use subagents (not teams) for internal parallelization.

</details>

## Evaluation Rubrics

Four scored rubrics (1-5 per criterion, weighted average):

| Rubric | Pass Threshold | Auto-Fail |
|--------|---------------|-----------|
| Code Quality | ≥ 3.5 | Any criterion = 1 |
| Security | ≥ 4.0 | Input Validation or Auth ≤ 2 |
| Architecture | ≥ 3.5 | Consistency < 3 |
| Infrastructure | ≥ 4.0 | Security Posture or Blast Radius < 3 |

## Cross-Platform

Forge works with any AI coding tool that has Bash access:

```bash
# Claude Code (native plugin)
claude plugin install forge@forge-marketplace

# Cargo (any platform)
cargo install forge-agentic-os

# Any tool with Bash
forge doctor && forge recall --list && forge remember --type decision --title "..." --content "..."
```

See [AGENTS.md](AGENTS.md) for Codex, Gemini CLI, and generic tool integration.

## Companion Plugins (Optional)

Forge works standalone. These enhance it:

| Plugin | Purpose | Required? |
|--------|---------|-----------|
| `codex` | Cross-model adversarial review (OpenAI) | Recommended |
| `superpowers` | Methodology skills (TDD, brainstorming) | Recommended |
| `serena` | LSP-grade symbol navigation | Optional |

## Configuration

| Option | Default | Description |
|--------|---------|-------------|
| `codex_enabled` | `true` | Enable Codex adversarial review |
| `prod_paths` | `infrastructure/**,terraform/**,k8s/**` | Paths requiring hard Codex gate |
| `default_generator_model` | `opus` | Model for generators (`opus` or `sonnet`) |

## Architecture

```
forge (Rust, 4.3MB)              — CLI: 14 subcommands, <5ms for hot paths
forge-graph (Python, 115 tests)  — LadybugDB graph, called by forge via subprocess
forge-hud (Rust, 476KB)          — StatusLine: <2ms render, real-time stats
```

No MCP server. No persistent process. Rust for speed. Python for graph writes. Each operation opens DB, writes, closes, exits.

## First Principles

> Every component in an agent harness encodes an assumption about what the model cannot do on its own. These assumptions should be stress-tested because they may be incorrect and they will go stale as the model improves.
> — [Anthropic Engineering](https://www.anthropic.com/engineering/harness-design-long-running-apps)

Forge keeps only what the model genuinely needs help with. Everything else is delegated or removed.

## Acknowledgments

Built on: [Claude Code](https://docs.anthropic.com/en/docs/claude-code) · [Codex](https://github.com/openai/codex) · [LadybugDB](https://github.com/LadybugDB/ladybug) · [tree-sitter](https://tree-sitter.github.io/) · [Playwright](https://playwright.dev/) · [Superpowers](https://github.com/obra/superpowers) · and every dependency beneath them. Thank you.

## License

MIT
