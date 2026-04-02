#!/usr/bin/env bash
# SessionEnd hook — close session, update HUD state
set -euo pipefail

PLUGIN_DATA="${CLAUDE_PLUGIN_DATA:-$HOME/.claude/plugin-data/forge}"

# Drain barrier: wait for pending async writes (max 5s)
DRAIN_START=$(date +%s)
while [ -f "${PLUGIN_DATA}/.async-pending" ]; do
    ELAPSED=$(( $(date +%s) - DRAIN_START ))
    if [ "$ELAPSED" -ge 5 ]; then
        break
    fi
    sleep 0.5
done

# Update HUD state to show session ended
if [ -f "${PLUGIN_DATA}/hud/hud-state.json" ]; then
    python3 -c "
import json
try:
    with open('${PLUGIN_DATA}/hud/hud-state.json', 'r') as f:
        data = json.load(f)
    data['session']['phase'] = 'ended'
    with open('${PLUGIN_DATA}/hud/hud-state.json', 'w') as f:
        json.dump(data, f)
except: pass
" 2>/dev/null || true
fi

echo '{"hookSpecificOutput":{"additionalContext":"Session ended. Memory saved to graph."}}'
