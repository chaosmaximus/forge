<!-- SPDX-License-Identifier: Apache-2.0 -->
---
name: forge-handoff
description: "Use when pausing work or before ending a session with in-progress Forge work. Example triggers: 'save progress', 'I need to stop', 'checkpoint this', 'handoff', 'pause the build', 'end session'. Saves STATE.md, HANDOFF.md, commits checkpoint, shuts down agents."
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

3. Stage source files and forge state files separately:
   ```
   git add -u                    # tracked files with changes
   git add '*.py' '*.ts' '*.js' '*.jsx' '*.tsx' '*.go' '*.rs' '*.tf' '*.java' '*.kt' '*.c' '*.cpp' '*.h' '*.rb' '*.swift' '*.dart' '*.css' '*.scss' '*.html' 2>/dev/null  # new source files
   git add STATE.md HANDOFF.md 2>/dev/null  # forge state (needed for resume)
   git commit -m 'wip: forge handoff checkpoint'
   ```
   Do NOT use `git add -A` which may stage sensitive files.
   Note: STATE.md and HANDOFF.md contain project context. If your repo is public, add them to `.gitignore` and track state externally.

4. Shut down agent team gracefully:
   - Ask each teammate to shut down (one by one)
   - Wait for confirmations
   - Then clean up the team via the lead
   - Do NOT use killall. Kill only specific forge-related processes.

5. Announce: "Session checkpointed. To resume: open this directory and run /forge — it will read STATE.md and continue."
