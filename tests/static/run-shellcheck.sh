#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"

echo "=== ShellCheck Analysis ==="

if ! command -v shellcheck &>/dev/null; then
  echo "[SKIP] shellcheck not installed (apt install shellcheck)"
  exit 0
fi

errors=0
# Scan scripts/ (incl. scripts/hooks/) plus tests/scripts/ — the latter carries
# fixture/integration test runners that ship as part of the public surface
# (run from CI, called by operators reproducing dogfood failures).
while IFS= read -r script; do
  name="${script#"$PLUGIN_ROOT"/}"
  if shellcheck -x -S warning "$script" 2>&1; then
    echo "[PASS] $name"
  else
    echo "[FAIL] $name has shellcheck warnings"
    errors=$((errors + 1))
  fi
done < <(
  {
    find "$PLUGIN_ROOT/scripts" -maxdepth 2 -type f -name '*.sh'
    find "$PLUGIN_ROOT/tests/scripts" -maxdepth 1 -type f -name '*.sh' 2>/dev/null
  } | sort
)

[ $errors -eq 0 ] && echo "=== All scripts pass shellcheck ===" || { echo "=== $errors script(s) have warnings ==="; exit 1; }
