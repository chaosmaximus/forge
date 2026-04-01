---
name: forge-handoff
description: Use when pausing work or before ending a session with in-progress Forge work
---

# Forge Handoff

## Save State

1. Update STATE.md with:
   - Current phase: [explore|classify|discover|prd|design|plan|build|review|ship]
   - Current wave (if in build)
   - Decisions made so far
   - Blockers encountered
   - What was completed, what remains

2. Create/update HANDOFF.md with:
   - What to tell the next session to get it up to speed (2-3 paragraphs max)
   - Active branch names
   - Any critical context that STATE.md doesn't capture

3. Stage only tracked files and new source files: `git add -u && git add '*.py' '*.ts' '*.js' '*.go' '*.rs' '*.tf' '*.md' 2>/dev/null; git commit -m 'wip: forge handoff checkpoint'`. Do NOT use `git add -A` which may stage sensitive files.

4. Shut down agent team gracefully:
   - Ask each teammate to shut down (one by one)
   - Wait for confirmations
   - Then clean up the team via the lead
   - Do NOT use killall. Kill only specific forge-related processes.

5. Announce: "Session checkpointed. To resume: open this directory and run /forge — it will read STATE.md and continue."
