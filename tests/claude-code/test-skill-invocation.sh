#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge: Test skill invocation via Claude Code CLI
# Tests each skill's description-based auto-invocation and direct invocation.
#
# Prerequisites:
#   - claude CLI installed and authenticated
#   - Forge plugin installed: claude plugin install /path/to/forge
#
# Usage:
#   bash tests/claude-code/test-skill-invocation.sh
#
# Each test runs claude in non-interactive mode with --max-turns to prevent runaway.
# Output is captured and checked for expected keywords.
set -euo pipefail

PLUGIN_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
RESULTS_DIR="${PLUGIN_DIR}/tests/claude-code/results"
mkdir -p "$RESULTS_DIR"

passed=0
failed=0
skipped=0
total=0

# Colors (only if terminal supports it)
if [ -t 1 ]; then
  GREEN='\033[0;32m'
  RED='\033[0;31m'
  YELLOW='\033[0;33m'
  NC='\033[0m'
else
  GREEN='' RED='' YELLOW='' NC=''
fi

log_pass() { echo -e "${GREEN}[PASS]${NC} $1"; passed=$((passed + 1)); total=$((total + 1)); }
log_fail() { echo -e "${RED}[FAIL]${NC} $1"; failed=$((failed + 1)); total=$((total + 1)); }
log_skip() { echo -e "${YELLOW}[SKIP]${NC} $1"; skipped=$((skipped + 1)); total=$((total + 1)); }

# Check prerequisites
if ! command -v claude &>/dev/null; then
  echo "ERROR: claude CLI not found in PATH. Install Claude Code first."
  exit 1
fi

CLAUDE_VERSION=$(claude --version 2>/dev/null || echo "unknown")
echo "========================================"
echo "  Forge Skill Invocation Tests"
echo "  Claude version: $CLAUDE_VERSION"
echo "  Plugin: $PLUGIN_DIR"
echo "========================================"
echo ""

# Helper: run a test with claude CLI
# Arguments: test_name, working_dir, prompt, max_turns, expected_keywords (pipe-separated)
run_skill_test() {
  local test_name="$1"
  local work_dir="$2"
  local prompt="$3"
  local max_turns="$4"
  local expected="$5"
  local output_file="$RESULTS_DIR/${test_name}.txt"

  echo "--- Testing: $test_name ---"

  # Run claude with plugin-dir and capture output
  local exit_code=0
  local output=""
  output=$(cd "$work_dir" && claude \
    --plugin-dir "$PLUGIN_DIR" \
    -p "$prompt" \
    --max-turns "$max_turns" \
    --output-format text \
    2>&1) || exit_code=$?

  # Save output for inspection
  echo "$output" > "$output_file"

  # Check for expected keywords (pipe-separated: "keyword1|keyword2|keyword3")
  local all_found=true
  IFS='|' read -ra KEYWORDS <<< "$expected"
  for keyword in "${KEYWORDS[@]}"; do
    keyword=$(echo "$keyword" | xargs)  # trim whitespace
    if ! echo "$output" | grep -qi "$keyword"; then
      echo "  Missing keyword: '$keyword'"
      all_found=false
    fi
  done

  if [ "$all_found" = true ]; then
    log_pass "$test_name"
  else
    log_fail "$test_name (output saved to $output_file)"
  fi
}

# ============================================================
# TEST 1: Forge router in empty directory (should suggest greenfield)
# ============================================================
EMPTY_DIR=$(mktemp -d /tmp/forge-test-empty-XXXX)

run_skill_test \
  "router-empty-dir" \
  "$EMPTY_DIR" \
  "I want to build a new REST API for a payment processing system. What should I do?" \
  3 \
  "greenfield|forge"

rm -rf "$EMPTY_DIR"

# ============================================================
# TEST 2: Forge router in directory with existing code (should suggest existing)
# ============================================================
EXISTING_DIR=$(mktemp -d /tmp/forge-test-existing-XXXX)
mkdir -p "$EXISTING_DIR/src"
cat > "$EXISTING_DIR/src/main.py" << 'PYEOF'
from flask import Flask
app = Flask(__name__)

@app.route("/api/health")
def health():
    return {"status": "ok"}

if __name__ == "__main__":
    app.run()
PYEOF
cat > "$EXISTING_DIR/requirements.txt" << 'REQEOF'
flask==3.0.0
pytest==8.0.0
REQEOF
# Initialize git so forge recognizes it as a project
(cd "$EXISTING_DIR" && git init -q && git add . && git commit -q -m "init")

run_skill_test \
  "router-existing-code" \
  "$EXISTING_DIR" \
  "I want to add user authentication to this Flask API. What approach should we use?" \
  3 \
  "existing|forge"

rm -rf "$EXISTING_DIR"

# ============================================================
# TEST 3: Forge setup checks (direct invocation)
# ============================================================
SETUP_DIR=$(mktemp -d /tmp/forge-test-setup-XXXX)
(cd "$SETUP_DIR" && git init -q && echo "init" > README.md && git add . && git commit -q -m "init")

run_skill_test \
  "setup-checks" \
  "$SETUP_DIR" \
  "/forge:setup -- Check all prerequisites and report status." \
  5 \
  "prerequisite|setup|agent teams"

