#!/usr/bin/env bash
# Forge: SessionEnd hook
# Triggers episodic-memory sync if available
# Security: validates the CLI path belongs to the expected plugin structure
set -euo pipefail

# Drain stdin
cat > /dev/null 2>/dev/null || true

EM_CLI=""
CLAUDE_DIR="${HOME}/.claude/plugins/cache"

# Search plugin cache for episodic-memory CLI
for candidate in "$CLAUDE_DIR"/*/episodic-memory/*/cli/episodic-memory.js; do
  if [ -f "$candidate" ]; then
    # Security: verify the path is under the expected Claude plugin cache
    RESOLVED=$(readlink -f "$candidate" 2>/dev/null || echo "$candidate")
    case "$RESOLVED" in
      "$HOME/.claude/plugins/cache"/*)
        EM_CLI="$RESOLVED"
        break
        ;;
      *)
        # Path resolved outside expected location — skip
        ;;
    esac
  fi
done

if [ -n "$EM_CLI" ]; then
  node "$EM_CLI" sync 2>/dev/null &
fi

exit 0
