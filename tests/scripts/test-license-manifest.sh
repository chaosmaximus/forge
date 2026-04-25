#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# tests/scripts/test-license-manifest.sh — fixture tests for the SPDX
# sidecar validator at scripts/check-license-manifest.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-license-manifest.sh"
FIXTURES="$REPO_ROOT/tests/fixtures/license-manifest"

[ -x "$SCRIPT" ] || { echo "missing or non-executable: $SCRIPT" >&2; exit 2; }
[ -d "$FIXTURES" ] || { echo "missing fixtures dir: $FIXTURES" >&2; exit 2; }

PASS=0
FAIL=0

run_fixture() {
    local fixture_name="$1"
    bash "$SCRIPT" --root "$FIXTURES/$fixture_name" --manifest manifest.yaml 2>&1
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
# Test 1: clean fixture
# ============================================================================
echo "Test 1: clean fixture"
set +e
output=$(run_fixture clean)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "license-manifest: OK" "$output"
assert_contains "coverage clean" "$output"

# ============================================================================
# Test 2: bad schema_version
# ============================================================================
echo "Test 2: bad-schema fixture"
set +e
output=$(run_fixture bad-schema)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "schema_version must be 1" "$output"

# ============================================================================
# Test 3: coverage gap (undeclared.json present in coverage_path)
# ============================================================================
echo "Test 3: coverage-gap fixture"
set +e
output=$(run_fixture coverage-gap)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "coverage gap" "$output"
assert_contains "undeclared.json" "$output"

# ============================================================================
# Test 4: bad license string
# ============================================================================
echo "Test 4: bad-license fixture"
set +e
output=$(run_fixture bad-license)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "must be a valid SPDX expression" "$output"

# ============================================================================
# Test 5: missing file (manifest references file that doesn't exist)
# ============================================================================
echo "Test 5: missing-file fixture"
set +e
output=$(run_fixture missing-file)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "files[0].path not found in repo" "$output"

# ============================================================================
# Test 6: path-traversal escape
# ============================================================================
echo "Test 6: escape-path fixture"
set +e
output=$(run_fixture escape-path)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "escapes repo root or is absolute" "$output"

# ============================================================================
# Test 7: whitespace-only license (SPDX strictness — W3 review HIGH-1)
# ============================================================================
echo "Test 7: whitespace-license fixture"
set +e
output=$(run_fixture whitespace-license)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "must be a valid SPDX expression" "$output"

# ============================================================================
# Test 8: free-form prose license ("mit license" without operator)
# ============================================================================
echo "Test 8: prose-license fixture"
set +e
output=$(run_fixture prose-license)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "must be a valid SPDX expression" "$output"

# ============================================================================
# Test 9: dangling SPDX operator ("Apache-2.0 OR")
# ============================================================================
echo "Test 9: dangling-op-license fixture"
set +e
output=$(run_fixture dangling-op-license)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "must be a valid SPDX expression" "$output"

# ============================================================================
# Test 10: stringy schema_version (W3 review MED-1)
# ============================================================================
echo "Test 10: stringy-schema fixture"
set +e
output=$(run_fixture stringy-schema)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "schema_version must be 1" "$output"
assert_contains "type str" "$output"

# ============================================================================
# Test 11: references[] entry doesn't exist on disk (W3 review MED-4)
# ============================================================================
echo "Test 11: missing-reference fixture"
set +e
output=$(run_fixture missing-reference)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "references[0] not found in repo" "$output"

echo
echo "license-manifest fixture tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
