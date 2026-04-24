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
# Scan both scripts/ and scripts/hooks/ — the latter carries the agent-facing
# hook surface and used to be excluded by the top-level glob (2P-1b §10).
while IFS= read -r script; do
  name="${script#"$PLUGIN_ROOT"/}"
  if shellcheck -x -S warning "$script" 2>&1; then
    echo "[PASS] $name"
  else
    echo "[FAIL] $name has shellcheck warnings"
    errors=$((errors + 1))
  fi
done < <(find "$PLUGIN_ROOT/scripts" -maxdepth 2 -type f -name '*.sh' | sort)

[ $errors -eq 0 ] && echo "=== All scripts pass shellcheck ===" || { echo "=== $errors script(s) have warnings ==="; exit 1; }
