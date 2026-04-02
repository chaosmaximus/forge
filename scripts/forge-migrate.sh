#!/usr/bin/env bash
# forge-migrate.sh — Migrate from Forge v0.1.5 to v0.2.0
set -euo pipefail

PLUGIN_DATA="${CLAUDE_PLUGIN_DATA:-$HOME/.claude/plugin-data/forge}"
BACKUP_DIR="${PLUGIN_DATA}/v0.1.5-backup"

echo "=== Forge v0.1.5 → v0.2.0 Migration ==="

# Step 1: Inventory
echo "[1/5] Inventory..."
if [ -f "MEMORY.md" ]; then
    echo "  Found: MEMORY.md"
fi
if [ -d "${PLUGIN_DATA}/graph" ]; then
    echo "  Found: existing graph data"
fi

# Step 2: Backup
echo "[2/5] Backup..."
mkdir -p "${BACKUP_DIR}"
if [ -f "MEMORY.md" ]; then
    cp "MEMORY.md" "${BACKUP_DIR}/"
    echo "  Backed up: MEMORY.md"
fi
if [ -d "${PLUGIN_DATA}/graph" ]; then
    cp -r "${PLUGIN_DATA}/graph" "${BACKUP_DIR}/graph-v015" 2>/dev/null || true
    echo "  Backed up: graph data"
fi
echo "  Backup dir: ${BACKUP_DIR}"

# Step 3: Dry run
echo "[3/5] Dry run..."
if [ -f "MEMORY.md" ]; then
    ENTRY_COUNT=$(grep -c "^-\|^##" "MEMORY.md" 2>/dev/null || echo "0")
    echo "  MEMORY.md: ~${ENTRY_COUNT} entries importable as Decision nodes"
fi

# Step 4: Prepare
echo "[4/5] Prepare..."
mkdir -p "${PLUGIN_DATA}/graph"
mkdir -p "${PLUGIN_DATA}/hud"
echo "  Created: ${PLUGIN_DATA}/graph/"
echo "  Created: ${PLUGIN_DATA}/hud/"

# Step 5: Summary
echo "[5/5] Ready."
echo ""
echo "  Backup: ${BACKUP_DIR}"
echo "  Next: forge-graph will create forge.lbdb on first launch"
echo "  Import: Use forge_remember to import MEMORY.md decisions after launch"
echo ""
echo "=== Migration complete ==="
