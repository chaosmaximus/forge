#!/usr/bin/env bash
# SessionEnd hook — forge-core (Rust, <5ms). No fallbacks.
cat 2>/dev/null || true
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
exec "$SCRIPT_DIR/../servers/forge-core" hook session-end --state-dir "${CLAUDE_PLUGIN_DATA:-.forge}" 2>/dev/null
