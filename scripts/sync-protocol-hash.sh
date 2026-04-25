#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# scripts/sync-protocol-hash.sh — refresh the 2A-4d interlock hash.
#
# Recomputes SHA-256 of crates/core/src/protocol/request.rs and writes it
# into .claude-plugin/plugin.json's protocol_hash field, preserving the
# rest of the JSON layout via a regex substitution. Uses python3 +
# hashlib for cross-platform portability (avoids sha256sum / BSD-grep
# `\s` differences flagged in the W4 review).

set -euo pipefail

REPO_ROOT_OVERRIDE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --root)
            if [ -z "${2:-}" ]; then
                echo "ERROR: --root requires a path" >&2
                exit 2
            fi
            case "$2" in
                -*)
                    echo "ERROR: --root path must not start with '-' (got: $2)" >&2
                    exit 2
                    ;;
            esac
            REPO_ROOT_OVERRIDE="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--root <repo-root>]"
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

if [ -n "$REPO_ROOT_OVERRIDE" ]; then
    REPO_ROOT="$REPO_ROOT_OVERRIDE"
else
    REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
fi

PROTOCOL_FILE="$REPO_ROOT/crates/core/src/protocol/request.rs"
PLUGIN_FILE="$REPO_ROOT/.claude-plugin/plugin.json"

[ -f "$PROTOCOL_FILE" ] || { echo "ERROR: missing $PROTOCOL_FILE" >&2; exit 2; }
[ -f "$PLUGIN_FILE" ]   || { echo "ERROR: missing $PLUGIN_FILE" >&2; exit 2; }

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 required" >&2
    exit 2
fi

python3 - "$PROTOCOL_FILE" "$PLUGIN_FILE" <<'PYTHON'
import hashlib
import re
import sys

proto_path, plugin_path = sys.argv[1], sys.argv[2]

with open(proto_path, "rb") as f:
    new_hash = hashlib.sha256(f.read()).hexdigest()

with open(plugin_path) as f:
    text = f.read()

# Match "protocol_hash"<ws>:<ws>"<hex>" — \s tolerates wrapped layouts
# (key/value on separate lines after a JSON auto-formatter run); the hex
# class is case-insensitive so an uppercase value still matches.
pattern = re.compile(
    r'("protocol_hash"\s*:\s*")[a-fA-F0-9]+(")',
)
new_text, count = pattern.subn(rf"\g<1>{new_hash}\g<2>", text, count=1)
if count == 0:
    sys.stderr.write(
        f"ERROR: no protocol_hash field found in {plugin_path}\n"
        f'  Suggested line: "protocol_hash": "{new_hash}"\n'
    )
    sys.exit(2)

with open(plugin_path, "w") as f:
    f.write(new_text)

print(f"protocol-hash synced: {new_hash}")
PYTHON
