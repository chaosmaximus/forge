#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# scripts/check-sideload-state.sh — detect pre-2026-04-23 Forge plugin
# sideload setups.
#
# Reads $CLAUDE_SETTINGS (default: ~/.claude/settings.json) and reports
# any private-repo references — `forge-app` paths or `forge-private`
# plugin names that need to be migrated to the public marketplace.
#
# Exits:
#   0 — no sideload references found (or settings file missing,
#       which is not an error condition)
#   1 — private sideload detected; printed actionable migration pointer
#   2 — usage / parse error
#
# Same script works on Linux and macOS (Claude Code uses ~/.claude/ on
# both platforms).

set -euo pipefail

CLAUDE_SETTINGS_OVERRIDE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --settings)
            if [ -z "${2:-}" ]; then
                echo "ERROR: --settings requires a path" >&2
                exit 2
            fi
            case "$2" in
                -*)
                    echo "ERROR: --settings path must not start with '-' (got: $2)" >&2
                    exit 2
                    ;;
            esac
            CLAUDE_SETTINGS_OVERRIDE="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [--settings <path-to-claude-settings.json>]"
            exit 0
            ;;
        *)
            echo "ERROR: unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

if [ -n "$CLAUDE_SETTINGS_OVERRIDE" ]; then
    SETTINGS="$CLAUDE_SETTINGS_OVERRIDE"
else
    SETTINGS="${CLAUDE_SETTINGS:-$HOME/.claude/settings.json}"
fi

if [ ! -f "$SETTINGS" ]; then
    echo "sideload-state: no Claude Code settings at $SETTINGS — nothing to check"
    exit 0
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 required" >&2
    exit 2
fi

python3 - "$SETTINGS" <<'PYTHON'
import json
import sys

settings_path = sys.argv[1]
try:
    with open(settings_path) as f:
        settings = json.load(f)
except json.JSONDecodeError as e:
    sys.stderr.write(f"sideload-state: cannot parse {settings_path}: {e}\n")
    sys.exit(2)

if not isinstance(settings, dict):
    sys.stderr.write(f"sideload-state: top-level of {settings_path} is not an object\n")
    sys.exit(2)

# Canonical pre-2026-04-23 private-plugin name fragments. Verified
# against docs/superpowers/plans/2P-1a-inventory.md — no other names
# (e.g. forge-internal, bhairavi-forge) shipped pre-ban-lift. Future
# private forks with novel names will need a script update.
PRIVATE_FRAGMENTS = ("forge-app", "forge-private")


def _has_private_fragment(s) -> bool:
    if not isinstance(s, str):
        return False
    lower = s.lower()
    return any(f in lower for f in PRIVATE_FRAGMENTS)


issues = []

plugins = settings.get("enabledPlugins") or {}
if isinstance(plugins, dict):
    for name, enabled in plugins.items():
        if not _has_private_fragment(name):
            continue
        # Distinguish "active sideload" from "stale entry" — both should
        # be removed but the latter is informational, not blocking the
        # daemon from running.
        active_note = (
            "" if enabled
            else " (entry present, value=false — remove the entry)"
        )
        issues.append(
            f"enabledPlugins[{name!r}]: private sideload plugin{active_note}"
        )

markets = settings.get("extraKnownMarketplaces") or {}
if isinstance(markets, dict):
    for mname, mval in markets.items():
        if not isinstance(mval, dict):
            continue
        src = mval.get("source") or {}
        if not isinstance(src, dict):
            continue
        path = src.get("path") or ""
        repo = src.get("repo") or ""
        if (
            _has_private_fragment(mname)
            or _has_private_fragment(path)
            or _has_private_fragment(repo)
        ):
            target = path or repo or "(no path or repo)"
            issues.append(
                f"extraKnownMarketplaces[{mname!r}]: source pointer = {target}"
            )

if not issues:
    print(f"sideload-state: OK — {settings_path} has no private sideload references")
    sys.exit(0)

sys.stderr.write(
    f"sideload-state: detected pre-2026-04-23 sideload at {settings_path}\n\n"
)
for line in issues:
    sys.stderr.write(f"  - {line}\n")
sys.stderr.write(
    "\nMigration: https://github.com/chaosmaximus/forge/blob/master/"
    "docs/operations/sideload-migration.md\n"
    "(in-tree path if you cloned the repo: docs/operations/"
    "sideload-migration.md)\n"
)
sys.exit(1)
PYTHON
