#!/usr/bin/env bash
# scripts/chaos/restart-drill.sh — daemon restart persistence drill (P3-3 2A-7).
#
# Purpose: validate that the forge-daemon survives a mid-pass kill +
# restart with no data loss. Wraps `forge-bench forge-persist` (which
# spawns a real daemon, issues a deterministic seeded workload of
# Remember/RawIngest/SessionSend ops, SIGKILLs the daemon mid-pass,
# restarts on the same DB, and verifies byte-exact consistency).
#
# Usage:
#   scripts/chaos/restart-drill.sh [--seed N] [--memories N] [--chunks N] \
#       [--fisp-messages N] [--kill-after FRACTION] [--output DIR]
#
# Acceptance criteria (PASS = all four):
#   1. recovery_rate == 1.0 (every acked op survived restart)
#   2. consistency_rate == 1.0 (every recovered row's content matches pre-kill canonical hash)
#   3. recovery_time_ms < 5000 (daemon answered Health within 5s of restart spawn)
#   4. zero pre-kill ack failures (workload completed cleanly until kill point)
#
# This is a SIGKILL-only drill (forge-persist's Child::kill() = SIGKILL).
# SIGTERM / SIGINT modes are deferred to a v2 drill — see plan-doc P3-3
# Stage 3 deferred backlog.
#
# Exits 0 on PASS, 1 on FAIL, 2 on tooling error.

set -euo pipefail

# ── Defaults (small workload for fast operator drill) ───────────────────
# chunks=0 by default: raw-layer (RawIngest) ops require the MiniLM
# embedder to be loaded into the daemon process, which adds ~30s of
# model-download time on a cold cache and is not needed to demonstrate
# restart persistence. Memory + FISP ops are sufficient to exercise the
# WAL durability + HLC checkpoint path. Pass --chunks N to opt in.
SEED=42
MEMORIES=10
CHUNKS=0
FISP_MESSAGES=5
KILL_AFTER=0.5
OUTPUT_DIR=""

# ── Arg parsing ─────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --seed)           SEED="$2";          shift 2 ;;
        --memories)       MEMORIES="$2";      shift 2 ;;
        --chunks)         CHUNKS="$2";        shift 2 ;;
        --fisp-messages)  FISP_MESSAGES="$2"; shift 2 ;;
        --kill-after)     KILL_AFTER="$2";    shift 2 ;;
        --output)         OUTPUT_DIR="$2";    shift 2 ;;
        --help|-h)
            sed -n '1,/^set -euo/p' "$0" | sed '$d'
            exit 0 ;;
        *)
            echo "unknown arg: $1" >&2
            exit 2 ;;
    esac
done

# ── Output dir ──────────────────────────────────────────────────────────
if [[ -z "$OUTPUT_DIR" ]]; then
    OUTPUT_DIR="$(mktemp -d -t forge-restart-drill.XXXXXX)"
fi
mkdir -p "$OUTPUT_DIR"

# ── Locate forge-bench binary ───────────────────────────────────────────
ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
cd "$ROOT"

BENCH_BIN="$ROOT/target/release/forge-bench"
DAEMON_BIN="$ROOT/target/release/forge-daemon"

if [[ ! -x "$BENCH_BIN" || ! -x "$DAEMON_BIN" ]]; then
    echo "[restart-drill] building forge-bench + forge-daemon (release, --features bench)..."
    cargo build --release --features bench --bin forge-bench --bin forge-daemon
fi

# ── ORT runtime (Linux glibc<2.38 hosts; harmless elsewhere) ────────────
if [[ -d "$ROOT/.tools/onnxruntime-linux-x64-1.23.0/lib" ]]; then
    export LD_LIBRARY_PATH="$ROOT/.tools/onnxruntime-linux-x64-1.23.0/lib:${LD_LIBRARY_PATH:-}"
fi

# ── Run the drill ───────────────────────────────────────────────────────
echo "[restart-drill] === forge-daemon restart persistence drill ==="
echo "[restart-drill] seed=$SEED memories=$MEMORIES chunks=$CHUNKS fisp=$FISP_MESSAGES kill-after=$KILL_AFTER"
echo "[restart-drill] output=$OUTPUT_DIR"
echo

if ! "$BENCH_BIN" forge-persist \
        --seed "$SEED" \
        --memories "$MEMORIES" \
        --chunks "$CHUNKS" \
        --fisp-messages "$FISP_MESSAGES" \
        --kill-after "$KILL_AFTER" \
        --output "$OUTPUT_DIR" \
        --daemon-bin "$DAEMON_BIN"; then
    echo "[restart-drill] forge-bench forge-persist FAILED (exit non-zero)" >&2
    exit 1
fi

# ── Parse summary.json ──────────────────────────────────────────────────
SUMMARY="$OUTPUT_DIR/summary.json"
if [[ ! -f "$SUMMARY" ]]; then
    echo "[restart-drill] FAIL: summary.json missing at $SUMMARY" >&2
    exit 1
fi

