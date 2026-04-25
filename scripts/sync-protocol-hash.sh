#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# scripts/sync-protocol-hash.sh — refresh the 2A-4d interlock hash.
#
# Recomputes SHA-256 of crates/core/src/protocol/request.rs and writes it
# into .claude-plugin/plugin.json's protocol_hash field, preserving the
# existing JSON layout via in-place sed substitution.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
PROTOCOL_FILE="$REPO_ROOT/crates/core/src/protocol/request.rs"
PLUGIN_FILE="$REPO_ROOT/.claude-plugin/plugin.json"

[ -f "$PROTOCOL_FILE" ] || { echo "ERROR: missing $PROTOCOL_FILE" >&2; exit 2; }
[ -f "$PLUGIN_FILE" ]   || { echo "ERROR: missing $PLUGIN_FILE" >&2; exit 2; }

HASH=$(sha256sum "$PROTOCOL_FILE" | awk '{print $1}')
[ -n "$HASH" ] || { echo "ERROR: sha256sum produced empty output" >&2; exit 2; }

# Sed substitution of the existing line. Fails (no replacements) if the
# protocol_hash field doesn't exist yet — in that case, manually add it.
if ! grep -qE '"protocol_hash"\s*:\s*"[a-f0-9]+"' "$PLUGIN_FILE"; then
    echo "ERROR: $PLUGIN_FILE has no protocol_hash field — add manually first" >&2
    echo "  Suggested line: \"protocol_hash\": \"$HASH\"" >&2
    exit 2
fi

# Use a temp file for portability between GNU and BSD sed.
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT
sed -E "s|\"protocol_hash\"[[:space:]]*:[[:space:]]*\"[a-f0-9]+\"|\"protocol_hash\": \"$HASH\"|" \
    "$PLUGIN_FILE" > "$TMP"
mv "$TMP" "$PLUGIN_FILE"

echo "protocol-hash synced: $HASH"
