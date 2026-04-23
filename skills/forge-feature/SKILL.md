---
name: forge-feature
description: "Use when adding features, fixing bugs, refactoring, or modifying code in an existing codebase. Explores the codebase using Forge memory (recall, blast-radius), plans changes with user approval, builds with tests, and reviews. Example triggers: 'add a login page', 'fix the auth bug', 'refactor the API layer', 'update the database schema', 'implement the new feature from the spec'."
---

# Forge — Existing Codebase Mode

Modify existing code. Understand what exists before changing it.

**User-guided principle:** Be proactive in communication — announce what you're doing, present options with recommendations. But never act autonomously on decisions that affect the user's project. The user decides.

## Workflow

Five phases, in order. Each builds on the previous.

1. **Explore** — Understand the codebase via memory + blast radius
2. **Clarify** — Ask targeted questions if requirements are ambiguous
3. **Plan** — Design the implementation, get user approval
4. **Build** — Implement with tests
5. **Review** — Verify quality

---

## Phase 1: Explore

The goal is to understand what exists before planning changes. Forge's daemon is your primary exploration tool — it stores decisions, patterns, and relationships from prior sessions.

**Step 1: Recall relevant context**

```bash
forge-next recall "<keywords from user's feature request>" --type decision --limit 5
forge-next recall "<related area keywords>" --limit 5
```

This returns stored architectural decisions, lessons, and patterns relevant to the change. Read the output carefully — it often contains critical context like "we chose X because Y" or "don't do Z, it caused problems."

**Step 2: Check blast radius** (ISSUE-19: conditional — skip if not indexed)

First check if the project is indexed:
```bash
forge-next code-search "main" 2>&1 | head -3
```
If you see "No symbols found" or an empty result, the project hasn't been indexed yet. In that case:
- Skip blast-radius (it will return empty/zeros for unindexed projects)
- Note "code graph not available — using grep/read instead" in your findings
- Use `grep -r` and `git log` for impact analysis as a fallback
- Suggest `forge-next force-index --path <project-dir>` to the user

If the project IS indexed, check blast radius for every key file:
```bash
forge-next blast-radius --file <path/to/file>
```

This tells you which other files, decisions, and clusters are affected. Use it to:
- Identify test files that need updating
- Spot modules that depend on what you're changing
- Understand the ripple effect before you start

**Step 3: Check recent changes**

```bash
git log --oneline -10 -- <affected-area/>
```

Flag any recent changes in the same area — potential merge conflicts or duplicate work.

**Step 4: Understand specific symbols**

When you need to understand exactly how a function or struct works:
- Use Serena tools (`find_symbol`, `get_symbols_overview`, `find_referencing_symbols`) if available
- Otherwise use Grep + Read on specific line ranges
- Only read the function bodies you actually need — don't read entire files

**Step 5: Present findings**

Summarize for the user (keep it short):
- How many files/modules are affected
- Key entry points and dependencies
- Blast radius assessment (low/medium/high)
- Existing patterns to follow
- Recent changes that may interact

Wait for user acknowledgment before proceeding.

---

## Phase 2: Clarify

Skip this if the user's request is specific enough (e.g., "fix the null check at line 42").

For feature additions, ask 2-3 targeted questions:
- What's the expected behavior? (if not fully clear)
- Any constraints? (performance, compatibility, API shape)
- What should happen in edge/error cases?

Ask one question at a time, multiple choice preferred, lead with your recommendation.

---

## Phase 3: Plan

Design the implementation. For large features, group work into waves — each wave independently testable.

**For each wave, specify:**
- What to change and why
- Which files to modify/create
- Acceptance criteria (specific test assertions)
- Dependencies on prior waves

**Present to the user:**
> "Here's the implementation plan. Please review:
> (a) Approved — let's build
> (b) Changes needed — tell me what to adjust
> (c) More exploration needed"

Do NOT build until the user explicitly approves. Even simple plans need a "yes."

For complex plans, use the forge-planner agent to produce the wave breakdown. For simple changes (1-3 files), plan inline — don't over-engineer the planning step.

---

## Phase 4: Build

Execute the approved plan wave by wave.

**Per wave:**
1. Create tasks via TaskCreate for tracking
2. Implement the changes (following existing code patterns)
3. Write tests — aim for the acceptance criteria in the plan
4. Run the project's test suite (check `<project-conventions>` in context for the command)
5. Run lint (check conventions for lint_command)

**Pre-flight for Wave 2+:**
Before starting each subsequent wave, verify prior wave changes are on the branch:
```bash
git log --oneline -3
```
Check that canary files/symbols from prior waves exist.

