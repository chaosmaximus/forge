#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# tests/scripts/test-sideload-state.sh — fixture tests for the sideload
# detection helper at scripts/check-sideload-state.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-sideload-state.sh"
FIXTURES="$REPO_ROOT/tests/fixtures/sideload-state"

[ -x "$SCRIPT" ] || { echo "missing or non-executable: $SCRIPT" >&2; exit 2; }
[ -d "$FIXTURES" ] || { echo "missing fixture dir: $FIXTURES" >&2; exit 2; }

PASS=0
FAIL=0

run() {
    bash "$SCRIPT" --settings "$FIXTURES/$1" 2>&1
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
# Test 1: missing settings file (Claude Code not installed)
# ============================================================================
echo "Test 1: missing settings"
set +e
output=$(bash "$SCRIPT" --settings "$FIXTURES/does-not-exist.json" 2>&1)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "nothing to check" "$output"

# ============================================================================
# Test 2: clean settings (public marketplace only)
# ============================================================================
echo "Test 2: clean settings"
set +e
output=$(run clean.json)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "no private sideload references" "$output"

# ============================================================================
# Test 3: drift via enabledPlugins (forge@forge-app-marketplace)
# ============================================================================
echo "Test 3: drift-plugin (enabledPlugins entry)"
set +e
output=$(run drift-plugin.json)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "forge@forge-app-marketplace" "$output"
assert_contains "Migration: https://" "$output"

# ============================================================================
# Test 4: drift via extraKnownMarketplaces source.path
# ============================================================================
echo "Test 4: drift-marketplace (source.path)"
set +e
output=$(run drift-marketplace.json)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "/home/user/forge-app" "$output"

# ============================================================================
# Test 5: drift via forge-private plugin name
# ============================================================================
echo "Test 5: drift-private (forge-private name)"
set +e
output=$(run drift-private.json)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "forge-private" "$output"

# ============================================================================
# Test 6: drift via extraKnownMarketplaces source.repo (W7 review M1)
# ============================================================================
echo "Test 6: drift-marketplace-repo (source.repo)"
set +e
output=$(run drift-marketplace-repo.json)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "chaosmaximus/forge-app" "$output"

# ============================================================================
# Test 7: malformed JSON → exit 2
# ============================================================================
echo "Test 7: malformed JSON"
set +e
output=$(run malformed.json)
status=$?
set -e
assert_exit 2 "$status" "$output"
assert_contains "cannot parse" "$output"

echo
echo "sideload-state fixture tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
