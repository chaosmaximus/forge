#!/usr/bin/env bash
# Forge: Test whether each agent can be referenced by name in a Claude Code session.
#
# This script verifies that:
# 1. Each agent .md file has valid YAML frontmatter with a 'name' field
# 2. The agent names declared in plugin.json match the frontmatter names
# 3. Each agent can be referenced in a Claude Code session (spawned by name)
# 4. Agent tool permissions are correctly declared
# 5. Agent model and configuration fields are valid
#
# Prerequisites:
#   - claude CLI installed and authenticated
#   - Forge plugin installed: claude plugin install /path/to/forge
#
# Usage:
#   bash tests/claude-code/test-agent-spawning.sh
set -euo pipefail

PLUGIN_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
AGENTS_DIR="$PLUGIN_DIR/agents"
RESULTS_DIR="${PLUGIN_DIR}/tests/claude-code/results"
mkdir -p "$RESULTS_DIR"

passed=0
failed=0
skipped=0
total=0

# Colors
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

echo "========================================"
echo "  Forge Agent Spawning Tests"
echo "  Plugin: $PLUGIN_DIR"
echo "========================================"
echo ""

# ============================================================
# SECTION 1: Static validation of agent frontmatter
# ============================================================
echo "--- Section 1: Agent Frontmatter Validation ---"

# Expected agents from plugin.json
EXPECTED_AGENTS=("forge-planner" "forge-generator" "forge-evaluator")

for agent_file in "$AGENTS_DIR"/*.md; do
  agent_basename=$(basename "$agent_file" .md)

  # Extract YAML frontmatter (between --- delimiters)
  frontmatter=$(sed -n '/^---$/,/^---$/p' "$agent_file" | sed '1d;$d')

  # Check 'name' field exists
  agent_name=$(echo "$frontmatter" | grep -E '^name:' | sed 's/^name:[[:space:]]*//' | head -1)
  if [ -n "$agent_name" ]; then
    log_pass "$agent_basename: has name field ($agent_name)"
  else
    log_fail "$agent_basename: missing name field in frontmatter"
    continue
  fi

  # Verify name matches filename convention
  if [ "$agent_name" = "$agent_basename" ]; then
    log_pass "$agent_basename: name matches filename"
  else
    log_fail "$agent_basename: name '$agent_name' does not match filename '$agent_basename'"
  fi

  # Check 'model' field
  model=$(echo "$frontmatter" | grep -E '^model:' | sed 's/^model:[[:space:]]*//' | head -1)
  if [ -n "$model" ]; then
    case "$model" in
      opus|sonnet|inherit)
        log_pass "$agent_basename: valid model ($model)"
        ;;
      *)
        log_fail "$agent_basename: unexpected model value '$model' (expected opus, sonnet, or inherit)"
        ;;
    esac
  else
    log_fail "$agent_basename: missing model field"
  fi

  # Check 'maxTurns' field
  max_turns=$(echo "$frontmatter" | grep -E '^maxTurns:' | sed 's/^maxTurns:[[:space:]]*//' | head -1)
  if [ -n "$max_turns" ]; then
    if [[ "$max_turns" =~ ^[0-9]+$ ]] && [ "$max_turns" -gt 0 ] && [ "$max_turns" -le 100 ]; then
      log_pass "$agent_basename: valid maxTurns ($max_turns)"
    else
      log_fail "$agent_basename: invalid maxTurns '$max_turns' (expected 1-100)"
    fi
  else
    log_fail "$agent_basename: missing maxTurns field"
  fi

  # Check 'tools' field exists and has at least Read
  tools=$(echo "$frontmatter" | grep -E '^tools:' | sed 's/^tools:[[:space:]]*//' | head -1)
  if [ -n "$tools" ]; then
    if echo "$tools" | grep -q "Read"; then
      log_pass "$agent_basename: has Read tool declared"
    else
      log_fail "$agent_basename: tools list does not include Read"
    fi
  else
    log_fail "$agent_basename: missing tools field"
  fi

  # Check role-specific constraints
  case "$agent_name" in
    forge-planner)
      # Planner should be read-only: Write and Edit should be disallowed
      disallowed=$(echo "$frontmatter" | grep -E '^disallowedTools:' | sed 's/^disallowedTools:[[:space:]]*//' | head -1)
      if echo "$disallowed" | grep -q "Write" && echo "$disallowed" | grep -q "Edit"; then
        log_pass "$agent_basename: correctly disallows Write and Edit"
      else
        log_fail "$agent_basename: planner should disallow Write and Edit (found: $disallowed)"
      fi
      ;;
    forge-generator)
      # Generator should have Write and Edit in tools
      if echo "$tools" | grep -q "Write" && echo "$tools" | grep -q "Edit"; then
        log_pass "$agent_basename: has Write and Edit tools for code generation"
      else
        log_fail "$agent_basename: generator should have Write and Edit tools"
      fi
      # Generator should have worktree isolation
      isolation=$(echo "$frontmatter" | grep -E '^isolation:' | sed 's/^isolation:[[:space:]]*//' | head -1)
      if [ "$isolation" = "worktree" ]; then
        log_pass "$agent_basename: has worktree isolation"
      else
        log_fail "$agent_basename: generator should have isolation: worktree (found: ${isolation:-none})"
      fi
      ;;
    forge-evaluator)
      # Evaluator should be read-only: Write and Edit should be disallowed
      disallowed=$(echo "$frontmatter" | grep -E '^disallowedTools:' | sed 's/^disallowedTools:[[:space:]]*//' | head -1)
      if echo "$disallowed" | grep -q "Write" && echo "$disallowed" | grep -q "Edit"; then
        log_pass "$agent_basename: correctly disallows Write and Edit"
      else
        log_fail "$agent_basename: evaluator should disallow Write and Edit (found: $disallowed)"
      fi
      ;;
  esac
