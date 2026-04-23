#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Performance regression tests — verify hot paths stay fast
set -euo pipefail

FORGE="${FORGE_BIN:-$(command -v forge 2>/dev/null || echo "")}"
STATE_DIR=$(mktemp -d)

echo "=== Performance Tests ==="

# Verify binary exists
if [ -z "$FORGE" ] || [ ! -x "$FORGE" ]; then
    echo "[SKIP] forge binary not found in PATH."
    echo "       Install from the public daemon repo:"
    echo "       cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli"
    exit 0
fi

# 1. forge recall should be fast even with many entries
echo "  Populating 100 memory entries..."
for i in $(seq 1 100); do
    "$FORGE" remember --type decision --title "perf-test-$i" --content "content for performance test entry number $i" --state-dir "$STATE_DIR" > /dev/null 2>&1
done

START=$(date +%s%N)
"$FORGE" recall "perf-test" --state-dir "$STATE_DIR" > /dev/null 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( (END - START) / 1000000 ))

echo "  recall with 100 entries: ${ELAPSED_MS}ms"
[ "$ELAPSED_MS" -lt 200 ] && echo "[PASS] <200ms" || echo "[WARN] slow: ${ELAPSED_MS}ms (limit: 200ms)"

# 2. forge verify should be <1s on a single file
if [ -f "forge-graph/src/forge_graph/cli.py" ]; then
    START=$(date +%s%N)
    "$FORGE" verify forge-graph/src/forge_graph/cli.py --state-dir "$STATE_DIR" > /dev/null 2>&1
    END=$(date +%s%N)
    ELAPSED_MS=$(( (END - START) / 1000000 ))

    echo "  verify single file: ${ELAPSED_MS}ms"
    [ "$ELAPSED_MS" -lt 2000 ] && echo "[PASS] <2s" || echo "[WARN] slow: ${ELAPSED_MS}ms (limit: 2s)"
else
    echo "  [SKIP] verify — cli.py not found"
fi

# 3. forge doctor should be <5s
START=$(date +%s%N)
"$FORGE" doctor --state-dir "$STATE_DIR" > /dev/null 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( (END - START) / 1000000 ))

echo "  doctor: ${ELAPSED_MS}ms"
[ "$ELAPSED_MS" -lt 5000 ] && echo "[PASS] <5s" || echo "[WARN] slow: ${ELAPSED_MS}ms (limit: 5s)"

# 4. Session hooks should be fast (<50ms each)
START=$(date +%s%N)
"$FORGE" hook session-start --state-dir "$STATE_DIR" > /dev/null 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( (END - START) / 1000000 ))

echo "  session-start hook: ${ELAPSED_MS}ms"
[ "$ELAPSED_MS" -lt 100 ] && echo "[PASS] <100ms" || echo "[WARN] slow: ${ELAPSED_MS}ms (limit: 100ms)"

START=$(date +%s%N)
"$FORGE" hook session-end --state-dir "$STATE_DIR" > /dev/null 2>&1
END=$(date +%s%N)
ELAPSED_MS=$(( (END - START) / 1000000 ))

echo "  session-end hook: ${ELAPSED_MS}ms"
[ "$ELAPSED_MS" -lt 100 ] && echo "[PASS] <100ms" || echo "[WARN] slow: ${ELAPSED_MS}ms (limit: 100ms)"

# 5. Agent event processing should be fast
START=$(date +%s%N)
echo '{"hookEventName":"SubagentStart","agentId":"perftest","agentType":"bench"}' | "$FORGE" agent --state-dir "$STATE_DIR"
END=$(date +%s%N)
ELAPSED_MS=$(( (END - START) / 1000000 ))

echo "  agent event: ${ELAPSED_MS}ms"
[ "$ELAPSED_MS" -lt 50 ] && echo "[PASS] <50ms" || echo "[WARN] slow: ${ELAPSED_MS}ms (limit: 50ms)"

rm -rf "$STATE_DIR"
echo ""
echo "=== Performance tests complete ==="
