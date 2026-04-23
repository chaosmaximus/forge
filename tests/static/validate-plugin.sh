#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
PLUGIN_JSON="$PLUGIN_ROOT/.claude-plugin/plugin.json"
errors=0

echo "=== Plugin.json Validation ==="

# 1. Valid JSON
if ! jq empty "$PLUGIN_JSON" 2>/dev/null; then
  echo "[FAIL] Invalid JSON syntax"; exit 1
fi
echo "[PASS] Valid JSON"

# 2. Required field: name
NAME=$(jq -r '.name // empty' "$PLUGIN_JSON")
if [ -z "$NAME" ]; then echo "[FAIL] Missing name"; errors=$((errors + 1)); else echo "[PASS] name: $NAME"; fi

# 3. Name format (kebab-case)
if ! [[ "$NAME" =~ ^[a-z][a-z0-9-]*$ ]]; then
  echo "[FAIL] name must be kebab-case lowercase"; errors=$((errors + 1))
else
  echo "[PASS] name format OK"
fi

# 4. Version is semver
VERSION=$(jq -r '.version // empty' "$PLUGIN_JSON")
if [ -n "$VERSION" ] && ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
  echo "[FAIL] version not semver: $VERSION"; errors=$((errors + 1))
else
  echo "[PASS] version: $VERSION"
fi

# 5. Component paths exist (agents can be array of files or directory string)
for field in agents skills; do
  field_type=$(jq -r ".$field | type" "$PLUGIN_JSON" 2>/dev/null)
  if [ "$field_type" = "array" ]; then
    # Array of file paths — check each file exists
    all_exist=true
    for file in $(jq -r ".$field[]" "$PLUGIN_JSON"); do
      if [ ! -f "$PLUGIN_ROOT/$file" ]; then
        echo "[FAIL] $field file not found: $file"; errors=$((errors + 1)); all_exist=false
      fi
    done
    [ "$all_exist" = "true" ] && echo "[PASS] $field files all exist"
  elif [ "$field_type" = "string" ]; then
    path=$(jq -r ".$field" "$PLUGIN_JSON")
    if [ -n "$path" ] && [ ! -d "$PLUGIN_ROOT/$path" ]; then
      echo "[FAIL] $field path not found: $path"; errors=$((errors + 1))
    else
      echo "[PASS] $field directory exists"
    fi
  else
    echo "[PASS] $field not specified (auto-discovery)"
  fi
done

# 6. Hooks file exists
HOOKS_PATH=$(jq -r '.hooks // empty' "$PLUGIN_JSON")
if [ -n "$HOOKS_PATH" ] && [ ! -f "$PLUGIN_ROOT/$HOOKS_PATH" ]; then
  echo "[FAIL] hooks file not found: $HOOKS_PATH"; errors=$((errors + 1))
else
  echo "[PASS] hooks file exists"
fi

# 7. Hooks JSON is valid
if [ -n "$HOOKS_PATH" ] && [ -f "$PLUGIN_ROOT/$HOOKS_PATH" ]; then
  if ! jq empty "$PLUGIN_ROOT/$HOOKS_PATH" 2>/dev/null; then
    echo "[FAIL] hooks.json is invalid JSON"; errors=$((errors + 1))
  else
    echo "[PASS] hooks.json is valid JSON"
  fi
fi

# 8. .mcp.json is valid (if exists)
if [ -f "$PLUGIN_ROOT/.mcp.json" ]; then
  if ! jq empty "$PLUGIN_ROOT/.mcp.json" 2>/dev/null; then
    echo "[FAIL] .mcp.json is invalid JSON"; errors=$((errors + 1))
  else
    echo "[PASS] .mcp.json is valid JSON"
  fi
fi

# 9. No hardcoded absolute paths in mcpServers
if jq -r '.mcpServers // {} | .. | strings' "$PLUGIN_JSON" 2>/dev/null | grep -qE '^/(home|usr|tmp|mnt)'; then
  echo "[WARN] Possible hardcoded absolute path in mcpServers"
else
  echo "[PASS] No hardcoded paths in mcpServers"
fi

[ $errors -eq 0 ] && echo "=== All plugin.json checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
