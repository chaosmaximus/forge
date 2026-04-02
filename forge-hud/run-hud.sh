#!/usr/bin/env bash
# Find Forge's data dir for the HUD binary.
# CLAUDE_PLUGIN_DATA may point to another plugin (e.g., Codex) — always use Forge's.
FORGE_DIR=""
for candidate in \
    "$HOME/.claude/plugins/data/forge-forge-marketplace" \
    "$HOME/.claude/plugins/data/forge" \
    "$HOME/.claude/plugin-data/forge"; do
    if [ -d "$candidate" ]; then
        FORGE_DIR="$candidate"
        break
    fi
done
export CLAUDE_PLUGIN_DATA="${FORGE_DIR:-$HOME/.claude/plugins/data/forge-forge-marketplace}"
exec "$(dirname "$(readlink -f "$0")")/target/release/forge-hud" "$@"
