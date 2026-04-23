#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.7.0 — TaskCompleted hook
# Verifies task completion criteria: checks if there are outstanding warnings
# or forgotten steps related to the completed task.
# CONTEXT BUDGET: 300 chars. Silent if no warnings.

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

# Extract task subject from stdin JSON
SUBJECT=$(echo "$INPUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    # Try common field names for task subject
    subj = d.get('subject', '')
    if not subj:
        subj = d.get('task', '')
    if not subj:
        subj = d.get('taskSubject', '')
    if not subj:
        subj = d.get('task_subject', '')
    print(subj[:500] if subj else '')
except:
    print('')
" 2>/dev/null)

# Session ID
SESSION_ID="${CLAUDE_SESSION_ID:-}"
if [ -z "$SESSION_ID" ]; then
  FORGE_SESSION_DIR="${XDG_RUNTIME_DIR:-$HOME/.forge/sessions}"
  CWD_HASH=$(echo "${CLAUDE_CWD:-$(pwd)}" | md5sum | cut -d' ' -f1)
  SESSION_FILE="$FORGE_SESSION_DIR/forge-session-${CWD_HASH}"
  if [ -f "$SESSION_FILE" ] && [ ! -L "$SESSION_FILE" ]; then
    SESSION_ID=$(cat "$SESSION_FILE" 2>/dev/null || true)
  fi
fi
[ -z "$SESSION_ID" ] && exit 0

# Security: reject subject with shell metacharacters
if [ -n "$SUBJECT" ]; then
  [[ "$SUBJECT" =~ [';|&$`\\'] ]] && SUBJECT=""
fi

# Call daemon for task completion check
RESULT=$(timeout 3 "$FORGE_NEXT" task-completion-check \
  --session-id "$SESSION_ID" \
  ${SUBJECT:+--subject "$SUBJECT"} 2>/dev/null || echo "")

# Silent if no warnings
[ -z "$RESULT" ] && exit 0

# Trim to 300 char budget
CONTEXT=$(echo "$RESULT" | head -5 | tr '\n' ' ' | cut -c1-300)
[ -z "$CONTEXT" ] && exit 0

# Escape XML entities first, then JSON
XML_SAFE=$(echo "$CONTEXT" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
ESCAPED=$(echo "$XML_SAFE" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ' | sed 's/[[:space:]]*$//')
# TaskCompleted uses top-level systemMessage (not hookSpecificOutput)
echo "{\"systemMessage\":\"<forge-task-check>${ESCAPED}</forge-task-check>\"}"