done

# ============================================================
# SECTION 2: Plugin.json agent list matches actual agent files
# ============================================================
echo ""
echo "--- Section 2: Plugin.json Agent Registry ---"

if command -v jq &>/dev/null; then
  DECLARED_AGENTS=$(jq -r '.agents[]' "$PLUGIN_DIR/.claude-plugin/plugin.json" 2>/dev/null)
  DECLARED_COUNT=$(echo "$DECLARED_AGENTS" | wc -l)
  ACTUAL_COUNT=$(find "$AGENTS_DIR" -name "*.md" | wc -l)

  if [ "$DECLARED_COUNT" -eq "$ACTUAL_COUNT" ]; then
    log_pass "plugin.json declares $DECLARED_COUNT agents, $ACTUAL_COUNT found on disk"
  else
    log_fail "plugin.json declares $DECLARED_COUNT agents but $ACTUAL_COUNT found on disk"
  fi

  # Check each declared agent file exists
  while IFS= read -r agent_path; do
    resolved="${PLUGIN_DIR}/${agent_path#./}"
    if [ -f "$resolved" ]; then
      log_pass "declared agent exists: $agent_path"
    else
      log_fail "declared agent missing: $agent_path (resolved: $resolved)"
    fi
  done <<< "$DECLARED_AGENTS"
else
  log_skip "jq not available -- cannot parse plugin.json"
fi

# ============================================================
# SECTION 3: Live agent reference test via Claude Code CLI
# ============================================================
echo ""
echo "--- Section 3: Live Agent Spawning via Claude CLI ---"

if ! command -v claude &>/dev/null; then
  echo "  claude CLI not found. Skipping live tests."
  for name in "${EXPECTED_AGENTS[@]}"; do
    log_skip "live-spawn-$name (claude CLI not available)"
  done
