#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/post-edit-format.sh"
FIXTURES="$SCRIPT_DIR/fixtures"

@test "exits 0 for nonexistent file" {
  run bash -c 'echo "{\"tool_input\":{\"file_path\":\"/tmp/nonexistent-forge-test-xyz.py\"}}" | bash '"'$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "exits 0 for missing file_path" {
  run bash -c 'echo "{\"tool_input\":{}}" | bash '"'$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "exits 0 for empty JSON" {
  run bash -c "cat '$FIXTURES/empty.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "exits 0 for real python file" {
  tmpfile=$(mktemp /tmp/forge-test-XXXX.py)
  echo 'x=1' > "$tmpfile"
  run bash -c "echo '{\"tool_input\":{\"file_path\":\"$tmpfile\"}}' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
  rm -f "$tmpfile"
}

@test "exits 0 for unknown extension" {
  tmpfile=$(mktemp /tmp/forge-test-XXXX.xyz)
  echo 'data' > "$tmpfile"
  run bash -c "echo '{\"tool_input\":{\"file_path\":\"$tmpfile\"}}' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
  rm -f "$tmpfile"
}
