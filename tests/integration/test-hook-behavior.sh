#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== Hook Behavior Integration Test ==="

# Helper: test a hook script with input and expected exit code
test_hook() {
  local name="$1" script="$2" input="$3" expected_exit="$4" description="$5"
  actual_exit=0
  echo "$input" | bash "$script" >/dev/null 2>&1 || actual_exit=$?
  if [ "$actual_exit" -eq "$expected_exit" ]; then
    echo "[PASS] $name: $description (exit=$actual_exit)"
  else
    echo "[FAIL] $name: $description (expected exit=$expected_exit, got=$actual_exit)"
    errors=$((errors + 1))
  fi
}

# --- protect-sensitive-files.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/protect-sensitive-files.sh"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/main.py"}}' 0 "allows normal files"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/.env"}}' 2 "blocks .env"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/.env.local"}}' 2 "blocks .env.local"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/server.pem"}}' 2 "blocks .pem"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/credentials.json"}}' 2 "blocks credentials"
test_hook "protect" "$SCRIPT" '{"tool_input":{"file_path":"/tmp/poetry.lock"}}' 2 "blocks lock files"
test_hook "protect" "$SCRIPT" '{}' 0 "handles empty input"

# --- teammate-idle-checkpoint.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/teammate-idle-checkpoint.sh"
test_hook "idle" "$SCRIPT" '{"teammate_name":"gen-1"}' 0 "exits 0 normally"

# --- task-completed-gate.sh (in a temp git repo) ---
SCRIPT="$PLUGIN_ROOT/scripts/task-completed-gate.sh"
TMPDIR=$(mktemp -d /tmp/forge-hook-test-XXXX)
cd "$TMPDIR"
git init -q && echo "init" > README.md && git add . && git commit -q -m "init"
test_hook "taskgate" "$SCRIPT" '{"task_subject":"test task"}' 0 "passes with no test framework"

# With failing tests
echo '{"scripts":{"test":"exit 1"}}' > package.json
test_hook "taskgate" "$SCRIPT" '{"task_subject":"test task"}' 2 "blocks when npm tests fail"

# With passing tests
echo '{"scripts":{"test":"echo ok"}}' > package.json
test_hook "taskgate" "$SCRIPT" '{"task_subject":"test task"}' 0 "passes when npm tests pass"

rm -rf "$TMPDIR"
cd "$PLUGIN_ROOT"

# --- forge-graph-start.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/forge-graph-start.sh"
export CLAUDE_PLUGIN_ROOT="$PLUGIN_ROOT"
output=$(bash "$SCRIPT" < /dev/null 2>/dev/null || true)
if echo "$output" | grep -q "hookSpecificOutput"; then
  echo "[PASS] forge-graph-start: outputs valid context injection JSON"
else
  echo "[FAIL] forge-graph-start: no valid context injection JSON"; errors=$((errors + 1))
fi

# --- session-end-graph.sh ---
SCRIPT="$PLUGIN_ROOT/scripts/session-end-graph.sh"
test_hook "session-end" "$SCRIPT" '{}' 0 "exits 0 gracefully"

# --- session-start.sh stdin-JSON parsing (P3-4 Wave Y Y1 / cc-voice §C) ---
# Pre-Y1, the hook discarded stdin and fell back to env vars Claude
# Code doesn't export, so SESSION_ID became a Unix timestamp and
# --cwd was never passed to compile-context. Verifies stdin-first
# precedence: a sample SessionStart JSON payload must surface
# session_id and cwd in the FORGE_HOOK_VERBOSE log lines.
SCRIPT="$PLUGIN_ROOT/scripts/hooks/session-start.sh"
WAVE_Y_TMP=$(mktemp -d /tmp/forge-y1-stdin-XXXX)
# Stub `forge-next` with a no-op so the hook doesn't try to talk to a
# real daemon. The test only checks that the hook would have passed
# the right values — the daemon-side wiring is exercised by Y2's
# Rust-level tests separately.
cat > "$WAVE_Y_TMP/forge-next" <<'STUB'
#!/usr/bin/env bash
# Forge Y1 test stub — succeeds silently, ignores all args.
exit 0
STUB
chmod +x "$WAVE_Y_TMP/forge-next"
# Sample payload with an obviously-not-a-timestamp UUID and a CWD that
# differs from $PWD (so we can prove stdin won, not env or PWD fallback).
Y1_PAYLOAD='{"session_id":"467e15b8-7fa0-478c-8e1e-8ce71809aa27","cwd":"/tmp/forge-y1-stdin-cwd","hook_event_name":"SessionStart","source":"startup","model":"claude-opus-4-7"}'
Y1_LOG=$(echo "$Y1_PAYLOAD" | env -i \
  PATH="$WAVE_Y_TMP:$PATH" \
  HOME="$HOME" \
  PWD="$PLUGIN_ROOT" \
  FORGE_HOOK_VERBOSE=1 \
  bash "$SCRIPT" 2>&1 >/dev/null || true)

if echo "$Y1_LOG" | grep -q "id=467e15b8-7fa0-478c-8e1e-8ce71809aa27"; then
  echo "[PASS] session-start: stdin session_id wins over env/timestamp fallback"
else
  echo "[FAIL] session-start: stdin session_id was discarded; got log: $Y1_LOG"
  errors=$((errors + 1))
fi
if echo "$Y1_LOG" | grep -q "cwd=/tmp/forge-y1-stdin-cwd"; then
  echo "[PASS] session-start: stdin cwd wins over env/PWD fallback"
else
  echo "[FAIL] session-start: stdin cwd was discarded; got log: $Y1_LOG"
  errors=$((errors + 1))
fi

# Negative case: empty stdin → falls through to PWD, NOT a timestamp
# fallback for cwd. (SESSION_ID still timestamp-falls-back since we
# have no UUID source in this scenario — that's intentional.)
Y1_FALLBACK_LOG=$(echo "" | env -i \
  PATH="$WAVE_Y_TMP:$PATH" \
  HOME="$HOME" \
  PWD="$PLUGIN_ROOT" \
  FORGE_HOOK_VERBOSE=1 \
  bash "$SCRIPT" 2>&1 >/dev/null || true)
if echo "$Y1_FALLBACK_LOG" | grep -q "cwd=$PLUGIN_ROOT"; then
  echo "[PASS] session-start: empty stdin falls back to PWD for cwd"
else
  echo "[FAIL] session-start: empty stdin did not fall back to PWD; got log: $Y1_FALLBACK_LOG"
  errors=$((errors + 1))
fi

rm -rf "$WAVE_Y_TMP"

echo ""
if [ $errors -eq 0 ]; then
  echo "=== All hook behavior tests passed ==="
else
  echo "=== $errors error(s) ==="
  exit 1
fi
