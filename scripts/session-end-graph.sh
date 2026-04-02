#!/usr/bin/env bash
# SessionEnd hook — delegates to Python for graph operations
set -euo pipefail

cat > /dev/null 2>/dev/null || true

PLUGIN_ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
FORGE_GRAPH="$PLUGIN_ROOT/forge-graph"

# Drain barrier: wait for pending async writes (max 5s)
PLUGIN_DATA="${CLAUDE_PLUGIN_DATA:-$HOME/.claude/plugin-data/forge}"
DRAIN_START=$(date +%s)
while [ -f "${PLUGIN_DATA}/.async-pending" ]; do
    ELAPSED=$(( $(date +%s) - DRAIN_START ))
    if [ "$ELAPSED" -ge 5 ]; then break; fi
    sleep 0.5
done

if [ -d "$FORGE_GRAPH/src" ]; then
    PYTHONPATH="$FORGE_GRAPH/src" python3 -m forge_graph.hooks.session_end 2>/dev/null
else
    echo '{"hookSpecificOutput":{"additionalContext":"Session ended."}}'
fi
