#!/usr/bin/env bash
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== Agents Validation ==="

AGENT_COUNT=0
for agent_file in "$PLUGIN_ROOT"/agents/*.md; do
  [ -f "$agent_file" ] || continue
  AGENT_COUNT=$((AGENT_COUNT + 1))
  agent_name=$(basename "$agent_file" .md)

  # 1. Frontmatter exists
  if ! head -1 "$agent_file" | grep -q '^---$'; then
    echo "[FAIL] $agent_name: Missing YAML frontmatter"; ((errors++)); continue
  fi

  fm_close=$(sed -n '2,$ { /^---$/= }' "$agent_file" | head -1)
  if [ -z "$fm_close" ]; then
    echo "[FAIL] $agent_name: Missing frontmatter closing ---"; ((errors++)); continue
  fi

  FM=$(sed -n "2,$((fm_close-1))p" "$agent_file")

  # 2. Required fields
  for field in name description model; do
    if ! echo "$FM" | grep -q "^${field}:"; then
      echo "[FAIL] $agent_name: Missing '$field'"; ((errors++))
    fi
  done

  # 3. Model is valid
  model=$(echo "$FM" | grep '^model:' | head -1 | sed 's/model: *//')
  if [ -n "$model" ] && ! echo "$model" | grep -qE '^(opus|sonnet|haiku|inherit)$'; then
    echo "[WARN] $agent_name: Unusual model value '$model'"
  else
    echo "[PASS] $agent_name: model=$model"
  fi

  # 4. Has color field (recommended)
  if ! echo "$FM" | grep -q "^color:"; then
    echo "[WARN] $agent_name: Missing 'color' (recommended for visual distinction)"
  else
    color=$(echo "$FM" | grep '^color:' | sed 's/color: *//')
    echo "[PASS] $agent_name: color=$color"
  fi

  # 5. Has tools field
  if echo "$FM" | grep -q "^tools:"; then
    echo "[PASS] $agent_name: has tools list"
  else
    echo "[WARN] $agent_name: No tools field (inherits all tools)"
  fi

  # 6. System prompt exists (content after frontmatter)
  body_lines=$(($(wc -l < "$agent_file") - fm_close))
  if [ "$body_lines" -lt 5 ]; then
    echo "[WARN] $agent_name: System prompt is very short ($body_lines lines)"
  else
    echo "[PASS] $agent_name: System prompt has $body_lines lines"
  fi
done

echo "[INFO] Found $AGENT_COUNT agents"
[ "$AGENT_COUNT" -eq 3 ] || { echo "[WARN] Expected 3 agents, found $AGENT_COUNT"; }

[ $errors -eq 0 ] && echo "=== All agent checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
