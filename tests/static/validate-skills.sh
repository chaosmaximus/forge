#!/usr/bin/env bash
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== Skills Validation ==="

SKILL_COUNT=0
for skill_file in "$PLUGIN_ROOT"/skills/*/SKILL.md; do
  [ -f "$skill_file" ] || continue
  SKILL_COUNT=$((SKILL_COUNT + 1))
  skill_name=$(basename "$(dirname "$skill_file")")

  # 1. Frontmatter exists (starts with ---)
  if ! head -1 "$skill_file" | grep -q '^---$'; then
    echo "[FAIL] $skill_name: Missing YAML frontmatter opening ---"; ((errors++)); continue
  fi

  # 2. Frontmatter closes
  fm_close=$(sed -n '2,$ { /^---$/= }' "$skill_file" | head -1)
  if [ -z "$fm_close" ]; then
    echo "[FAIL] $skill_name: Missing YAML frontmatter closing ---"; ((errors++)); continue
  fi

  # 3. Has name field
  fm_name=$(sed -n "2,$((fm_close-1))p" "$skill_file" | grep '^name:' | head -1 | sed 's/name: *//')
  if [ -z "$fm_name" ]; then
    echo "[FAIL] $skill_name: Missing 'name' in frontmatter"; ((errors++))
  else
    echo "[PASS] $skill_name: name=$fm_name"
  fi

  # 4. Has description field
  if ! sed -n "2,$((fm_close-1))p" "$skill_file" | grep -q '^description:'; then
    echo "[FAIL] $skill_name: Missing 'description' in frontmatter"; ((errors++))
  else
    echo "[PASS] $skill_name: has description"
  fi

  # 5. Description doesn't leak workflow details (heuristic: check for step-like words)
  desc=$(sed -n "2,$((fm_close-1))p" "$skill_file" | sed -n '/^description:/,/^[a-z]/p' | head -5 | tr '\n' ' ')
  if echo "$desc" | grep -qiE 'then builds|guides through|runs final|generates PR|saves.*state'; then
    echo "[WARN] $skill_name: Description may contain workflow summary (should be trigger-only)"
  fi
done

echo "[INFO] Found $SKILL_COUNT skills"
[ "$SKILL_COUNT" -eq 8 ] || { echo "[WARN] Expected 8 skills, found $SKILL_COUNT"; }

[ $errors -eq 0 ] && echo "=== All skills checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
