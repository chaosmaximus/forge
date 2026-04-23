#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
PLUGIN_ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
HOOKS_JSON="$PLUGIN_ROOT/hooks/hooks.json"
errors=0

echo "=== Hooks.json Validation ==="

# 1. Valid JSON
if ! jq empty "$HOOKS_JSON" 2>/dev/null; then
  echo "[FAIL] Invalid JSON"; exit 1
fi
echo "[PASS] Valid JSON"

# 2. Has hooks top-level key
if ! jq -e '.hooks' "$HOOKS_JSON" >/dev/null 2>&1; then
  echo "[FAIL] Missing top-level 'hooks' key"; exit 1
fi
echo "[PASS] Has 'hooks' key"

# 3. Validate event names
VALID_EVENTS="SessionStart|SessionEnd|PreToolUse|PostToolUse|PostToolUseFailure|TaskCompleted|TeammateIdle|TaskCreated|Stop|StopFailure|Notification|SubagentStart|SubagentStop|UserPromptSubmit|PermissionRequest|ConfigChange|CwdChanged|FileChanged|WorktreeCreate|WorktreeRemove|PreCompact|PostCompact|InstructionsLoaded|Elicitation|ElicitationResult"

for event in $(jq -r '.hooks | keys[]' "$HOOKS_JSON"); do
  if echo "$event" | grep -qE "^($VALID_EVENTS)$"; then
    echo "[PASS] Valid event: $event"
  else
    echo "[FAIL] Unknown event: $event"; errors=$((errors + 1))
  fi
done

# 4. Each hook entry has required fields
for event in $(jq -r '.hooks | keys[]' "$HOOKS_JSON"); do
  entries=$(jq -r ".hooks.\"$event\" | length" "$HOOKS_JSON")
  for i in $(seq 0 $((entries - 1))); do
    hooks_count=$(jq -r ".hooks.\"$event\"[$i].hooks | length" "$HOOKS_JSON")
    if [ "$hooks_count" -eq 0 ]; then
      echo "[FAIL] $event[$i]: empty hooks array"; errors=$((errors + 1))
      continue
    fi

    for j in $(seq 0 $((hooks_count - 1))); do
      hook_type=$(jq -r ".hooks.\"$event\"[$i].hooks[$j].type // empty" "$HOOKS_JSON")
      if [ -z "$hook_type" ]; then
        echo "[FAIL] $event[$i].hooks[$j]: missing 'type'"; errors=$((errors + 1))
      elif ! echo "$hook_type" | grep -qE "^(command|http|prompt|agent)$"; then
        echo "[FAIL] $event[$i].hooks[$j]: invalid type '$hook_type'"; errors=$((errors + 1))
      else
        echo "[PASS] $event[$i].hooks[$j]: type=$hook_type"
      fi

      # Check command hooks have command field
      if [ "$hook_type" = "command" ]; then
        cmd=$(jq -r ".hooks.\"$event\"[$i].hooks[$j].command // empty" "$HOOKS_JSON")
        if [ -z "$cmd" ]; then
          echo "[FAIL] $event[$i].hooks[$j]: command hook missing 'command'"; errors=$((errors + 1))
        fi

        # Check for hardcoded paths
        if echo "$cmd" | grep -qE '^/(home|usr|tmp|mnt)'; then
          echo "[WARN] $event[$i].hooks[$j]: possible hardcoded path in command"
        fi
      fi

      # Check timeout is reasonable (1-600 seconds)
      timeout=$(jq -r ".hooks.\"$event\"[$i].hooks[$j].timeout // empty" "$HOOKS_JSON")
      if [ -n "$timeout" ] && { [ "$timeout" -lt 1 ] || [ "$timeout" -gt 600 ]; }; then
        echo "[WARN] $event[$i].hooks[$j]: unusual timeout value: $timeout"
      fi
    done
  done
done

# 5. Referenced scripts exist
for event in $(jq -r '.hooks | keys[]' "$HOOKS_JSON"); do
  jq -r ".hooks.\"$event\"[].hooks[]?.command // empty" "$HOOKS_JSON" 2>/dev/null | while IFS= read -r cmd; do
    [ -z "$cmd" ] && continue
    # Extract script path (strip bash prefix and quotes, resolve CLAUDE_PLUGIN_ROOT)
    script_path=$(echo "$cmd" | sed 's/^bash //' | tr -d '"' | sed "s|\\\${CLAUDE_PLUGIN_ROOT}|$PLUGIN_ROOT|g" | awk '{print $1}')
    if [ -n "$script_path" ] && [[ "$script_path" == *scripts/* ]] && [ ! -f "$script_path" ]; then
      echo "[FAIL] Script not found: $script_path"
    fi
  done
done

# 6. Count hooks
HOOK_COUNT=$(jq '[.hooks | to_entries[].value[].hooks[]?] | length' "$HOOKS_JSON" 2>/dev/null || echo 0)
echo "[INFO] Total hook entries: $HOOK_COUNT"

[ $errors -eq 0 ] && echo "=== All hooks checks passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
