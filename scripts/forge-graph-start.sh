#!/usr/bin/env bash
# SessionStart hook — forge (Rust, <5ms). No fallbacks.
cat 2>/dev/null || true
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
FORGE="$SCRIPT_DIR/../servers/forge"
[ -x "$FORGE" ] || exit 0
exec "$FORGE" hook session-start --state-dir "${CLAUDE_PLUGIN_DATA:-.forge}" 2>/dev/null
