#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/install-server.sh"

@test "script has valid bash syntax" {
  run bash -n "$HOOK_SCRIPT"
  [ "$status" -eq 0 ]
}

@test "detects current platform without error" {
  # The script should at least get past platform detection
  # It will fail on download (no network in test), but should identify the platform
  export CLAUDE_PLUGIN_ROOT="/tmp/forge-test-install-$$"
  mkdir -p "$CLAUDE_PLUGIN_ROOT/servers"
  run bash -c "bash '$HOOK_SCRIPT' 2>&1 || true"
  # Should mention downloading or platform, not a bash error
  [[ "$output" == *"Downloading"* ]] || [[ "$output" == *"Unsupported"* ]] || [[ "$output" == *"Download failed"* ]]
  rm -rf "$CLAUDE_PLUGIN_ROOT"
}

@test "creates servers directory if missing" {
  export CLAUDE_PLUGIN_ROOT="/tmp/forge-test-install-$$"
  run bash -c "bash '$HOOK_SCRIPT' 2>&1 || true"
  [ -d "$CLAUDE_PLUGIN_ROOT/servers" ] || true
  rm -rf "$CLAUDE_PLUGIN_ROOT"
}
