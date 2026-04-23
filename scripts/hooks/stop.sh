#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.7.0 — Stop hook
# Completion verification: detects completion signals in the assistant's last message,
# then asks the daemon if there are any lessons or reminders to surface.
# CONTEXT BUDGET: 200 chars max. Silent if no completion signal detected.
# NEVER blocks — always exit 0.

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

# Extract last assistant message from stdin JSON
LAST_MSG=$(echo "$INPUT" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    # Try common field names for the assistant message
    msg = d.get('stop_reason', '')
    if not msg:
        msg = d.get('last_assistant_message', '')
    if not msg:
        msg = d.get('assistantMessage', '')
    if not msg:
        msg = d.get('message', '')
    print(msg[:2000] if msg else '')
except:
    print('')
" 2>/dev/null)

# Fast path: local keyword scan for completion signals
# Only call daemon if completion keywords detected
COMPLETION_KEYWORDS="done|complete|completed|shipped|finished|deployed|merged|DONE|COMPLETE|SHIPPED|ready for review|all tests pass|task complete|implementation complete"
if ! echo "$LAST_MSG" | grep -qiE "$COMPLETION_KEYWORDS"; then
  exit 0
fi

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

# Call daemon for completion check
RESULT=$(timeout 3 "$FORGE_NEXT" completion-check \
  --session-id "$SESSION_ID" \
  --claimed-done 2>/dev/null || echo "")

# Silent if no lessons found
[ -z "$RESULT" ] && exit 0

# Trim to 200 char budget
CONTEXT=$(echo "$RESULT" | head -3 | tr '\n' ' ' | cut -c1-200)
[ -z "$CONTEXT" ] && exit 0

# Stop hook uses top-level "systemMessage" (not hookSpecificOutput.additionalContext)
# per Claude Code's Stop event schema
XML_SAFE=$(echo "$CONTEXT" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
ESCAPED=$(echo "$XML_SAFE" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ' | sed 's/[[:space:]]*$//')
echo "{\"systemMessage\":\"<forge-completion-check>${ESCAPED}</forge-completion-check>\"}"
