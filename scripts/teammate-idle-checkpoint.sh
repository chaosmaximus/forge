#!/usr/bin/env bash
# Forge: TeammateIdle hook
# Updates STATE.md timestamp if it exists
set -euo pipefail

cat > /dev/null  # consume stdin

if [ -f "STATE.md" ]; then
  sed -i "s/\*\*Last updated:\*\*.*/\*\*Last updated:\*\* $(date -Iseconds)/" STATE.md 2>/dev/null || true
fi

exit 0
