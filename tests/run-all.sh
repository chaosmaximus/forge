#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
TESTS_DIR="$(cd "$(dirname "$0")" && pwd)"
PLUGIN_ROOT="$(cd "$TESTS_DIR/.." && pwd)"

passed=0
failed=0
skipped=0

run_test() {
  local name="$1" script="$2"
  echo ""
  echo "======== $name ========"
  if bash "$script" "$PLUGIN_ROOT"; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
  fi
}

echo "========================================"
echo "  Forge Plugin Test Suite"
echo "========================================"

# Layer 1: Static validation
run_test "Plugin.json validation" "$TESTS_DIR/static/validate-plugin.sh"
run_test "Hooks.json validation" "$TESTS_DIR/static/validate-hooks.sh"
run_test "Skills validation" "$TESTS_DIR/static/validate-skills.sh"
run_test "Agents validation" "$TESTS_DIR/static/validate-agents.sh"
run_test "Agent-tools validation" "$TESTS_DIR/static/validate-agent-tools.sh"
run_test "ShellCheck analysis" "$TESTS_DIR/static/run-shellcheck.sh"

# Layer 2: Unit tests (BATS)
if command -v bats &>/dev/null; then
  echo ""
  echo "======== BATS Unit Tests ========"
  if bats "$TESTS_DIR/unit/"; then
    passed=$((passed + 1))
  else
    failed=$((failed + 1))
  fi
else
  echo ""
  echo "======== BATS Unit Tests ========"
  echo "[SKIP] BATS not installed"
  skipped=$((skipped + 1))
fi

# Layer 3: Integration tests
run_test "Plugin loading smoke test" "$TESTS_DIR/integration/test-plugin-loading.sh"
run_test "Hook behavior integration" "$TESTS_DIR/integration/test-hook-behavior.sh"

echo ""
echo "========================================"
echo "  Results: $passed passed, $failed failed, $skipped skipped"
echo "========================================"
[ $failed -eq 0 ] || exit 1