rm -rf "$SETUP_DIR"

# ============================================================
# TEST 4: forge-new description triggers on greenfield prompt
# ============================================================
NEW_DIR=$(mktemp -d /tmp/forge-test-new-XXXX)
(cd "$NEW_DIR" && git init -q && echo "init" > README.md && git add . && git commit -q -m "init")

run_skill_test \
  "forge-new-trigger" \
  "$NEW_DIR" \
  "Build a new e-commerce platform from scratch with product catalog, shopping cart, and checkout." \
  3 \
  "greenfield|classify|project"

rm -rf "$NEW_DIR"

# ============================================================
# TEST 5: forge-feature description triggers on existing code prompt
# ============================================================
FEATURE_DIR=$(mktemp -d /tmp/forge-test-feature-XXXX)
mkdir -p "$FEATURE_DIR/src"
cat > "$FEATURE_DIR/src/app.ts" << 'TSEOF'
import express from "express";
const app = express();
app.get("/api/users", (req, res) => { res.json([]); });
app.listen(3000);
TSEOF
cat > "$FEATURE_DIR/package.json" << 'PKGEOF'
{ "name": "test-app", "scripts": { "test": "echo ok" } }
PKGEOF
(cd "$FEATURE_DIR" && git init -q && git add . && git commit -q -m "init")

run_skill_test \
  "forge-feature-trigger" \
  "$FEATURE_DIR" \
  "Add rate limiting middleware to the Express API in this codebase." \
  3 \
  "existing|explore|codebase"

rm -rf "$FEATURE_DIR"

# ============================================================
# TEST 6: forge-review description does NOT trigger on build prompt
# ============================================================
REVIEW_DIR=$(mktemp -d /tmp/forge-test-review-XXXX)
(cd "$REVIEW_DIR" && git init -q && echo "init" > README.md && git add . && git commit -q -m "init")

run_skill_test \
  "forge-review-no-false-trigger" \
  "$REVIEW_DIR" \
  "Write a simple hello world function in Python." \
  2 \
  "hello|world|def"

rm -rf "$REVIEW_DIR"

# ============================================================
# TEST 7: forge-handoff description triggers on pause/end prompt
# ============================================================
HANDOFF_DIR=$(mktemp -d /tmp/forge-test-handoff-XXXX)
(cd "$HANDOFF_DIR" && git init -q && echo "init" > README.md && git add . && git commit -q -m "init")
# Create a STATE.md to simulate in-progress work
cat > "$HANDOFF_DIR/STATE.md" << 'STEOF'
# Forge State
**Last updated:** 2026-04-01T10:00:00+00:00
**Current phase:** build
**Mode:** greenfield
**Active branch:** feat/new-api
STEOF
(cd "$HANDOFF_DIR" && git add STATE.md && git commit -q -m "add state")

run_skill_test \
  "forge-handoff-trigger" \
  "$HANDOFF_DIR" \
  "I need to pause this work and come back later. Save the current state." \
  3 \
  "handoff|checkpoint|state"

rm -rf "$HANDOFF_DIR"

# ============================================================
# TEST 8: Session start hook injects forge context
# ============================================================
echo "--- Testing: session-start-context-injection ---"
SESSION_OUTPUT=$(bash "$PLUGIN_DIR/scripts/forge-graph-start.sh" < /dev/null 2>/dev/null || true)
if echo "$SESSION_OUTPUT" | grep -q "hookSpecificOutput" && echo "$SESSION_OUTPUT" | grep -q "forge"; then
  log_pass "session-start-context-injection"
else
  log_fail "session-start-context-injection"
fi

# ============================================================
# TEST 9: Sensitive file protection blocks .env
# ============================================================
echo "--- Testing: protect-blocks-env ---"
PROTECT_EXIT=0
echo '{"tool_input":{"file_path":"/tmp/.env"}}' | bash "$PLUGIN_DIR/scripts/protect-sensitive-files.sh" >/dev/null 2>&1 || PROTECT_EXIT=$?
if [ "$PROTECT_EXIT" -eq 2 ]; then
  log_pass "protect-blocks-env"
else
  log_fail "protect-blocks-env (expected exit 2, got $PROTECT_EXIT)"
fi

# ============================================================
# TEST 10: Sensitive file protection allows normal files
# ============================================================
echo "--- Testing: protect-allows-normal ---"
PROTECT_EXIT=0
echo '{"tool_input":{"file_path":"/tmp/main.py"}}' | bash "$PLUGIN_DIR/scripts/protect-sensitive-files.sh" >/dev/null 2>&1 || PROTECT_EXIT=$?
if [ "$PROTECT_EXIT" -eq 0 ]; then
  log_pass "protect-allows-normal"
else
  log_fail "protect-allows-normal (expected exit 0, got $PROTECT_EXIT)"
fi

# ============================================================
# RESULTS
# ============================================================
echo ""
echo "========================================"
echo "  Results: $passed passed, $failed failed, $skipped skipped (of $total)"
echo "  Detailed output saved to: $RESULTS_DIR/"
echo "========================================"

[ $failed -eq 0 ] || exit 1
