---
name: forge-agents
description: Use when user wants to see detailed status of running Forge agents, their tool calls, files, and token usage
---

# Agent Status Drill-Down

Show detailed status of all Forge agents from the HUD state.

## Steps

1. Read the HUD state file at `${CLAUDE_PLUGIN_DATA}/hud/hud-state.json`
2. For each agent in `team`, display:
   - Agent name and status (running/done/pending)
   - Last tool called
   - Current file being worked on
   - Token usage if available
3. Format as a clean markdown table

## Example Output

| Agent | Status | Last Tool | Current File |
|-------|--------|-----------|-------------|
| planner | done | axon_context | — |
| generator | running | forge_recall | src/auth/middleware.py |
| evaluator | pending | — | — |
