---
name: forge-review
description: "Use after forge-build completes all waves, before merging or shipping any code. Example triggers: 'review the code', 'check my changes', 'run code review', 'evaluate this changeset', 'is this ready to merge'. Runs evaluator + optional Codex adversarial review."
---

# Forge Review

## Step 1: Run Evaluator

If not already done per-wave, spawn forge-evaluator on the full changeset.
The evaluator performs two-stage review (spec compliance, then code quality).

## Step 2: Present Findings

Show the evaluator's structured review to the user:
- Spec compliance checklist
- Code quality scores
- Critical findings (must fix)
- Suggestions (optional)

## Step 3: Codex Gate

Determine gate level:
1. Check changed files against `prod_paths` (from CLAUDE_PLUGIN_OPTION_PROD_PATHS):
   - Match → HARD GATE. Run Codex. Must pass.
   - No match but shared modules → AUTO-REVIEW. Run Codex. Surface findings.
   - No match → ON-DEMAND. Ask user: "Want a Codex adversarial review? Recommended for [reason]."

When spawning the evaluator, include the custom prod_paths in the spawn prompt if they differ from defaults. Example: "Review this changeset. Custom production paths: [paths from userConfig]." The evaluator cannot read environment variables directly.

2. Before running Codex for the FIRST TIME in a session, inform the user:
   "Codex adversarial review will send code diffs to OpenAI's API for analysis. Proceed?"
   Wait for confirmation. If the user declines, skip Codex and proceed with evaluator review only.

   To run Codex:
   Run `/codex:adversarial-review`. If the Codex plugin supports background mode, add `--background`. Specify the base branch for comparison. If the exact syntax differs from what's documented here, follow the Codex plugin's own help output (`/codex` to see available commands).
   Focus areas: auth, data loss, rollback safety, race conditions, idempotency

3. Wait for result. Present findings with options:
   ```
   Codex adversarial review complete:
   Verdict: [approve/needs-attention]

   [If needs-attention:]
   CRITICAL: [finding with file:line]
   HIGH: [finding with file:line]

   Options:
   1. Fix critical issues and re-review (recommended)
   2. Fix all issues and re-review
   3. Override and proceed (NOT for prod paths)
   4. Revise the plan
   ```

## Step 4: Loop Until Pass

- If fixes needed: send findings to generators, re-review
- If Codex blocks: fix and re-run Codex
- If user overrides (non-prod only): log the override in STATE.md

## Transition to Ship

When all reviews pass (evaluator + Codex if applicable), announce: "Review complete. All gates passed." Then return control to the lead skill (forge-new or forge-feature) which will invoke forge-ship as the next phase.

## Iron Law
```
NO MERGE WITHOUT EVALUATOR SIGN-OFF.
NO MERGE TO PROD PATHS WITHOUT CODEX SIGN-OFF.
"It passed locally" is not sufficient. Run the full gate.
```

## Council Review Mode

For critical changes, use multi-reviewer council:

1. **Generate context:** `forge review . --base <merge-base>`
2. **Dispatch reviewers in parallel:**
   - Forge Evaluator agent (spec compliance + code quality)
   - Codex adversarial: `codex exec --model gpt-5.2 "<review prompt>"`
3. **Synthesize:** Compare findings, deduplicate, rank by severity
4. **Report:** P0/P1/P2/P3 findings with file:line references

### When to use council vs standard

- **Standard** (single reviewer): Non-critical changes, refactors, docs
- **Council** (multi-reviewer): Production code, security, new features, auth/payments/infrastructure

## When Codex Plugin Is Not Installed

- For prod paths: BLOCK the workflow. Tell the user: "Codex plugin is required for production path changes. Install it with `/plugin marketplace add openai/codex-plugin-cc` and restart the session. Cannot proceed without adversarial review on production code."
- For non-prod: WARN but proceed with evaluator review only.
- NEVER silently skip the Codex gate for prod paths
