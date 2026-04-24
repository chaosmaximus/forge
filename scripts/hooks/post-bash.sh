#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.6.0 — PostToolUse hook for Bash
# On command failure, surfaces relevant lessons and skills.
# CONTEXT BUDGET: 200 chars max. Silent on success.

set -euo pipefail
INPUT=$(cat)

FORGE_NEXT="${FORGE_NEXT:-forge-next}"
if ! command -v "$FORGE_NEXT" &>/dev/null; then
  for candidate in "$HOME/.local/bin/forge-next" "/usr/local/bin/forge-next"; do
    [ -x "$candidate" ] && FORGE_NEXT="$candidate" && break
  done
fi
command -v "$FORGE_NEXT" &>/dev/null || exit 0

# Extract command and check if it failed
CMD=$(echo "$INPUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    ti = d.get('tool_input', d.get('toolInput', {}))
    print(ti.get('command', ''))
except: print('')
" 2>/dev/null)

EXIT_CODE=$(echo "$INPUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    r = d.get('tool_result', d.get('toolResult', {}))
    # Check for error indicators
    if r.get('is_error') or r.get('isError'):
        print('1')
    else:
        print('0')
except: print('0')
" 2>/dev/null)

[ -z "$CMD" ] && exit 0
[ "$EXIT_CODE" = "0" ] && exit 0  # Silent on success

RESULT=$("$FORGE_NEXT" post-bash-check --command "$CMD" --exit-code "$EXIT_CODE" 2>/dev/null || echo "")
[ -z "$RESULT" ] && exit 0

# Output suggestions (only on failure)
if echo "$RESULT" | grep -q "Lesson\|Skill\|suggestion"; then
    CONTEXT=$(echo "$RESULT" | head -3 | tr '\n' ' ' | cut -c1-200)
    ESCAPED=$(echo "$CONTEXT" | sed 's/\\/\\\\/g; s/"/\\"/g')
    echo "{\"hookSpecificOutput\":{\"hookEventName\":\"PostToolUse\",\"additionalContext\":\"<forge-bash-failure>${ESCAPED}</forge-bash-failure>\"}}"
fi
