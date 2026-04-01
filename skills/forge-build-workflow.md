# Forge Build Workflow (Shared Reference)

This file defines the shared build workflow used by both forge-new (greenfield) and forge-feature (existing codebase) modes. It is NOT a skill — it is a reference document read by the lead skill during the build phase.

## Step 1: Prepare

1. Read the approved plan (PRD wave plan for greenfield, implementation plan for existing)
2. Identify waves and task-per-wave groupings
3. Verify agent teams are enabled

## Step 2: Execute Waves

For each wave:

1. **Create tasks** via TaskCreate — one per parallel task in this wave
2. **Spawn generator teammates:**
   - One forge-generator per parallel task (max 4 per wave)
   - Each in `isolation: worktree`
   - Each receives: the specific task description, acceptance criteria, relevant context from exploration/PRD
   - Model: use `default_generator_model` from userConfig (opus or sonnet)
3. **Spawn evaluator teammate** — one forge-evaluator that waits for generators
4. **Enter delegate mode:**
   "You are the team coordinator. You NEVER write code, edit files, or implement features.
   You delegate to generator teammates, monitor progress, relay evaluator feedback, and
   manage the task list. Enter delegate mode. Tell the user: 'I recommend enabling delegate mode to restrict me to coordination only. Press Shift+Tab in your terminal to activate it.'"
5. **Monitor:** Check task progress. If a generator reports BLOCKED or NEEDS_CONTEXT, provide the needed context or escalate to the user.

**Circuit breaker:** If a generator fails evaluation 3 times for the same task, STOP retrying. Present the findings to the user and ask:
  1. Provide additional context and retry (recommended)
  2. Simplify the task scope
  3. Skip this task and continue with the next wave
  4. Take over implementation manually

6. **Review:** When generators complete, evaluator reviews each. On FAIL, relay findings back to generator. Present findings to user before requesting generator rework.
7. **Merge:** On PASS, merge generator worktrees to main branch.

Merge strategy: Use `git merge --no-ff <worktree-branch>` to preserve task-level commit history. If merge conflicts occur, present the conflicts to the user and ask how to resolve. Do NOT auto-resolve merge conflicts.

8. **Advance:** Update STATE.md. Move to next wave.

## Step 3: Session Guard

To track time, note the timestamp when the build phase starts by running `date +%s` via Bash. Periodically check elapsed time with `echo $(( $(date +%s) - START_TIME ))`. Compare against 5400 (90 min) and 7200 (120 min).

- At 90 minutes: "We've been running for 90 minutes. Recommend checkpointing. Run /forge:handoff?"
- At 120 minutes: "Session limit reached. Auto-checkpointing to STATE.md."
  Save state and recommend starting a fresh session.

## Wave Execution Diagram

```
Wave 1: [Task A, Task B, Task C] — independent, parallel generators
  | all complete + evaluator approved
Wave 2: [Task D, Task E] — depend on Wave 1
  | all complete + evaluator approved
Wave 3: [Task F] — depends on Wave 2
```

## Team Size Guidelines

- 1-2 tasks: 1 generator + 1 evaluator (skip agent team, use subagents)
- 3-4 tasks: 2-3 generators + 1 evaluator
- 5+ tasks: 3-4 generators + 1 evaluator (max 5 teammates total)
