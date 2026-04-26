# Forge — Observability SLO registry

**Status:** Locked at v0.6.0-rc.3 (P3-3.5 W7).
**Source of truth:** this file. Other docs cross-reference it; if the
numbers diverge, this document wins until reconciled.

The SLOs below are operator-facing — they answer "what should the daemon
look like in steady state?". They are **not** product SLAs.

## Headline SLO table

| # | Surface | Indicator | Target | Window | Source-of-record | Alert |
|--:|---------|-----------|--------|--------|------------------|-------|
| 1 | Recall latency               | `forge_recall_latency_seconds` p50         | < 10 ms              | 5m  | `docs/operations.md` line 29       | `ForgeHighRecallLatency` (p99 > 5s) |
| 2 | Recall latency               | `forge_recall_latency_seconds` p99         | < 100 ms             | 5m  | `docs/operations.md` line 29       | `ForgeHighRecallLatency` (p99 > 5s) |
| 3 | Extraction duration          | `forge_extraction_duration_seconds` p50    | < 5 s                | 5m  | `docs/operations.md` line 30       | `ForgeExtractionSlow` (p95 > 60s) |
| 4 | Extraction duration          | `forge_extraction_duration_seconds` p99    | < 30 s               | 5m  | `docs/operations.md` line 30       | `ForgeExtractionSlow` (p95 > 60s) |
| 5 | Consolidator phase duration  | `forge_phase_duration_seconds` p95         | < 5 s (outlier)      | 5m  | operator dashboard panel "phase duration p95" | `ForgePhaseStuck` (p95 > 5s 5m) |
| 6 | Phase persistence error rate | `forge_phase_persistence_errors_total`     | < 0.01 / s steady-state | 5m  | operator dashboard panel "phase persistence error rate" | `ForgePhasePersistenceErrorRateHigh` |
| 7 | Layer freshness              | `forge_layer_freshness_seconds`            | < 3600 s actively-touched | 10m | operator dashboard panel "layer freshness" | `ForgeLayerFreshnessStaleHour` |
| 8 | Bench composite              | per-bench composite (Tier 3 leaderboard)   | ≥ `composite_min` per bench | release | `docs/benchmarks/baselines/composites.json` | `bench-regression` workflow (ci) |
| 9 | Worker liveness              | `forge_worker_healthy`                     | == 1 per worker      | 5m  | operator dashboard panel "worker status"  | `ForgeWorkerDown`, `ForgeAllWorkersDown` |
| 10 | Memory growth                | `delta(forge_memories_total[1h])`          | > 0 during active hours | 1h  | user/dev dashboard panel "memory growth" | `ForgeMemoryStale` |
| 11 | Active sessions              | `forge_active_sessions`                    | ≥ 1 during active hours | 30m | user/dev dashboard | `ForgeNoActiveSessions` |

## SLO categories

### Latency (rows 1-5)

These map to user-perceived performance. Recall is the hot path —
breaching p99 at 100ms degrades agent UX. Extraction is async — slow
extraction drains the queue but doesn't block the agent. Phase duration
is operator-facing — outliers > 5s usually indicate a workload shift
or a code regression.

### Durability (rows 6-7)

`forge_phase_persistence_errors_total` is the canonical "did the
consolidator fail to write?" signal. Steady-state should be ~0; brief
spikes during a daemon restart are expected.

`forge_layer_freshness_seconds` measures wall-time since the layer was
last updated. Actively touched layers should refresh within an hour;
dormant layers (e.g., declared facets that haven't changed) can stay
stale much longer without alarm.

### Quality (row 8)

The bench composite SLO is the contract that "consolidation works at
all" — `forge-consolidation` ≥ 0.95 means the headline thesis holds.
Locked floors are in `docs/benchmarks/baselines/composites.json` (one
per bench); the 2C-2 auto-PR-on-regression workflow consults that file
+ a 5% pairwise drop check.

### Availability (rows 9-11)

`forge_worker_healthy` per-worker liveness; aggregate via `count()`
to detect total daemon-down state (`ForgeAllWorkersDown`).
`forge_active_sessions` measures observed traffic.

## How alerts and SLOs relate

The 9 alerts in `deploy/grafana/forge-alerts.yml` are the **breach
detectors** for these SLOs. Each alert names the runbook to follow
when it fires:

| SLO row | Alert | Runbook |
|---------|-------|---------|
| 1, 2    | `ForgeHighRecallLatency`            | `runbooks/high-recall-latency.md` |
| 3, 4    | `ForgeExtractionSlow`               | `runbooks/extraction-slow.md` |
| 5       | `ForgePhaseStuck`                   | `runbooks/phase-stuck.md` |
| 6       | `ForgePhasePersistenceErrorRateHigh`| `runbooks/phase-persistence-error.md` |
| 7       | `ForgeLayerFreshnessStaleHour`      | `runbooks/layer-stale.md` |
| 8       | (CI workflow, not Prometheus alert)  | n/a — opens GitHub Issue automatically |
| 9       | `ForgeWorkerDown`, `ForgeAllWorkersDown` | `runbooks/worker-down.md`, `runbooks/all-workers-down.md` |
| 10      | `ForgeMemoryStale`                  | `runbooks/memory-stale.md` |
| 11      | `ForgeNoActiveSessions`             | `runbooks/no-active-sessions.md` |

## Per-tenant labels (deferred)

Several rows above would be more informative as **per-tenant**
quantities. Currently all metrics are scoped to a single deployment.
The deferred backlog item "Per-tenant label dimensions in Prometheus
metrics" tracks this; expected to land in v0.6.1+.

## Recalibration cadence

* **Latency SLOs** (rows 1-5): re-derive from a 30-day rolling p99 +
  20% headroom buffer at every release boundary.
* **Persistence error rate** (row 6): tighten if observed steady-state
  goes below 0.001/s for two consecutive releases.
* **Layer freshness** (row 7): keep at 1h until per-layer SLOs justify
  finer-grained gates.
* **Bench composite** (row 8): see `docs/benchmarks/baselines/README.md`.
* **Availability** (rows 9-11): tied to deployment scale; revisit when
  multi-tenant lands.

## Related docs

* Plan: [`../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`](../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md) (P3-3.5 W7).
* Operations index: [`../operations.md`](../operations.md) — runbooks now under `runbooks/`, SLO numbers cross-reference here.
* Operator dashboard: [`../observability/grafana-operator-dashboard.md`](../observability/grafana-operator-dashboard.md) — visual SLO panels.
* Alerts: [`../../deploy/grafana/forge-alerts.yml`](../../deploy/grafana/forge-alerts.yml).
* Bench baselines: [`../benchmarks/baselines/composites.json`](../benchmarks/baselines/composites.json).
