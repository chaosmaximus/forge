#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# tests/scripts/test-review-artifacts.sh — fixture tests for the review
# artifacts validator at scripts/check-review-artifacts.sh.
#
# Tests:
#   1. clean fixture → exit 0, "OK" message
#   2. drift fixture → exit 1, validation errors for each malformed YAML

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-review-artifacts.sh"
CLEAN_FIXTURE="$REPO_ROOT/tests/fixtures/review-artifacts/clean"
DRIFT_FIXTURE="$REPO_ROOT/tests/fixtures/review-artifacts/drift"

[ -x "$SCRIPT" ] || { echo "missing or non-executable: $SCRIPT" >&2; exit 2; }
[ -d "$CLEAN_FIXTURE" ] || { echo "missing fixture: $CLEAN_FIXTURE" >&2; exit 2; }
[ -d "$DRIFT_FIXTURE" ] || { echo "missing fixture: $DRIFT_FIXTURE" >&2; exit 2; }

PASS=0
FAIL=0

assert_exit() {
    local expected="$1"
    local actual="$2"
    local output="$3"
    if [ "$expected" -eq "$actual" ]; then
        echo "  PASS — exit $actual"
        PASS=$((PASS + 1))
    else
        echo "  FAIL — expected exit $expected, got $actual"
        echo "    output:"
        printf '%s\n' "$output" | awk '{print "      " $0}'
        FAIL=$((FAIL + 1))
    fi
}

assert_contains() {
    local needle="$1"
    local haystack="$2"
    if echo "$haystack" | grep -qF "$needle"; then
        echo "  PASS — contains '$needle'"
        PASS=$((PASS + 1))
    else
        echo "  FAIL — missing '$needle' in output"
        echo "    output:"
        printf '%s\n' "$haystack" | awk '{print "      " $0}'
        FAIL=$((FAIL + 1))
    fi
}

# ============================================================================
# Test 1: clean fixture
# ============================================================================
echo "Test 1: clean fixture"
set +e
output=$(bash "$SCRIPT" --root "$CLEAN_FIXTURE" 2>&1)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "review-artifacts: OK" "$output"

# ============================================================================
# Test 2: drift fixture — every failure mode triggered
# ============================================================================
echo "Test 2: drift fixture (5 malformed YAMLs)"
set +e
output=$(bash "$SCRIPT" --root "$DRIFT_FIXTURE" 2>&1)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "schema_version must be 1" "$output"
assert_contains "artifacts must be a non-empty list" "$output"
assert_contains "open BLOCKING-severity findings" "$output"
assert_contains "verdict must be in" "$output"
assert_contains "target_paths entry not found" "$output"

echo
echo "review-artifacts fixture tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
