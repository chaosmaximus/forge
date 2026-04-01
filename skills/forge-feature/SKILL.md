---
name: forge-feature
description: Use when adding features, fixing bugs, or modifying code in an existing codebase with source files
---

# Forge — Existing Codebase Mode

Modifying existing code. Focus: understand WHAT EXISTS before changing it.

**Proactive-but-user-guided principle:** Forge is proactive in COMMUNICATION — it announces phases, explains what is happening, presents clear options with recommendations. But it NEVER acts autonomously on decisions that affect the user's project. The user is always the guide.
- ALWAYS present options with a recommendation, then WAIT for the user's choice
- ALWAYS ask before: starting a build, merging code, creating PRs, modifying architecture
- ALWAYS surface findings and let the user decide the response
- NEVER skip a user approval gate because "it's obvious"
- NEVER auto-fix evaluator findings without presenting them first
- Proactive = "Here's what I found, here's what I recommend, what do you want to do?"
- NOT proactive = "I found an issue and fixed it for you"

## Checklist

You MUST create a TaskCreate item for each phase and complete them in order:

1. **Explore codebase** — graph queries + symbol resolution
2. **Plan changes** — with blast radius awareness
3. **User approves plan**
4. **Build** — agent team execution
5. **Review** — invoke forge-review
6. **Ship** — invoke forge-ship

---

## Phase 1: Explore

This is NOT a separate agent or 4-phase pipeline. The planner uses graph tools directly.

0. Create STATE.md from the template at `${CLAUDE_PLUGIN_ROOT}/templates/STATE.md`. Set mode to 'existing' and phase to 'explore'.

1. Check graph index: call `mcp__forge_forge-graph__index_status`
   - If status is "not_indexed": call `mcp__forge_forge-graph__index_repository` with current directory. Wait for completion before proceeding.
   - If status is "indexed": proceed (SessionStart hook keeps it current)
   - If status is "stale": call `mcp__forge_forge-graph__index_repository` to refresh, then proceed
   - On failure: warn user "Graph indexing failed. Falling back to Serena symbol tools only." Do NOT skip exploration entirely.

2. Get architecture overview:
   ```
   Call mcp__forge_forge-graph__get_architecture
   ```
   This returns: languages, routes, hotspots, clusters, architecture decision records.
   **What to do with the output:** Extract the top-level module structure, primary language(s), and any architectural patterns. Present a 3-5 line summary to the user:
   "Here's what I see in the codebase: [summary]"
   Include: number of modules, primary framework, key architectural patterns, any ADRs found.

3. Search the specific area being modified:
   ```
   Call mcp__forge_forge-graph__search_graph with the feature area keywords
   ```
   **What to do with the output:** Identify which files, modules, and functions are relevant to the requested change. Note the module boundaries — changes should respect existing module boundaries unless the user explicitly wants to refactor.
   ```
   Call mcp__forge_forge-graph__trace_call_path for relevant entry points
   ```
   **What to do with the output:** Map the call chain from entry point (route handler, CLI command, event listener) to the lowest-level function. This reveals every layer the change will touch and helps estimate blast radius.

4. For symbol-level understanding (when you need to know exactly how something works):
   ```
   Call mcp__plugin_serena_serena__find_symbol with the symbol name
   ```
   **What to do with the output:** Get the symbol's location, type (class/function/method), and signature. Use this to understand the interface contract.
   ```
   Call mcp__plugin_serena_serena__get_symbols_overview for the relevant file
   ```
   **What to do with the output:** See all symbols in the file to understand its structure. Identify which symbols are public API vs internal.
   ```
   Call mcp__plugin_serena_serena__find_referencing_symbols to find all callers
   ```
   **What to do with the output:** Build a dependency map of everything that calls this symbol. These callers are the blast radius — any change to the symbol's contract affects all of them.

   Only read the actual function bodies you need. Do NOT read entire files.

### If Serena is not available

If `mcp__plugin_serena_serena__find_symbol` is not available (Serena plugin not installed):
- Use `Grep` with function/class name patterns to find symbols
- Use `Read` with specific line ranges to examine function bodies
- Use `Glob` to find files by naming conventions
- Warn the user: "Serena is not installed. Exploration will be less precise. Consider installing: /plugin install serena@claude-plugins-official"

5. Check recent changes in the area:
   ```
   Call mcp__forge_forge-graph__detect_changes
   ```
   **What to do with the output:** Identify recently modified files in the affected area. If another developer (or previous session) recently changed these files, flag potential conflicts. Note any patterns in recent changes that should inform the current work.

6. Summarize findings for the user:
   "Based on the codebase analysis:
   - The feature area involves [N] files across [M] modules
   - Key entry points: [list]
   - Dependencies: [list]
   - Blast radius: [assessment — low/medium/high with explanation]
   - Existing patterns to follow: [list]
   - Recent changes that may interact: [list or 'none']"

   Wait for user acknowledgment before proceeding.

---

## Phase 1b: Clarify Requirements

