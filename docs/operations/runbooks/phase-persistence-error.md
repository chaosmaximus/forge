# Runbook — `ForgePhasePersistenceErrorRateHigh`

## Symptom

`Phase {{ $labels.phase_name }} persistence error rate elevated` —
`rate(forge_phase_persistence_errors_total[5m]) > 0.01` for ≥ 5 min.
**Severity: warning.**

## Likely causes

1. SQLite database is locked by an external reader / writer (lsof on
   the file shows another process holding it).
2. WAL file unable to checkpoint due to long-held read transaction.
3. Schema drift — recent ALTER TABLE landed but a worker is using
   stale prepared statements.
4. Disk-full or quota exceeded; INSERTs return `SQLITE_FULL` (errno 13).
5. Constraint violation due to corrupt input (rare; would also surface
   in extractor logs).

## First-response steps

```bash
# Which phase? Which error?
forge-next observe phase-summary --phase {{ $labels.phase_name }} --window 30m

# Recent errors in raw form
forge-next observe phase-errors --phase {{ $labels.phase_name }} --limit 20

# Disk + db file
df -h ~/.forge
ls -lh ~/.forge/forge.db*

# External holders
lsof ~/.forge/forge.db 2>/dev/null
```

## Remediation

* If lock contention: identify and stop the external holder. If it's
  another forge process, `pkill -SIGTERM forge-daemon` and restart.
* If WAL not checkpointing: run admin checkpoint. If a long-running
  transaction is preventing it, kill the transaction holder.
* If schema drift: `forge-next service restart` reloads the prepared
  statements against the current schema.
* If disk-full: free space (truncate events older than 7 days,
  `forge-next admin truncate-events --older-than 7d`), then restart.
* If constraint violation: capture an example error; file as a bug.

## Escalation

* Warning — investigate within 1h.
* If sustained > 1h with active write workload, escalate to
  ForgeWorkerDown territory (worker may stop reporting healthy).
* If isolated to a single phase that's never errored before, file
  a code-regression issue with the error sample.
