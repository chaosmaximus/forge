#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.7.0 — UserPromptSubmit hook
# Per-turn delta refresh: surfaces new memories since the last prompt.
# Maintains a timestamp file for incremental context injection.
# CONTEXT BUDGET: 300 chars max. Silent if no delta.

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

# Session ID
SESSION_ID="${CLAUDE_SESSION_ID:-}"
if [ -z "$SESSION_ID" ]; then
  # Try to read saved session ID
  FORGE_SESSION_DIR="${XDG_RUNTIME_DIR:-$HOME/.forge/sessions}"
  CWD_HASH=$(echo "${CLAUDE_CWD:-$(pwd)}" | md5sum | cut -d' ' -f1)
  SESSION_FILE="$FORGE_SESSION_DIR/forge-session-${CWD_HASH}"
  if [ -f "$SESSION_FILE" ] && [ ! -L "$SESSION_FILE" ]; then
    SESSION_ID=$(cat "$SESSION_FILE" 2>/dev/null || true)
  fi
fi
[ -z "$SESSION_ID" ] && exit 0

# Timestamp file for tracking last refresh
# Uses private dir (not /tmp) to prevent symlink attacks
FORGE_REFRESH_DIR="${XDG_RUNTIME_DIR:-$HOME/.forge/refresh}"
mkdir -p "$FORGE_REFRESH_DIR" 2>/dev/null && chmod 700 "$FORGE_REFRESH_DIR" 2>/dev/null || true
TS_FILE="$FORGE_REFRESH_DIR/forge-refresh-${SESSION_ID}"
SINCE=""
if [ -f "$TS_FILE" ] && [ ! -L "$TS_FILE" ]; then
  SINCE=$(cat "$TS_FILE" 2>/dev/null || true)
fi

# Call context-refresh BEFORE advancing watermark (Finding 4: avoid losing deltas on failure)
RESULT=$(timeout 3 "$FORGE_NEXT" context-refresh \
  --session-id "$SESSION_ID" \
  ${SINCE:+--since "$SINCE"} 2>/dev/null || echo "")

# Only advance watermark AFTER successful fetch
if [ -n "$RESULT" ] || [ -z "$SINCE" ]; then
  NOW=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
  TS_TMP=$(mktemp "$FORGE_REFRESH_DIR/forge-refresh-XXXXXX")
  echo "$NOW" > "$TS_TMP"
  mv "$TS_TMP" "$TS_FILE"
  chmod 600 "$TS_FILE" 2>/dev/null || true
fi

# Silent if no delta
[ -z "$RESULT" ] && exit 0

# Trim to 300 char budget
CONTEXT=$(echo "$RESULT" | head -5 | tr '\n' ' ' | cut -c1-300)
[ -z "$CONTEXT" ] && exit 0

# Escape XML entities first, then JSON
XML_SAFE=$(echo "$CONTEXT" | sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g')
ESCAPED=$(echo "$XML_SAFE" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ' | sed 's/[[:space:]]*$//')
echo "{\"hookSpecificOutput\":{\"additionalContext\":\"<forge-delta>${ESCAPED}</forge-delta>\"}}"
