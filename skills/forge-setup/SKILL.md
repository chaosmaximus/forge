---
name: forge-setup
description: "Use on first run in a new project directory, or when Forge prerequisites need checking. Example triggers: 'set up forge', 'check forge prereqs', 'forge doctor', 'is forge ready', 'initialize forge'."
---

# Forge Setup

Run through this checklist and report status:

## Prerequisites

1. Claude Code version:
   ```bash
   claude --version
   ```
   Requires >= 2.1.32.

2. Agent orchestration:
   Agent orchestration is handled by the host agent. No special env vars needed.

3. Forge daemon health:
   Run `forge-next health` to check daemon status.
   Run `forge-next doctor` to check full system health.
   Run `forge-next manas-health` to check 8-layer memory health.
   Run `forge-next identity` to check agent identity (Ahankara).
   If not responding: check that `forge-next` binary is on PATH. Install from the public repo: `cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli`.

4. Two CLIs are available — know when to use each:
   - `forge` — code operations: index, scan, query, verify, test, research, review, hook
   - `forge-next` — memory/daemon operations: recall, remember, forget, health, doctor, manas-health, identity, blast-radius, check, sessions

## Companion Plugins (check and recommend)

> Install commands below are based on standard Claude Code plugin syntax. If a command fails, check the plugin's documentation for updated install instructions.

| Plugin | Status | Purpose | Install Command |
|--------|--------|---------|----------------|
| codex-plugin-cc | [check] | Cross-model adversarial review | `/plugin marketplace add openai/codex-plugin-cc` |
| serena | [check] | LSP-grade code navigation | `/plugin install serena@claude-plugins-official` |
| context7 | [check] | Library documentation lookup | `/plugin install context7@claude-plugins-official` |
| playwright | [optional] | Browser E2E testing | `/plugin install playwright@claude-plugins-official` |

**Note:** Forge now includes built-in skills for TDD (`forge-tdd`), debugging (`forge-debug`), verification (`forge-verify`), and memory (`forge-next recall/remember`). The `superpowers` and `episodic-memory` plugins are no longer needed.

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

## Stitch MCP (Optional)

Stitch MCP is configured in the plugin's .mcp.json. If you don't use visual design, you can delete that file from the plugin directory to avoid loading it: `rm ${CLAUDE_PLUGIN_ROOT}/.mcp.json`

## Done

"Forge is set up. Run `/forge:new` for a new project or `/forge:feature` for existing code."
