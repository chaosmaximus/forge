#!/usr/bin/env bash
# SessionStart hook — forge (Rust, <5ms). No fallbacks.
cat 2>/dev/null || true
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
FORGE="$SCRIPT_DIR/../servers/forge"
if [ -x "$FORGE" ]; then
  exec "$FORGE" hook session-start --state-dir "${CLAUDE_PLUGIN_DATA:-.forge}" 2>/dev/null
fi
# Fallback: minimal valid JSON when binary not available (CI)
echo '{"hookSpecificOutput":{"additionalContext":"[Forge] Binary not available."}}'
