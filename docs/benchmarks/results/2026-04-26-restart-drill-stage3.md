# Daemon Restart Persistence Drill — operator report

**Drill date:** 2026-04-26T04:07:55Z
**Phase:** P3-3 Stage 3 (2A-7)
**Driver:** `scripts/chaos/restart-drill.sh`
**Underlying harness:** `forge-bench forge-persist` (Rust subprocess harness).

## Configuration

| Parameter | Value |
|-----------|-------|
| seed | 42 |
| memories | 10 |
| chunks | 0 |
| fisp_messages | 5 |
| kill_after | 0.5 (fraction of total_ops) |
| total_ops | 15 |
| kill_signal | SIGKILL (Child::kill()) |

## Results

| Metric | Value | Threshold | Pass |
|--------|-------|-----------|------|
| acked_pre_kill | 7 | n/a | n/a |
| recovered | 7 | == acked_pre_kill | yes |
| recovery_rate | 1.0 | 1.0 | yes |
| matched | 7 | == recovered | yes |
| consistency_rate | 1.0 | 1.0 | yes |
| recovery_time_ms | 256 | < 5000 | yes |

## Verdict

**PASS**

## Reproduction

```bash
cd /mnt/colab-disk/DurgaSaiK/forge/forge
scripts/chaos/restart-drill.sh \
    --seed 42 \
    --memories 10 \
    --chunks 0 \
    --fisp-messages 5 \
    --kill-after 0.5 \
    --output /tmp/forge_chaos_dogfood2
```

## Artifacts

- `/tmp/forge_chaos_dogfood2/summary.json` — full forge-persist summary (machine-readable)
- `/tmp/forge_chaos_dogfood2/repro.sh` — exact forge-bench reproduction command
- `/tmp/forge_chaos_dogfood2/drill-report.md` — this file (human-readable)

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
