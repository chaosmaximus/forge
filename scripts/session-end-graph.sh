#!/usr/bin/env bash
# SessionEnd hook — update HUD + sync pending memory to graph.
cat 2>/dev/null || true
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
STATE_DIR="${CLAUDE_PLUGIN_DATA:-.forge}"
# 1. Update HUD state (Rust, <5ms)
"$SCRIPT_DIR/../servers/forge" hook session-end --state-dir "$STATE_DIR" 2>/dev/null
# 2. Sync pending memory to graph (Rust+Python, best-effort, background)
"$SCRIPT_DIR/../servers/forge" sync --state-dir "$STATE_DIR" 2>/dev/null &
