#!/usr/bin/env bash
# Forge: PreToolUse hook for Edit|Write
# Blocks edits to sensitive files
set -euo pipefail

# Security hook: fail closed if jq is unavailable
if ! command -v jq &>/dev/null; then
  echo "Forge: jq not found. Blocking edit as a precaution (install jq for proper file path checking)." >&2
  exit 2
fi

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // .tool_input.path // empty')

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

BASENAME=$(basename "$FILE_PATH")

case "$BASENAME" in
  .env|.env.*|credentials*|secrets*|*.key|*.pem|poetry.lock|package-lock.json|yarn.lock)
    echo "Protected file: $BASENAME. Edit manually or use the appropriate package manager." >&2
    exit 2
    ;;
esac

exit 0
