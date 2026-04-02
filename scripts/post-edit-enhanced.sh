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

# Validate FILE_PATH doesn't contain shell metacharacters
if [[ "$FILE_PATH" =~ [';|&$`\\'] ]] || [[ "$FILE_PATH" == *'$('* ]]; then
    exit 0
fi

# Resolve canonical path (symlink defense)
CANONICAL=$(readlink -f -- "$FILE_PATH" 2>/dev/null || echo "$FILE_PATH")
WORKSPACE=$(readlink -f "$(pwd)")

# Workspace boundary check (P2: require trailing slash to prevent /repo matching /repo_evil)
if [[ "$CANONICAL" != "$WORKSPACE/"* ]] && [[ "$CANONICAL" != "$WORKSPACE" ]]; then
    echo '{"decision":"block","reason":"Path outside workspace boundary"}'
    exit 0
fi

# Quick secret scan (regex only — fast)
ALERTS=""
if [ -f "$CANONICAL" ]; then
    if grep -qP -- 'AKIA[A-Z0-9]{16}' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}AWS access key detected. "
    fi
    if grep -qP -- 'ghp_[A-Za-z0-9]{36,}' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}GitHub PAT detected. "
    fi
    if grep -q -- 'BEGIN.*PRIVATE KEY' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}Private key detected. "
    fi
    if grep -qPi -- '(password|secret|token|api_key)\s*[:=]\s*["\x27][^\s"'\'']{16,}' "$CANONICAL" 2>/dev/null; then
        ALERTS="${ALERTS}Possible hardcoded secret. "
    fi
fi

if [ -n "$ALERTS" ]; then
    jq -n --arg path "$FILE_PATH" --arg alerts "$ALERTS" \
        '{"hookSpecificOutput":{"additionalContext":("SECRET ALERT in " + $path + ": " + $alerts + "Consider moving to .env or .gitignore.")}}'
fi

# Decision-awareness check (async, non-blocking)
SCRIPT_DIR_PARENT="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
if [ -d "${SCRIPT_DIR_PARENT}/forge-graph/src" ] && [ -n "$FILE_PATH" ]; then
    DECISION_OUTPUT=$(PYTHONPATH="${SCRIPT_DIR_PARENT}/forge-graph/src" python3 -m forge_graph.hooks.post_edit "$FILE_PATH" 2>/dev/null || echo "")
    if [ -n "$DECISION_OUTPUT" ]; then
        echo "$DECISION_OUTPUT"
    fi
fi

# Run existing formatter if available
if [ -f "${SCRIPT_DIR}/post-edit-format.sh" ]; then
    echo "$INPUT" | bash "${SCRIPT_DIR}/post-edit-format.sh" 2>/dev/null || true
fi
