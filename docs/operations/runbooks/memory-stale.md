# Runbook — `ForgeMemoryStale`

## Symptom

`Forge memory count unchanged for over 1 hour` — `delta(forge_memories_total[1h]) == 0`.
**Severity: warning.**

## Likely causes

1. Extractor worker stopped (would also fire `ForgeWorkerDown`).
2. No new transcripts being submitted (legitimate during off-hours).
3. Extraction is running but every chunk is being deduped before
   producing a memory.
4. Database write path returning errors silently (would also produce
   `forge_phase_persistence_errors_total` upticks).
5. Quota / per-tenant limits hit.

## First-response steps

```bash
# Confirm extractor is alive
forge-next observe worker-status | grep extractor

# Recent extractions vs recent memories
forge-next observe recent-extractions --limit 5
forge-next observe recent-memories --limit 5

# Persistence error rate
forge-next observe phase-summary --phase extraction --window 60m
```

## Remediation

* If off-hours and no transcripts inbound: silence the alert via
  schedule (e.g. only alert during 09:00-18:00).
* If extractor running but producing 0 memories: inspect dedup ratio
  via `forge-next observe phase-summary`. If dedup is 100%, all input
  is duplicates — likely a transcription loop.
* If write errors: cross-reference `phase-persistence-error.md` runbook.
* If quota hit: raise quota or rotate retention.

## Escalation

* Warning — silence overnight, investigate during business hours.
* If sustained 24h+ during expected-active periods, file an issue.