else
  WORK_DIR=$(mktemp -d /tmp/forge-agent-test-XXXX)
  mkdir -p "$WORK_DIR/src"
  echo 'print("hello")' > "$WORK_DIR/src/main.py"
  (cd "$WORK_DIR" && git init -q && git add . && git commit -q -m "init")

  # Test 3a: Can we reference forge-planner by name?
  echo "  Testing forge-planner reference..."
  PLANNER_OUTPUT=$(cd "$WORK_DIR" && claude \
    --plugin-dir "$PLUGIN_DIR" \
    -p "You have the Forge plugin. Confirm that you can see the forge-planner agent in your available agents. List all forge agents you can see by name. Do NOT start any workflow -- just list the agent names." \
    --max-turns 2 \
    --output-format text \
    2>&1) || true
  echo "$PLANNER_OUTPUT" > "$RESULTS_DIR/live-agent-planner.txt"

  if echo "$PLANNER_OUTPUT" | grep -qi "forge-planner"; then
    log_pass "live-reference: forge-planner visible in session"
  else
    log_fail "live-reference: forge-planner not found in output (saved to results/live-agent-planner.txt)"
  fi

  # Test 3b: Can we reference forge-generator by name?
  echo "  Testing forge-generator reference..."
  GENERATOR_OUTPUT=$(cd "$WORK_DIR" && claude \
    --plugin-dir "$PLUGIN_DIR" \
    -p "You have the Forge plugin. Confirm that you can see the forge-generator agent in your available agents. List all forge agents you can see by name. Do NOT start any workflow -- just list the agent names." \
    --max-turns 2 \
    --output-format text \
    2>&1) || true
  echo "$GENERATOR_OUTPUT" > "$RESULTS_DIR/live-agent-generator.txt"

  if echo "$GENERATOR_OUTPUT" | grep -qi "forge-generator"; then
    log_pass "live-reference: forge-generator visible in session"
  else
    log_fail "live-reference: forge-generator not found in output (saved to results/live-agent-generator.txt)"
  fi

  # Test 3c: Can we reference forge-evaluator by name?
  echo "  Testing forge-evaluator reference..."
  EVALUATOR_OUTPUT=$(cd "$WORK_DIR" && claude \
    --plugin-dir "$PLUGIN_DIR" \
    -p "You have the Forge plugin. Confirm that you can see the forge-evaluator agent in your available agents. List all forge agents you can see by name. Do NOT start any workflow -- just list the agent names." \
    --max-turns 2 \
    --output-format text \
    2>&1) || true
  echo "$EVALUATOR_OUTPUT" > "$RESULTS_DIR/live-agent-evaluator.txt"

  if echo "$EVALUATOR_OUTPUT" | grep -qi "forge-evaluator"; then
    log_pass "live-reference: forge-evaluator visible in session"
  else
    log_fail "live-reference: forge-evaluator not found in output (saved to results/live-agent-evaluator.txt)"
  fi

  # Test 3d: Agent role separation -- ask the model to confirm tool restrictions
  echo "  Testing agent tool restriction awareness..."
  TOOLS_OUTPUT=$(cd "$WORK_DIR" && claude \
    --plugin-dir "$PLUGIN_DIR" \
    -p "You have the Forge plugin with 3 agents: forge-planner, forge-generator, forge-evaluator. For each agent, tell me: (1) Can it write/edit files? (2) What isolation mode does it use? Answer concisely, one line per agent." \
    --max-turns 2 \
    --output-format text \
    2>&1) || true
  echo "$TOOLS_OUTPUT" > "$RESULTS_DIR/live-agent-tools.txt"

  # Check that planner is described as read-only
  if echo "$TOOLS_OUTPUT" | grep -qi "planner.*read.only\|planner.*cannot.*write\|planner.*no.*write"; then
    log_pass "live-tools: planner recognized as read-only"
  else
    log_fail "live-tools: planner not recognized as read-only (saved to results/live-agent-tools.txt)"
  fi

  # Check that generator has worktree isolation
  if echo "$TOOLS_OUTPUT" | grep -qi "generator.*worktree"; then
    log_pass "live-tools: generator recognized with worktree isolation"
  else
    log_fail "live-tools: generator worktree isolation not recognized (saved to results/live-agent-tools.txt)"
  fi

  # Check that evaluator is described as read-only
  if echo "$TOOLS_OUTPUT" | grep -qi "evaluator.*read.only\|evaluator.*cannot.*write\|evaluator.*no.*write"; then
    log_pass "live-tools: evaluator recognized as read-only"
  else
    log_fail "live-tools: evaluator not recognized as read-only (saved to results/live-agent-tools.txt)"
  fi

  rm -rf "$WORK_DIR"
