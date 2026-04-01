# Forge

Production-grade agent team orchestration for Claude Code.

Two modes. Three agents. Cross-model adversarial review. Bundled code intelligence.

## Requirements

- **Claude Code v2.1.32+** (agent teams support)
- **Agent Teams enabled** (experimental feature, required):
  ```json
  // Add to ~/.claude/settings.json
  {
    "env": {
      "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS": "1"
    }
  }
  ```
- **Node.js 18+** (for companion plugins)
- **jq** (for hook scripts — `apt install jq` or `brew install jq`)
- **git** (for worktree isolation and atomic commits)

## Quick Start

```bash
# Install the plugin
claude plugin install /path/to/forge

# First-time setup (checks all prerequisites)
/forge:setup

# Start building
/forge:new      # New project from scratch
/forge:feature  # Add to existing codebase
```

## Two Modes

### Greenfield (`/forge:new`)
For building from scratch. Guides through structured PRD creation with domain-specific knowledge injection (auto-surfaces compliance requirements for fintech, healthcare, govtech, etc.), optional visual design via Google Stitch, then agent team build with wave-based parallel execution.

### Existing Codebase (`/forge:feature`)
For modifying existing code. Explores the codebase using a graph database (call chains, blast radius, architecture overview) before planning, then builds with agent teams in isolated git worktrees.

## Architecture

```
Planner (Opus, read-only) --> Generator(s) (Opus, worktree-isolated) --> Evaluator (Opus, read-only) --> Codex (GPT 5.4, adversarial)
```

- **Planner**: Product-level planning. Never specifies implementation details. Adapts to mode.
- **Generator**: Implements tasks with deviation rules (auto-fix bugs, STOP on architecture changes). Reports: DONE / DONE_WITH_CONCERNS / NEEDS_CONTEXT / BLOCKED.
- **Evaluator**: Two-stage graded review (spec compliance THEN code quality). Scores 1-5 against rubrics. Distrusts claims — verifies on disk.
- **Codex**: Cross-model adversarial review via codex-plugin-cc. Hard gate on production paths, on-demand elsewhere.

## Skills

| Skill | Purpose |
|-------|---------|
| `/forge` | Router — detects mode, checks prerequisites |
| `/forge:new` | Greenfield: classify, discover, PRD, design, build |
| `/forge:feature` | Existing: explore graph, plan, build, review |
| `/forge:review` | Two-stage evaluation + Codex adversarial gate |
| `/forge:ship` | Final verification + PR + memory save |
| `/forge:handoff` | Session pause/resume with state preservation |
| `/forge:setup` | First-time prerequisite checks |

## Agent Teams: Known Limitations and Mitigations

Agent teams are an experimental Claude Code feature. Forge is designed to handle their limitations, but you should understand them.

### 1. Teammate context exhaustion (CRITICAL)

**Problem:** When a teammate's context window fills up (from large tool outputs, long conversations, or many file reads), the teammate may become unresponsive, produce degraded output, or appear stuck in a loop. Unlike the lead session which can compact, teammates have no mechanism to recover from context exhaustion mid-task.

**How Forge handles it:**
- Generators work in isolated worktrees with focused, single-task prompts (minimal context load)
- The `maxTurns` frontmatter (50 for generators, 30 for evaluator/planner) prevents infinite loops
- The session guard in forge-build warns at 90 minutes and forces a checkpoint at 120 minutes
- The `TeammateIdle` hook detects when teammates stop producing output
- If a teammate appears stuck: the lead should shut it down and spawn a replacement

**What you should do:** Monitor teammate progress. If output quality degrades or a teammate repeats itself, tell the lead: "Shut down [teammate-name] and spawn a replacement."

### 2. No session resumption for teammates

**Problem:** `/resume` and `/rewind` do NOT restore in-process teammates. After resuming a session, the lead may try to message teammates that no longer exist, causing errors or silent failures.

**How Forge handles it:**
- `forge-handoff` saves all state to STATE.md and HANDOFF.md before shutdown
- On resume, the forge router reads STATE.md and spawns fresh teammates (never reconnects dead ones)
- All generator work is committed atomically, so no progress is lost when teammates are replaced

**What you should do:** Always use `/forge:handoff` before ending a session with active teammates. If you resume without handoff, run `/forge` — it will detect the stale state and guide you.

### 3. Task status can lag

**Problem:** Teammates sometimes fail to mark tasks as completed, which blocks dependent tasks in subsequent waves. The lead may not have accurate visibility into actual task progress.

**How Forge handles it:**
- The `TaskCompleted` hook validates actual completion (runs tests) before allowing status change
- The lead is instructed to periodically check task status and nudge lagging teammates
- Wave-based execution limits cross-dependencies (tasks within a wave are independent)

**What you should do:** If a task appears stuck, check whether the teammate actually finished the work and tell the lead to update the status manually.

### 4. Lead does work instead of delegating

**Problem:** The lead agent sometimes starts implementing tasks itself instead of waiting for teammates, defeating the purpose of parallel execution and potentially causing file conflicts.

**How Forge handles it:**
- The build phase explicitly instructs: "You are the team coordinator. You NEVER write code. Delegate only."
- Use **delegate mode** (Shift+Tab after the team starts) to restrict the lead to orchestration tools only

**What you should do:** After the team is spawned, press Shift+Tab to enable delegate mode. If you see the lead editing files, tell it: "Stop. Delegate this to a generator teammate."

### 5. File conflicts between teammates

**Problem:** Two teammates editing the same file causes overwrites and data corruption. Git detects text-level conflicts but not semantic contradictions.

