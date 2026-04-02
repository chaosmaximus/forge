#!/usr/bin/env bash
# SessionStart hook — delegates to Python for graph operations
set -euo pipefail

cat > /dev/null 2>/dev/null || true

PLUGIN_ROOT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
FORGE_GRAPH="$PLUGIN_ROOT/forge-graph"

if [ -d "$FORGE_GRAPH/src" ]; then
    PYTHONPATH="$FORGE_GRAPH/src" python3 -m forge_graph.hooks.session_start 2>/dev/null \
        || echo '{"hookSpecificOutput":{"additionalContext":"[Forge v0.2.0] Graph server not available."}}'
else
    echo '{"hookSpecificOutput":{"additionalContext":"[Forge v0.2.0] Graph server not available."}}'
fi
