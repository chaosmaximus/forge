#!/usr/bin/env bash
# SessionStart hook — delegates to forge-core (Rust, <5ms) for context injection.
# Falls back to Python if forge-core is unavailable, then to static output.
# Never exits non-zero — Claude Code treats non-zero as "hook error".

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
PLUGIN_ROOT="$(dirname "$SCRIPT_DIR")"
FORGE_CORE="$PLUGIN_ROOT/servers/forge-core"
STATE_DIR="${CLAUDE_PLUGIN_DATA:-.forge}"

# Drain stdin so Claude Code doesn't get SIGPIPE
INPUT=$(cat 2>/dev/null || true)

# Prefer Rust binary (fast, no DB lock issues)
if [ -x "$FORGE_CORE" ]; then
    "$FORGE_CORE" hook session-start --state-dir "$STATE_DIR" 2>/dev/null && exit 0
fi

# Fallback: Python hook (may fail if DB is locked by MCP server)
FORGE_GRAPH="$PLUGIN_ROOT/forge-graph"
if [ -d "$FORGE_GRAPH/src" ]; then
    PYTHONPATH="$FORGE_GRAPH/src" python3 -m forge_graph.hooks.session_start 2>/dev/null && exit 0
fi

# Final fallback: static output
echo '{"hookSpecificOutput":{"additionalContext":"[Forge v0.2.0]"}}'
