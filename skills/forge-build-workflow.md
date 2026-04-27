<!-- SPDX-License-Identifier: Apache-2.0 -->
# Forge Build Workflow (Shared Reference)

This file defines the shared build workflow used by both forge-new (greenfield) and forge-feature (existing codebase) modes. It is NOT a skill — it is a reference document read by the lead skill during the build phase.

## Step 1: Prepare

1. Read the approved plan (PRD wave plan for greenfield, implementation plan for existing)
2. Identify waves and task-per-wave groupings
3. Verify agent teams are enabled
4. Load context for agents:
   ```bash
   forge-next recall "project context" --type decision
   ```
   Store the output — this will be injected into each agent's spawn prompt.

## Step 2: Execute Waves

For each wave:

1. **Create tasks** via the `TodoWrite` tool — one per parallel task in this wave

2. **Build spawn context** for each agent. Include ALL of these in the spawn prompt:
   ```xml
   <forge-agent-context>
     <task>
       <description>[task description from plan]</description>
       <acceptance-criteria>[criteria from plan]</acceptance-criteria>
       <wave>[current wave number]</wave>
     </task>
     <prior-wave-summary>
       [For wave 2+: summarize what was built in the prior wave]
       [For wave 1: "First wave — no prior context"]
     </prior-wave-summary>
     <decisions>
       [Paste relevant decisions from forge-next recall output]
     </decisions>
     <relevant-files>
       [List files this task should touch, from the plan]
     </relevant-files>
   </forge-agent-context>
   ```

3. **Pre-flight verification (CRITICAL for Wave 2+):**
   Before spawning any generator, verify that prior wave changes are on the current branch:
   - Check `git log --oneline -3` to confirm prior wave merges are present
   - List 2-3 "canary files" — files or symbols created by prior waves that this wave depends on
   - Include canary files in the spawn context as `<canary-files>`:
     ```xml
     <canary-files>
       <file path="crates/core/src/types/entity.rs" contains="Portability" />
       <file path="crates/daemon/src/db/ops.rs" contains="ensure_defaults" />
     </canary-files>
     ```
   - If using worktree isolation and prior waves created new files, the worktree may branch from a stale commit. Consider running generators WITHOUT worktree isolation (directly on master) if worktrees consistently miss prior wave changes.

4. **Spawn generator teammates:**
   - One forge-generator per parallel task (max 4 per wave)
   - **DEFAULT: NO worktree isolation.** Run generators directly on the current branch. Worktrees frequently branch from stale commits, missing prior wave changes. This has caused repeated build failures.
   - Only use `isolation: worktree` when: (a) multiple generators modify overlapping files in the same wave, AND (b) you have verified the worktree will branch from the correct commit
   - Each receives the XML spawn context above INCLUDING canary-files
   - Model: use `default_generator_model` from userConfig (opus or sonnet)
   - Include explicit scope guard in prompt: "ONLY modify these files: [list]"
   - Include dependency check in prompt: "Check Cargo.toml for available crates before using any (use ulid NOT uuid)"

5. **Spawn evaluator teammate** — one forge-evaluator that waits for generators.
   Evaluator receives: the plan, the spawn context given to generators, and the acceptance criteria.

6. **Enter delegate mode:**
   "You are the team coordinator. You NEVER write code, edit files, or implement features.
   You delegate to generator teammates, monitor progress, relay evaluator feedback, and
   manage the task list. Enter delegate mode. Tell the user: 'I recommend enabling delegate mode to restrict me to coordination only. Press Shift+Tab in your terminal to activate it.'"

7. **Monitor:** Check task progress. If a generator reports BLOCKED or NEEDS_CONTEXT, provide the needed context or escalate to the user.

**Circuit breaker:** If a generator fails evaluation 3 times for the same task, STOP retrying. Present the findings to the user and ask:
  1. Provide additional context and retry (recommended)
  2. Simplify the task scope
  3. Skip this task and continue with the next wave
  4. Take over implementation manually

8. **Review:** When generators complete, evaluator reviews each. On FAIL, relay findings back to generator. Present findings to user before requesting generator rework.

9. **Merge:** On PASS, merge generator output to main branch.

Merge strategy: If worktrees were used, `git merge --no-ff <worktree-branch>`. If generators ran directly on the branch, changes are already committed. If merge conflicts occur, present the conflicts to the user and ask how to resolve. Do NOT auto-resolve merge conflicts.

10. **Record wave results:**
   ```bash
   forge-next remember --type pattern --title "Wave N results" \
     --content "[summary: tasks completed, files changed, test results, evaluator scores]"
   ```
   This ensures future sessions can recall what was built.

11. **Advance:** Move to next wave.

## Step 3: Session Guard

Monitor session health based on **context usage and progress**, not wall-clock time. Time is a poor proxy — a focused 3-hour session is fine, a scattered 30-minute session may need a checkpoint.

**When to suggest a checkpoint:**
- Context is getting compressed (you see `[compacted]` markers or feel you're losing earlier context)
- You've completed a logical milestone (wave, feature, major fix) and want to preserve state
- You're about to start a fundamentally different task (switching from bug fixing to feature building)
- Multiple failed attempts suggest you need fresh context to re-approach

**How to checkpoint:**
```bash
forge-next remember --type decision --title "<what was accomplished>" --content "<summary + test count + next steps>"
```

**When NOT to suggest a checkpoint:**
- In the middle of implementing a feature (break context = break flow)
- Just because "it's been a while" — context quality matters, not duration

## Wave Execution Diagram

```
Wave 1: [Task A, Task B, Task C] — independent, parallel generators
  │ all complete + evaluator approved
  │ → Record wave results via forge-next remember
  │
Wave 2: [Task D, Task E] — depend on Wave 1
  │ each generator receives Wave 1 summary in spawn context
  │ all complete + evaluator approved
  │ → Record wave results
  │
Wave 3: [Task F] — depends on Wave 2
  │ receives Wave 1 + Wave 2 summaries
  │ complete + evaluator approved
  │ → Record wave results
```

## Team Size Guidelines

- 1-2 tasks: 1 generator + 1 evaluator (skip agent team, use subagents)
- 3-4 tasks: 2-3 generators + 1 evaluator
- 5+ tasks: 3-4 generators + 1 evaluator (max 5 teammates total)

## Memory Best Practices

- **Before build:** `forge-next recall "project" --type decision` to load prior decisions
- **After each wave:** `forge-next remember --type pattern` to record what was built
- **After build complete:** `forge-next remember --type decision` to record key architectural choices
- **On evaluator findings:** `forge-next remember --type lesson` to capture learnings
