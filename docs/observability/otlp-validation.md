# OTLP validation procedure

**Status:** Locked at v0.6.0-rc.3 (P3-3.5 W8). Manual procedure — not yet
in CI; see backlog at the bottom.

This document is the canonical "did the OTLP path work?" verification
for an operator running Forge against a Jaeger / OTLP-receiver setup.

## Environment variables

| Var | Required when | Effect |
|-----|---------------|--------|
| `FORGE_OTLP_ENABLED` | always (default `false`) | When `true`, daemon registers a `tracing-opentelemetry` layer on top of the JSON-stdout layer. |
| `FORGE_OTLP_ENDPOINT` | when `FORGE_OTLP_ENABLED=true` | OTLP/HTTP receiver URL (typical: `http://localhost:4318`). If unset/empty, the OTLP layer is **silently disabled** even when the flag is `true` — see Troubleshooting §3. |
| `FORGE_OTLP_SERVICE_NAME` | optional | Resource attribute `service.name`; defaults to `forge-daemon`. |

Source: `crates/daemon/src/main.rs:91-160`.

## Local Jaeger setup

```bash
# 1. Start Jaeger all-in-one (UI on :16686, OTLP/HTTP on :4318)
docker run --rm -d --name jaeger \
    -p 16686:16686 \
    -p 4318:4318 \
    jaegertracing/all-in-one:latest

# 2. Configure daemon to export
export FORGE_OTLP_ENABLED=true
export FORGE_OTLP_ENDPOINT=http://localhost:4318
export FORGE_OTLP_SERVICE_NAME=forge-daemon-dev

# 3. Restart daemon to pick up env
forge-next service restart

# 4. Generate traffic — recall + a consolidator pass
forge-next recall "test query" --limit 5
forge-next admin consolidate-now
```

## Expected span tree shape

Open Jaeger UI at `http://localhost:16686`, select `service.name = forge-daemon-dev`. Expected views:

### Recall path

```
recall (root span)
├── recall_bm25_project
├── load_facets
├── load_active_protocols
└── (filter / scoring)
```

### Consolidator pass

```
consolidate_pass (root span; run_id ULID set as span attribute)
├── phase_1_dedup_memories
├── phase_2_semantic_dedup
├── phase_3_link_memories
├── ... (23 phase spans total — one per consolidation phase)
└── phase_22_apply_quality_pressure
```

Each phase span carries a `phase_name` attribute and a `phase_outcome`
attribute (one of: `succeeded`, `partial`, `failed`).

## Trace-query examples

In Jaeger UI:

1. **Find slow consolidator passes:** filter by `service = forge-daemon-dev`,
   operation = `consolidate_pass`, min-duration = `1s`.
2. **Find phase failures:** filter by `phase_outcome = failed`. Confirms
   the `forge_phase_persistence_errors_total` counter has matching
   trace evidence.
3. **Per-run drill-down:** filter by `run_id = <ULID>` from a
   `consolidate_pass_completed` event in the daemon log.

## Troubleshooting

### 1. `FORGE_OTLP_ENABLED=true` but no traces appear

Check that `FORGE_OTLP_ENDPOINT` is **set and non-empty**. The daemon
silently disables the OTLP layer if the endpoint is empty
(`crates/daemon/src/main.rs:158` — `if otlp_enabled && !otlp_endpoint.is_empty()`).
A future cleanup should warn-on-empty; tracked as backlog item.

### 2. Jaeger receives spans but service name is wrong

Set `FORGE_OTLP_SERVICE_NAME` explicitly. Default is `forge-daemon`; if
multiple daemons share a Jaeger instance, distinguish them via this
variable.

### 3. Spans appear but lack `phase_name` attributes

This is a regression in the consolidator instrumentation
(`crates/daemon/src/consolidator/mod.rs`). Verify with
`bash scripts/ci/check_spans.sh` — the static check ensures every
phase has `info_span!(...)` + `record(phase_outcome)` pairs.

### 4. Daemon CPU spikes after enabling OTLP

The OTLP exporter batches by default. If you've configured
`tracing-opentelemetry` to use a synchronous exporter, switch to the
default `BatchSpanProcessor`. The T10 latency calibration (P3-2 W3)
documented a ≤ 1.20× overhead with the batch processor.

### 5. trace_id missing from KPI events

`kpi_events.metadata_json.trace_id` should be a hex OTLP trace_id when
the layer is active (per `events-namespace.md` line 57-58). If it's
`null` despite `FORGE_OTLP_ENABLED=true`, the trace context isn't
propagating into the event emit site — file a bug with a sample
event row.

## Backlog (deferred from W8 scope)

* **Automated CI validation.** A CI job that spins up Jaeger, drives
  daemon traffic, then queries Jaeger's `/api/traces` endpoint and
  asserts span counts > N. Out of scope for v0.6.0; tracked in the
  cumulative deferred backlog.
* **Programmatic span-shape assertions.** Today the expected span tree
  is documented prose; should become an executable test fixture.

## Related

* Plan: [`../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md`](../superpowers/plans/2026-04-26-v0.6.0-polish-wave.md) (P3-3.5 W8)
* Daemon source: [`crates/daemon/src/main.rs:91-160`](../../crates/daemon/src/main.rs)
* OTLP latency budget: T10 calibration result doc.
* Span CI guard: `scripts/ci/check_spans.sh`
