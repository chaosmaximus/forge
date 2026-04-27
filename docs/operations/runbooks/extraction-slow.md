# Runbook — `ForgeExtractionSlow`

## Symptom

`Forge extraction p95 latency exceeds 60s` — extraction durations at
the 95th percentile exceed 60 seconds for ≥ 5 minutes.
**Severity: warning.**

## Likely causes

1. Backlogged `transcript_chunk` queue forcing the extractor to crunch
   many chunks per pass.
2. Embedder cold-load (~6s per ONNX session re-init) repeatedly hitting
   if the embedder is being recycled.
3. CPU pressure from another worker (consolidator pass running in
   parallel with batch extraction).
4. Disk I/O saturation on the SQLite backing store (high WAL flush
   latency).
5. Anti-pattern: extraction got pushed onto the request hot path
   accidentally.

## First-response steps

```bash
# Per-phase duration breakdown
forge-next observe --shape phase-run-summary --phase extraction --window 30m

# Queue depth
forge-next observe layer-freshness

# Top recent extractions
forge-next observe recent-extractions --limit 10

# System pressure
top -b -n 1 | head -20
iostat -x 1 5
```

## Remediation

* If queue is backlogged: pause new transcript ingestion until extractor
  catches up; `forge-next config set ingestion.paused=true`.
* If embedder churn: check whether embedder TTL has been set too low
  (`forge-next config get embedder.ttl_seconds`); raise to 3600+.
* If disk-saturation: investigate WAL size (`ls -lh ~/.forge/forge.db*`).
  Run `PRAGMA wal_checkpoint(TRUNCATE)` via daemon admin endpoint.
* If consolidator co-running: schedule consolidator off-hours via cron
  config.

## Escalation

* Warning — investigate within business hours.
* If sustained > 4h without root cause, file an issue.
