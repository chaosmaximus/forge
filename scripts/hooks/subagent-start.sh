#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.7.0 — SubagentStart hook
# Injects context into subagents: anti-patterns, lessons, and key decisions.
# Subagents need context since they don't inherit the parent session's memory.
# CONTEXT BUDGET: 500 chars. Always outputs something.

set -euo pipefail
cat 2>/dev/null || true  # consume stdin

# Find forge-next binary
FORGE_NEXT="${FORGE_NEXT:-forge-next}"
if ! command -v "$FORGE_NEXT" &>/dev/null; then
  for candidate in "$HOME/.local/bin/forge-next" "/usr/local/bin/forge-next"; do
    [ -x "$candidate" ] && FORGE_NEXT="$candidate" && break
  done
fi
command -v "$FORGE_NEXT" &>/dev/null || exit 0

# Detect project from CWD
PROJECT=""
if [ -n "${CLAUDE_CWD:-}" ]; then
  PROJECT=$(basename "$CLAUDE_CWD")
elif [ -n "${PWD:-}" ]; then
  PROJECT=$(basename "$PWD")
fi

# Recall anti-patterns and lessons for subagent context
LESSONS=$(timeout 3 "$FORGE_NEXT" recall "anti-pattern" \
  --type lesson \
  --limit 3 \
  ${PROJECT:+--project "$PROJECT"} 2>/dev/null || echo "")

# Also recall key decisions
DECISIONS=$(timeout 3 "$FORGE_NEXT" recall "architecture decision" \
  --type decision \
  --limit 2 \
  ${PROJECT:+--project "$PROJECT"} 2>/dev/null || echo "")

# Build context within 500 char budget
BUDGET=500
XML=""
REMAINING=$BUDGET

# Helper: append to XML if budget allows
append_if_budget() {
  local item="$1"
  local len=${#item}
  if [ "$len" -le "$REMAINING" ]; then
    XML="${XML}${item}"
    REMAINING=$((REMAINING - len))
    return 0
  fi
  return 1
}

# Add lessons first (higher priority for subagents)
while IFS= read -r line; do
  [ -z "$line" ] && continue
  append_if_budget "<lesson>${line}</lesson>" || break
done <<< "$LESSONS"

# Add decisions
while IFS= read -r line; do
  [ -z "$line" ] && continue
  append_if_budget "<decision>${line}</decision>" || break
done <<< "$DECISIONS"

# Always output something — fallback if daemon returned nothing
if [ -z "$XML" ]; then
  XML="<note>No lessons or decisions found. Follow project conventions.</note>"
fi

XML_SAFE=$(echo "$XML" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
FULL="<forge-subagent-context>${XML_SAFE}</forge-subagent-context>"
ESCAPED=$(echo "$FULL" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ' | sed 's/[[:space:]]*$//')
echo "{\"hookSpecificOutput\":{\"additionalContext\":\"${ESCAPED}\"}}"
