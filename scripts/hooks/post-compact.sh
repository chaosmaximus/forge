#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.7.0 — PostCompact hook
# Re-injects full context after context window compression.
# Same as session-start but triggered by compaction — ensures Forge context
# survives the compression event.
# CONTEXT BUDGET: ~4000 chars (full context, same as session-start).

set -euo pipefail
cat 2>/dev/null || true  # consume stdin

# Find forge-next binary
FORGE_NEXT="${FORGE_NEXT:-forge-next}"
if ! command -v "$FORGE_NEXT" &>/dev/null; then
  for candidate in "$HOME/.local/bin/forge-next" "/usr/local/bin/forge-next"; do
    [ -x "$candidate" ] && FORGE_NEXT="$candidate" && break
  done
fi
command -v "$FORGE_NEXT" &>/dev/null || exit 0

# Detect project from CWD
PROJECT=""
if [ -n "${CLAUDE_CWD:-}" ]; then
  PROJECT=$(basename "$CLAUDE_CWD")
elif [ -n "${PWD:-}" ]; then
  PROJECT=$(basename "$PWD")
fi

# Compile full context (static + dynamic) — same as session-start
CONTEXT=$("$FORGE_NEXT" compile-context \
  --agent claude-code \
  ${PROJECT:+--project "$PROJECT"} 2>/dev/null || echo "<forge-context version=\"0.7.0\"/>")

# Escape for JSON output
CONTEXT_ESCAPED=$(echo "$CONTEXT" | sed 's/\\/\\\\/g; s/"/\\"/g' | tr '\n' ' ' | sed 's/[[:space:]]*$//')

# PostCompact uses top-level systemMessage (not hookSpecificOutput)
echo "{\"systemMessage\":\"$CONTEXT_ESCAPED\"}"
