#!/usr/bin/env bash
# SessionStart hook — create session, check secrets, check evolution suggestions
set -euo pipefail

PLUGIN_DATA="${CLAUDE_PLUGIN_DATA:-$HOME/.claude/plugin-data/forge}"

# Read session info
SESSION_ID="session-$(date +%s)-$(head -c 4 /dev/urandom | xxd -p)"

# Check for stale secrets from HUD state
STALE_SECRETS=0
FIX_COUNT=0
if [ -f "${PLUGIN_DATA}/hud/hud-state.json" ]; then
    STALE_SECRETS=$(python3 -c "
import json
try:
    data = json.load(open('${PLUGIN_DATA}/hud/hud-state.json'))
    print(data.get('security', {}).get('stale', 0))
except: print(0)
" 2>/dev/null || echo "0")
    FIX_COUNT=$(python3 -c "
import json
try:
    data = json.load(open('${PLUGIN_DATA}/hud/hud-state.json'))
    print(data.get('skills', {}).get('fix_candidates', 0))
except: print(0)
" 2>/dev/null || echo "0")
fi

# Build context
CONTEXT="[Forge v0.2.0] Session ${SESSION_ID}."
if [ "$STALE_SECRETS" -gt 0 ]; then
    CONTEXT="${CONTEXT} WARNING: ${STALE_SECRETS} secrets need rotation."
fi
if [ "$FIX_COUNT" -gt 0 ]; then
    CONTEXT="${CONTEXT} ${FIX_COUNT} skill(s) need attention."
fi
CONTEXT="${CONTEXT} Tools: forge_remember, forge_recall, forge_link, forge_decisions, forge_patterns, forge_timeline, forge_forget, forge_usage, forge_scan + 15 axon_* code intelligence tools."

cat <<EOF
{"hookSpecificOutput":{"additionalContext":"${CONTEXT}"}}
EOF
