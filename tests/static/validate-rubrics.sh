#!/usr/bin/env bash
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== Evaluation Criteria Validation ==="

for rubric_file in "$PLUGIN_ROOT"/evaluation-criteria/*.md; do
  [ -f "$rubric_file" ] || continue
  name=$(basename "$rubric_file" .md)

  # 1. Has scoring levels (1-5)
  if grep -q -- '- 1:' "$rubric_file" && grep -q -- '- 5:' "$rubric_file"; then
    echo "[PASS] $name: has 1-5 scoring levels"
  else
    echo "[FAIL] $name: missing 1-5 scoring levels"; errors=$((errors + 1))
  fi

  # 2. Has weight indicators
  if grep -q 'weight:' "$rubric_file"; then
    echo "[PASS] $name: has weight indicators"
  else
    echo "[FAIL] $name: missing weight indicators"; errors=$((errors + 1))
  fi

  # 3. Has pass threshold
  if grep -qi 'pass threshold\|pass:' "$rubric_file"; then
    echo "[PASS] $name: has pass threshold"
  else
    echo "[FAIL] $name: missing pass threshold"; errors=$((errors + 1))
  fi

  # 4. Has auto-fail conditions
  if grep -qi 'auto.fail\|automatic.*fail\|FAIL' "$rubric_file"; then
    echo "[PASS] $name: has auto-fail conditions"
  else
    echo "[WARN] $name: no auto-fail conditions found"
  fi

  # 5. Has at least 3 criteria (## headings excluding Pass Threshold)
  criteria_count=$(grep -c '^## ' "$rubric_file" | head -1)
  criteria_count=$((criteria_count - 1))  # subtract the Pass Threshold heading
  if [ "$criteria_count" -ge 3 ]; then
    echo "[PASS] $name: $criteria_count criteria"
  else
    echo "[WARN] $name: only $criteria_count criteria (expected >= 3)"
  fi
done

RUBRIC_COUNT=$(find "$PLUGIN_ROOT/evaluation-criteria" -name "*.md" | wc -l)
echo "[INFO] Found $RUBRIC_COUNT rubrics"
[ "$RUBRIC_COUNT" -eq 4 ] || echo "[WARN] Expected 4 rubrics, found $RUBRIC_COUNT"

[ $errors -eq 0 ] && echo "=== All rubric checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
