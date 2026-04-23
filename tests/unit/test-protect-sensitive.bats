#!/usr/bin/env bats

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/../../scripts/protect-sensitive-files.sh"
FIXTURES="$SCRIPT_DIR/fixtures"

@test "allows normal Python file edits" {
  run bash -c "cat '$FIXTURES/pretooluse-edit-normal.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "blocks .env file edits with exit 2" {
  run bash -c "cat '$FIXTURES/pretooluse-edit-env.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 2 ]
}

@test "blocks .pem file edits with exit 2" {
  run bash -c "cat '$FIXTURES/pretooluse-edit-pem.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 2 ]
}

@test "blocks lock file edits with exit 2" {
  run bash -c "cat '$FIXTURES/pretooluse-edit-lockfile.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 2 ]
}

@test "handles empty JSON gracefully" {
  run bash -c "cat '$FIXTURES/empty.json' | bash '$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "blocks credentials file" {
  run bash -c 'echo "{\"tool_input\":{\"file_path\":\"/tmp/credentials.json\"}}" | bash '"'$HOOK_SCRIPT'"
  [ "$status" -eq 2 ]
}

@test "blocks secrets directory file" {
  run bash -c 'echo "{\"tool_input\":{\"file_path\":\"/tmp/secrets.yaml\"}}" | bash '"'$HOOK_SCRIPT'"
  [ "$status" -eq 2 ]
}

@test "allows Dockerfile edits" {
  run bash -c 'echo "{\"tool_input\":{\"file_path\":\"/tmp/Dockerfile\"}}" | bash '"'$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "allows README edits" {
  run bash -c 'echo "{\"tool_input\":{\"file_path\":\"/tmp/README.md\"}}" | bash '"'$HOOK_SCRIPT'"
  [ "$status" -eq 0 ]
}

@test "blocks .env.production" {
  run bash -c 'echo "{\"tool_input\":{\"file_path\":\"/tmp/.env.production\"}}" | bash '"'$HOOK_SCRIPT'"
  [ "$status" -eq 2 ]
}
