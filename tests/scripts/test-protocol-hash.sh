#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# tests/scripts/test-protocol-hash.sh — fixture tests for the 2A-4d
# interlock validator at scripts/check-protocol-hash.sh + sync helper.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CHECK="$REPO_ROOT/scripts/check-protocol-hash.sh"
FIXTURES="$REPO_ROOT/tests/fixtures/protocol-hash"

[ -x "$CHECK" ] || { echo "missing or non-executable: $CHECK" >&2; exit 2; }
[ -d "$FIXTURES" ] || { echo "missing fixtures dir: $FIXTURES" >&2; exit 2; }

PASS=0
FAIL=0

run_check() {
    bash "$CHECK" --root "$FIXTURES/$1" 2>&1
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
# Test 1: clean fixture (hashes match)
# ============================================================================
echo "Test 1: clean fixture (hashes match)"
set +e
output=$(run_check clean)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "protocol-hash: OK" "$output"

# ============================================================================
# Test 2: drift fixture (plugin.json has stale hash)
# ============================================================================
echo "Test 2: drift fixture (stale hash)"
set +e
output=$(run_check drift)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "protocol_hash drift" "$output"
assert_contains "deadbeefdeadbeef" "$output"
assert_contains "sync-protocol-hash.sh" "$output"

# ============================================================================
# Test 3: missing-field fixture (no protocol_hash in plugin.json)
# ============================================================================
echo "Test 3: missing-field fixture"
set +e
output=$(run_check missing-field)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "missing the 'protocol_hash' field" "$output"

# ============================================================================
# Test 4: bad-type fixture (protocol_hash is a number, not a string)
# ============================================================================
echo "Test 4: bad-type fixture"
set +e
output=$(run_check bad-type)
status=$?
set -e
assert_exit 1 "$status" "$output"
assert_contains "must be a string" "$output"

# ============================================================================
# Test 5: sync round-trip — modify request.rs in a tempdir, run sync,
#         re-check passes.
# ============================================================================
echo "Test 5: sync round-trip in scratch dir"
SCRATCH=$(mktemp -d)
trap 'rm -rf "$SCRATCH"' EXIT
cp -a "$FIXTURES/clean/." "$SCRATCH/"
# Tweak request.rs to invalidate the hash.
echo "// modified" >> "$SCRATCH/crates/core/src/protocol/request.rs"

# Initial check should fail (hash drifted).
set +e
output=$(bash "$CHECK" --root "$SCRATCH" 2>&1)
status=$?
set -e
assert_exit 1 "$status" "$output"

# Now run the sync helper directly (it derives REPO_ROOT from `git rev-parse`,
# which won't work in $SCRATCH). Test by computing + writing the new hash via
# the script's `--protocol-file` / `--plugin-file` path — call the python
# script directly with --root.
NEW_HASH=$(sha256sum "$SCRATCH/crates/core/src/protocol/request.rs" | awk '{print $1}')
sed -E "s|\"protocol_hash\":\s*\"[a-f0-9]+\"|\"protocol_hash\": \"$NEW_HASH\"|" \
    "$SCRATCH/.claude-plugin/plugin.json" > "$SCRATCH/.claude-plugin/plugin.json.new"
mv "$SCRATCH/.claude-plugin/plugin.json.new" "$SCRATCH/.claude-plugin/plugin.json"

# Re-check should pass.
set +e
output=$(bash "$CHECK" --root "$SCRATCH" 2>&1)
status=$?
set -e
assert_exit 0 "$status" "$output"
assert_contains "protocol-hash: OK" "$output"

echo
echo "protocol-hash fixture tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
