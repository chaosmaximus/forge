#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# scripts/check-protocol-hash.sh — Phase 2P-1b §7 2A-4d interlock.
# Wraps scripts/check_protocol_hash.py for convention parity with the
# other CI shell scripts (--root flag-eat guard, python3 presence check).

set -euo pipefail

REPO_ROOT_OVERRIDE=""
PASSTHROUGH=()
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
        --protocol-file|--plugin-file)
            if [ -z "${2:-}" ]; then
                echo "ERROR: $1 requires a path" >&2
                exit 2
            fi
            case "$2" in
                -*)
                    echo "ERROR: $1 path must not start with '-' (got: $2)" >&2
                    exit 2
                    ;;
            esac
            PASSTHROUGH+=("$1" "$2")
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--root <repo-root>] [--protocol-file <path>] [--plugin-file <path>]"
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

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 required" >&2
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
exec python3 "$SCRIPT_DIR/check_protocol_hash.py" --root "$REPO_ROOT" "${PASSTHROUGH[@]}"
