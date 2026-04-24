# `kpi_events` namespace register

`kpi_events` (`crates/daemon/src/db/schema.rs:255-266`) is a shared
observation log. This page is the coordination surface for every writer.

## Table shape (reference)

```sql
CREATE TABLE IF NOT EXISTS kpi_events (
    id TEXT PRIMARY KEY,
    timestamp INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    project TEXT,
    latency_ms INTEGER,
    result_count INTEGER,
    success INTEGER NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_kpi_events_timestamp ON kpi_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_kpi_events_type      ON kpi_events(event_type);
```

`metadata_json` is a free-form string at the DB level — each
`event_type` defines its own JSON schema. **Every `metadata_json`
payload MUST include a `metadata_schema_version` integer so readers can
version-validate.**

## Claimed `event_type` values

| `event_type` | Owner | Claimed | Contract version | Metadata schema |
|--------------|-------|---------|------------------|-----------------|
| `phase_completed` | consolidator (Tier 1 of 2A-4d) | 2026-04-24 | `metadata_schema_version: 1` | see below |

### `phase_completed` — metadata_schema_version: 1

Emitted by `crates/daemon/src/workers/instrumentation.rs::record`
after every consolidator phase under `run_all_phases`. One row per
phase per consolidate_pass.

```jsonc
{
  "metadata_schema_version": 1,

  // Span metadata
  "phase_name": "phase_23_infer_skills_from_behavior", // see PHASE_SPAN_NAMES
  "run_id": "01HXYZ...",                                // ULID for the outer consolidate_pass
  "correlation_id": "01HXYZ...",                        // same as run_id today; kept separate for future use

  // Populated only when OTLP is enabled; hex OTLP trace id (32 chars)
  "trace_id": "abc123..." | null,

  // Phase accounting
  "output_count": 1,                                    // phase-specific, see PhaseOutputProjection §3.1a
  "error_count": 0,                                     // sum of recoverable failures observed inside the phase
                                                        // (e.g. per-row insert/rollback); 0 means success.
                                                        // NOTE: 11 helpers currently `.unwrap_or(0)` on inner
                                                        // errors — see 2A-4d.1.1 follow-up to make these honest.
                                                        // **Instrumentation-layer failures** (kpi_events insert
                                                        // blew up, ULID collision absorbed) do NOT contribute
                                                        // here — they land in
                                                        // `forge_phase_persistence_errors_total{kind}` only,
                                                        // so phase-level SLOs stay untainted by persistence
                                                        // hiccups.

  // Phase-specific payload
  "extra": { /* keys vary by phase; stable per phase */ }
}
```

Column mappings:

- `latency_ms` = `duration_ms` of the phase.
- `result_count` = `output_count` (fast roll-up without JSON parsing).
- `success` = `1` if `error_count == 0`, else `0`.
- `project` = `NULL` (phases are project-agnostic; per-project counts
  come from per-table gauges).
- `id` = `phase-<ULID>` (prefix identifies provenance without a join).

### Version bump protocol

Any change to the `metadata_json` contract that a reader depends on:

1. Bump `metadata_schema_version` to the next integer.
2. Update this register with the new contract section.
3. Readers must branch on `metadata_schema_version` and handle both
   the old and new versions for at least one release cycle.
4. Dropping a version is a breaking change; announce in HANDOFF
   §Lifted constraints.

## Claiming a new `event_type`

Open a PR that:

1. Adds a row to the table above.
2. Adds a `### <event_type> — metadata_schema_version: 1` section
   below, documenting the full JSON contract.
3. Includes the writer (`INSERT INTO kpi_events`) in the same PR so
   the table and writer land atomically.

Do NOT write a new `event_type` without registering it first; the
register is the coordination mechanism that prevents two writers from
colliding on the same namespace with incompatible payloads.
