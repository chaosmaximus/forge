#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/teammate-idle-checkpoint.sh"

@test "exits 0 when no STATE.md exists" {
  cd /tmp
  run bash -c "echo '{}' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "exits 0 when STATE.md exists" {
  tmpdir=$(mktemp -d /tmp/forge-test-XXXX)
  echo '**Last updated:** never' > "$tmpdir/STATE.md"
  cd "$tmpdir"
  run bash -c "echo '{}' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
  rm -rf "$tmpdir"
}
