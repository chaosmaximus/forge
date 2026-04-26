# Forge alert runbooks

This directory holds the response runbooks for every alert defined in
[`deploy/grafana/forge-alerts.yml`](../../../deploy/grafana/forge-alerts.yml).
Each `runbook_url` in that file points at one of the `<name>.md` files
below.

## Runbook contract

Each runbook has the same five sections:

| Section | Contents |
|---------|----------|
| **Symptom** | What the alert literally says (the `summary` annotation). |
| **Likely causes** | 3-5 enumerated causes, ordered by frequency. |
| **First-response steps** | What to check immediately (read-only investigation). |
| **Remediation** | How to fix once root cause is identified. |
| **Escalation** | When to page humans / file an issue. |

## Runbook index

| Alert | Runbook | Severity |
|-------|---------|----------|
| `ForgeWorkerDown`                     | [`worker-down.md`](worker-down.md)                         | critical |
| `ForgeExtractionSlow`                 | [`extraction-slow.md`](extraction-slow.md)                 | warning  |
| `ForgeMemoryStale`                    | [`memory-stale.md`](memory-stale.md)                       | warning  |
| `ForgeHighRecallLatency`              | [`high-recall-latency.md`](high-recall-latency.md)         | warning  |
| `ForgeNoActiveSessions`               | [`no-active-sessions.md`](no-active-sessions.md)           | info     |
| `ForgeAllWorkersDown`                 | [`all-workers-down.md`](all-workers-down.md)               | critical |
| `ForgePhasePersistenceErrorRateHigh`  | [`phase-persistence-error.md`](phase-persistence-error.md) | warning  |
| `ForgeLayerFreshnessStaleHour`        | [`layer-stale.md`](layer-stale.md)                         | warning  |
| `ForgePhaseStuck`                     | [`phase-stuck.md`](phase-stuck.md)                         | warning  |

## Hosted-Grafana note

The `runbook_url` annotations use **relative repo paths** so the alert
file works when cloned. If you operate a hosted Grafana, rewrite each
`runbook_url` to its `https://github.com/<org>/forge/blob/<sha>/docs/operations/runbooks/<name>.md`
absolute form before applying.