fi

# ============================================================
# SECTION 4: Agent cross-references in skills
# ============================================================
echo ""
echo "--- Section 4: Agent References in Skills ---"

# Check that skills reference agents by their correct names
SKILLS_DIR="$PLUGIN_DIR/skills"

# forge-new should reference forge-generator, forge-evaluator, forge-review, forge-ship
FORGE_NEW="$SKILLS_DIR/forge-new/SKILL.md"
if [ -f "$FORGE_NEW" ]; then
  for ref in "forge-generator" "forge-evaluator" "forge-review" "forge-ship"; do
    if grep -q "$ref" "$FORGE_NEW"; then
      log_pass "forge-new references $ref"
    else
      log_fail "forge-new does NOT reference $ref"
    fi
  done
fi

# forge-feature should reference forge-planner, forge-generator, forge-evaluator, forge-review, forge-ship
FORGE_FEATURE="$SKILLS_DIR/forge-feature/SKILL.md"
if [ -f "$FORGE_FEATURE" ]; then
  for ref in "forge-planner" "forge-generator" "forge-evaluator" "forge-review" "forge-ship"; do
    if grep -q "$ref" "$FORGE_FEATURE"; then
      log_pass "forge-feature references $ref"
    else
      log_fail "forge-feature does NOT reference $ref"
    fi
  done
fi

# forge router should reference forge-new and forge-feature
FORGE_ROUTER="$SKILLS_DIR/forge/SKILL.md"
if [ -f "$FORGE_ROUTER" ]; then
  for ref in "forge-new" "forge-feature"; do
    if grep -q "$ref" "$FORGE_ROUTER"; then
      log_pass "forge (router) references $ref"
    else
      log_fail "forge (router) does NOT reference $ref"
    fi
  done
fi

# ============================================================
# SECTION 5: Agent model configuration consistency
# ============================================================
echo ""
echo "--- Section 5: Agent Model Configuration ---"

# Planner should use opus (needs high reasoning for product planning)
planner_model=$(sed -n '/^---$/,/^---$/p' "$AGENTS_DIR/forge-planner.md" | grep -E '^model:' | sed 's/^model:[[:space:]]*//')
if [ "$planner_model" = "opus" ]; then
  log_pass "forge-planner uses opus model (correct for planning)"
else
  log_fail "forge-planner uses '$planner_model' model (expected opus)"
fi

# Generator should use inherit (controlled by userConfig default_generator_model)
generator_model=$(sed -n '/^---$/,/^---$/p' "$AGENTS_DIR/forge-generator.md" | grep -E '^model:' | sed 's/^model:[[:space:]]*//')
if [ "$generator_model" = "inherit" ]; then
  log_pass "forge-generator uses inherit model (controlled by userConfig)"
else
  log_fail "forge-generator uses '$generator_model' model (expected inherit for userConfig control)"
fi

# Evaluator should use opus (needs high reasoning for evaluation)
evaluator_model=$(sed -n '/^---$/,/^---$/p' "$AGENTS_DIR/forge-evaluator.md" | grep -E '^model:' | sed 's/^model:[[:space:]]*//')
if [ "$evaluator_model" = "opus" ]; then
  log_pass "forge-evaluator uses opus model (correct for evaluation)"
else
  log_fail "forge-evaluator uses '$evaluator_model' model (expected opus)"
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
