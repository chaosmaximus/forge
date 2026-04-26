---
name: forge-think
description: "Product discovery and specification — BDD-style requirements gathering, PRD generation, feature spec creation. Use when starting a new feature, project, or when requirements are unclear. Invoked by forge:forge during the Think phase. Example triggers: 'what should we build', 'help me define requirements', 'write a PRD', 'scope this feature', 'I have an idea but need to flesh it out'."
---

# Forge Think — Product Discovery

Guided product discovery that produces actionable specifications. Uses BDD (Behavior-Driven Development) patterns to create acceptance criteria that become tests.

## When to Use

- User has an idea but not clear requirements
- Starting a new feature that needs scoping
- Requirements are ambiguous and need structured exploration
- User says "I want to build..." or "what should we build?"

## The Process

### Phase 1: Understand Intent (1-3 questions)

Ask ONE question at a time. Multiple choice preferred. Lead with your recommendation.

1. **What's the core problem?** Not "what feature" — what problem does the user face?
2. **Who uses it?** Developer? End user? API consumer?
3. **What does success look like?** How will we know it works?

If the user already described the feature clearly, skip to Phase 2.

### Phase 2: Explore the Codebase (if existing)

Before specifying, understand what exists:

```bash
# Check memory for relevant decisions
forge-next recall "relevant keywords"

# Check code structure
forge query "MATCH (f:File) RETURN f.name LIMIT 20"

# Recent changes
git log --oneline -10
```

Present findings: "Based on the codebase, I see [summary]. The feature would touch [areas]."

### Phase 3: Write Feature Specs (BDD Style)

For each feature/behavior, create a spec using Given/When/Then:

```markdown
## Feature: [Name]

### Acceptance Criteria

**Scenario 1: [Happy path]**
- Given [precondition]
- When [action]
- Then [expected result]

**Scenario 2: [Error case]**
- Given [precondition]
- When [invalid action]
- Then [error handling]

### Contracts
- Input: {field: type, field: type}
- Output: {field: type}

### Invariants
- [Rule that must always hold]
- [Security constraint]
- [Performance requirement]
```

### Phase 4: Generate PRD (if greenfield)

For new projects, generate a PRD using the template at `${CLAUDE_PLUGIN_ROOT}/templates/PRD.md`. Include:

1. Problem statement (from Phase 1)
2. User persona (from Phase 1)
3. Feature specs (from Phase 3)
4. Success metrics
5. Non-functional requirements (security, performance, scalability)

### Phase 5: Store Decisions

After user approves the specs:

```bash
forge-next remember --type decision --title "[Feature name] — requirements" \
  --content "[Summary of agreed requirements and key decisions]"
```

### Phase 6: Transition to Plan

"Requirements are captured. Ready to plan the implementation?"
→ Invoke `forge:forge-feature` (existing codebase) or `forge:forge-new` (greenfield)

The feature specs become the acceptance criteria for the planner and the test conditions for the evaluator.

## Self-Review Checklist

After writing specs, review them yourself before presenting to the user. This catches gaps early.

### 1. Coverage check
For each user requirement mentioned in the conversation:
- Can you point to a scenario that covers it?
- If not, add the missing scenario.

### 2. Placeholder scan
Search your specs for these red flags:
- "TBD", "TODO", "to be determined"
- "appropriate error handling" (specify WHAT handling)
- "etc." (list ALL items, not "etc.")
- Vague inputs: "data", "payload", "info" (specify the actual fields)

### 3. Edge case review
For each scenario, ask:
- What happens with empty input?
- What happens with very large input?
- What happens when the dependency is unavailable?
- What happens with concurrent requests?
- What happens with invalid/malicious input?

If any of these are relevant but not covered, add a scenario.

### 4. Type consistency
Check that all type names, field names, and method names used across scenarios are consistent. "UserProfile" in one scenario and "user_profile" in another is a bug in the spec.

### 5. Testability check
For each acceptance criterion:
- Can this be verified with an automated test?
- If not, can it be verified with a manual UAT step?
- If neither, the criterion is too vague — rewrite it.

## Key Principles

- **Specs are tests.** Every Given/When/Then becomes a test case during Build phase.
- **Invariants are security gates.** The evaluator checks invariants during Review phase.
- **Contracts are type definitions.** Generators use contracts to define interfaces.
- **Ask, don't assume.** If the user hasn't specified error handling, ASK — don't invent.
- **Blast-radius awareness.** If the feature touches existing code, check `forge-next blast-radius --file <path> --project <project-name>` for each file and include affected modules in the spec's "Impact" section. Always pass `--project` — without it the daemon mixes call graphs from every indexed project on the same host, polluting impact analysis.

## What Forge Think Does NOT Do

- Does not write code (that's forge build)
- Does not plan implementation (that's forge plan)
- Does not design UI (suggest the user use a design tool)
- Does not make architectural decisions without user approval