Before planning, ask 2-3 targeted clarifying questions about the feature. The exploration tells us WHAT EXISTS. Now we need to understand WHAT THE USER WANTS.

1. Based on the exploration findings, identify the key decision points for this feature. Ask ONE question at a time, multiple choice preferred, lead with your recommendation.

2. Minimum questions:
   - "What is the expected behavior?" (if not fully clear from the initial request)
   - "Are there constraints I should know about?" (performance targets, compatibility, etc.)
   - "What should happen in error/edge cases?"

3. Skip this phase ONLY if the user's initial request was already specific enough (e.g., "fix the null pointer at line 42 in auth.py"). For feature additions, always clarify.

4. Use `[NEEDS CLARIFICATION]` for anything still ambiguous after asking.

---

## Phase 2: Plan

1. Spawn the forge-planner agent with mode=existing and the exploration findings AND the clarified requirements
2. Planner produces:
   - What to change and why
   - Wave groupings for parallel execution
   - Which files each wave touches (for worktree isolation)
   - Acceptance criteria per wave
3. Present plan to user with explicit approval request:
   "Here's the implementation plan. Please review:
   (a) Approved — let's build
   (b) Changes needed — tell me what to adjust
   (c) More exploration needed — I'll dig deeper into [area]"

<HARD-GATE>
Do NOT begin building until the user approves the plan.
Even for "simple" changes — the plan can be 2-3 sentences, but it must be approved.
Do NOT interpret silence or partial acknowledgment as approval. Wait for explicit "yes" or "approved."
</HARD-GATE>

---

## Phase 3: Build

### Build Phase (shared across modes)

#### Step 1: Prepare

1. Read the approved plan
2. Identify waves and task-per-wave groupings
3. Verify agent teams are enabled

#### Step 2: Execute Waves

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
8. **Advance:** Update STATE.md. Move to next wave.

#### Step 3: Session Guard

- At 90 minutes: "We've been running for 90 minutes. Recommend checkpointing. Run /forge:handoff?"
- At 120 minutes: "Session limit reached. Auto-checkpointing to STATE.md."
  Save state and recommend starting a fresh session.

#### Wave Execution Diagram

```
Wave 1: [Task A, Task B, Task C] — independent, parallel generators
  ↓ all complete + evaluator approved
Wave 2: [Task D, Task E] — depend on Wave 1
  ↓ all complete + evaluator approved
Wave 3: [Task F] — depends on Wave 2
```

#### Team Size Guidelines

- 1-2 tasks: 1 generator + 1 evaluator (skip agent team, use subagents)
- 3-4 tasks: 2-3 generators + 1 evaluator
- 5+ tasks: 3-4 generators + 1 evaluator (max 5 teammates total)

<HARD-GATE>
Do NOT merge any generator output without evaluator review passing.
No "looks fine to me" overrides from the lead. The evaluator must run.
Exception: if the task is trivial (1-2 files, < 50 lines changed), the lead
can review directly instead of spawning an evaluator agent.
</HARD-GATE>

---

## Phase 4: Review

Invoke `forge-review` skill.

The review skill runs a two-stage evaluation:
1. Internal evaluator (forge-evaluator) reviews for code quality, architecture, and security
2. Cross-model adversarial review via Codex for different-perspective analysis

Present all findings to the user. Let the user decide which findings to address.
Do NOT auto-fix findings without presenting them first.

---

## Phase 5: Ship

Invoke `forge-ship` skill.

The ship skill handles:
1. PR creation with structured summary
2. Final gate verification
3. Episodic memory save for future sessions

---

## When to Use Superpowers Instead

For tasks that are purely design/brainstorming (no codebase modification yet):
-> Invoke `superpowers:brainstorming` instead of forge-feature

For tasks that need TDD enforcement:
-> During the Build phase, instruct generators: "Follow TDD. Write the failing test first, verify it fails, then implement, verify it passes. If `superpowers:test-driven-development` is available, follow its Iron Law."

---

## Rationalization Prevention

| If you're thinking... | The answer is... |
|----------------------|-----------------|
| "I already know this codebase, skip exploration" | The GRAPH knows things you don't. Query it. It takes 2 seconds. |
| "This is a small change, no plan needed" | Small changes break production. 2-sentence plan. Get approval. |
| "I'll just read the files directly" | Use graph queries first. 99% fewer tokens. Read files only for the specific symbols you need. |
| "Serena seems slow, I'll use grep" | Grep finds text. Serena finds symbols. Use the right tool. |
| "The blast radius is obvious" | Call `trace_call_path` anyway. Transitive dependencies surprise you. |
| "The evaluator is slowing us down" | The evaluator catches bugs before users do. It stays. |
| "I can review this myself, no need for evaluator" | Self-review has blind spots. The evaluator uses different criteria. No merge without it. |
| "The user seems impatient, skip the approval gate" | Skipping approval leads to rework. A 30-second approval saves hours. |
| "Graph indexing failed, skip exploration" | Fall back to Serena symbol tools. Never skip understanding the codebase. |
