#!/usr/bin/env bash
# Forge: PostToolUse hook for Edit|Write
# Auto-detects project linter and formats the edited file
set -euo pipefail

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null || echo "")

if [ -z "$FILE_PATH" ] || [ ! -f "$FILE_PATH" ]; then
  exit 0
fi

EXT="${FILE_PATH##*.}"

case "$EXT" in
  py)
    command -v ruff &>/dev/null && ruff format "$FILE_PATH" 2>/dev/null && ruff check --fix "$FILE_PATH" 2>/dev/null || true
    ;;
  ts|tsx|js|jsx)
    if [ -f "node_modules/.bin/eslint" ]; then
      npx eslint --fix "$FILE_PATH" 2>/dev/null || true
    elif [ -f "node_modules/.bin/prettier" ]; then
      npx prettier --write "$FILE_PATH" 2>/dev/null || true
    fi
    ;;
  rs)
    command -v rustfmt &>/dev/null && rustfmt "$FILE_PATH" 2>/dev/null || true
    ;;
  go)
    command -v gofmt &>/dev/null && gofmt -w "$FILE_PATH" 2>/dev/null || true
    ;;
esac

exit 0
