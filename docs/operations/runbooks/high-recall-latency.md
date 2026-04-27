# Runbook — `ForgeHighRecallLatency`

## Symptom

`Forge recall p99 latency exceeds 5s` — recall p99 > 5s for ≥ 10 min.
**Severity: warning.**

## Likely causes

1. Embedding-search index is missing or fragmented (sqlite-vec ANN
   structure not loaded).
2. SQLite WAL grew large enough to slow `SELECT` paths; checkpoint
   overdue.
3. Co-running consolidator pass holding write locks the recall reader
   blocks on (despite WAL allowing concurrent reads — long-held shared
   locks still hurt).
4. Workload shifted: queries hitting cold layers (declared, perception)
   that materialize more rows than usual.
5. Recall batching via `forge-next recall` got large `--limit` hits.

## First-response steps

```bash
# p50/p95/p99 + per-call breakdown
forge-next observe recall-latency --window 30m

# WAL size + recent checkpoint timing
ls -lh ~/.forge/forge.db-wal ~/.forge/forge.db-shm
forge-next observe --shape phase-run-summary --phase wal_checkpoint --window 24h

# Concurrent worker activity
curl -s http://127.0.0.1:8420/metrics | grep forge_worker_healthy
```

## Remediation

* If WAL is > 1 GB: run admin checkpoint, `forge-next admin checkpoint`.
* If sqlite-vec not loaded: `forge-next doctor` to verify; daemon
  restart re-initialises.
* If co-running consolidator: reschedule pass off-peak.
* If query payload size: cap recall `--limit` at 50 in calling tools.

## Escalation

* Warning — investigate within 4h.
* If sustained 24h+, file an issue with timing histogram + workload
  pattern.