# Use python (always available in dev env) for portable JSON parsing.
RECOVERY_RATE=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['recovery_rate'])")
CONSISTENCY_RATE=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['consistency_rate'])")
RECOVERY_TIME_MS=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['recovery_time_ms'])")
ACKED_PRE_KILL=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['acked_pre_kill'])")
RECOVERED=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['recovered'])")
MATCHED=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['matched'])")
TOTAL_OPS=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['total_ops'])")
PASS=$(python3 -c "import json; print(json.load(open('$SUMMARY'))['pass'])")

# ── Acceptance evaluation ───────────────────────────────────────────────
RECOVERY_OK=$(python3 -c "print('yes' if abs($RECOVERY_RATE - 1.0) < 1e-9 else 'no')")
CONSISTENCY_OK=$(python3 -c "print('yes' if abs($CONSISTENCY_RATE - 1.0) < 1e-9 else 'no')")
RECOVERY_TIME_OK=$(python3 -c "print('yes' if $RECOVERY_TIME_MS < 5000 else 'no')")

# ── Print summary ───────────────────────────────────────────────────────
echo
echo "[restart-drill] === results ==="
echo "[restart-drill] total_ops=$TOTAL_OPS"
echo "[restart-drill] acked_pre_kill=$ACKED_PRE_KILL"
echo "[restart-drill] recovered=$RECOVERED (rate=$RECOVERY_RATE, ok=$RECOVERY_OK)"
echo "[restart-drill] matched=$MATCHED (rate=$CONSISTENCY_RATE, ok=$CONSISTENCY_OK)"
echo "[restart-drill] recovery_time_ms=$RECOVERY_TIME_MS (< 5000ms ok=$RECOVERY_TIME_OK)"
echo "[restart-drill] forge-persist verdict: pass=$PASS"

# ── Write operator-friendly results doc ─────────────────────────────────
DRILL_DOC="$OUTPUT_DIR/drill-report.md"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
DRILL_VERDICT="UNKNOWN"
if [[ "$RECOVERY_OK" == "yes" && "$CONSISTENCY_OK" == "yes" && "$RECOVERY_TIME_OK" == "yes" ]]; then
    DRILL_VERDICT="PASS"
else
    DRILL_VERDICT="FAIL"
fi

cat > "$DRILL_DOC" <<EOF
# Daemon Restart Persistence Drill — operator report

**Drill date:** $TIMESTAMP
**Phase:** P3-3 Stage 3 (2A-7)
**Driver:** \`scripts/chaos/restart-drill.sh\`
**Underlying harness:** \`forge-bench forge-persist\` (Rust subprocess harness).

## Configuration

| Parameter | Value |
|-----------|-------|
| seed | $SEED |
| memories | $MEMORIES |
| chunks | $CHUNKS |
| fisp_messages | $FISP_MESSAGES |
| kill_after | $KILL_AFTER (fraction of total_ops) |
| total_ops | $TOTAL_OPS |
| kill_signal | SIGKILL (Child::kill()) |

## Results

| Metric | Value | Threshold | Pass |
|--------|-------|-----------|------|
| acked_pre_kill | $ACKED_PRE_KILL | n/a | n/a |
| recovered | $RECOVERED | == acked_pre_kill | $(if [ "$RECOVERED" = "$ACKED_PRE_KILL" ]; then echo yes; else echo NO; fi) |
| recovery_rate | $RECOVERY_RATE | 1.0 | $RECOVERY_OK |
| matched | $MATCHED | == recovered | $(if [ "$MATCHED" = "$RECOVERED" ]; then echo yes; else echo NO; fi) |
| consistency_rate | $CONSISTENCY_RATE | 1.0 | $CONSISTENCY_OK |
| recovery_time_ms | $RECOVERY_TIME_MS | < 5000 | $RECOVERY_TIME_OK |

## Verdict

**$DRILL_VERDICT**

## Reproduction

\`\`\`bash
cd $ROOT
scripts/chaos/restart-drill.sh \\
    --seed $SEED \\
    --memories $MEMORIES \\
    --chunks $CHUNKS \\
    --fisp-messages $FISP_MESSAGES \\
    --kill-after $KILL_AFTER \\
    --output $OUTPUT_DIR
\`\`\`

## Artifacts

- \`$OUTPUT_DIR/summary.json\` — full forge-persist summary (machine-readable)
- \`$OUTPUT_DIR/repro.sh\` — exact forge-bench reproduction command
- \`$OUTPUT_DIR/drill-report.md\` — this file (human-readable)

## Notes

- This drill exercises **SIGKILL** only (the abrupt-termination case;
  matches the rollback-playbook's worst-case operator scenario).
  SIGTERM / SIGINT graceful-shutdown drills are deferred to v2.
- The underlying harness validates **byte-exact** content survival via
  SHA-256 canonical hashes — a row that survives but with mutated
  content fails consistency_rate < 1.0.
- HLC monotonicity is **not directly probed** by this drill but is
  exercised transitively via forge-persist's session-message ordering
  audit. A regression in HLC checkpoint serialization would surface
  as recovery_rate < 1.0 or consistency_rate < 1.0.
EOF

echo
echo "[restart-drill] drill report: $DRILL_DOC"
echo "[restart-drill] verdict: $DRILL_VERDICT"

if [[ "$DRILL_VERDICT" == "PASS" ]]; then
    exit 0
else
    exit 1
fi
