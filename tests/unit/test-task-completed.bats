#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/task-completed-gate.sh"
FIXTURES="$SCRIPT_DIR/fixtures"

setup() {
  TEST_DIR=$(mktemp -d /tmp/forge-test-XXXX)
  cd "$TEST_DIR"
  git init -q
  echo "init" > README.md
  git add . && git commit -q -m "init"
}

teardown() {
  rm -rf "$TEST_DIR"
}

@test "exits 0 when no test framework detected" {
  run bash -c "cat '$FIXTURES/taskcompleted-basic.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "exits 0 with empty JSON" {
  run bash -c "cat '$FIXTURES/empty.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "exits 2 when npm tests fail" {
  echo '{"scripts":{"test":"exit 1"}}' > package.json
  run bash -c "cat '$FIXTURES/taskcompleted-basic.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 2 ]
}

@test "exits 0 when npm tests pass" {
  echo '{"scripts":{"test":"echo ok"}}' > package.json
  run bash -c "cat '$FIXTURES/taskcompleted-basic.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}
