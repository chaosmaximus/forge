# Forge Operator Dashboard (P3-3 2C-1)

**Status:** SHIPPED — 2026-04-26.
**Phase:** P3-3 Stage 4.
**Files:**
- `deploy/grafana/forge-operator-dashboard.json` (Grafana 10+ JSON)
- `deploy/grafana/forge-alerts.yml` (preexisting; alerts stay co-located)

The operator dashboard surfaces the **5 metric families** spec'd in
`docs/superpowers/plans/2026-04-25-complete-production-readiness.md`
P3-3 Stage 4 (2C-1):

| Panel | Source | Metric | Healthy range |
|-------|--------|--------|--------------|
| Phase duration p95 | Prometheus | `forge_phase_duration_seconds` | depends on phase; outliers > 5s warrant investigation |
| Phase persistence error rate | Prometheus | `forge_phase_persistence_errors_total` (counter) | steady-state < 0.01/s |
| Phase output rows total | Prometheus | `forge_phase_output_rows_total` | non-zero per active phase |
| Table rows | Prometheus | `forge_table_rows` | monotonically non-decreasing during normal operation |
| Layer freshness | Prometheus | `forge_layer_freshness_seconds` | < 3600s for actively-touched layers |
| Bench composite trend | SQLite (kpi_events) | `bench_run_completed` events | composite ≥ 0.95 per bench |

## Why two dashboards

`deploy/grafana/forge-dashboard.json` (preexisting; 15 panels) is the
**user/developer dashboard** — memory counts, recall latency, worker
health, session timelines.

`deploy/grafana/forge-operator-dashboard.json` (this file) is the
**operator dashboard** — phase-level instrumentation, error rates, layer
freshness, bench composite trend. Targets the family of operators
running forge-daemon in production who need to spot stuck phases,
runaway error rates, or bench regressions without drilling into every
panel of the user dashboard.

Both dashboards import side-by-side; folder them under
`Dashboards → Forge` for clarity.

## Datasource setup

### Prometheus (5 of 6 panels)

The daemon emits Prometheus text exposition at `:8420/metrics` (per
CLAUDE.md HTTP API note + `crates/daemon/src/server/metrics.rs::handle_metrics`).
Configure a Prometheus datasource pointed at your scrape target.

Standard scrape config:

```yaml
scrape_configs:
  - job_name: forge-daemon
    scrape_interval: 30s
    static_configs:
      - targets: ['<host>:8420']
```

### SQLite (1 of 6 panels — bench composite trend)

Panel 6 queries `kpi_events` via the Grafana SQLite datasource plugin
(`frser-sqlite-datasource` or `marcusolsson/grafana-falconlogscale-datasource`).
Point it at `~/.forge/forge.db`:

```yaml
datasource:
  name: ForgeKpiEvents
  type: frser-sqlite-datasource
  jsonData:
    path: /var/lib/forge/forge.db
```

If installing a Grafana SQLite plugin is undesirable, the alternative
is to extend the daemon with a `forge_bench_composite` Prometheus
gauge populated from `bench_run_completed` events. Currently bench
composites are **not** exposed via Prometheus (only via the
`bench_run_summary` /inspect shape from Tier 3). Track as backlog if
operator demand surfaces.

## Importing the dashboard

```bash
# Via Grafana UI:
#   1. Dashboards → Import
#   2. Upload deploy/grafana/forge-operator-dashboard.json
#   3. Map "Prometheus" datasource to your Prometheus instance
#   4. Map "SQLite" datasource (panel 6) — or skip panel 6 if not desired

# Via Grafana API (provisioning):
curl -X POST -H "Authorization: Bearer ${GF_TOKEN}" \
     -H "Content-Type: application/json" \
     -d @deploy/grafana/forge-operator-dashboard.json \
     http://grafana.example/api/dashboards/db
```

## Validation

- **JSON shape:** validates against Grafana 10+ schema (`schemaVersion: 38`).
- **Metric names:** every PromQL expression queries a metric registered
  in `crates/daemon/src/server/metrics.rs:210-258`. Renaming any of
  these is a v2 break — coordinate via the harness-sync gate.
- **Alerts:** no inline alert rules in the dashboard; ops alerts live
  in `deploy/grafana/forge-alerts.yml` (preexisting).

## Backlog / not-in-scope (deferred to future stages)

- **Bench composite as Prometheus gauge** — would obviate the SQLite
  datasource panel 6 dependency.
- **Per-tenant filtering** (organization_id, project) — not yet wired
  into the metric labels; would require `crates/daemon/src/server/metrics.rs`
  to add label dimensions.
- **Distributed-trace timeline panel** — would require the OTLP path
  from 2A-4d.1 to be wired to a Jaeger/Tempo datasource. Defer until
  multi-instance deployments justify it.
