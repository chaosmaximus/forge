# Runbook — `ForgeWorkerDown`

## Symptom

`Forge worker {{ $labels.worker }} is down` — `forge_worker_healthy == 0`
held for ≥ 5 minutes for the named worker. **Severity: critical.**

## Likely causes

1. Worker task panicked and the parent did not respawn (panic in
   `tokio::spawn` not caught by a supervisor).
2. SQLite write contention deadlocked the worker's DB-write path
   (especially `consolidator`, `indexer`, or `extractor`).
3. The worker was disabled via config but the metric still emits at 0
   instead of being absent.
4. Daemon process is alive but the worker's healthcheck (heartbeat
   write) is not landing — could indicate process freeze, GC pause,
   or hung syscall.
5. Embedder worker stalled waiting for ONNX runtime initialization.

## First-response steps

```bash
# 1. Daemon overall health
forge-next health

# 2. Worker-specific status
forge-next observe worker-status

# 3. Recent worker errors
forge-next observe phase-summary --window 30m

# 4. Daemon log tail (last 200 lines)
forge-next logs --tail 200

# 5. Process check
ps -p $(pgrep forge-daemon) -o pid,etime,stat,cmd
```

## Remediation

* If panic seen in logs and worker is `consolidator` or `extractor`:
  daemon will auto-respawn within 30s. If it doesn't, restart the
  daemon: `forge-next service restart`.
* If SQLite deadlock: identify the long-running query via the journal
  (`PRAGMA wal_checkpoint(TRUNCATE);`) and kill the holder. Avoid
  manual SQLite manipulation — prefer `forge-next service restart`.
* If healthcheck stale on otherwise-running daemon: send `SIGTERM` to
  trigger graceful shutdown + restart (P3-2 W7 added SIGTERM handler).

## Escalation

* Critical — page within 15 min if not resolved automatically.
* If the same worker re-fires within 1h after restart, file an issue
  with logs + metrics snapshot. Likely a code regression.
