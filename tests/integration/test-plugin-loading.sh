#!/usr/bin/env bash
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"

echo "=== Plugin Loading Smoke Test ==="
errors=0

# 1. Required directories
for dir in skills agents hooks scripts data evaluation-criteria templates; do
  if [ -d "$PLUGIN_ROOT/$dir" ]; then
    echo "[PASS] $dir/ exists"
  else
    echo "[FAIL] $dir/ missing"; errors=$((errors + 1))
  fi
done

# 2. Expected skill count
SKILL_COUNT=$(find "$PLUGIN_ROOT/skills" -name "SKILL.md" | wc -l)
if [ "$SKILL_COUNT" -eq 7 ]; then
  echo "[PASS] Found $SKILL_COUNT skills"
else
  echo "[FAIL] Expected 7 skills, found $SKILL_COUNT"; errors=$((errors + 1))
fi

# 3. Expected agent count
AGENT_COUNT=$(find "$PLUGIN_ROOT/agents" -name "*.md" | wc -l)
if [ "$AGENT_COUNT" -eq 3 ]; then
  echo "[PASS] Found $AGENT_COUNT agents"
else
  echo "[FAIL] Expected 3 agents, found $AGENT_COUNT"; errors=$((errors + 1))
fi

# 4. All scripts have valid bash syntax
for script in "$PLUGIN_ROOT/scripts/"*.sh; do
  name=$(basename "$script")
  if bash -n "$script" 2>/dev/null; then
    echo "[PASS] $name syntax OK"
  else
    echo "[FAIL] $name has syntax errors"; errors=$((errors + 1))
  fi
done

# 5. All scripts are executable
for script in "$PLUGIN_ROOT/scripts/"*.sh; do
  name=$(basename "$script")
  if [ -x "$script" ]; then
    echo "[PASS] $name is executable"
  else
    echo "[FAIL] $name is not executable"; errors=$((errors + 1))
  fi
done

# 6. CSV data files exist with content
for csv in project-types.csv domain-complexity.csv; do
  if [ -f "$PLUGIN_ROOT/data/$csv" ] && [ "$(wc -l < "$PLUGIN_ROOT/data/$csv")" -gt 1 ]; then
    echo "[PASS] data/$csv has content"
  else
    echo "[FAIL] data/$csv missing or empty"; errors=$((errors + 1))
  fi
done

# 7. Evaluation criteria exist
for rubric in code-quality.md security.md architecture.md infrastructure.md; do
  if [ -f "$PLUGIN_ROOT/evaluation-criteria/$rubric" ]; then
    echo "[PASS] evaluation-criteria/$rubric exists"
  else
    echo "[FAIL] evaluation-criteria/$rubric missing"; errors=$((errors + 1))
  fi
done

# 8. Templates exist
for tmpl in CONSTITUTION.md STATE.md HANDOFF.md PRD.md; do
  if [ -f "$PLUGIN_ROOT/templates/$tmpl" ]; then
    echo "[PASS] templates/$tmpl exists"
  else
    echo "[FAIL] templates/$tmpl missing"; errors=$((errors + 1))
  fi
done

echo ""
[ $errors -eq 0 ] && echo "=== All loading checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
