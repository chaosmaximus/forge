#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# PostToolUse hook — secret detection on edited files via forge (Rust).
INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | jq -r '.tool_input.file_path // .tool_input.filePath // .toolInput.file_path // .toolInput.filePath // empty' 2>/dev/null)
[ -z "$FILE_PATH" ] && exit 0
[[ "$FILE_PATH" =~ [';|&$`\\'] ]] && exit 0
SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"
FORGE="$SCRIPT_DIR/../servers/forge"
[ -x "$FORGE" ] || exit 0
exec "$FORGE" hook post-edit "$FILE_PATH" 2>/dev/null
