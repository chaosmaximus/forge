#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/session-start.sh"

# session-start.sh drains stdin with 'cat > /dev/null', so we can't pipe to it.
# Instead we test that the script produces the expected JSON on stdout.

@test "outputs JSON containing hookSpecificOutput" {
  export CLAUDE_PLUGIN_ROOT="$SCRIPT_DIR/../.."
  output=$(bash "$HOOK_SCRIPT" < /dev/null 2>/dev/null)
  echo "$output" | grep -q "hookSpecificOutput"
}

@test "outputs JSON with SessionStart event name" {
  export CLAUDE_PLUGIN_ROOT="$SCRIPT_DIR/../.."
  output=$(bash "$HOOK_SCRIPT" < /dev/null 2>/dev/null)
  echo "$output" | grep -q "SessionStart"
}

@test "outputs JSON mentioning forge:new" {
  export CLAUDE_PLUGIN_ROOT="$SCRIPT_DIR/../.."
  output=$(bash "$HOOK_SCRIPT" < /dev/null 2>/dev/null)
  echo "$output" | grep -q "forge:new"
}

@test "outputs JSON mentioning forge:feature" {
  export CLAUDE_PLUGIN_ROOT="$SCRIPT_DIR/../.."
  output=$(bash "$HOOK_SCRIPT" < /dev/null 2>/dev/null)
  echo "$output" | grep -q "forge:feature"
}

@test "exits 0 even when server binary is missing" {
  export CLAUDE_PLUGIN_ROOT="/tmp/nonexistent-forge-test"
  run bash -c "bash '$HOOK_SCRIPT' < /dev/null 2>/dev/null"
  [ "$status" -eq 0 ]
}
