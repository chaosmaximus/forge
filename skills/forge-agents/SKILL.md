<!-- SPDX-License-Identifier: Apache-2.0 -->
---
name: forge-agents
description: "View detailed status of running and recent Forge agents — tool calls, files touched, recent activity timeline. Use when user asks 'show agents', 'agent status', 'what are the agents doing', 'how is the build going', or wants to see forge-planner/forge-generator/forge-evaluator progress."
---

# Agent Status Drilldown

Show the user detailed information about Forge agents (planner, generator, evaluator).

## Data Sources

Read these files to get agent information:

1. **Agent roster:** Read the HUD state file. Find the Forge plugin data directory by checking these paths in order:
   - `~/.claude/plugins/data/forge-forge-marketplace/`
   - `~/.claude/plugins/data/forge/`
   Then read `hud/hud-state.json` from that directory. The `.team` object contains agent entries — each key is an `agent_id`, each value has `type`, `status`, `started_at`, `ended_at`, `tool_calls`, `files`, `last_tool`, `current_file`.

2. **Agent timeline:** For detailed drilldown on a specific agent, read `agents/{agent_id}.jsonl` from the same data directory. Each line is a JSON event: `{"event": "start|tool|stop", "ts": "...", "tool": "...", "file": "..."}`.

## Display Format

### No agents in team
```
No agents tracked this session.
```

### Agents present
For each agent, show:
- Status icon: ▶ running, ✓ done, ⏳ pending, ✗ blocked, ⚠ stale
- Agent type (e.g., "generator", "evaluator") — strip "forge-" prefix
- Duration (calculate from started_at if available)
- Tool call count
- Files touched (from the `files` array)
- Last tool (if running)

### Detailed drilldown
If the user asks about a specific agent (e.g., `/forge-agents generator`), find the matching agent by type substring, read its JSONL file, and show the full activity timeline with timestamps and tool calls.

## Instructions

1. Find the Forge data directory (check paths listed above)
2. Read `hud/hud-state.json`
3. Present the `.team` data in the format above
4. If user wants details on a specific agent, read its JSONL from `agents/`
5. Skip lines in JSONL that fail to parse (corrupted lines are expected)
