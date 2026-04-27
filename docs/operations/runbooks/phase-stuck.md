# Runbook — `ForgePhaseStuck`

## Symptom

`Phase {{ $labels.phase_name }} p95 duration > 5s` —
`histogram_quantile(0.95, rate(forge_phase_duration_seconds_bucket[5m]))`
exceeds 5s for ≥ 5 min on the named phase. **Severity: warning.**

## Likely causes

1. Runaway query inside the phase (e.g. consolidator Phase 14 reweave
   on a large memory set with poor predicate selectivity).
2. SQLite lock contention with another writer.
3. Workload regression — ingestion volume jumped, phase that was
   tuned for N memories is now seeing 10×N.
4. Performance regression in the phase's code path (recent commit
   added an N²-like scan).
5. JSON-extraction path that's not using an expression index — Forge
   has known cases (P3-2 W5 LOW-1).

## First-response steps

```bash
# Per-phase duration histogram
forge-next observe --shape phase-run-summary --phase {{ $labels.phase_name }} --window 30m

# Was this fast before? Compare last 24h.
forge-next observe --shape phase-run-summary --phase {{ $labels.phase_name }} --window 24h

# Recent commits
git -C ~/.forge log --oneline -20 | grep -i {{ $labels.phase_name }}

# Concurrent workers
curl -s http://127.0.0.1:8420/metrics | grep forge_worker_healthy

# Memory count growth (workload shift indicator)
forge-next observe layer-activity --layer memory --window 24h
```

## Remediation

* If runaway query: examine the phase's SQL via daemon source +
  `EXPLAIN QUERY PLAN`. Likely missing or unused index.
* If lock contention: stagger the conflicting workers (e.g. pause
  extractor during consolidator pass).
* If workload shifted: scale up; consider sharding by tenant or
  raising the phase's batch limit.
* If recent commit caused regression: `git revert` the offending
  commit, file a perf-regression issue with `EXPLAIN QUERY PLAN`
  output before/after.
* If JSON-extract path: file a backlog item for an expression index
  (existing precedent: P3-2 W5 LOW-1 deferred for `kpi_events`).

## Escalation

* Warning — investigate within 4h during business hours.
* If sustained > 4h, escalate to a higher-severity classification —
  user-visible recall latency will start drifting (cross-reference
  `high-recall-latency.md`).
