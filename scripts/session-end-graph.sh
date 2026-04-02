#!/usr/bin/env bash
# SessionEnd hook — update HUD + sync pending memory to graph.
cat 2>/dev/null || true
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
FORGE="$SCRIPT_DIR/../servers/forge"
STATE_DIR="${CLAUDE_PLUGIN_DATA:-.forge}"
[ -x "$FORGE" ] || exit 0
"$FORGE" hook session-end --state-dir "$STATE_DIR" 2>/dev/null
"$FORGE" sync --state-dir "$STATE_DIR" 2>/dev/null &
