#!/usr/bin/env bash
# Forge: SessionEnd hook
# Triggers episodic-memory sync if available
set -euo pipefail

# Drain stdin
cat > /dev/null 2>/dev/null || true

EM_CLI=""
# Search all plugin cache locations for episodic-memory CLI
for base in "$HOME/.claude/plugins/cache"/*/episodic-memory/*/cli/episodic-memory.js; do
  if [ -f "$base" ]; then
    EM_CLI="$base"
    break
  fi
done

if [ -n "$EM_CLI" ]; then
  node "$EM_CLI" sync 2>/dev/null &
fi

exit 0
