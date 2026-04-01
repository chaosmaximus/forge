#!/usr/bin/env bash
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

# Layer 1: Static validation (fast, no external deps)
run_test "Plugin.json validation" "$TESTS_DIR/static/validate-plugin.sh"
run_test "Skills validation" "$TESTS_DIR/static/validate-skills.sh"
run_test "Agents validation" "$TESTS_DIR/static/validate-agents.sh"
run_test "CSV data validation" "$TESTS_DIR/static/validate-csv.sh"
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
  echo "[SKIP] BATS not installed (npm i -g bats or git clone bats-core)"
  skipped=$((skipped + 1))
fi

# Layer 3: Integration tests
run_test "Plugin loading smoke test" "$TESTS_DIR/integration/test-plugin-loading.sh"

echo ""
echo "========================================"
echo "  Results: $passed passed, $failed failed, $skipped skipped"
echo "========================================"
[ $failed -eq 0 ] || exit 1
