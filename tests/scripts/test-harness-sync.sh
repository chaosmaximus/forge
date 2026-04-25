#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# tests/scripts/test-harness-sync.sh — fixture tests for the harness-sync
# drift detector at scripts/check-harness-sync.sh.
#
# Tests four scenarios:
#   1. clean fixture, default mode → exit 0, "no drift"
#   2. drift fixture, warn mode (FORGE_HARNESS_SYNC_ENFORCE=0) → exit 0,
#      drift entries reported
#   3. drift fixture, enforce mode → exit 1, drift entries reported
#   4. drift fixture, legacy FORCE_FAIL=1 → exit 1 (back-compat)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-harness-sync.sh"
CLEAN_FIXTURE="$REPO_ROOT/tests/fixtures/harness-sync/clean"
DRIFT_FIXTURE="$REPO_ROOT/tests/fixtures/harness-sync/drift"

[ -x "$SCRIPT" ] || { echo "missing or non-executable: $SCRIPT" >&2; exit 2; }
[ -d "$CLEAN_FIXTURE" ] || { echo "missing fixture: $CLEAN_FIXTURE" >&2; exit 2; }
[ -d "$DRIFT_FIXTURE" ] || { echo "missing fixture: $DRIFT_FIXTURE" >&2; exit 2; }

PASS=0
FAIL=0

# Strip env vars that affect mode; the test sets them explicitly per case.
# Keep MIN_* threshold overrides — fixtures only have 6 variants each.
runner() {
    local extra=()
    while [ "${1:-}" = "--env" ]; do
        extra+=("$2")
        shift 2
    done
    env -u FORGE_HARNESS_SYNC_ENFORCE -u FORCE_FAIL \
        FORGE_HARNESS_SYNC_MIN_REQUEST=1 \
        FORGE_HARNESS_SYNC_MIN_CLI=1 \
        "${extra[@]}" \
        bash "$SCRIPT" "$@"
}

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
# Test 1: clean fixture default mode
# ============================================================================
echo "Test 1: clean fixture, default mode"
set +e
output=$(runner --root "$CLEAN_FIXTURE" 2>&1)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "authoritative, no drift" "$output"

# ============================================================================
# Test 2: drift fixture warn mode (override date-based auto-flip)
# ============================================================================
echo "Test 2: drift fixture, warn mode"
set +e
output=$(runner --env FORGE_HARNESS_SYNC_ENFORCE=0 --root "$DRIFT_FIXTURE" 2>&1)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "drift entries detected" "$output"

# ============================================================================
# Test 3: drift fixture enforce mode
# ============================================================================
echo "Test 3: drift fixture, enforce mode"
set +e
output=$(runner --env FORGE_HARNESS_SYNC_ENFORCE=1 --root "$DRIFT_FIXTURE" 2>&1)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "drift entries detected" "$output"

# ============================================================================
# Test 4: drift fixture legacy FORCE_FAIL=1
# ============================================================================
echo "Test 4: drift fixture, legacy FORCE_FAIL=1"
set +e
output=$(runner --env FORCE_FAIL=1 --root "$DRIFT_FIXTURE" 2>&1)
status=$?
set -e
assert_exit 1 "$status" "$output"

echo
echo "harness-sync fixture tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
