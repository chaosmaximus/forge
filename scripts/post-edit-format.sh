#!/usr/bin/env bash
# Forge: PostToolUse hook for Edit|Write
# Auto-detects project linter and formats the edited file
# Security: validates file path is within workspace
set -euo pipefail

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null || echo "")

if [ -z "$FILE_PATH" ] || [ ! -f "$FILE_PATH" ]; then
  exit 0
fi

# Security: resolve canonical path and verify it's within the working directory
RESOLVED_PATH=$(readlink -f "$FILE_PATH" 2>/dev/null || echo "$FILE_PATH")
WORKSPACE=$(pwd)
case "$RESOLVED_PATH" in
  "$WORKSPACE"/*) ;; # Path is within workspace — OK
  *)
    # File is outside workspace — skip formatting silently
    exit 0
    ;;
esac

EXT="${RESOLVED_PATH##*.}"

case "$EXT" in
  py)
    command -v ruff &>/dev/null && ruff format "$RESOLVED_PATH" 2>/dev/null && ruff check --fix "$RESOLVED_PATH" 2>/dev/null || true
    ;;
  ts|tsx|js|jsx)
    if [ -f "node_modules/.bin/eslint" ]; then
      npx eslint --fix "$RESOLVED_PATH" 2>/dev/null || true
    elif [ -f "node_modules/.bin/prettier" ]; then
      npx prettier --write "$RESOLVED_PATH" 2>/dev/null || true
    fi
    ;;
  rs)
    command -v rustfmt &>/dev/null && rustfmt "$RESOLVED_PATH" 2>/dev/null || true
    ;;
  go)
    command -v gofmt &>/dev/null && gofmt -w "$RESOLVED_PATH" 2>/dev/null || true
    ;;
  tf|tfvars)
    command -v terraform &>/dev/null && terraform fmt "$RESOLVED_PATH" 2>/dev/null || true
    ;;
esac

exit 0
