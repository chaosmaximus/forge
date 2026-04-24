#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.7.0 — PostToolUse hook for Edit|Write
# Runs after file edits to surface relevant context, diagnostics, and warnings.
# Reads stdin (tool result JSON), extracts file path, calls forge-next post-edit-check.
# Output respects a 500 character budget for XML context injection.

set -euo pipefail
INPUT=$(cat)

# Find forge-next binary
FORGE_NEXT="${FORGE_NEXT:-forge-next}"
if ! command -v "$FORGE_NEXT" &>/dev/null; then
  for candidate in "$HOME/.local/bin/forge-next" "/usr/local/bin/forge-next"; do
    [ -x "$candidate" ] && FORGE_NEXT="$candidate" && break
  done
fi
command -v "$FORGE_NEXT" &>/dev/null || exit 0

# Extract tool name, file path, and raw tool_input (multiple field names).
# Tab-separated so the tool_args JSON (which contains spaces) parses cleanly.
IFS=$'\t' read -r TOOL_NAME FILE TOOL_ARGS < <(echo "$INPUT" | python3 -c "
import sys, json
try:
    data = json.load(sys.stdin)
    tn = data.get('tool_name', data.get('toolName', 'Edit'))
    ti = data.get('tool_input', data.get('toolInput', {}))
    fp = ti.get('file_path', ti.get('filePath', ti.get('path', '')))
    print(f'{tn}\t{fp}\t{json.dumps(ti)}')
except:
    print('Edit\t\t{}')
" 2>/dev/null)

[ -z "$FILE" ] && exit 0

# Security: reject paths with shell metacharacters
[[ "$FILE" =~ [';|&$`\\'] ]] && exit 0

# Phase 23 data feed: record every edit so session_tool_call has data
# for the consolidator. Silent failure if the daemon is down (2P-1b §10.5).
SESSION_ID="${CLAUDE_SESSION_ID:-}"
if [ -z "$SESSION_ID" ]; then
    FORGE_SESSION_DIR="${XDG_RUNTIME_DIR:-$HOME/.forge/sessions}"
    CWD_HASH=$(echo "${CLAUDE_CWD:-$(pwd)}" | md5sum | cut -d' ' -f1)
    SESSION_FILE="$FORGE_SESSION_DIR/forge-session-${CWD_HASH}"
    if [ -f "$SESSION_FILE" ] && [ ! -L "$SESSION_FILE" ]; then
        SESSION_ID=$(cat "$SESSION_FILE" 2>/dev/null || true)
    fi
fi
if [ -n "$SESSION_ID" ]; then
    timeout 3 "$FORGE_NEXT" record-tool-use \
        --session-id "$SESSION_ID" \
        --agent claude-code \
        --tool-name "$TOOL_NAME" \
        --tool-args "$TOOL_ARGS" \
        --success true 2>/dev/null || true
fi

# Run post-edit check (non-blocking — 5s timeout in hooks.json)
RESULT=$("$FORGE_NEXT" post-edit-check --file "$FILE" 2>/dev/null || echo "")
[ -z "$RESULT" ] && exit 0

# Build XML output within 500 character budget.
# Priority order: diagnostic errors, dangerous patterns, diagnostic warnings,
# lessons, skills (omit if over budget).
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

# 1. Diagnostic errors (from cached diagnostics line format: [source:severity] message)
while IFS= read -r line; do
  # Extract source and message from the line format
  if [[ "$line" =~ ^\[([^:]+):error\]\ (.+)$ ]]; then
    src="${BASH_REMATCH[1]}"
    msg="${BASH_REMATCH[2]}"
    append_if_budget "<error source=\"${src}\">${msg}</error>" || break
  fi
done <<< "$(echo "$RESULT" | grep '^\[.*:error\]' 2>/dev/null || true)"

# 2. Dangerous patterns
while IFS= read -r line; do
  [ -z "$line" ] && continue
  pattern="${line#Dangerous: }"
  append_if_budget "<warning source=\"forge-memory\">${pattern}</warning>" || break
done <<< "$(echo "$RESULT" | grep '^Dangerous:' 2>/dev/null || true)"

# 3. Diagnostic warnings (from cached diagnostics)
while IFS= read -r line; do
  if [[ "$line" =~ ^\[([^:]+):warning\]\ (.+)$ ]]; then
    src="${BASH_REMATCH[1]}"
    msg="${BASH_REMATCH[2]}"
    append_if_budget "<warning source=\"${src}\">${msg}</warning>" || break
  fi
done <<< "$(echo "$RESULT" | grep '^\[.*:warning\]' 2>/dev/null || true)"

# 4. Callers (blast radius)
CALLERS_LINE=$(echo "$RESULT" | grep "^callers:" | head -1 || true)
if [ -n "$CALLERS_LINE" ]; then
  # Extract count from "callers: N file(s)..."
  COUNT=$(echo "$CALLERS_LINE" | grep -oP '\d+' | head -1 || echo "0")
  if [ "$COUNT" -gt 0 ]; then
    SEV="LOW"
    [ "$COUNT" -gt 2 ] && SEV="MEDIUM"
    [ "$COUNT" -gt 5 ] && SEV="HIGH"
    append_if_budget "<callers count=\"${COUNT}\">${SEV} blast radius</callers>" || true
  fi
fi

# 5. Lessons
while IFS= read -r line; do
  [ -z "$line" ] && continue
  lesson="${line#Lesson: }"
  append_if_budget "<lesson>${lesson}</lesson>" || break
done <<< "$(echo "$RESULT" | grep '^Lesson:' 2>/dev/null || true)"

# 6. Skills (omit if over budget)
while IFS= read -r line; do
  [ -z "$line" ] && continue
  skill="${line#Skill: }"
  append_if_budget "<skill>${skill}</skill>" || break
done <<< "$(echo "$RESULT" | grep '^Skill:' 2>/dev/null || true)"

# 7. Decisions to review
while IFS= read -r line; do
  [ -z "$line" ] && continue
  decision="${line#Decision to review: }"
  append_if_budget "<decision>${decision}</decision>" || break
done <<< "$(echo "$RESULT" | grep '^Decision to review:' 2>/dev/null || true)"

[ -z "$XML" ] && exit 0

# Wrap in diagnostics tag and output as JSON
FULL="<forge-post-edit><diagnostics>${XML}</diagnostics></forge-post-edit>"
# Escape for JSON
ESCAPED=$(echo "$FULL" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ' | sed 's/[[:space:]]*$//')
echo "{\"hookSpecificOutput\":{\"hookEventName\":\"PostToolUse\",\"additionalContext\":\"${ESCAPED}\"}}"
