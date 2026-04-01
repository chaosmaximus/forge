#!/usr/bin/env bash
# Forge: TaskCompleted gate
# Reads task metadata from stdin, runs tests, surfaces Codex warning for prod paths
# Exit 0 = allow completion, Exit 2 = block with feedback
set -euo pipefail

INPUT=$(cat)
# TASK_SUBJECT available for future use in logging
export TASK_SUBJECT
TASK_SUBJECT=$(echo "$INPUT" | jq -r '.task_subject // empty' 2>/dev/null || echo "")

# Auto-detect and run test suite
if [ -f "package.json" ]; then
  TEST_OUTPUT=$(npm test 2>&1) || {
    echo "Tests failed. Fix before completing task:" >&2
    echo "$TEST_OUTPUT" | tail -20 >&2
    exit 2
  }
elif [ -f "pyproject.toml" ] || [ -f "setup.py" ]; then
  TEST_OUTPUT=$(python -m pytest 2>&1) || {
    echo "Tests failed. Fix before completing task:" >&2
    echo "$TEST_OUTPUT" | tail -20 >&2
    exit 2
  }
elif [ -f "Makefile" ] && grep -q "^test:" Makefile; then
  TEST_OUTPUT=$(make test 2>&1) || {
    echo "Tests failed. Fix before completing task:" >&2
    echo "$TEST_OUTPUT" | tail -20 >&2
    exit 2
  }
fi

# Check if changed files match prod_paths (surface Codex recommendation)
PROD_PATHS="${CLAUDE_PLUGIN_OPTION_PROD_PATHS:-infrastructure/**,terraform/**,k8s/**,helm/**,production/**}"
CHANGED_FILES=$(git diff --name-only HEAD~1 HEAD 2>/dev/null || echo "")

if [ -n "$CHANGED_FILES" ]; then
  IFS=',' read -ra PATTERNS <<< "$PROD_PATHS"
  for pattern in "${PATTERNS[@]}"; do
    pattern=$(echo "$pattern" | xargs)
    if echo "$CHANGED_FILES" | grep -q "^${pattern%/\*\*}/"; then
      echo "Production path detected ($pattern). Codex adversarial review recommended." >&2
      echo "The forge-review skill will handle the Codex gate." >&2
      break
    fi
  done
fi

exit 0
