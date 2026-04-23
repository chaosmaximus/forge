#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Static validation: agent .md files must NOT reference MCP tools (MCP removed in v0.3.0).
# Agents use Bash + forge CLI for all graph/memory operations.
set -euo pipefail

PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
errors=0

echo "=== Agent Tool Reference Validation (v0.3.0 — no MCP) ==="

for agent_file in "$PLUGIN_ROOT"/agents/*.md; do
  agent_name=$(basename "$agent_file")

  # Check for any remaining MCP tool references
  refs=$(grep -oP 'mcp__forge_forge-graph__\w+' "$agent_file" 2>/dev/null || true)
  if [ -n "$refs" ]; then
    echo "[FAIL] $agent_name still references MCP tools (removed in v0.3.0): $refs"
    errors=$((errors + 1))
  fi

  # Check that Bash is in tools list (needed for forge CLI)
  if grep -q "^tools:" "$agent_file" && ! grep "^tools:" "$agent_file" | grep -q "Bash"; then
    echo "[FAIL] $agent_name missing Bash in tools (needed for forge CLI)"
    errors=$((errors + 1))
  fi
done

if [ $errors -eq 0 ]; then
  echo "[PASS] All agent definitions are CLI-first (no MCP refs)"
else
  echo "=== $errors error(s) ==="
  exit 1
fi
