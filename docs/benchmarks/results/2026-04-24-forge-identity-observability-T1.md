# 2A-4d.1 T11 — Live-Daemon Jaeger Dogfood

- **Date:** 2026-04-24
- **Tier:** 2A-4d.1 Instrumentation (OTLP + spans + kpi_events + Prometheus)
- **Commit:** `f67122b` (current HEAD after T10 lands)
- **Host:** Linux x86_64, release profile, docker-local Jaeger all-in-one

## Goal

End-to-end verification that the 23 phase spans the daemon emits actually reach an OTLP
collector and that every phase also persists a `kpi_events` row with the documented v1
metadata schema. This is the "does the wiring work?" dogfood; the harness-level latency
budget is in the T10 baseline doc.

## Setup

```bash
# 1. Start Jaeger all-in-one with OTLP/gRPC on :4317 and UI on :16686.
docker run -d --rm --name forge-jaeger \
  -p 16686:16686 -p 4317:4317 -p 4318:4318 \
  -e COLLECTOR_OTLP_ENABLED=true \
  jaegertracing/all-in-one:latest

# 2. Build release binaries.
cargo build --release -p forge-daemon -p forge-cli

# 3. Launch an isolated daemon pointed at Jaeger.
#    FORGE_DIR puts state in /tmp so it doesn't collide with the live daemon.
#    ORT must be on LD_LIBRARY_PATH for glibc <2.38 hosts (this one).
export LD_LIBRARY_PATH="$(pwd)/.tools/onnxruntime-linux-x64-1.23.0/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
FORGE_DIR=/tmp/forge-t11 \
FORGE_OTLP_ENABLED=true \
FORGE_OTLP_ENDPOINT=http://127.0.0.1:4317 \
FORGE_OTLP_SERVICE_NAME=forge-daemon-t11 \
RUST_LOG=forge_daemon=info \
./target/release/forge-daemon 2>/tmp/forge-t11/stderr.log &

# 4. Wait for startup consolidation, then trigger one more pass explicitly.
sleep 6
FORGE_DIR=/tmp/forge-t11 ./target/release/forge-next consolidate
```

## What the daemon did

On startup the daemon ran one consolidate pass as part of its background ingestion; the
`forge-next consolidate` command then forced a second pass through the `ForceConsolidate`
handler. That gives us two independent traces to compare.

Daemon stderr (abridged) confirms OTLP init and the span flow through the nested
`consolidate_pass` → `phase_N_*` hierarchy:

```
[daemon] OTLP trace export enabled (endpoint=http://127.0.0.1:4317)
{"level":"INFO","fields":{"message":"forge-daemon starting", ...}}
{"level":"INFO","fields":{"message":"phase_9a: 0 valence-based contradictions", ...},
 "target":"forge_daemon::workers::consolidator",
 "span":{"name":"phase_9_detect_contradictions"},
 "spans":[{"run_id":"01KPZT6C28E0JREYCPSQ29WKHE","name":"consolidate_pass"},
          {"name":"phase_9_detect_contradictions"}]}
[daemon] startup consolidation: dedup=0, semantic=0, linked=0, faded=0, promoted=0, ...
```

## Jaeger API verification

`GET /api/services` returns `forge-daemon-t11` and `jaeger-all-in-one`. Pulling traces
for the service:

```bash
curl -sS "http://localhost:16686/api/traces?service=forge-daemon-t11&limit=3"
```

Result (summary):

| Trace # | Trace ID                              | Span count | Parent span      | Distinct `phase_N_*` children |
| ------- | ------------------------------------- | ---------: | ---------------- | ----------------------------: |
| 0       | `20600a3bd99f22af2f6793ff07acf514`    | **24**     | `consolidate_pass` | **23** (one of each) |
| 1       | `9106090928806ac7a367540d6a77c0eb`    | **24**     | `consolidate_pass` | **23** (one of each) |

Both traces show the full 23-phase hierarchy with **no duplicates** and **no missing
phases**. The execution order reflected in the span parenting is phases 1 → 17, then
phase_23, then 18 → 22 (matches `PHASE_SPAN_NAMES` in `workers/instrumentation.rs`).

## kpi_events verification

```sql
-- Row count: 46 == 23 spans × 2 passes (startup + ForceConsolidate).
SELECT COUNT(*) FROM kpi_events WHERE event_type = 'phase_completed';
-- 46

-- Distinct phase names (23 total, matching PHASE_SPAN_NAMES).
SELECT DISTINCT json_extract(metadata_json, '$.phase_name') AS phase
FROM kpi_events WHERE event_type = 'phase_completed'
ORDER BY phase;
-- phase_10_decay_activation … phase_9_detect_contradictions (23 names)
```

Sample row:

```json
{
  "correlation_id": "01KPZT6C28E0JREYCPSQ29WKHE",
  "error_count": 0,
  "extra": {},
  "metadata_schema_version": 1,
  "output_count": 0,
  "phase_name": "phase_1_dedup_memories",
  "run_id": "01KPZT6C28E0JREYCPSQ29WKHE",
  "trace_id": null
}
```

All v1 contract fields present. `metadata_schema_version: 1` pinned. `correlation_id`
aliases `run_id` and `trace_id` is null — both documented as deferred to the Tier 2 /
`2A-4d.1.1` follow-up (see plan appendix).

## What this verified

- [x] OTLP/gRPC wiring from daemon → Jaeger works end to end.
- [x] `consolidate_pass` → 23 `phase_N_*` children parent/child relationship is correct.
- [x] Every phase emits its span exactly once per pass; CI span-integrity guard matches.
- [x] `kpi_events` has `event_type='phase_completed'` rows; row count = 23 per pass.
- [x] Every row carries `metadata_schema_version: 1` and all 8 v1 fields.
- [x] Phase 23 (Behavioral Skill Inference, 2A-4c2) appears between phases 17 and 18 per
      spec — not phase 18 or phase 24.
- [x] Startup consolidation and `ForceConsolidate` both exercise the same path (including
      the `metrics: Option<Arc<ForgeMetrics>>` field added in T9.1).

## Known gaps (by design, deferred)

- `trace_id: null` in every `kpi_events` row even though OTLP is enabled. The OpenTelemetry
  trace id must be pulled from the current span via
  `tracing_opentelemetry::OpenTelemetrySpanExt::context().span().span_context().trace_id()`
  and threaded into `PhaseOutcome::trace_id`. Deferred as Claude HIGH-1 in the plan.
- `correlation_id == run_id` 100% of the time. Same deferral (`2A-4d.1.1`).
- Variant B observability overhead in `kpi_events`/Prometheus: documented in the T10
  latency baseline at
  `docs/benchmarks/baselines/2026-04-24-consolidation-latency.md`.

## Teardown

```bash
kill $(cat /tmp/forge-t11/forge.pid)
docker stop forge-jaeger
rm -rf /tmp/forge-t11
```

## Reproduction note

This dogfood was run on Ubuntu 22.04 (glibc <2.38), which requires the Microsoft
`manylinux_2_17` ONNX Runtime bundled in `.tools/`. On macOS or glibc ≥2.38 Linux hosts,
skip the `LD_LIBRARY_PATH` export.

Jaeger UI for manual inspection: <http://localhost:16686/search?service=forge-daemon-t11>.
