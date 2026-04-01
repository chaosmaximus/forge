#!/usr/bin/env bash
# Forge: SessionStart hook (sync)
# 1. Checks and installs codebase-memory-mcp binary if missing
# 2. Starts background indexing
# 3. Injects Forge context into the session
set -euo pipefail

# Drain stdin to prevent pipe issues
cat > /dev/null 2>/dev/null || true

PLUGIN_ROOT="${CLAUDE_PLUGIN_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
SERVER_BIN="$PLUGIN_ROOT/servers/codebase-memory-mcp"

# Auto-install server binary if missing
if [ ! -x "$SERVER_BIN" ]; then
  bash "$PLUGIN_ROOT/scripts/install-server.sh" 2>/dev/null || true
fi

# Background index current project (fire and forget)
if [ -x "$SERVER_BIN" ]; then
  "$SERVER_BIN" index --project "$(pwd)" --background 2>/dev/null &
fi

# Inject Forge context into session
cat <<'CONTEXT_EOF'
{
  "hookSpecificOutput": {
    "hookEventName": "SessionStart",
    "additionalContext": "You have the Forge plugin installed. Forge provides production-grade agent team orchestration with two modes:\n- `/forge:new` — for building new projects from scratch (PRD creation, visual design, agent team build)\n- `/forge:feature` — for modifying existing codebases (graph-powered exploration, agent team build)\n- `/forge:review` — two-stage evaluation with Codex adversarial review gate\n- `/forge:ship` — final verification and PR creation\n- `/forge:handoff` — session pause/resume\n- `/forge:setup` — first-time prerequisite checks\n\nWhen the user asks to build something new or modify existing code, consider whether Forge's workflow would be appropriate. Present the option but let the user decide."
  }
}
CONTEXT_EOF