**Agent dispatch (for large waves):**
- Spawn forge-generator agents for parallel tasks within a wave
- Default: NO worktree isolation (worktrees branch from stale commits — this has caused repeated failures)
- Each generator gets: task description, acceptance criteria, relevant decisions from recall, and files it should touch
- Include project conventions from context (test command, lint command, test patterns)

**Cross-cutting mode (for protocol/schema changes):**
When a task spans the protocol boundary (e.g., adding a field to a response type that requires updating handler constructors, CLI matchers, and contract tests), include `<cross-cutting>true</cross-cutting>` in the generator's spawn context. This:
- Relaxes the scope guard so the generator can follow the dependency chain
- Requires the generator to grep for all callers and update them
- Must be explicitly opted into by the lead — never auto-enabled
- Use when: adding/modifying protocol types, changing function signatures used across crates, schema migrations that affect multiple layers

**Handling generator status (MANDATORY):**

The generator reports one of four statuses. Handle each:

| Status | Action |
|--------|--------|
| **DONE** | Proceed to evaluator review |
| **DONE_WITH_CONCERNS** | Read concerns. If correctness/scope issue → address before review. If observation → note and proceed to review |
| **NEEDS_CONTEXT** | Provide the missing context and re-dispatch the generator |
| **BLOCKED** | Assess: (1) context problem → provide + re-dispatch, (2) needs more reasoning → re-dispatch with more capable model, (3) task too large → break into pieces, (4) plan wrong → escalate to user |

**Never** ignore a BLOCKED or NEEDS_CONTEXT status. Never force-accept DONE_WITH_CONCERNS without reading the concerns.

**After each wave — MANDATORY two-stage review:**

> **This is not optional.** Every generator output gets evaluated. Self-review has blind spots.

1. **Spec compliance review (MANDATORY):** Does the output match the plan's acceptance criteria?
   - Dispatch forge-evaluator agent with the plan's acceptance criteria
   - Evaluator checks every criterion — missing or extra items are flagged
   - If evaluator finds gaps → generator fixes before proceeding

2. **Code quality review (MANDATORY):** Is the code well-written?
   - Evaluator runs the project's test suite independently (from conventions, not hardcoded)
   - Evaluator verifies test output contains pass/fail counts
   - Evaluator scores against rubrics (code-quality.md, security.md if applicable)
   - For 5+ file changes: dispatch Codex adversarial review
   - Fix all HIGH findings, address MEDIUM findings
   - Store the review outcome: `forge-next remember --type lesson`

3. **Production verification (for daemon/service changes):**
   - Rebuild, sync, restart
   - UAT the feature against the running system
   - Check health: `forge-next doctor` and `forge-next manas-health`

Only proceed to next wave after all checks pass.

**After the FINAL wave — cross-wave integration smoke test:**

This is the most critical verification. Individual waves may pass their own tests but fail when assembled together (namespace collisions, missing dependencies, import chain breaks).

1. Check if the plan specifies a cross-wave integration test command. If so, run it.
2. If no explicit command, auto-detect:
   - Python: `python -c "from <main_module> import app"` or `python -m <package>`
   - Rust: `cargo build --release` (full project builds)
   - Go: `go build ./...`
   - Node: `npm run build` or `tsc --noEmit`
3. If the smoke test fails, this is a BLOCKING issue — do not proceed to review.

---

## Phase 5: Review

After building all waves, final review:

1. Run the full test suite (from project conventions)
2. Run lint with zero warnings required
3. **ALWAYS** dispatch a forge-evaluator for the complete change set
4. For critical changes, dispatch adversarial review via Codex if available

Present all findings to the user. Let them decide which to address.

---

## Storing Decisions

Throughout the workflow, store important decisions in Forge memory:

```bash
forge-next remember --type decision --title "..." --content "..."
```

This is how context persists across sessions. Future `forge-next recall` calls will surface these decisions when relevant. Good candidates: architectural choices, trade-offs made, patterns established, gotchas discovered.

---

## Anti-Patterns

| Don't do this | Do this instead |
|---------------|-----------------|
| Skip recall because you "know the codebase" | Run `forge-next recall` — 2 seconds, surfaces prior decisions |
| Read entire files to understand them | Use blast-radius + Serena/Grep for targeted symbol lookup |
| Build without user approval | Present plan, wait for explicit "yes" |
| Self-review and ship | Run the evaluator — self-review has blind spots |
| Forget to store decisions | `forge-next remember` after every architectural choice |
| Use worktree isolation by default | Run generators on current branch — worktrees cause stale commit issues |
