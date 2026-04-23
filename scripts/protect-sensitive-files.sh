#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Forge: PreToolUse hook for Edit|Write
# Blocks edits to sensitive files
# Security: resolves symlinks, checks canonical path
set -euo pipefail

# Security hook: fail closed if jq is unavailable
if ! command -v jq &>/dev/null; then
  echo "Forge: jq not found. Blocking edit as a precaution (install jq for proper file path checking)." >&2
  exit 2
fi

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // .tool_input.path // empty')

if [ -z "$FILE_PATH" ]; then
  exit 0
fi

# Resolve symlinks to get canonical path (prevents symlink bypass)
RESOLVED_PATH="$FILE_PATH"
if command -v readlink &>/dev/null; then
  RESOLVED_PATH=$(readlink -f "$FILE_PATH" 2>/dev/null || echo "$FILE_PATH")
fi

BASENAME=$(basename "$RESOLVED_PATH")

case "$BASENAME" in
  .env|.env.*|credentials*|secrets*|*.key|*.pem|*.tfstate|*.tfstate.backup|poetry.lock|package-lock.json|yarn.lock|*.p12|*.pfx|*.jks|*.keystore|.npmrc|.pypirc|id_rsa|id_ed25519|*.pub|kubeconfig|.git-credentials|service-account.json|*-sa-key.json|token.json|*.gpg)
    echo "Protected file: $BASENAME (resolved from $(basename "$FILE_PATH")). Edit manually or use the appropriate tool." >&2
    exit 2
    ;;
esac

exit 0
