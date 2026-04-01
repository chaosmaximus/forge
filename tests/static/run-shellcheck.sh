#!/usr/bin/env bash
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"

echo "=== ShellCheck Analysis ==="

if ! command -v shellcheck &>/dev/null; then
  echo "[SKIP] shellcheck not installed (apt install shellcheck)"
  exit 0
fi

errors=0
for script in "$PLUGIN_ROOT"/scripts/*.sh; do
  name=$(basename "$script")
  if shellcheck -x -S warning "$script" 2>&1; then
    echo "[PASS] $name"
  else
    echo "[FAIL] $name has shellcheck warnings"
    errors=$((errors + 1))
  fi
done

[ $errors -eq 0 ] && echo "=== All scripts pass shellcheck ===" || { echo "=== $errors script(s) have warnings ==="; exit 1; }
