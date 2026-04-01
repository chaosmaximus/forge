#!/usr/bin/env bash
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== CSV Data Validation ==="

# project-types.csv
PT="$PLUGIN_ROOT/data/project-types.csv"
if [ ! -f "$PT" ]; then
  echo "[FAIL] project-types.csv not found"; errors=$((errors + 1))
else
  # Check header
  HEADER=$(head -1 "$PT")
  EXPECTED="type,detection_signals,key_questions,required_sections,skip_sections"
  if [ "$HEADER" = "$EXPECTED" ]; then
    echo "[PASS] project-types.csv: header matches"
  else
    echo "[FAIL] project-types.csv: unexpected header"; errors=$((errors + 1))
    echo "  Expected: $EXPECTED"
    echo "  Got:      $HEADER"
  fi

  # Check row count (should have at least 7 data rows)
  ROWS=$(($(wc -l < "$PT") - 1))
  if [ "$ROWS" -ge 7 ]; then
    echo "[PASS] project-types.csv: $ROWS data rows"
  else
    echo "[WARN] project-types.csv: only $ROWS rows (expected >= 7)"
  fi

  # Check no empty type field
  if awk -F',' 'NR>1 && $1==""' "$PT" | grep -q .; then
    echo "[FAIL] project-types.csv: empty type field found"; errors=$((errors + 1))
  fi
fi

# domain-complexity.csv
DC="$PLUGIN_ROOT/data/domain-complexity.csv"
if [ ! -f "$DC" ]; then
  echo "[FAIL] domain-complexity.csv not found"; errors=$((errors + 1))
else
  HEADER=$(head -1 "$DC")
  EXPECTED="domain,complexity,key_concerns,special_sections"
  if [ "$HEADER" = "$EXPECTED" ]; then
    echo "[PASS] domain-complexity.csv: header matches"
  else
    echo "[FAIL] domain-complexity.csv: unexpected header"; errors=$((errors + 1))
  fi

  ROWS=$(($(wc -l < "$DC") - 1))
  if [ "$ROWS" -ge 6 ]; then
    echo "[PASS] domain-complexity.csv: $ROWS data rows"
  else
    echo "[WARN] domain-complexity.csv: only $ROWS rows (expected >= 6)"
  fi
fi

[ $errors -eq 0 ] && echo "=== All CSV checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
