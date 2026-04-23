#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== Hook Behavior Integration Test ==="

# Helper: test a hook script with input and expected exit code
test_hook() {
  local name="$1" script="$2" input="$3" expected_exit="$4" description="$5"
  actual_exit=0
  echo "$input" | bash "$script" >/dev/null 2>&1 || actual_exit=$?
  if [ "$actual_exit" -eq "$expected_exit" ]; then
    echo "[PASS] $name: $description (exit=$actual_exit)"
  else
    echo "[FAIL] $name: $description (expected exit=$expected_exit, got=$actual_exit)"
    errors=$((errors + 1))
  fi
}

# --- protect-sensitive-files.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/protect-sensitive-files.sh"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/main.py"}}' 0 "allows normal files"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/.env"}}' 2 "blocks .env"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/.env.local"}}' 2 "blocks .env.local"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/server.pem"}}' 2 "blocks .pem"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/credentials.json"}}' 2 "blocks credentials"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/poetry.lock"}}' 2 "blocks lock files"
test_hook "protect" "$SCRIPT" '{}' 0 "handles empty input"

# --- teammate-idle-checkpoint.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/teammate-idle-checkpoint.sh"
test_hook "idle" "$SCRIPT" '{"teammate_name":"gen-1"}' 0 "exits 0 normally"

# --- task-completed-gate.sh (in a temp git repo) ---
SCRIPT="$PLUGIN_ROOT/scripts/task-completed-gate.sh"
TMPDIR=$(mktemp -d /tmp/forge-hook-test-XXXX)
cd "$TMPDIR"
git init -q && echo "init" > README.md && git add . && git commit -q -m "init"
test_hook "taskgate" "$SCRIPT" '{"task_subject":"test task"}' 0 "passes with no test framework"

# With failing tests
echo '{"scripts":{"test":"exit 1"}}' > package.json
test_hook "taskgate" "$SCRIPT" '{"task_subject":"test task"}' 2 "blocks when npm tests fail"

# With passing tests
echo '{"scripts":{"test":"echo ok"}}' > package.json
test_hook "taskgate" "$SCRIPT" '{"task_subject":"test task"}' 0 "passes when npm tests pass"

rm -rf "$TMPDIR"
cd "$PLUGIN_ROOT"

# --- forge-graph-start.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/forge-graph-start.sh"
export CLAUDE_PLUGIN_ROOT="$PLUGIN_ROOT"
output=$(bash "$SCRIPT" < /dev/null 2>/dev/null || true)
if echo "$output" | grep -q "hookSpecificOutput"; then
  echo "[PASS] forge-graph-start: outputs valid context injection JSON"
else
  echo "[FAIL] forge-graph-start: no valid context injection JSON"; errors=$((errors + 1))
fi

# --- session-end-graph.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/session-end-graph.sh"
test_hook "session-end" "$SCRIPT" '{}' 0 "exits 0 gracefully"

echo ""
if [ $errors -eq 0 ]; then
  echo "=== All hook behavior tests passed ==="
else
  echo "=== $errors error(s) ==="
  exit 1
fi
