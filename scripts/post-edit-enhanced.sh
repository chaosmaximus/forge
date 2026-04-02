#!/usr/bin/env bash
# PostToolUse hook — secret detection on edited files + code formatting
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"

# Read tool input from stdin
INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.toolInput.file_path // .toolInput.filePath // empty' 2>/dev/null)

if [ -z "$FILE_PATH" ]; then
    exit 0
fi

# Resolve canonical path (symlink defense)
CANONICAL=$(readlink -f "$FILE_PATH" 2>/dev/null || echo "$FILE_PATH")
WORKSPACE=$(readlink -f "$(pwd)")

# Workspace boundary check
if [[ "$CANONICAL" != "$WORKSPACE"* ]]; then
    echo '{"decision":"block","reason":"Path outside workspace boundary"}'
    exit 0
fi

# Quick secret scan (regex only — fast)
ALERTS=""
if [ -f "$CANONICAL" ]; then
    if grep -qP 'AKIA[A-Z0-9]{16}' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}AWS access key detected. "
    fi
    if grep -qP 'ghp_[A-Za-z0-9]{36,}' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}GitHub PAT detected. "
    fi
    if grep -q 'BEGIN.*PRIVATE KEY' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}Private key detected. "
    fi
    if grep -qPi '(password|secret|token|api_key)\s*[:=]\s*["\x27][^\s"'\'']{16,}' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}Possible hardcoded secret. "
    fi
fi

if [ -n "$ALERTS" ]; then
    # Escape for JSON
    ESCAPED_ALERTS=$(echo "$ALERTS" | sed 's/"/\\"/g')
    ESCAPED_PATH=$(echo "$FILE_PATH" | sed 's/"/\\"/g')
    echo "{\"hookSpecificOutput\":{\"additionalContext\":\"SECRET ALERT in ${ESCAPED_PATH}: ${ESCAPED_ALERTS}Consider moving to .env or .gitignore.\"}}"
fi

# Run existing formatter if available
if [ -f "${SCRIPT_DIR}/post-edit-format.sh" ]; then
    echo "$INPUT" | bash "${SCRIPT_DIR}/post-edit-format.sh" 2>/dev/null || true
fi
