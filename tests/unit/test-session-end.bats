#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/session-end-graph.sh"

@test "exits 0 when forge-graph source is missing" {
  export CLAUDE_PLUGIN_ROOT="/tmp/nonexistent-forge-test"
  run bash -c "bash '$HOOK_SCRIPT' < /dev/null 2>/dev/null"
  [ "$status" -eq 0 ]
}

@test "exits 0 with no errors" {
  run bash -c "echo '{}' | bash '$HOOK_SCRIPT' 2>/dev/null"
  [ "$status" -eq 0 ]
}
