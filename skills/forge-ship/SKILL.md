---
name: forge-ship
description: Use when forge-review has passed and work is ready to integrate or create a PR
---

# Forge Ship

## Step 1: Final Verification

Invoke `superpowers:verification-before-completion` if available. Otherwise:
1. Run the full test suite one final time. Capture output.
2. Check for uncommitted changes.
3. Check for any NEEDS_CLARIFICATION markers in the PRD or plan that were never resolved.

If verification fails: DO NOT proceed. Report the failure and return to forge-review.

## Step 2: Integration Options

Present exactly these options:
1. Push and create Pull Request (recommended)
2. Merge locally to base branch
3. Keep branch as-is (user handles later)
4. Discard this work

If option 1: generate PR with:
- Title from the plan/PRD (under 70 chars)
- Body with: Summary (what was built), Test plan, Forge review scores, Codex review verdict

## Step 3: Cleanup

1. Clean up agent team (shut down teammates, then lead cleanup)
2. Update STATE.md with completion status

## Step 4: Save to Memory

1. Write a session summary to project MEMORY.md (if it exists):
   ```
   ## [date] — Forge: [feature name]
   - Mode: [greenfield/existing]
   - Built: [1-2 sentence summary]
   - Key decisions: [list]
   - Codex findings addressed: [list]
   ```

2. If `episodic-memory` plugin is available:
   The SessionEnd hook (`session-end-memory.sh`) will auto-trigger indexing.
   The next session can search for this work via `episodic-memory:search-conversations`.

## Step 5: Announce Completion

"Forge complete. [Feature name] shipped to [branch/PR].
Review scores: [X/5 average]. Codex verdict: [approve/N/A].
To resume Forge on a new task, run /forge."
