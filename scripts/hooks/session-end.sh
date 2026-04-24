#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.6.0 — SessionEnd hook
# Ends the session registration and triggers final memory extraction.

set -euo pipefail
cat 2>/dev/null || true  # consume stdin

FORGE_NEXT="${FORGE_NEXT:-forge-next}"
command -v "$FORGE_NEXT" &>/dev/null || FORGE_NEXT="$HOME/.local/bin/forge-next"
[ -x "$FORGE_NEXT" ] || exit 0

# End the session
SESSION_ID="${CLAUDE_SESSION_ID:-}"
SESSION_FILE=""
if [ -z "$SESSION_ID" ]; then
  # Try to read saved session ID from session-start hook (secure state dir)
  FORGE_SESSION_DIR="${XDG_RUNTIME_DIR:-$HOME/.forge/sessions}"
  CWD_HASH=$(echo "${CLAUDE_CWD:-$(pwd)}" | md5sum | cut -d' ' -f1)
  SESSION_FILE="$FORGE_SESSION_DIR/forge-session-${CWD_HASH}"
  if [ -f "$SESSION_FILE" ] && [ ! -L "$SESSION_FILE" ]; then
    SESSION_ID=$(cat "$SESSION_FILE" 2>/dev/null || true)
  else
    SESSION_FILE=""
  fi
fi
# Call end-session BEFORE removing the session file so a transient
# failure (daemon down, binary absent) doesn't lose the ID — we can
# retry on the next SessionEnd (2P-1b §10).
if [ -n "$SESSION_ID" ]; then
  if "$FORGE_NEXT" end-session --id "$SESSION_ID" 2>/dev/null; then
    if [ -n "$SESSION_FILE" ]; then
      rm -f "$SESSION_FILE" 2>/dev/null || true
    fi
  fi
fi

# Ingest any new Claude memory files
"$FORGE_NEXT" ingest-claude 2>/dev/null || true
