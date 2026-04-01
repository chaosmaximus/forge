---
name: forge
description: Use when starting any development task, building features, or creating new projects — before writing any code
---

# Forge — Production-Grade Agent Orchestration

You are using Forge to orchestrate a production-grade development workflow.

## Step 1: Detect Mode

Check the current directory:
1. Is there existing source code (beyond config files)? → **Existing Codebase Mode**
2. Is this a new/empty project? → **Greenfield Mode**
3. Is there a STATE.md with an in-progress session? → **Resume**: read STATE.md, determine the mode (greenfield or existing) and current phase. If in greenfield, invoke forge-new (it will detect the phase from STATE.md). If in existing, invoke forge-feature. Tell the lead to spawn fresh teammates if needed — do NOT try to reconnect to old teammates.

Announce: "I'm using Forge in [greenfield/existing codebase] mode."

## Step 2: Check Prerequisites

1. Verify agent teams are enabled: check for `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` in settings
2. If not: "Agent teams are recommended for multi-task builds. Add this to your settings.json:
   `{ "env": { "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS": "1" } }`
   For simple 1-2 task changes, Forge can work with subagents instead."
3. Check if codebase-memory-mcp graph exists (for existing codebase mode):
   Call `mcp__forge_forge-graph__index_status`. If not indexed: "Indexing your codebase for the first time. This runs in the background."
4. Check Codex plugin: if not found, warn: "Codex plugin not installed. Adversarial review will be unavailable. Install: `/plugin marketplace add openai/codex-plugin-cc`"
5. Check Serena plugin: if `mcp__plugin_serena_serena__find_symbol` is not available, note: "Serena plugin not installed. Code exploration will use basic search. For better results, install Serena."

## Step 3: Route

- Greenfield → Invoke `forge-new` skill
- Existing codebase → Invoke `forge-feature` skill

<HARD-GATE>
Do NOT skip mode detection. Do NOT skip prerequisite checks. Do NOT jump
to code generation. Every project goes through this router.
</HARD-GATE>
