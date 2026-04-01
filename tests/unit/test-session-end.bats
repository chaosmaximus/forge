#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/session-end-memory.sh"

@test "exits 0 when episodic-memory is not installed" {
  # On most test environments, episodic-memory CLI won't be at the expected path
  run bash -c "bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "exits 0 with no errors" {
  run bash -c "echo '{}' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}
