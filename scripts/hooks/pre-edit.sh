#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge v0.6.0 — PreToolUse hook for Edit|Write
# Runs guardrails check before file edits.
# If decisions are linked to the file, outputs a warning.
# Exit code 0 = allow, exit code 2 = block.

set -uo pipefail
# NOTE: -e removed — hook must NEVER fail and block the user.
INPUT=$(cat) || true

# Extract file path from hook input (multiple possible field names)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // .tool_input.filePath // .tool_input.path // .toolInput.file_path // empty' 2>/dev/null)
[ -z "$FILE_PATH" ] && exit 0

# Security: reject paths with shell metacharacters
[[ "$FILE_PATH" =~ [';|&$`\\'] ]] && exit 0

# Resolve symlinks
if command -v readlink &>/dev/null; then
  FILE_PATH=$(readlink -f "$FILE_PATH" 2>/dev/null || echo "$FILE_PATH")
fi

# Block sensitive files (same list as v0.3.0)
BASENAME=$(basename "$FILE_PATH")
case "$BASENAME" in
  .env|.env.*|credentials*|secrets*|*.key|*.pem|*.tfstate|*.p12|*.pfx|id_rsa|id_ed25519|kubeconfig|.git-credentials|service-account.json|token.json)
    echo "Protected file: $BASENAME. Edit manually." >&2
    exit 2
    ;;
esac

# Find forge-next
FORGE_NEXT="${FORGE_NEXT:-forge-next}"
command -v "$FORGE_NEXT" &>/dev/null || FORGE_NEXT="$HOME/.local/bin/forge-next"
[ -x "$FORGE_NEXT" ] || exit 0  # if no `forge-next` binary on PATH, allow the edit

# Run guardrails check (non-blocking — just warn, don't block code edits)
CHECK_OUTPUT=$("$FORGE_NEXT" check --file "$FILE_PATH" --action edit 2>/dev/null || echo "")

# If guardrails found decisions, output a warning via additionalContext with XML tags
if echo "$CHECK_OUTPUT" | grep -q "decision(s) linked"; then
  CONTEXT=$(echo "$CHECK_OUTPUT" | head -3 | tr '\n' ' ' | cut -c1-200)
  ESCAPED=$(echo "$CONTEXT" | sed 's/\\/\\\\/g; s/"/\\"/g')
  echo "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"additionalContext\":\"<forge-pre-edit>${ESCAPED}</forge-pre-edit>\"}}"
fi

exit 0
