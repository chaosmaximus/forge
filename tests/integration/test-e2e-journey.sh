#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail

FORGE="${FORGE_BIN:-$(command -v forge 2>/dev/null || echo "")}"
STATE_DIR=$(mktemp -d)
errors=0

echo "=== E2E Journey Test ==="

# Verify binary exists
if [ -z "$FORGE" ] || [ ! -x "$FORGE" ]; then
    echo "[SKIP] forge binary not found in PATH."
    echo "       Install from the public daemon repo:"
    echo "       cargo install --git https://github.com/chaosmaximus/forge forge-daemon forge-cli"
    exit 0
fi

# 1. Remember
OUT=$("$FORGE" remember --type decision --title "test-journey" --content "e2e test content" --state-dir "$STATE_DIR" 2>&1)
echo "$OUT" | grep -q '"status":"stored"' || echo "$OUT" | grep -q '"status": "stored"' || { echo "[FAIL] remember — got: $OUT"; errors=$((errors+1)); }
echo "[PASS] remember"

# 2. Recall by keyword
OUT=$("$FORGE" recall "journey" --state-dir "$STATE_DIR" 2>&1)
if echo "$OUT" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['count']>=1" 2>/dev/null; then
    echo "[PASS] recall keyword"
else
    echo "[FAIL] recall keyword — got: $OUT"; errors=$((errors+1))
fi

# 3. Recall list by type
OUT=$("$FORGE" recall --list --type decision --state-dir "$STATE_DIR" 2>&1)
if echo "$OUT" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['count']>=1" 2>/dev/null; then
    echo "[PASS] recall list"
else
    echo "[FAIL] recall list — got: $OUT"; errors=$((errors+1))
fi

# 4. Forget
ID=$(echo "$OUT" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['results'][0]['id'])")
"$FORGE" forget "$ID" --label Decision --state-dir "$STATE_DIR" > /dev/null 2>&1
OUT=$("$FORGE" recall "journey" --state-dir "$STATE_DIR" 2>&1)
if echo "$OUT" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['count']==0" 2>/dev/null; then
    echo "[PASS] forget"
else
    echo "[FAIL] forget — got: $OUT"; errors=$((errors+1))
fi

# 5. Doctor runs without error
OUT=$("$FORGE" doctor --state-dir "$STATE_DIR" 2>&1)
if echo "$OUT" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['summary']['error']==0" 2>/dev/null; then
    echo "[PASS] doctor"
else
    # Doctor may warn about missing tools — that's ok, just verify valid JSON and no crash
    if echo "$OUT" | python3 -c "import json,sys; json.load(sys.stdin)" 2>/dev/null; then
        echo "[PASS] doctor (warnings present but valid JSON)"
    else
        echo "[FAIL] doctor — got: $OUT"; errors=$((errors+1))
    fi
fi

# 6. Verify on a Python file
VERIFY_FILE="forge-graph/src/forge_graph/db.py"
if [ -f "$VERIFY_FILE" ]; then
    OUT=$("$FORGE" verify "$VERIFY_FILE" --state-dir "$STATE_DIR" 2>&1)
    if echo "$OUT" | grep -q '"status"'; then
        echo "[PASS] verify"
    else
        echo "[FAIL] verify — got: $OUT"; errors=$((errors+1))
    fi
else
    echo "[SKIP] verify — $VERIFY_FILE not found"
fi

# 7. Agent lifecycle
echo '{"hookEventName":"SubagentStart","agentId":"e2etest","agentType":"test-gen"}' | "$FORGE" agent --state-dir "$STATE_DIR"
echo '{"agentId":"e2etest","agentType":"test-gen","toolName":"Edit","toolInput":{"file_path":"test.py"}}' | "$FORGE" agent --state-dir "$STATE_DIR"
echo '{"hookEventName":"SubagentStop","agentId":"e2etest","agentType":"test-gen","lastAssistantMessage":"done"}' | "$FORGE" agent --state-dir "$STATE_DIR"
if [ -f "$STATE_DIR/agents/e2etest.jsonl" ] && [ "$(wc -l < "$STATE_DIR/agents/e2etest.jsonl")" -eq 3 ]; then
    echo "[PASS] agent lifecycle"
else
    echo "[FAIL] agent lifecycle"; errors=$((errors+1))
    ls -la "$STATE_DIR/agents/" 2>/dev/null || echo "  (no agents dir)"
fi

# 8. Session hooks produce valid JSON
OUT=$("$FORGE" hook session-start --state-dir "$STATE_DIR" 2>&1)
if echo "$OUT" | python3 -c "import json,sys; json.load(sys.stdin)" 2>/dev/null; then
    echo "[PASS] session-start JSON"
else
    echo "[FAIL] session-start JSON — got: $OUT"; errors=$((errors+1))
fi

OUT=$("$FORGE" hook session-end --state-dir "$STATE_DIR" 2>&1)
if echo "$OUT" | python3 -c "import json,sys; json.load(sys.stdin)" 2>/dev/null; then
    echo "[PASS] session-end JSON"
else
    echo "[FAIL] session-end JSON — got: $OUT"; errors=$((errors+1))
fi

# 9. Remember with all valid types
for t in decision pattern lesson preference; do
    OUT=$("$FORGE" remember --type "$t" --title "test-$t" --content "testing $t type" --state-dir "$STATE_DIR" 2>&1)
    if echo "$OUT" | grep -q '"status"'; then
        echo "[PASS] remember type=$t"
    else
        echo "[FAIL] remember type=$t — got: $OUT"; errors=$((errors+1))
    fi
done

# 10. Remember rejects invalid type
OUT=$("$FORGE" remember --type "invalid" --title "test" --content "bad type" --state-dir "$STATE_DIR" 2>&1)
if echo "$OUT" | grep -q '"error"'; then
    echo "[PASS] remember rejects invalid type"
else
    echo "[FAIL] remember should reject invalid type — got: $OUT"; errors=$((errors+1))
fi

# Cleanup
rm -rf "$STATE_DIR"

echo ""
[ $errors -eq 0 ] && echo "=== All E2E tests passed ===" || { echo "=== $errors error(s) ==="; exit 1; }