**How Forge handles it:**
- Every generator runs in `isolation: worktree` — each gets its own complete copy of the repo
- Wave-based execution ensures tasks within a wave are independent
- Worktree branches are merged after evaluator approval, catching conflicts at merge time

**What you should do:** If a merge conflict occurs, the lead will present it. Review the conflict and tell the lead which version to keep, or how to combine them.

### 6. Graceful shutdown can be slow

**Problem:** Teammates finish their current request/tool call before shutting down, which can take significant time if they're mid-operation (running tests, reading large files).

**How Forge handles it:**
- `forge-handoff` shuts down teammates one by one and waits for confirmation
- Work is committed before shutdown requests are sent
- As a last resort, orphaned tmux sessions can be killed: `tmux ls && tmux kill-session -t <name>`

**What you should do:** Be patient during shutdown. If a teammate hangs for more than 5 minutes, ask the lead to force-terminate it.

### 7. Orphaned processes

**Problem:** When a terminal is closed without graceful shutdown, agent processes and their child processes (LSP servers, build watchers, dev servers) persist as orphans, leaking 150-800 MB per session.

**How Forge handles it:**
- `forge-handoff` includes explicit cleanup steps
- `forge-ship` cleans up the team as its final step
- The README (this section) documents manual cleanup

**What you should do:** After an unexpected crash, check for orphans:
```bash
tmux ls                          # Find orphaned tmux sessions
tmux kill-session -t <name>      # Kill specific session
# Do NOT use killall — kill only specific forge-related processes
```

### 8. Token costs scale with team size

**Problem:** Each teammate has its own context window and consumes tokens independently. A team of 5 uses roughly 5x the tokens of a single session.

**How Forge handles it:**
- Team size guidelines in the build phase: 1-2 tasks use subagents (no team), 3-4 tasks use 2-3 generators + 1 evaluator, 5+ tasks use 3-4 generators + 1 evaluator (max 5 teammates)
- Never exceeds 5 teammates per wave
- Generators use focused single-task prompts to minimize token usage per teammate
- codebase-memory-mcp provides 99% token reduction for code exploration

**What you should do:** For small tasks (1-2 files), skip agent teams entirely — use a single session. Forge's router skill detects task size and recommends appropriately.

### 9. One team per session

**Problem:** A lead can only manage one team at a time. You cannot run multiple teams in parallel from a single session.

**How Forge handles it:**
- Each build phase creates one team, runs it to completion, cleans it up, then proceeds
- For multi-phase builds, teams are created and destroyed per wave group

### 10. No nested teams

**Problem:** Teammates cannot spawn their own teams or teammates. Only the lead can manage the team.

**How Forge handles it:**
- Generator teammates use regular subagents (not agent teams) if they need to parallelize within their task
- The evaluator runs as a single agent that reviews sequentially

## Companion Plugins

Forge delegates to existing plugins rather than reinventing:

| Plugin | Purpose | Required? |
|--------|---------|-----------|
| `codex-plugin-cc` | Cross-model adversarial review | Recommended |
| `superpowers` | Brainstorming, TDD, verification | Recommended |
| `episodic-memory` | Cross-session memory recall | Optional |
| `serena` | LSP-grade symbol navigation | Optional |
| `frontend-design` | Production-grade UI code | Optional |
| `stitch-mcp` | Visual design generation (greenfield) | Optional |

Install companions:
```
/plugin marketplace add openai/codex-plugin-cc
/plugin install codex@openai-codex
/codex:setup
```

## Bundled Code Intelligence

Forge bundles [codebase-memory-mcp](https://github.com/DeusData/codebase-memory-mcp) — a code graph database supporting 66 languages with sub-millisecond queries.

Auto-indexes on session start. Provides:
- `get_architecture` — codebase overview (languages, routes, hotspots, clusters)
- `search_graph` — find symbols by name, label, or file pattern
- `trace_call_path` — bidirectional call chain traversal
- `detect_changes` — map git diffs to affected symbols
- `get_code_snippet` — retrieve source by qualified name

99% token reduction compared to file-by-file grep.

## Configuration

Set in `userConfig` when enabling the plugin:

| Option | Default | Description |
|--------|---------|-------------|
| `codex_enabled` | `true` | Enable Codex adversarial review |
| `stitch_enabled` | `false` | Enable Google Stitch for visual design |
| `prod_paths` | `infrastructure/**,terraform/**,k8s/**,helm/**,production/**` | Glob patterns requiring hard Codex gate. Customize for your project -- common additions: `prod/**`, `deploy/**`, `live/**`, or project-specific patterns like `hive_production/**`. |
| `default_generator_model` | `opus` | Model for generator agents |

The `.mcp.json` file configures optional Stitch MCP for visual design. Delete it if you don't use Stitch to avoid unnecessary MCP server loading.

## Evaluation Rubrics

Four scored rubrics (1-5 per criterion):

| Rubric | Pass Threshold | Auto-Fail |
|--------|---------------|-----------|
| Code Quality | >= 3.5 avg | Any criterion = 1 |
| Security | >= 4.0 avg | Input Validation or Auth <= 2 |
| Architecture | >= 3.5 avg | Consistency < 3 |
| Infrastructure | >= 4.0 avg | Security Posture or Blast Radius < 3 |

## First Principles

> Every component in an agent harness encodes an assumption about what the model cannot do on its own. These assumptions should be stress-tested because they may be incorrect and they will go stale as the model improves.
> — [Anthropic Engineering](https://www.anthropic.com/engineering/harness-design-long-running-apps)

Forge keeps only what Opus 4.6 genuinely needs help with. Everything else is delegated or removed.

## License

MIT
