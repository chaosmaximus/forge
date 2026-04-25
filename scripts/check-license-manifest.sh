#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# scripts/check-license-manifest.sh — Phase 2P-1b §3 SPDX sidecar validator.
# Wraps scripts/check_license_manifest.py for convention parity with the
# other CI shell scripts (--root flag-eat guard, python3 presence check).

set -euo pipefail

REPO_ROOT_OVERRIDE=""
MANIFEST_PATH=""
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
        --manifest)
            if [ -z "${2:-}" ]; then
                echo "ERROR: --manifest requires a path" >&2
                exit 2
            fi
            case "$2" in
                -*)
                    echo "ERROR: --manifest path must not start with '-' (got: $2)" >&2
                    exit 2
                    ;;
            esac
            MANIFEST_PATH="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--root <repo-root>] [--manifest <path-from-root>]"
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
args=(--root "$REPO_ROOT")
if [ -n "$MANIFEST_PATH" ]; then
    args+=(--manifest "$MANIFEST_PATH")
fi
exec python3 "$SCRIPT_DIR/check_license_manifest.py" "${args[@]}"
