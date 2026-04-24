#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.6.0 — PreToolUse hook for Bash
# Warns about destructive commands, surfaces relevant skills/lessons.
# CONTEXT BUDGET: 200 chars max. Silent if safe.

set -uo pipefail
# NOTE: -e removed intentionally — hook must NEVER fail and block the user.
# All errors are handled explicitly. Exit 0 always.
INPUT=$(cat) || true

FORGE_NEXT="${FORGE_NEXT:-forge-next}"
if ! command -v "$FORGE_NEXT" &>/dev/null; then
  for candidate in "$HOME/.local/bin/forge-next" "/usr/local/bin/forge-next"; do
    [ -x "$candidate" ] && FORGE_NEXT="$candidate" && break
  done
fi
command -v "$FORGE_NEXT" &>/dev/null || exit 0

# Extract command from tool input
CMD=$(echo "$INPUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    ti = d.get('tool_input', d.get('toolInput', {}))
    print(ti.get('command', ''))
except: print('')
" 2>/dev/null) || true

[ -z "$CMD" ] && exit 0

RESULT=$("$FORGE_NEXT" pre-bash-check --command "$CMD" 2>/dev/null) || true
[ -z "$RESULT" ] && exit 0

# Only output if there are warnings (silent by default -- context budget rule)
if echo "$RESULT" | grep -q "Destructive\|Lesson\|Skill"; then
    CONTEXT=$(echo "$RESULT" | head -3 | tr '\n' ' ' | cut -c1-200)
    ESCAPED=$(echo "$CONTEXT" | sed 's/\\/\\\\/g; s/"/\\"/g')
    echo "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"additionalContext\":\"<forge-bash-check>${ESCAPED}</forge-bash-check>\"}}"
fi
