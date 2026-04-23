#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/forge-graph-start.sh"

# forge-graph-start.sh drains stdin with 'cat > /dev/null', so we can't pipe to it.
# Instead we test that the script produces the expected JSON on stdout.

@test "outputs JSON containing hookSpecificOutput" {
  export CLAUDE_PLUGIN_ROOT="$SCRIPT_DIR/../.."
  output=$(bash "$HOOK_SCRIPT" < /dev/null 2>/dev/null)
  echo "$output" | grep -q "hookSpecificOutput"
}

@test "exits 0 even when forge-graph source is missing" {
  export CLAUDE_PLUGIN_ROOT="/tmp/nonexistent-forge-test"
  run bash -c "bash '$HOOK_SCRIPT' < /dev/null 2>/dev/null"
  [ "$status" -eq 0 ]
}
