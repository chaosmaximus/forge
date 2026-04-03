# Forge — Cross-Platform Usage

Forge works with any AI coding tool that supports skills and Bash access.

## Quick Start

```bash
# Option 1: Install as Claude Code plugin
claude plugin install forge@forge-marketplace

# Option 2: Install binary only (works with any tool)
curl -sSf https://raw.githubusercontent.com/chaosmaximus/forge/master/install.sh | bash

# Option 3: Build from source
cargo install forge-agentic-os

# Verify installation
forge doctor --format text
```

## Core Commands (Universal)

These work in **any** AI coding tool via Bash:

```bash
# Memory — store and recall decisions, patterns, lessons
forge remember --type decision --title "..." --content "..."
forge recall "keyword"
forge recall --list --type decision
forge forget <node_id> --label Decision

# Code intelligence
forge index .                              # Parse codebase → symbol graph
forge scan .                               # Detect exposed secrets
forge query "MATCH (f:File) RETURN f.name" # Cypher graph queries

# System
forge doctor --format text                 # Health check (13 checks)
forge health                               # Graph node/edge counts
forge sync                                 # Flush pending memory to graph DB

# Agent tracking (called by hooks automatically)
forge agent                                # Process hook payload from stdin
forge hook session-start                   # Inject context at session start
forge hook session-end                     # Update HUD state
forge hook post-edit <file>                # Scan file for secrets
```

## Tool Mapping

| Claude Code | Codex | Gemini CLI | Generic |
|------------|-------|------------|---------|
| `forge remember ...` | Same | Same | Same |
| `forge recall ...` | Same | Same | Same |
| `forge doctor` | Same | Same | Same |
| Hooks (automatic) | Manual: `forge hook session-start` | Manual | Manual |
| HUD (statusLine) | N/A | N/A | `forge recall --list` |
| `Skill("forge:forge")` | Load `skills/*/SKILL.md` | `activate_skill("forge")` | Read skill files |

## Skills (Portable)

Skills in `skills/` are markdown files that work in any tool supporting skill loading:

| Skill | Purpose |
|-------|---------|
| `forge` | Main router — detects greenfield vs existing codebase |
| `forge-feature` | Modify existing code (explore → plan → build) |
| `forge-new` | Build new project from scratch (PRD → design → build) |
| `forge-review` | Code review with rubrics |
| `forge-security` | Security scanning |
| `forge-research` | Autonomous research loop |
| `forge-agents` | View agent status and activity |
| `forge-doctor` | System health viewer |

To use in your tool:
1. Copy `skills/` to your tool's skill directory
2. Each `SKILL.md` has YAML frontmatter with `name` and `description`
3. Skills reference `forge` CLI commands via Bash

## Agents (Require Subagent Support)

Agent definitions in `agents/` require tools that support spawning subagents:

| Agent | Role | Required Capabilities |
|-------|------|----------------------|
| `forge-planner` | Architecture + planning | Read, Grep, Bash |
| `forge-generator` | Implementation | Full: Read, Write, Edit, Bash |
| `forge-evaluator` | Code review + testing | Read, Grep, Bash (read-only) |

If your tool doesn't support agents, execute the workflow manually using skills as guides.

## Architecture (v0.6.0 — Manas)

```
forge-daemon (Rust, single binary) — always-on daemon, Unix socket API
├── 8-layer memory             — episodic, semantic, procedural, decision,
│                                identity, perception, disposition, working
├── 7 background workers       — extraction, embedding, compaction, sync,
│                                health, adapters, events
├── guardrails engine          — check + blast_radius
├── multi-agent adapters       — Claude Code + Cline + Codex CLI
├── auto-extraction            — claude -p / ollama
├── session tracking           — active agent sessions
└── event bus                  — tokio::broadcast

forge-next (Rust CLI)          — client for daemon, auto-starts daemon
forge-hud (Rust)               — StatusLine rendering
```

## Publishing

```bash
# crates.io
cargo install forge-agentic-os

# Homebrew (planned)
brew install chaosmaximus/tap/forge

# Claude Code plugin
claude plugin install forge@forge-marketplace
```
