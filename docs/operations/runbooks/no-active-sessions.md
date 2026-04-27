# Runbook — `ForgeNoActiveSessions`

## Symptom

`No active Forge sessions for 30 minutes` — `forge_active_sessions == 0`
for ≥ 30 min. **Severity: info.**

## Likely causes

1. Off-hours / weekend / holiday with no human users (legitimate).
2. The CLI / plugin is mis-configured and not registering sessions.
3. Daemon is up but session table writes are failing (would also fire
   `forge_phase_persistence_errors_total`).
4. Heartbeat path broken — sessions exist but `update_heartbeat`
   isn't landing, so they age out.
5. All sessions are in `dormant` state (post-2A-4d.2 W1 rename); the
   metric only counts `active`.

## First-response steps

```bash
# Session table state
forge-next list-sessions --include-dormant

# Recent heartbeat events
forge-next observe --shape phase-run-summary --phase update_heartbeat --window 30m

# Daemon health
forge-next health
```

## Remediation

* If legitimate off-hours: silence the alert via schedule (e.g. only
  alert during 09:00-18:00).
* If sessions exist but in `dormant` state: fix the calling tool's
  heartbeat cadence; sessions transition `active → dormant → archived`
  if no heartbeat for the configured TTL.
* If session writes failing: investigate via `phase-persistence-error.md`.

## Escalation

* Info — informational. Tune the schedule + thresholds.
* No paging.
