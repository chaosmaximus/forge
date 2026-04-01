---
name: forge-setup
description: Use on first run in a new project directory, or when Forge prerequisites need checking
---

# Forge Setup

Run through this checklist and report status:

## Prerequisites

1. Claude Code version:
   ```bash
   claude --version
   ```
   Requires >= 2.1.32 for agent teams.

2. Agent teams enabled:
   Check settings for `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS`.
   If missing: guide user to add it.

3. Code intelligence (codebase-memory-mcp):
   ```
   Call mcp__forge_forge-graph__index_status
   ```
   If not responding: check that the binary is at `${CLAUDE_PLUGIN_ROOT}/servers/codebase-memory-mcp`

## Companion Plugins (check and recommend)

> Install commands below are based on standard Claude Code plugin syntax. If a command fails, check the plugin's documentation for updated install instructions.

| Plugin | Status | Purpose | Install Command |
|--------|--------|---------|----------------|
| codex-plugin-cc | [check] | Cross-model adversarial review | `/plugin marketplace add openai/codex-plugin-cc` |
| superpowers | [check] | Brainstorming, TDD, verification | `/plugin marketplace add claude-plugins-official && /plugin install superpowers` |
| episodic-memory | [check] | Cross-session recall | `/plugin marketplace add obra/superpowers-marketplace && /plugin install episodic-memory` |
| serena | [check] | LSP-grade code navigation | `/plugin install serena@claude-plugins-official` |
| frontend-design | [check] | Production-grade UI code | `/plugin install frontend-design@claude-plugins-official` |
| stitch-mcp | [check] | Visual design generation | Configure in .mcp.json (see README) |
| mcp2cli | [optional] | Token optimization for MCP-heavy projects | See github.com/myeolinmalchi/mcp2cli |

## Production Path Configuration

Your project may use non-standard production directory names. Would you like to customize the production path patterns for Codex hard gating? Current defaults: `infrastructure/**`, `terraform/**`, `k8s/**`, `helm/**`, `production/**`.

Common additions: `prod/**`, `deploy/**`, `live/**`, or project-specific patterns.

## Initial Project Files

If no CONSTITUTION.md exists, offer to create one:
"Want to set up a project constitution? This defines immutable principles
(e.g., 'test-first', 'library-first', 'no raw SQL') that Forge enforces. Takes 2 minutes."

If yes: ask 3-5 questions, create from template.
If no: skip.

Create STATE.md with initial state.

## Done

"Forge is set up. Run `/forge:new` for a new project or `/forge:feature` for existing code."
